//! Native Document catalog-context fields: purpose, filename, binary retention,
//! related content roles and the category collection annotation.

use std::collections::HashSet;

use prost::Message;
use prost_reflect::{DynamicMessage, ReflectMessage, Value};
use trogon_atlas_proto::{self as pb, entity, Entity};

use crate::decision_document::{self, CatalogError};

pub const COLLECTION: &str = "trogonatlas.eventmodel.v1alpha1.DocumentCollectionAnnotation";
const COLLECTION_SHORT: &str = "DocumentCollectionAnnotation";

#[must_use]
pub fn is_collection_annotation(type_url: &str) -> bool {
    type_url.contains(COLLECTION)
        || type_url
            .rsplit('/')
            .next()
            .is_some_and(|name| name == COLLECTION_SHORT)
}
pub const MAX_RELATED_CONTENT: usize = 128;
pub const MAX_RETAINED_BYTES: usize = 262_144;

type CatalogResult<T> = std::result::Result<T, CatalogError>;

fn require(valid: bool, message: &str) -> CatalogResult<()> {
    if valid {
        Ok(())
    } else {
        Err(CatalogError(message.into()))
    }
}

pub fn bytes_content_digest(content: &[u8]) -> pb::ContentDigest {
    use sha2::{Digest, Sha256};
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(content) {
        hex.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        hex.push(char::from(b"0123456789abcdef"[usize::from(byte & 15)]));
    }
    pb::ContentDigest { sha256: hex }
}

pub fn validate_retained_media_type(value: &pb::MediaType) -> CatalogResult<()> {
    let media = value.value.as_str();
    require(
        !media.is_empty() && media.len() <= 127 && media.is_ascii() && !media.contains(';'),
        "invalid retained media type",
    )?;
    let Some((ty, sub)) = media.split_once('/') else {
        return Err(CatalogError("invalid retained media type".into()));
    };
    let restricted_name = |name: &str| {
        name.bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"!#$&-^_.+".contains(&b))
    };
    require(
        restricted_name(ty) && restricted_name(sub),
        "invalid retained media type",
    )
}

pub fn validate_logical_filename(value: &pb::LogicalFilename) -> CatalogResult<()> {
    let name = value.value.as_str();
    require(
        !name.trim().is_empty() && name.len() <= 256,
        "logical filename blank or limit exceeded",
    )?;
    require(
        !name.contains('/') && !name.contains('\\'),
        "logical filename must not contain path separators",
    )?;
    require(
        !name.chars().any(char::is_control),
        "logical filename must not contain control characters",
    )?;
    require(name != "." && name != "..", "logical filename is reserved")
}

pub fn validate_purpose(value: pb::DocumentPurpose) -> CatalogResult<()> {
    require(
        matches!(
            value,
            pb::DocumentPurpose::General
                | pb::DocumentPurpose::Example
                | pb::DocumentPurpose::Diagram
                | pb::DocumentPurpose::Attachment
                | pb::DocumentPurpose::Changelog
        ),
        "document purpose required",
    )
}

pub fn purpose_matches_role(purpose: pb::DocumentPurpose, role: pb::RelatedContentRole) -> bool {
    matches!(
        (purpose, role),
        (
            pb::DocumentPurpose::Example,
            pb::RelatedContentRole::Example
        ) | (
            pb::DocumentPurpose::Diagram,
            pb::RelatedContentRole::Diagram
        ) | (
            pb::DocumentPurpose::Attachment,
            pb::RelatedContentRole::Attachment
        ) | (
            pb::DocumentPurpose::Changelog,
            pb::RelatedContentRole::Changelog
        )
    )
}

pub fn validate_related_role(value: pb::RelatedContentRole) -> CatalogResult<()> {
    require(
        matches!(
            value,
            pb::RelatedContentRole::Example
                | pb::RelatedContentRole::Diagram
                | pb::RelatedContentRole::Attachment
                | pb::RelatedContentRole::Changelog
        ),
        "related content role required",
    )
}

pub fn validate_retained_content(body: &pb::RetainedContent) -> CatalogResult<()> {
    let media = body
        .media_type
        .as_ref()
        .ok_or_else(|| CatalogError("retained media type required".into()))?;
    validate_retained_media_type(media)?;
    require(
        !body.content.is_empty() && body.content.len() <= MAX_RETAINED_BYTES,
        "retained binary outside byte bounds",
    )?;
    let supplied = body
        .digest
        .as_ref()
        .ok_or_else(|| CatalogError("retained content digest required".into()))?;
    require(
        supplied.sha256.len() == 64
            && supplied
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid SHA-256 digest",
    )?;
    require(
        supplied.sha256 == bytes_content_digest(&body.content).sha256,
        "retained content digest does not match exact bytes",
    )
}

