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
    [AGENT, LIFECYCLE, BADGES, GROUPS]
        .contains(&name)
        .then_some(name)
}

pub fn validate(entity: &Entity) -> Result<(), MetadataError> {
    let descriptor = crate::transcode::message_descriptor("trogonatlas.eventmodel.v1alpha1.Entity")
        .map_err(|_| MetadataError("entity".into()))?;
    let message = DynamicMessage::decode(descriptor, entity.encode_to_vec().as_slice())
        .map_err(|_| MetadataError("entity".into()))?;
    validate_tree(&message, "", 0)?;
    crate::connections::validate_connection_ids(entity).map_err(|e| MetadataError(e.to_string()))
}

fn validate_tree(message: &DynamicMessage, path: &str, depth: usize) -> Result<(), MetadataError> {
    let mut seen = HashSet::new();
    let mut encoded = 0usize;
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
                    if depth != 1
                        || field.name() != "metadata"
                        || !seen.insert(name.to_owned())
                        || (name == AGENT && message.descriptor().name() != "Component")
                    {
                        return Err(MetadataError(locator));
                    }
                    encoded += any.encoded_len();
                    if encoded > 65536 {
                        return Err(MetadataError(locator));
                    }
                    validate_payload(name, &any.value)
                        .map_err(|_| MetadataError(locator.clone()))?;
                    let descriptor = crate::transcode::message_descriptor(name)
                        .map_err(|_| MetadataError(locator.clone()))?;
                    let payload = DynamicMessage::decode(descriptor, any.value.as_slice())
                        .map_err(|_| MetadataError(locator.clone()))?;
                    if name == LIFECYCLE {
                        let status = payload
                            .get_field_by_name("status")
                            .ok_or_else(|| MetadataError(locator.clone()))?;
                        let status = status.as_str().unwrap_or_default().trim().to_lowercase();
                        if payload.has_field_by_name("deprecated_on") {
                            if status != "deprecated" {
                                return Err(MetadataError(locator));
                            }
                            let date = payload
                                .get_field_by_name("deprecated_on")
                                .ok_or_else(|| MetadataError(locator.clone()))?;
                            let date = date
                                .as_message()
                                .and_then(|m| m.get_field_by_name("value"))
                                .ok_or_else(|| MetadataError(locator.clone()))?;
                            date.as_str()
                                .unwrap_or_default()
                                .parse::<crate::authored_date::AuthoredDate>()
                                .map_err(|_| MetadataError(locator.clone()))?;
                        }
                    }
                }
            } else {
                validate_tree(child, &locator, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn text(value: &str, max: usize, multiline: bool) -> Result<(), MetadataError> {
    if value.trim().is_empty()
        || value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\t')))
    {
        return Err(MetadataError("value".into()));
    }
    Ok(())
}
fn icon(value: &str) -> Result<(), MetadataError> {
    if value.is_empty()
        || value.len() > 128
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
    {
        return Err(MetadataError("icon".into()));
    }
    Ok(())
}
fn url(value: &str) -> Result<(), MetadataError> {
    text(value, 2048, false)?;
    let parsed = url::Url::parse(value).map_err(|_| MetadataError("url".into()))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || value.split('/').nth(2).is_some_and(|s| s.contains('@'))
    {
        return Err(MetadataError("url".into()));
    }
    Ok(())
}
pub(crate) fn reject_unknown(message: &DynamicMessage) -> Result<(), MetadataError> {
    if message.unknown_fields().next().is_some() {
        return Err(MetadataError("unknown wire field".into()));
    }
    for field in message.descriptor().fields() {
        match message.get_field(&field).as_ref() {
            Value::Message(child) => reject_unknown(child)?,
            Value::List(values) => {
                for child in values.iter().filter_map(Value::as_message) {
                    reject_unknown(child)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_payload(name: &str, bytes: &[u8]) -> Result<(), MetadataError> {
    use trogon_atlas_proto::{
        AgentAnnotation, BadgesAnnotation, EntityKind, LifecycleAnnotation,
        ResourceGroupsAnnotation,
    };
    let error = || MetadataError("payload".into());
    let descriptor = crate::transcode::message_descriptor(name).map_err(|_| error())?;
    let payload = DynamicMessage::decode(descriptor, bytes).map_err(|_| error())?;
    reject_unknown(&payload)?;
    match name {
        AGENT => {
            let a = AgentAnnotation::decode(bytes).map_err(|_| error())?;
            if let Some(m) = a.model {
                if m.provider.is_none() && m.name.is_none() && m.version.is_none() {
                    return Err(error());
                }
                for v in [
                    m.provider.map(|v| v.value),
                    m.name.map(|v| v.value),
                    m.version.map(|v| v.value),
                ]
                .into_iter()
                .flatten()
                {
                    text(&v, 256, false)?;
                }
            }
            if a.tools.len() > 64 {
                return Err(error());
            }
            let mut seen = HashSet::new();
            for t in a.tools {
                let n = t.name.ok_or_else(error)?.value;
                let k = t.r#type.ok_or_else(error)?.value;
                text(&n, 256, false)?;
                text(&k, 256, false)?;
                if !seen.insert((n, k)) {
                    return Err(error());
                }
                if let Some(v) = t.description {
                    text(&v.value, 4096, true)?;
                }
                if let Some(v) = t.icon {
                    icon(&v.value)?;
                }
                if let Some(v) = t.url {
                    url(&v.value)?;
                }
            }
        }
        LIFECYCLE => {
            let a = LifecycleAnnotation::decode(bytes).map_err(|_| error())?;
            if let Some(d) = a.draft {
                if a.status.trim().to_lowercase() != "draft" {
                    return Err(error());
                }
                if let Some(t) = d.title {
                    text(&t.value, 256, false)?;
                }
                text(&d.message.ok_or_else(error)?.value, 4096, true)?;
            }
        }
        BADGES => {
            let a = BadgesAnnotation::decode(bytes).map_err(|_| error())?;
            if a.badges.len() > 32 {
                return Err(error());
            }
            let mut seen = HashSet::new();
            for b in a.badges {
                if !seen.insert(b.encode_to_vec()) {
                    return Err(error());
                }
                text(&b.content.ok_or_else(error)?.value, 256, false)?;
                for c in [b.background_color, b.text_color] {
                    let c = c.ok_or_else(error)?.value;
                    if c.len() != 7
                        || !c.starts_with('#')
                        || !c.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
                    {
                        return Err(error());
                    }
                }
                if let Some(v) = b.icon {
                    icon(&v.value)?;
                }
                if let Some(v) = b.url {
                    url(&v.value)?;
                }
            }
        }
        GROUPS => {
            let a = ResourceGroupsAnnotation::decode(bytes).map_err(|_| error())?;
            if a.groups.len() > 32 || a.groups.iter().map(|g| g.items.len()).sum::<usize>() > 256 {
                return Err(error());
            }
            for g in a.groups {
                if let Some(t) = g.title {
                    text(&t.value, 256, false)?;
                }
                if g.display_limit
                    .is_some_and(|v| !(1..=500).contains(&v.value))
                {
                    return Err(error());
                }
                for i in g.items {
                    let r = i.target.ok_or_else(error)?;
                    if EntityKind::try_from(r.kind)
                        .ok()
                        .is_none_or(|k| k == EntityKind::Unspecified)
                    {
                        return Err(error());
                    }
                    let id = r.id.ok_or_else(error)?;
                    if id.version == 0 || id.namespace.is_empty() || id.slug.is_empty() {
                        return Err(error());
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn map_groups(
    entity: &mut Entity,
    callback: &mut impl FnMut(&mut trogon_atlas_proto::ResourceGroupsAnnotation, &str),
) -> Result<(), MetadataError> {
    fn walk(
        message: &mut DynamicMessage,
        path: &str,
        callback: &mut impl FnMut(&mut trogon_atlas_proto::ResourceGroupsAnnotation, &str),
    ) -> Result<(), MetadataError> {
        if message.descriptor().full_name() == "google.protobuf.Any" {
            let mut any = prost_types::Any::decode(message.encode_to_vec().as_slice())
                .map_err(|_| MetadataError(path.into()))?;
            if let Some(name) = known_type(&any.type_url) {
                let descriptor = crate::transcode::message_descriptor(name)
                    .map_err(|_| MetadataError(path.into()))?;
                let payload = DynamicMessage::decode(descriptor, any.value.as_slice())
                    .map_err(|_| MetadataError(path.into()))?;
                reject_unknown(&payload)?;
                if name == GROUPS {
                    let mut groups =
                        trogon_atlas_proto::ResourceGroupsAnnotation::decode(any.value.as_slice())
                            .map_err(|_| MetadataError(path.into()))?;
                    callback(&mut groups, path);
                    any.value = groups.encode_to_vec();
                    *message = DynamicMessage::decode(
                        message.descriptor(),
                        any.encode_to_vec().as_slice(),
                    )
                    .map_err(|_| MetadataError(path.into()))?;
                }
            }
            return Ok(());
        }
        let fields: Vec<_> = message
            .descriptor()
            .fields()
            .filter(|f| message.has_field(f))
            .collect();
        for f in fields {
            let path = if path.is_empty() {
                f.name().to_owned()
            } else {
                format!("{path}.{}", f.name())
            };
            match message.get_field_mut(&f) {
                Value::Message(child) => walk(child, &path, callback)?,
                Value::List(values) => {
                    for (i, v) in values.iter_mut().enumerate() {
                        if let Value::Message(child) = v {
                            walk(child, &format!("{path}[{i}]"), callback)?;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    let descriptor = crate::transcode::message_descriptor("trogonatlas.eventmodel.v1alpha1.Entity")
        .map_err(|_| MetadataError("entity".into()))?;
    let mut message = DynamicMessage::decode(descriptor, entity.encode_to_vec().as_slice())
        .map_err(|_| MetadataError("entity".into()))?;
    walk(&mut message, "", callback)?;
    *entity = Entity::decode(message.encode_to_vec().as_slice())
        .map_err(|_| MetadataError("entity".into()))?;
    Ok(())
}

pub fn group_refs(entity: &Entity) -> Result<Vec<crate::refs::OutboundRef>, MetadataError> {
    let mut refs = Vec::new();
    map_groups(&mut entity.clone(), &mut |groups, path| {
        for (g, group) in groups.groups.iter().enumerate() {
            for (i, item) in group.items.iter().enumerate() {
                if let Some(target) = &item.target {
                    if let (Ok(kind), Some(id)) = (
                        trogon_atlas_proto::EntityKind::try_from(target.kind),
                        &target.id,
                    ) {
                        refs.push(crate::refs::OutboundRef {
                            to_kind: Some(kind),
                            to_id: id.clone(),
                            from_field: format!("{path}.groups[{g}].items[{i}].target"),
                        });
                    }
                }
            }
        }
    })?;
    Ok(refs)
}

pub(crate) fn retarget_groups(
    entity: &mut Entity,
    kind: trogon_atlas_proto::EntityKind,
    from: &trogon_atlas_proto::Id,
    to: &trogon_atlas_proto::Id,
) -> Result<Vec<String>, MetadataError> {
    let mut paths = Vec::new();
    map_groups(entity, &mut |groups, path| {
        for (g, group) in groups.groups.iter_mut().enumerate() {
            for (i, item) in group.items.iter_mut().enumerate() {
                if let Some(target) = &mut item.target {
                    if target.kind == kind as i32 && target.id.as_ref() == Some(from) && from != to
                    {
                        target.id = Some(to.clone());
                        item.connection_id = Some(crate::connections::ConnectionId::new().into());
                        paths.push(format!("{path}.groups[{g}].items[{i}].target"));
                    }
                }
            }
        }
    })?;
    Ok(paths)
}

pub fn is_native_addition(any: &prost_types::Any) -> Result<bool, MetadataError> {
    match known_type(&any.type_url) {
        Some(LIFECYCLE) => {
            let value = trogon_atlas_proto::LifecycleAnnotation::decode(any.value.as_slice())
                .map_err(|_| MetadataError("lifecycle".into()))?;
            Ok(value.draft.is_some() || value.deprecated_on.is_some())
        }
        Some(_) => Ok(true),
        None => Ok(false),
    }
}

pub fn has_native_additions(entity: &Entity) -> Result<bool, MetadataError> {
    let descriptor = crate::transcode::message_descriptor("trogonatlas.eventmodel.v1alpha1.Entity")
        .map_err(|_| MetadataError("entity".into()))?;
    let message = DynamicMessage::decode(descriptor, entity.encode_to_vec().as_slice())
        .map_err(|_| MetadataError("entity".into()))?;
    for field in message
        .descriptor()
        .fields()
        .filter(|f| message.has_field(f))
    {
        if let Some(owner) = message.get_field(&field).as_message() {
            if metadata_has_additions(owner)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(crate) fn metadata_has_additions(owner: &DynamicMessage) -> Result<bool, MetadataError> {
    if let Some(value) = owner.get_field_by_name("metadata") {
        if let Some(values) = value.as_list() {
            for message in values.iter().filter_map(Value::as_message) {
                let any = prost_types::Any::decode(message.encode_to_vec().as_slice())
                    .map_err(|_| MetadataError("metadata".into()))?;
                if is_native_addition(&any)? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
