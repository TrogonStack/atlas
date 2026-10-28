use std::collections::HashSet;

use prost::Message;
use prost_reflect::{DynamicMessage, ReflectMessage, Value};
use trogon_atlas_proto::Entity;

#[derive(Debug, thiserror::Error)]
#[error("invalid metadata at {0}")]
pub struct MetadataError(pub String);

pub const AGENT: &str = "trogonatlas.eventmodel.v1alpha1.AgentAnnotation";
pub const LIFECYCLE: &str = "trogonatlas.annotation.v1alpha1.LifecycleAnnotation";
pub const BADGES: &str = "trogonatlas.annotation.v1alpha1.BadgesAnnotation";
pub const GROUPS: &str = "trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation";

pub fn known_type(url: &str) -> Option<&str> {
    let name = url.rsplit('/').next()?;
    [AGENT, LIFECYCLE, BADGES, GROUPS].contains(&name).then_some(name)
}

pub fn validate(entity: &Entity) -> Result<(), MetadataError> {
    let descriptor = crate::transcode::message_descriptor("trogonatlas.eventmodel.v1alpha1.Entity")
        .map_err(|_| MetadataError("entity".into()))?;
    let message = DynamicMessage::decode(descriptor, entity.encode_to_vec().as_slice())
        .map_err(|_| MetadataError("entity".into()))?;
    validate_tree(&message, "", 0)
}

fn validate_tree(message: &DynamicMessage, path: &str, depth: usize) -> Result<(), MetadataError> {
    let mut seen = HashSet::new();
    for field in message.descriptor().fields() {
        let value = message.get_field(&field);
        let children = match value.as_ref() {
            Value::Message(child) => vec![child],
            Value::List(values) => values.iter().filter_map(Value::as_message).collect(),
            _ => vec![],
        };
        for (index, child) in children.into_iter().enumerate() {
            let locator = format!("{path}.{}[{index}]", field.name());
            if child.descriptor().full_name() == "google.protobuf.Any" {
                let any = prost_types::Any::decode(child.encode_to_vec().as_slice())
                    .map_err(|_| MetadataError(locator.clone()))?;
                if let Some(name) = known_type(&any.type_url) {
                    if depth != 1 || field.name() != "metadata" || !seen.insert(name.to_owned())
                        || (name == AGENT && message.descriptor().name() != "Component") {
                        return Err(MetadataError(locator));
                    }
                    let descriptor = crate::transcode::message_descriptor(name)
                        .map_err(|_| MetadataError(locator.clone()))?;
                    let payload = DynamicMessage::decode(descriptor, any.value.as_slice())
                        .map_err(|_| MetadataError(locator.clone()))?;
                    if name == LIFECYCLE {
                        let status = payload.get_field_by_name("status").ok_or_else(|| MetadataError(locator.clone()))?;
                        let status = status.as_str().unwrap_or_default().trim().to_lowercase();
                        if payload.has_field_by_name("deprecated_on") {
                            if status != "deprecated" { return Err(MetadataError(locator)); }
                            let date = payload.get_field_by_name("deprecated_on").ok_or_else(|| MetadataError(locator.clone()))?;
                            let date = date.as_message().and_then(|m| m.get_field_by_name("value")).ok_or_else(|| MetadataError(locator.clone()))?;
                            date.as_str().unwrap_or_default().parse::<crate::authored_date::AuthoredDate>()
                                .map_err(|_| MetadataError(locator.clone()))?;
                        }
                    }
                }
            } else { validate_tree(child, &locator, depth + 1)?; }
        }
    }
    Ok(())
}
