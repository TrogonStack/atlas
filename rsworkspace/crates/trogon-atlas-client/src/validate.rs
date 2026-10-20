//! Model-level validation (`ValidateEventModel`) of a stored EventModel.
//! Per-entity checks run on every write; the rules that need the whole
//! model in view (storyboard entries, projection order, field provenance)
//! run only here.

use anyhow::{Context as _, Result};
use trogon_atlas_proto as pb;

use crate::{
    client::Client,
    view::{self, EntityAddress},
};

/// Parse `<namespace>/<model>[@version]`; `version: None` means latest.
pub fn parse_model_address(s: &str) -> Result<EntityAddress> {
    if s.matches('/').count() != 1 {
        crate::invalid_input!("expected <namespace>/<model>[@version], got {s:?}");
    }
    view::parse_address(&format!("event_model/{s}"))
}

/// Validate the addressed EventModel and return the stored id it resolved
/// to along with the findings. Both reads carry the client's branch header,
/// so a branch-scoped client validates the branch's view of the model.
pub async fn validate_event_model(
    client: &mut Client,
    address: &EntityAddress,
) -> Result<(pb::Id, Vec<pb::ValidationIssue>)> {
    // The RPC needs a concrete version, so resolve "latest" first.
    let id = if let Some(version) = address.version {
        pb::Id {
            namespace: address.namespace.clone(),
            slug: address.slug.clone(),
            version,
        }
    } else {
        let entity = view::fetch_entity(client, address).await?;
        trogon_atlas_core::entity_id(&entity)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("stored event model has no id"))?
    };
    let resp = client
        .validate_event_model(pb::ValidateEventModelRequest {
            event_model: None,
            event_model_id: Some(id.clone()),
        })
        .await
        .context("ValidateEventModel failed")?
        .into_inner();
    Ok((id, resp.issues))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_namespace_model_and_optional_version() {
        let latest = parse_model_address("shop/checkout").unwrap();
        assert_eq!(latest.kind, pb::EntityKind::EventModel);
        assert_eq!(
            (
                latest.namespace.as_str(),
                latest.slug.as_str(),
                latest.version
            ),
            ("shop", "checkout", None)
        );
        let pinned = parse_model_address("shop/checkout@3").unwrap();
        assert_eq!(pinned.version, Some(3));
    }

    #[test]
    fn rejects_addresses_that_are_not_namespace_slash_model() {
        for bad in [
            "checkout",
            "event_model/shop/checkout",
            "shop/",
            "shop/checkout@0",
        ] {
            assert!(parse_model_address(bad).is_err(), "{bad} should not parse");
        }
    }
}
