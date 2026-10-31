//! Native business declarations on addressable Schema entities.
//!
//! Payload Schema messages stay nominal type bytes. These collections are the
//! first Schema connection IDs, so a present empty ID is rejected here before
//! generic connection normalization treats it as omitted.

use std::{collections::HashSet, str::FromStr};

use prost::Message;
use trogon_atlas_proto::{
    entity, field_type, BusinessIdentifierRole, BusinessPropertyRelationship, Entity, FieldSpec,
    PropertyPath, Schema,
};

use crate::{connections::ConnectionId, decision_document, schema};

pub const MAX_IDENTIFIERS: usize = 1_024;
pub const MAX_RELATIONSHIPS: usize = 1_024;
pub const MAX_SEGMENTS: usize = 32;
pub const MAX_SEGMENT_BYTES: usize = 256;
pub const MAX_SCHEMA_BYTES: usize = 1_048_576;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct BusinessError(pub String);

#[derive(Debug, Default)]
pub struct ReferenceBudget {
    rows: HashSet<String>,
    row_count: usize,
    occurrences: usize,
    records: usize,
}

impl ReferenceBudget {
    pub fn charge_row(&mut self, key: &str) -> Result<(), BusinessError> {
        if !self.rows.insert(key.to_owned()) {
            return Ok(());
        }
        self.row_count += 1;
        if self.row_count > decision_document::MAX_DISCOVERY_ROWS {
            return Err(BusinessError(
                "business reference row budget exceeded".into(),
            ));
        }
        Ok(())
    }

    pub fn charge_occurrences(&mut self, count: usize) -> Result<(), BusinessError> {
        self.occurrences = self
            .occurrences
            .checked_add(count)
            .filter(|count| *count <= decision_document::MAX_EXAMINED_OCCURRENCES)
            .ok_or_else(|| BusinessError("business reference occurrence budget exceeded".into()))?;
        Ok(())
    }

    pub fn charge_record(&mut self) -> Result<(), BusinessError> {
        self.records = self
            .records
            .checked_add(1)
            .filter(|count| *count <= decision_document::MAX_OUTPUT_RECORDS)
            .ok_or_else(|| BusinessError("business reference output budget exceeded".into()))?;
        Ok(())
    }
}

pub fn reject_unassigned(entity: &Entity) -> Result<(), BusinessError> {
    validate_entity(entity)
}

pub fn reject_encoded_size(entity: &Entity) -> Result<(), BusinessError> {
    if let Some(schema) = schema_of(entity) {
        if schema.encoded_len() > MAX_SCHEMA_BYTES {
            return Err(BusinessError(
                "prepared schema exceeds 1 MiB including assigned connection IDs".into(),
            ));
        }
    }
    Ok(())
}

pub fn validate_entity(entity: &Entity) -> Result<(), BusinessError> {
    reject_payload(entity)?;
    if let Some(schema) = schema_of(entity) {
        validate_schema(schema)?;
    }
    Ok(())
}

pub fn reject_source_local_paths(entity: &Entity) -> Result<(), BusinessError> {
    let Some(schema) = schema_of(entity) else {
        return Ok(());
    };
    for identifier in &schema.identifiers {
        resolve_path(
            &schema.fields,
            identifier
                .property
                .as_ref()
                .ok_or_else(|| BusinessError("identifier property is required".into()))?,
        )?;
    }
    for relationship in &schema.property_relationships {
        resolve_path(
            &schema.fields,
            relationship
                .source
                .as_ref()
                .ok_or_else(|| BusinessError("relationship source is required".into()))?,
        )?;
    }
    Ok(())
}

pub fn declares_business(entity: &Entity) -> bool {
    schema_of(entity).is_some_and(|schema| {
        schema.aggregate_root.is_some()
            || !schema.identifiers.is_empty()
            || !schema.property_relationships.is_empty()
    })
}

/// Source title and field updates clone the stored schema and replace only
/// the owned unit. Admission can skip when these three collections are the
/// stored ones, because a dangling `target_property` must not block that
/// unrelated edit. Any other byte, including a clear, is a new declaration.
pub fn business_declarations_match(current: &Entity, incoming: &Entity) -> bool {
    match (schema_of(current), schema_of(incoming)) {
        (Some(current), Some(incoming)) => {
            current.aggregate_root == incoming.aggregate_root
                && current.identifiers == incoming.identifiers
                && current.property_relationships == incoming.property_relationships
        }
        _ => false,
    }
}

pub fn resolve_path<'a>(
    fields: &'a [FieldSpec],
    path: &PropertyPath,
) -> Result<&'a FieldSpec, BusinessError> {
    if path.segments.is_empty() || path.segments.len() > MAX_SEGMENTS {
        return Err(BusinessError(
            "property path must contain 1 to 32 segments".into(),
        ));
    }
    let mut fields = fields;
    for (index, segment) in path.segments.iter().enumerate() {
        if segment.name.is_empty() || segment.name.len() > MAX_SEGMENT_BYTES {
            return Err(BusinessError(
                "property segment name must be 1 to 256 UTF-8 bytes".into(),
            ));
        }
        let matches: Vec<_> = fields
            .iter()
            .filter(|field| field.name == segment.name)
            .collect();
        if matches.len() != 1 {
            return Err(BusinessError(format!(
                "property segment {} is missing or ambiguous",
                segment.name
            )));
        }
        let field = matches[0];
        if field.repeated != segment.repeated {
            return Err(BusinessError(format!(
                "property segment {} repeated flag does not match the field",
                segment.name
            )));
        }
        let last = index + 1 == path.segments.len();
        if last {
            return Ok(field);
        }
        let Some(field_type::Kind::Object(object)) = field
            .r#type
            .as_ref()
            .and_then(|field_type| field_type.kind.as_ref())
        else {
            return Err(BusinessError(format!(
                "property segment {} is not an inline object",
                segment.name
            )));
        };
        fields = &object.fields;
    }
    Err(BusinessError("property path is empty".into()))
}

