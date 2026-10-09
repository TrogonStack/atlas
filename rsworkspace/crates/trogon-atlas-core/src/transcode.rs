//! One descriptor pool, one JSON transcode implementation.
//!
//! Every surface that turns proto messages into JSON or back (the CLI's
//! manifests, the MCP adapter's tool payloads, semantic equality) must go
//! through this module. The pool decodes once from the embedded
//! `FILE_DESCRIPTOR_SET`; keeping the primitive here (the lowest layer
//! above the generated bindings) is what prevents each surface from
//! growing its own copy that drifts.

use std::sync::OnceLock;

use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor};
use trogon_atlas_proto::{Entity, FILE_DESCRIPTOR_SET};

const ENTITY_MESSAGE: &str = "trogonatlas.eventmodel.v1alpha1.Entity";

#[derive(Debug, thiserror::Error)]
pub enum TranscodeError {
    #[error("decoding embedded file descriptor set: {0}")]
    Pool(String),
    #[error("layering types onto the descriptor pool: {0}")]
    Layer(String),
    #[error("descriptor for {0} not found")]
    UnknownMessage(String),
    #[error("parsing JSON as {name}: {source}")]
    FromJson {
        name: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("trailing JSON content after {name}: {source}")]
    TrailingJson {
        name: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("re-decoding transcoded {name}: {source}")]
    Redecode {
        name: String,
        #[source]
        source: prost::DecodeError,
    },
    #[error("decoding {name} for JSON rendering: {source}")]
    Decode {
        name: String,
        #[source]
        source: prost::DecodeError,
    },
    #[error("serializing proto JSON: {0}")]
    ToJson(#[source] serde_json::Error),
}

/// The global descriptor pool, decoded once.
pub fn pool() -> Result<&'static DescriptorPool, TranscodeError> {
    static POOL: OnceLock<Result<DescriptorPool, String>> = OnceLock::new();
    POOL.get_or_init(|| DescriptorPool::decode(FILE_DESCRIPTOR_SET).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| TranscodeError::Pool(e.clone()))
}

/// The messages a transcode can name: the built-in types, plus whatever
/// tenant types were layered on top. A `google.protobuf.Any` resolves its
/// `@type` against the same set, so a tenant payload only round-trips through
/// JSON once its library is layered in.
#[derive(Debug, Clone)]
pub struct TranscodePool {
    pool: DescriptorPool,
}

impl TranscodePool {
    pub fn builtin() -> Result<Self, TranscodeError> {
        Ok(Self {
            pool: pool()?.clone(),
        })
    }

    /// A copy of this pool that also knows `files`. Files whose name is
    /// already present are kept as they were.
    pub fn with_files(
        &self,
        files: impl IntoIterator<Item = prost_types::FileDescriptorProto>,
    ) -> Result<Self, TranscodeError> {
        let mut pool = self.pool.clone();
        pool.add_file_descriptor_protos(files)
            .map_err(|e| TranscodeError::Layer(e.to_string()))?;
        Ok(Self { pool })
    }

    #[must_use]
    pub fn descriptor_pool(&self) -> &DescriptorPool {
        &self.pool
    }

    pub fn message_descriptor(&self, full_name: &str) -> Result<MessageDescriptor, TranscodeError> {
        self.pool
            .get_message_by_name(full_name)
            .ok_or_else(|| TranscodeError::UnknownMessage(full_name.to_string()))
    }

    pub fn entity_descriptor(&self) -> Result<MessageDescriptor, TranscodeError> {
        self.message_descriptor(ENTITY_MESSAGE)
    }

    /// Transcode a proto3-JSON string into a concrete prost message. Unknown
    /// fields and trailing content are rejected so typos surface as errors.
    pub fn message_from_json<M: Message + Default>(
        &self,
        full_name: &str,
        json: &str,
    ) -> Result<M, TranscodeError> {
        let desc = self.message_descriptor(full_name)?;
        let mut deserializer = serde_json::Deserializer::from_str(json);
        let dynamic = DynamicMessage::deserialize(desc, &mut deserializer).map_err(|source| {
            TranscodeError::FromJson {
                name: full_name.to_string(),
                source,
            }
        })?;
        deserializer
            .end()
            .map_err(|source| TranscodeError::TrailingJson {
                name: full_name.to_string(),
                source,
            })?;
        redecode(full_name, &dynamic)
    }