pub fn validate_collection_annotation(payload: &[u8]) -> CatalogResult<()> {
    let value = pb::DocumentCollectionAnnotation::decode(payload)
        .map_err(|_| CatalogError("document collection annotation".into()))?;
    decision_document::validate_catalog_title(value.category.as_ref())?;
    if value.description.is_some() {
        decision_document::validate_catalog_text(value.description.as_ref())?;
    }
    Ok(())
}

pub fn validate_document_metadata(metadata: &[prost_types::Any]) -> CatalogResult<()> {
    let mut collection = 0usize;
    for any in metadata {
        if is_collection_annotation(&any.type_url) {
            collection += 1;
            validate_collection_annotation(&any.value)?;
        }
    }
    require(collection <= 1, "duplicate document collection annotation")
}

pub fn validate_related_content_list(
    values: &[pb::RelatedContent],
    owner_id: Option<&pb::Id>,
) -> CatalogResult<()> {
    require(
        values.len() <= MAX_RELATED_CONTENT,
        "related content limit exceeded",
    )?;
    let mut seen = HashSet::new();
    for item in values {
        validate_related_role(
            pb::RelatedContentRole::try_from(item.role)
                .map_err(|_| CatalogError("unknown related content role".into()))?,
        )?;
        let selection = item
            .document
            .as_ref()
            .ok_or_else(|| CatalogError("document selection required".into()))?;
        decision_document::validate_document_selection(selection)?;
        if let Some(owner_id) = owner_id {
            require(
                decision_document::document_owner(selection)?.id.as_ref() != Some(owner_id),
                "self related content forbidden",
            )?;
        }
        let role = pb::RelatedContentRole::try_from(item.role)
            .map_err(|_| CatalogError("unknown related content role".into()))?;
        let mut encoded = (role as i32).to_le_bytes().to_vec();
        encoded.extend(selection.encode_to_vec());
        require(
            seen.insert(encoded),
            "duplicate related content role and target",
        )?;
        if item.label.is_some() {
            decision_document::validate_catalog_title(item.label.as_ref())?;
        }
    }
    Ok(())
}

pub fn validate_document_extensions(document: &pb::Document) -> CatalogResult<()> {
    if let Some(purpose) = document.purpose {
        validate_purpose(
            pb::DocumentPurpose::try_from(purpose)
                .map_err(|_| CatalogError("unknown document purpose".into()))?,
        )?;
    }
    if let Some(filename) = &document.filename {
        validate_logical_filename(filename)?;
    }
    validate_document_metadata(&document.metadata)?;
    validate_related_content_list(&document.related_content, document.id.as_ref())?;
    match &document.body {
        Some(pb::document::Body::RetainedContent(body)) => {
            require(
                document.anchors.is_empty(),
                "binary body cannot declare anchors",
            )?;
            validate_retained_content(body)?;
        }
        Some(pb::document::Body::Retained(_) | pb::document::Body::Link(_)) | None => {}
    }
    Ok(())
}

pub fn has_native_extensions(document: &pb::Document) -> bool {
    document.purpose.is_some()
        || document.filename.is_some()
        || !document.related_content.is_empty()
        || matches!(document.body, Some(pb::document::Body::RetainedContent(_)))
        || document
            .metadata
            .iter()
            .any(|any| is_collection_annotation(&any.type_url))
}

pub fn document_has_native_extensions(entity: &Entity) -> CatalogResult<bool> {
    Ok(match &entity.kind {
        Some(entity::Kind::Document(document)) => has_native_extensions(document),
        _ => false,
    })
}

pub fn dynamic_has_native_extensions(document: &DynamicMessage) -> CatalogResult<bool> {
    if document.has_field_by_name("purpose") || document.has_field_by_name("filename") {
        return Ok(true);
    }
    if let Some(field) = document.descriptor().get_field_by_name("related_content") {
        if document
            .get_field(&field)
            .as_list()
            .is_some_and(|items| !items.is_empty())
        {
            return Ok(true);
        }
    }
    if document.has_field_by_name("retained_content") {
        return Ok(true);
    }
    metadata_has_collection(document)
}

pub fn metadata_has_collection(owner: &DynamicMessage) -> CatalogResult<bool> {
    if let Some(value) = owner.get_field_by_name("metadata") {
        if let Some(values) = value.as_list() {
            for message in values.iter().filter_map(Value::as_message) {
                let any = prost_types::Any::decode(message.encode_to_vec().as_slice())
                    .map_err(|_| CatalogError("metadata".into()))?;
                if is_collection_annotation(&any.type_url) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

pub fn resolve_related_purpose_agreement(
    role: pb::RelatedContentRole,
    target: &Entity,
) -> CatalogResult<()> {
    let Some(entity::Kind::Document(document)) = &target.kind else {
        return Ok(());
    };
    let Some(purpose) = document.purpose else {
        return Ok(());
    };
    let purpose = pb::DocumentPurpose::try_from(purpose)
        .map_err(|_| CatalogError("unknown document purpose".into()))?;
    require(
        purpose_matches_role(purpose, role),
        "related content role conflicts with declared document purpose",
    )
}