pub fn relationship_index(from_field: &str) -> Option<usize> {
    let rest = from_field.strip_prefix("property_relationships[")?;
    let (index, suffix) = rest.split_once(']')?;
    if suffix != ".target_entity" {
        return None;
    }
    index.parse().ok()
}

fn reject_payload(entity: &Entity) -> Result<(), BusinessError> {
    let Some(kind) = entity.kind.as_ref() else {
        return Ok(());
    };
    let Some(any) = schema::kind_schema(kind) else {
        return Ok(());
    };
    if !schema::is_schema(any) {
        return Ok(());
    }
    let Ok(payload) = Schema::decode(any.value.as_slice()) else {
        return Ok(());
    };
    if payload.aggregate_root.is_some()
        || !payload.identifiers.is_empty()
        || !payload.property_relationships.is_empty()
    {
        return Err(BusinessError(
            "business declarations are not allowed on a payload schema".into(),
        ));
    }
    Ok(())
}

fn validate_schema(schema: &Schema) -> Result<(), BusinessError> {
    if schema.identifiers.len() > MAX_IDENTIFIERS
        || schema.property_relationships.len() > MAX_RELATIONSHIPS
    {
        return Err(BusinessError(
            "a schema declares at most 1024 identifiers and 1024 relationships".into(),
        ));
    }
    let mut primary = false;
    let mut paths = HashSet::new();
    let mut ids = HashSet::new();
    for identifier in &schema.identifiers {
        remember_id(&mut ids, identifier.connection_id.as_ref())?;
        let property = identifier
            .property
            .as_ref()
            .ok_or_else(|| BusinessError("identifier property is required".into()))?;
        resolve_path(&schema.fields, property)?;
        if !paths.insert(path_key(property)) {
            return Err(BusinessError(
                "identifier property path is declared more than once".into(),
            ));
        }
        match BusinessIdentifierRole::try_from(identifier.role) {
            Ok(BusinessIdentifierRole::Primary) if !primary => primary = true,
            Ok(BusinessIdentifierRole::Primary) => {
                return Err(BusinessError(
                    "a schema declares at most one primary identifier".into(),
                ));
            }
            Ok(BusinessIdentifierRole::Alternate) => {}
            _ => {
                return Err(BusinessError(
                    "identifier role must be primary or alternate".into(),
                ));
            }
        }
        optional_text(identifier.doc.as_ref())?;
    }
    for relationship in &schema.property_relationships {
        validate_relationship(schema, relationship, &mut ids)?;
    }
    Ok(())
}

fn validate_relationship(
    schema: &Schema,
    relationship: &BusinessPropertyRelationship,
    ids: &mut HashSet<String>,
) -> Result<(), BusinessError> {
    remember_id(ids, relationship.connection_id.as_ref())?;
    resolve_path(
        &schema.fields,
        relationship
            .source
            .as_ref()
            .ok_or_else(|| BusinessError("relationship source is required".into()))?,
    )?;
    let target = relationship
        .target_entity
        .as_ref()
        .and_then(|target| target.id.as_ref())
        .ok_or_else(|| BusinessError("relationship target schema is required".into()))?;
    if target.version == 0
        || !crate::is_safe_id_component(&target.namespace)
        || !crate::is_safe_id_component(&target.slug)
    {
        return Err(BusinessError(
            "relationship target schema key is not exact".into(),
        ));
    }
    if let Some(path) = relationship.target_property.as_ref() {
        if path.segments.is_empty() || path.segments.len() > MAX_SEGMENTS {
            return Err(BusinessError(
                "target property path must contain 1 to 32 segments".into(),
            ));
        }
        for segment in &path.segments {
            if segment.name.is_empty() || segment.name.len() > MAX_SEGMENT_BYTES {
                return Err(BusinessError(
                    "target property segment name must be 1 to 256 UTF-8 bytes".into(),
                ));
            }
        }
    }
    if let Some(label) = relationship.label.as_ref() {
        decision_document::validate_catalog_title(Some(label))
            .map_err(|error| BusinessError(error.to_string()))?;
    }
    optional_text(relationship.doc.as_ref())?;
    Ok(())
}

fn optional_text(doc: Option<&trogon_atlas_proto::CatalogText>) -> Result<(), BusinessError> {
    if let Some(doc) = doc {
        decision_document::validate_catalog_text(Some(doc))
            .map_err(|error| BusinessError(error.to_string()))?;
    }
    Ok(())
}

fn remember_id(
    ids: &mut HashSet<String>,
    id: Option<&trogon_atlas_proto::ConnectionId>,
) -> Result<(), BusinessError> {
    let Some(id) = id else {
        return Ok(());
    };
    ConnectionId::from_str(&id.value).map_err(|error| BusinessError(error.to_string()))?;
    if !ids.insert(id.value.clone()) {
        return Err(BusinessError(
            "identifier and relationship connection IDs must be unique on the schema".into(),
        ));
    }
    Ok(())
}

fn path_key(path: &PropertyPath) -> Vec<(String, bool)> {
    path.segments
        .iter()
        .map(|segment| (segment.name.clone(), segment.repeated))
        .collect()
}

fn schema_of(entity: &Entity) -> Option<&Schema> {
    match entity.kind.as_ref() {
        Some(entity::Kind::Schema(schema)) => Some(schema),
        _ => None,
    }
}