    /// Same as [`Self::message_from_json`], from an in-memory JSON value.
    pub fn message_from_json_value<M: Message + Default>(
        &self,
        full_name: &str,
        json: &serde_json::Value,
    ) -> Result<M, TranscodeError> {
        let desc = self.message_descriptor(full_name)?;
        let dynamic = DynamicMessage::deserialize(desc, json.clone()).map_err(|source| {
            TranscodeError::FromJson {
                name: full_name.to_string(),
                source,
            }
        })?;
        redecode(full_name, &dynamic)
    }

    /// Render a prost message as pretty proto3 JSON.
    pub fn message_to_json<M: Message>(
        &self,
        full_name: &str,
        msg: &M,
    ) -> Result<String, TranscodeError> {
        serde_json::to_string_pretty(&self.to_dynamic(full_name, msg)?)
            .map_err(TranscodeError::ToJson)
    }

    /// JSON value of any proto message, by fully-qualified name. Object
    /// comparison ignores key order, so nondeterministic proto map encodings
    /// compare equal.
    pub fn message_json_value<M: Message>(
        &self,
        full_name: &str,
        msg: &M,
    ) -> Result<serde_json::Value, TranscodeError> {
        serde_json::to_value(&self.to_dynamic(full_name, msg)?).map_err(TranscodeError::ToJson)
    }

    pub fn entity_to_json(&self, entity: &Entity) -> Result<String, TranscodeError> {
        self.message_to_json(ENTITY_MESSAGE, entity)
    }

    pub fn entity_json_value(&self, entity: &Entity) -> Result<serde_json::Value, TranscodeError> {
        self.message_json_value(ENTITY_MESSAGE, entity)
    }

    fn to_dynamic<M: Message>(
        &self,
        full_name: &str,
        msg: &M,
    ) -> Result<DynamicMessage, TranscodeError> {
        let desc = self.message_descriptor(full_name)?;
        DynamicMessage::decode(desc, msg.encode_to_vec().as_slice()).map_err(|source| {
            TranscodeError::Decode {
                name: full_name.to_string(),
                source,
            }
        })
    }
}

pub fn message_descriptor(full_name: &str) -> Result<MessageDescriptor, TranscodeError> {
    TranscodePool::builtin()?.message_descriptor(full_name)
}

/// The `trogonatlas.eventmodel.v1alpha1.Entity` descriptor.
pub fn entity_descriptor() -> Result<MessageDescriptor, TranscodeError> {
    message_descriptor(ENTITY_MESSAGE)
}

/// [`TranscodePool::message_from_json`] over the built-in types.
pub fn message_from_json<M: Message + Default>(
    full_name: &str,
    json: &str,
) -> Result<M, TranscodeError> {
    TranscodePool::builtin()?.message_from_json(full_name, json)
}

/// [`TranscodePool::message_from_json_value`] over the built-in types.
pub fn message_from_json_value<M: Message + Default>(
    full_name: &str,
    json: &serde_json::Value,
) -> Result<M, TranscodeError> {
    TranscodePool::builtin()?.message_from_json_value(full_name, json)
}

fn redecode<M: Message + Default>(
    full_name: &str,
    dynamic: &DynamicMessage,
) -> Result<M, TranscodeError> {
    M::decode(dynamic.encode_to_vec().as_slice()).map_err(|source| TranscodeError::Redecode {
        name: full_name.to_string(),
        source,
    })
}

/// [`TranscodePool::message_to_json`] over the built-in types.
pub fn message_to_json<M: Message>(full_name: &str, msg: &M) -> Result<String, TranscodeError> {
    TranscodePool::builtin()?.message_to_json(full_name, msg)
}

/// Render an entity as pretty proto3 JSON.
pub fn entity_to_json(entity: &Entity) -> Result<String, TranscodeError> {
    message_to_json(ENTITY_MESSAGE, entity)
}

