//! The field list behind an entity's `schema` Any (Decision #33).
//!
//! `schema` holds either a `Schema` message or a message type whose
//! descriptor the server can resolve. Every surface that needs the
//! field names of an Event, Command or ReadModel reads them here so the
//! two forms never drift apart.

use prost::Message;
use prost_reflect::{Cardinality, FieldDescriptor, Kind, MessageDescriptor};
use prost_types::Any;
use trogon_atlas_proto::{entity, field_type, Entity, FieldSpec, FieldType, Schema};

use crate::transcode;

pub const SCHEMA_MESSAGE: &str = "trogonatlas.eventmodel.v1alpha1.Schema";

const TYPE_URL_PREFIX: &str = "type.googleapis.com/";
const TIMESTAMP_MESSAGE: &str = "google.protobuf.Timestamp";
const MAX_MESSAGE_DEPTH: usize = 16;

/// The message name an Any's `type_url` points at.
pub fn type_name(any: &Any) -> &str {
    any.type_url.rsplit('/').next().unwrap_or_default()
}

pub fn is_schema(any: &Any) -> bool {
    type_name(any) == SCHEMA_MESSAGE
}

/// Pack a `Schema` into the Any an entity's `schema` field carries.
pub fn pack_schema(schema: &Schema) -> Any {
    Any {
        type_url: format!("{TYPE_URL_PREFIX}{SCHEMA_MESSAGE}"),
        value: schema.encode_to_vec(),
    }
}

/// Pack an anonymous `Schema` holding `fields`.
pub fn pack_fields(fields: Vec<FieldSpec>) -> Any {
    pack_schema(&Schema {
        fields,
        ..Default::default()
    })
}

/// The `schema` Any of a field-bearing kind (Event, Command, ReadModel).
pub fn kind_schema(kind: &entity::Kind) -> Option<&Any> {
    match kind {
        entity::Kind::Event(x) => x.schema.as_ref(),
        entity::Kind::Command(x) => x.schema.as_ref(),
        entity::Kind::ReadModel(x) => x.schema.as_ref(),
        _ => None,
    }
}

pub fn entity_schema(entity: &Entity) -> Option<&Any> {
    entity.kind.as_ref().and_then(kind_schema)
}

/// The fields `schema` declares. A `Schema` yields its FieldSpecs; any
/// other message type is resolved through the descriptor pool; a type
/// that cannot be decoded or resolved yields no fields.
pub fn schema_fields(schema: Option<&Any>) -> Vec<FieldSpec> {
    let Some(any) = schema else {
        return Vec::new();
    };
    if is_schema(any) {
        return Schema::decode(any.value.as_slice()).map_or_else(|_| Vec::new(), |s| s.fields);
    }
    transcode::message_descriptor(type_name(any))
        .map_or_else(|_| Vec::new(), |desc| message_fields(&desc, 0))
}

pub fn kind_fields(kind: &entity::Kind) -> Vec<FieldSpec> {
    schema_fields(kind_schema(kind))
}

pub fn entity_fields(entity: &Entity) -> Vec<FieldSpec> {
    schema_fields(entity_schema(entity))
}

fn message_fields(desc: &MessageDescriptor, depth: usize) -> Vec<FieldSpec> {
    desc.fields().map(|f| field_spec(&f, depth)).collect()
}

fn field_spec(field: &FieldDescriptor, depth: usize) -> FieldSpec {
    FieldSpec {
        name: field.name().to_string(),
        r#type: Some(FieldType {
            kind: Some(field_kind(&field.kind(), depth)),
        }),
        repeated: field.is_list() || field.is_map(),
        optional: field.cardinality() == Cardinality::Optional && field.supports_presence(),
        ..Default::default()
    }
}

fn field_kind(kind: &Kind, depth: usize) -> field_type::Kind {
    use field_type::Kind as K;
    match kind {
        Kind::String => K::String(field_type::StringType {}),
        Kind::Bool => K::Bool(field_type::BoolType {}),
        Kind::Int32
        | Kind::Int64
        | Kind::Uint32
        | Kind::Uint64
        | Kind::Sint32
        | Kind::Sint64
        | Kind::Fixed32
        | Kind::Fixed64
        | Kind::Sfixed32
        | Kind::Sfixed64 => K::Int(field_type::IntType {}),
        Kind::Float | Kind::Double => K::Float(field_type::FloatType {}),
        Kind::Bytes => K::Bytes(field_type::BytesType {}),
        Kind::Enum(e) => K::Enumeration(field_type::EnumType {
            values: e.values().map(|v| v.name().to_string()).collect(),
        }),
        Kind::Message(m) if m.full_name() == TIMESTAMP_MESSAGE => {
            K::Timestamp(field_type::TimestampType {})
        }
        Kind::Message(m) => K::Object(field_type::ObjectType {
            fields: if depth >= MAX_MESSAGE_DEPTH {
                Vec::new()
            } else {
                message_fields(m, depth + 1)
            },
        }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use trogon_atlas_proto::{Event, ReadModel};

    use super::*;

    fn named(name: &str) -> FieldSpec {
        FieldSpec {
            name: name.into(),
            ..Default::default()
        }
    }

    fn names(fields: &[FieldSpec]) -> Vec<&str> {
        fields.iter().map(|f| f.name.as_str()).collect()
    }

    #[test]
    fn schema_any_yields_its_fields() {
        let any = pack_fields(vec![named("order_id"), named("amount")]);
        assert_eq!(names(&schema_fields(Some(&any))), ["order_id", "amount"]);
    }

    #[test]
    fn missing_schema_yields_nothing() {
        assert_eq!(schema_fields(None), Vec::new());
    }

    #[test]
    fn resolvable_message_type_yields_descriptor_fields() {
        let any = Any {
            type_url: "type.googleapis.com/trogonatlas.type.v1alpha1.Id".into(),
            value: Vec::new(),
        };
        let fields = schema_fields(Some(&any));
        assert_eq!(names(&fields), ["namespace", "slug", "version"]);
        assert!(matches!(
            fields[2].r#type.as_ref().and_then(|t| t.kind.as_ref()),
            Some(field_type::Kind::Int(_))
        ));
    }

    #[test]
    fn unresolvable_message_type_yields_nothing() {
        let any = Any {
            type_url: "type.googleapis.com/acme.orders.v1.Unknown".into(),
            value: vec![1, 2, 3],
        };
        assert_eq!(schema_fields(Some(&any)), Vec::new());
    }

    #[test]
    fn undecodable_schema_yields_nothing() {
        let any = Any {
            type_url: format!("{TYPE_URL_PREFIX}{SCHEMA_MESSAGE}"),
            value: vec![0xff, 0xff, 0xff],
        };
        assert_eq!(schema_fields(Some(&any)), Vec::new());
    }

    #[test]
    fn kind_fields_reads_each_field_bearing_kind() {
        let event = entity::Kind::Event(Event {
            schema: Some(pack_fields(vec![named("a")])),
            ..Default::default()
        });
        let read_model = entity::Kind::ReadModel(ReadModel {
            schema: Some(pack_fields(vec![named("b")])),
            ..Default::default()
        });
        assert_eq!(names(&kind_fields(&event)), ["a"]);
        assert_eq!(names(&kind_fields(&read_model)), ["b"]);
    }
}
