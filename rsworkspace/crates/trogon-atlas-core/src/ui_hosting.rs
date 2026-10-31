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
    if component.ui_hostings.len() > MAX_HOSTINGS || entity.encoded_len() > MAX_ENCODED_BYTES {
        return Err(reject("UI hosting owner exceeds limits"));
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