/// Canonical JSON value of an entity, for semantic (order-independent)
/// equality: `serde_json::Value` object comparison ignores key order, so
/// nondeterministic proto map encodings compare equal.
pub fn entity_json_value(entity: &Entity) -> Result<serde_json::Value, TranscodeError> {
    TranscodePool::builtin()?.entity_json_value(entity)
}

/// [`TranscodePool::message_json_value`] over the built-in types; used to
/// build canonical digests of arbitrary request messages (e.g. operation
/// idempotency keys).
pub fn message_json_value<M: Message>(
    full_name: &str,
    msg: &M,
) -> Result<serde_json::Value, TranscodeError> {
    TranscodePool::builtin()?.message_json_value(full_name, msg)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use trogon_atlas_proto as pb;

    use super::*;

    #[test]
    fn round_trips_an_entity_through_json() {
        let entity: pb::Entity = message_from_json(
            "trogonatlas.eventmodel.v1alpha1.Entity",
            r#"{"event": {"id": {"namespace": "ns", "slug": "a", "version": "1"}, "title": "A"}}"#,
        )
        .unwrap();
        let json = entity_to_json(&entity).unwrap();
        assert!(json.contains("\"title\": \"A\""), "{json}");
        let back: pb::Entity =
            message_from_json("trogonatlas.eventmodel.v1alpha1.Entity", &json).unwrap();
        assert_eq!(entity, back);
    }

    #[test]
    fn unknown_fields_and_trailing_content_are_rejected() {
        let err = message_from_json::<pb::Entity>(
            "trogonatlas.eventmodel.v1alpha1.Entity",
            r#"{"event": {"titel": "typo"}}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("parsing JSON"), "{err}");

        let err = message_from_json::<pb::Entity>(
            "trogonatlas.eventmodel.v1alpha1.Entity",
            r"{} trailing",
        )
        .unwrap_err();
        assert!(err.to_string().contains("trailing"), "{err}");
    }

    #[test]
    fn unknown_message_name_is_an_error() {
        let err = message_descriptor("trogonatlas.eventmodel.v1alpha1.NoSuchThing").unwrap_err();
        assert!(err.to_string().contains("NoSuchThing"), "{err}");
    }

    fn money_file() -> prost_types::FileDescriptorProto {
        use prost_types::{field_descriptor_proto::Type, DescriptorProto, FieldDescriptorProto};
        prost_types::FileDescriptorProto {
            name: Some("acme/money/v1/money.proto".into()),
            package: Some("acme.money.v1".into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("Money".into()),
                field: vec![FieldDescriptorProto {
                    name: Some("cents".into()),
                    json_name: Some("cents".into()),
                    number: Some(1),
                    r#type: Some(Type::Int64 as i32),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    const TENANT_EVENT: &str = r#"{"event": {"id": {"namespace": "ns", "slug": "paid", "version": "1"}, "schema": {"@type": "type.googleapis.com/acme.money.v1.Money", "cents": "250"}}}"#;

    #[test]
    fn tenant_payloads_need_their_types_layered_in() {
        let builtin = TranscodePool::builtin().unwrap();
        let err = builtin
            .message_from_json::<pb::Entity>(ENTITY_MESSAGE, TENANT_EVENT)
            .unwrap_err();
        assert!(err.to_string().contains("parsing JSON"), "{err}");

        let tenant = builtin.with_files([money_file()]).unwrap();
        let entity: pb::Entity = tenant
            .message_from_json(ENTITY_MESSAGE, TENANT_EVENT)
            .unwrap();
        let Some(pb::entity::Kind::Event(event)) = &entity.kind else {
            panic!("expected an event");
        };
        let schema = event.schema.as_ref().unwrap();
        assert_eq!(schema.type_url, "type.googleapis.com/acme.money.v1.Money");
        assert_ne!(schema.value, Vec::<u8>::new());

        let json = tenant.entity_json_value(&entity).unwrap();
        assert_eq!(json["event"]["schema"]["cents"], "250");
        assert!(builtin.entity_json_value(&entity).is_err());
    }
}
