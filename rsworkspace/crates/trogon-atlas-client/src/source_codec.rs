use std::collections::BTreeMap;

use anyhow::{Context, Result};
use prost::Message;
use prost_reflect::{DynamicMessage, ReflectMessage, Value as ProtoValue};
use serde_json::Value;
use trogon_atlas_core::{refs::entity_id, transcode::TranscodePool};
use trogon_atlas_proto as pb;

use crate::{client::Client, failure::invalid_input, tenant_types};

const ENTITY: &str = "trogonatlas.eventmodel.v1alpha1.Entity";

pub struct SourcePreviewDocument {
    request: pb::PreviewSourceRunRequest,
    entities: Vec<Value>,
}

pub fn prepare_preview_request(json: &Value) -> Result<SourcePreviewDocument> {
    let pool = TranscodePool::builtin()?;
    let descriptor =
        pool.message_descriptor("trogonatlas.api.eventmodel.v1alpha1.PreviewSourceRunRequest")?;
    if !crate::source_request::validate_deferred(&pool, &descriptor, json) {
        return Err(invalid_input("invalid source request"));
    }
    let mut skeleton = json.clone();
    let mut entities = Vec::new();
    if let Some(payloads) = skeleton
        .get_mut("nativePayloads")
        .and_then(Value::as_array_mut)
    {
        for payload in payloads {
            let entity = payload
                .get_mut("entity")
                .ok_or_else(|| invalid_input("complete native owner required"))?;
            entities.push(std::mem::replace(entity, serde_json::json!({})));
        }
    }
    Ok(SourcePreviewDocument {
        request: crate::source_integration::parse_request("PreviewSourceRunRequest", &skeleton)?,
        entities,
    })
}

impl SourcePreviewDocument {
    pub async fn load(mut self, client: &mut Client) -> Result<pb::PreviewSourceRunRequest> {
        for (payload, json) in self.request.native_payloads.iter_mut().zip(self.entities) {
            let namespace = json
                .as_object()
                .and_then(|owner| {
                    owner
                        .values()
                        .find_map(|body| body.get("id")?.get("namespace")?.as_str())
                })
                .filter(|namespace| !namespace.is_empty())
                .ok_or_else(|| invalid_input("complete native owner namespace required"))?;
            let mut scope = tenant_types::TypeScope::default();
            scope.add_namespace(namespace);
            let pool = tenant_types::transcode_pool(client, &scope).await?;
            let descriptor = pool.message_descriptor(ENTITY)?;
            if !crate::source_request::validate(&pool, &descriptor, &json) {
                return Err(invalid_input("invalid complete native owner"));
            }
            payload.entity = Some(
                pool.message_from_json_value(ENTITY, &json)
                    .map_err(|_| invalid_input("invalid complete native owner"))?,
            );
        }
        Ok(self.request)
    }
}

pub(crate) async fn response_json(
    client: &mut Client,
    name: &str,
    response: &impl Message,
) -> Result<Value> {
    let builtin = TranscodePool::builtin()?;
    let mut message = DynamicMessage::decode(
        builtin.message_descriptor(name)?,
        response.encode_to_vec().as_slice(),
    )?;
    let mut owners = Vec::new();
    separate_owners(&mut message, "", &mut owners)?;
    let mut json = serde_json::to_value(message)?;
    let mut pools = BTreeMap::new();
    for (path, entity) in owners {
        let value = if let Ok(value) = builtin.entity_json_value(&entity) {
            value
        } else {
            let namespace = &entity_id(&entity)
                .context("source report owner has no exact identity")?
                .namespace;
            if !pools.contains_key(namespace) {
                let mut scope = tenant_types::TypeScope::default();
                scope.add_namespace(namespace);
                pools.insert(
                    namespace.clone(),
                    tenant_types::transcode_pool(client, &scope).await?,
                );
            }
            pools
                .get(namespace)
                .context("source owner descriptor pool unavailable")?
                .entity_json_value(&entity)?
        };
        *json
            .pointer_mut(&path)
            .context("source report owner location unavailable")? = value;
    }
    Ok(json)
}

fn separate_owners(
    message: &mut DynamicMessage,
    path: &str,
    owners: &mut Vec<(String, pb::Entity)>,
) -> Result<()> {
    for (field, value) in message.fields_mut() {
        let path = format!("{path}/{}", field.json_name());
        match value {
            ProtoValue::Message(child) => separate_owner(child, &path, owners)?,
            ProtoValue::List(children) => {
                for (index, child) in children.iter_mut().enumerate() {
                    if let ProtoValue::Message(child) = child {
                        separate_owner(child, &format!("{path}/{index}"), owners)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn separate_owner(
    message: &mut DynamicMessage,
    path: &str,
    owners: &mut Vec<(String, pb::Entity)>,
) -> Result<()> {
    if message.descriptor().full_name() == ENTITY {
        owners.push((
            path.to_owned(),
            pb::Entity::decode(message.encode_to_vec().as_slice())?,
        ));
        *message = DynamicMessage::new(message.descriptor());
        Ok(())
    } else {
        separate_owners(message, path, owners)
    }
}
