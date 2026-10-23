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

/// What `ValidateProject` covers: every model in a namespace, or those a
/// Domain or Project claims. Strategic findings (BoundedContext, Domain,
/// Subdomain, Project rules) come back once on the report with no
/// `event_model_id`, so a scope with no EventModel still reports them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectScope {
    Namespace(String),
    Domain(EntityAddress),
    Project(EntityAddress),
}

impl ProjectScope {
    /// Scope for a Domain, parsed from `<namespace>/<slug>[@version]`.
    pub fn domain(s: &str) -> Result<Self> {
        parse_scope_address(pb::EntityKind::Domain, s).map(Self::Domain)
    }

    /// Scope for a Project, parsed from `<namespace>/<slug>[@version]`.
    pub fn project(s: &str) -> Result<Self> {
        parse_scope_address(pb::EntityKind::Project, s).map(Self::Project)
    }

    /// Scope for a namespace; the name must be a valid id component.
    pub fn namespace(s: &str) -> Result<Self> {
        trogon_atlas_core::validate_id_component(s)
            .map_err(|e| crate::failure::invalid_input(format!("namespace {e}")))?;
        Ok(Self::Namespace(s.to_string()))
    }
}

fn parse_scope_address(kind: pb::EntityKind, s: &str) -> Result<EntityAddress> {
    if s.matches('/').count() != 1 {
        crate::invalid_input!("expected <namespace>/<slug>[@version], got {s:?}");
    }
    let address = view::parse_address(&format!("{}/{s}", pb::canonical::kind_short(kind)))?;
    Ok(address)
}

async fn resolve_id(client: &mut Client, address: &EntityAddress) -> Result<pb::Id> {
    if let Some(version) = address.version {
        return Ok(pb::Id {
            namespace: address.namespace.clone(),
            slug: address.slug.clone(),
            version,
        });
    }
    let entity = view::fetch_entity(client, address).await?;
    trogon_atlas_core::entity_id(&entity)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("stored entity has no id"))
}

/// Validate every model in `scope`. Both the latest-version lookup and the
/// RPC carry the client's branch header, so a branch-scoped client validates
/// the branch's view.
pub async fn validate_project(
    client: &mut Client,
    scope: &ProjectScope,
) -> Result<pb::ValidateProjectResponse> {
    let scope = match scope {
        ProjectScope::Namespace(namespace) => {
            pb::validate_project_request::Scope::Namespace(namespace.clone())
        }
        ProjectScope::Domain(address) => {
            pb::validate_project_request::Scope::DomainId(resolve_id(client, address).await?)
        }
        ProjectScope::Project(address) => {
            pb::validate_project_request::Scope::ProjectId(resolve_id(client, address).await?)
        }
    };
    Ok(client
        .validate_project(pb::ValidateProjectRequest { scope: Some(scope) })
        .await
        .context("ValidateProject failed")?
        .into_inner())
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

    #[test]
    fn project_scopes_parse_namespace_slug_and_optional_version() {
        let ProjectScope::Domain(domain) = ProjectScope::domain("shop/shop").unwrap() else {
            panic!("expected a domain scope");
        };
        assert_eq!(domain.kind, pb::EntityKind::Domain);
        assert_eq!(domain.version, None);
        let ProjectScope::Project(project) = ProjectScope::project("shop/main@2").unwrap() else {
            panic!("expected a project scope");
        };
        assert_eq!(project.kind, pb::EntityKind::Project);
        assert_eq!(
            (
                project.namespace.as_str(),
                project.slug.as_str(),
                project.version
            ),
            ("shop", "main", Some(2))
        );
        assert_eq!(
            ProjectScope::namespace("shop").unwrap(),
            ProjectScope::Namespace("shop".into())
        );
    }

    #[test]
    fn project_scopes_reject_malformed_input() {
        for bad in ["shop", "domain/shop/shop", "shop/", "shop/shop@0"] {
            assert!(ProjectScope::domain(bad).is_err(), "{bad} should not parse");
            assert!(
                ProjectScope::project(bad).is_err(),
                "{bad} should not parse"
            );
        }
        assert!(ProjectScope::namespace("").is_err());
        assert!(ProjectScope::namespace("a/b").is_err());
    }
}
