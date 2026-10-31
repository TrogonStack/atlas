use prost::Message;
use trogon_atlas_proto::{entity, Entity};

use crate::connections::{self, ConnectionError};

pub const MAX_HOSTINGS: usize = 1_024;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;

pub fn validate_values(entity: &Entity) -> Result<(), ConnectionError> {
    let Some(entity::Kind::Component(component)) = &entity.kind else {
        return Ok(());
    };
    let reject = |message: &str| ConnectionError::Catalog(message.into());
    if component.ui_hostings.len() > MAX_HOSTINGS {
        return Err(reject(&format!(
            "UI hosting owner exceeds limits: at most {MAX_HOSTINGS} hostings"
        )));
    }
    if entity.encoded_len() > MAX_ENCODED_BYTES {
        return Err(reject(&format!(
            "UI hosting owner exceeds limits: at most {MAX_ENCODED_BYTES} encoded bytes"
        )));
    }
    for hosting in &component.ui_hostings {
        if let Some(id) = &hosting.connection_id {
            id.value.parse::<connections::ConnectionId>()?;
        }
        let id = hosting
            .ui
            .as_ref()
            .and_then(|ui| ui.id.as_ref())
            .ok_or_else(|| reject("UI hosting requires an exact UI"))?;
        if id.version == 0
            || !crate::is_safe_id_component(&id.namespace)
            || !crate::is_safe_id_component(&id.slug)
        {
            return Err(reject("UI hosting requires an exact UI"));
        }
        crate::decision_document::validate_catalog_title(hosting.label.as_ref())
            .map_err(|error| reject(&error.to_string()))?;
        if let Some(doc) = &hosting.doc {
            crate::decision_document::validate_catalog_text(Some(doc))
                .map_err(|error| reject(&error.to_string()))?;
        }
    }
    connections::validate_connection_ids(entity)
}

pub(crate) fn outbound(
    values: &[trogon_atlas_proto::UiHosting],
    out: &mut Vec<crate::refs::OutboundRef>,
) {
    for (index, value) in values.iter().enumerate() {
        if let Some(id) = value.ui.as_ref().and_then(|ui| ui.id.as_ref()) {
            out.push(crate::refs::OutboundRef {
                to_kind: Some(trogon_atlas_proto::EntityKind::Ui),
                to_id: id.clone(),
                from_field: format!("ui_hostings[{index}].ui"),
            });
        }
    }
}

pub(crate) fn retarget(
    values: &mut [trogon_atlas_proto::UiHosting],
    kind: trogon_atlas_proto::EntityKind,
    from: &trogon_atlas_proto::Id,
    to: &trogon_atlas_proto::Id,
    rewritten: &mut Vec<String>,
) {
    if kind != trogon_atlas_proto::EntityKind::Ui || from == to {
        return;
    }
    for (index, value) in values.iter_mut().enumerate() {
        if let Some(ui) = value.ui.as_mut() {
            if ui.id.as_ref() == Some(from) {
                ui.id = Some(to.clone());
                value.connection_id = None;
                rewritten.push(format!("ui_hostings[{index}].ui"));
            }
        }
    }
}
