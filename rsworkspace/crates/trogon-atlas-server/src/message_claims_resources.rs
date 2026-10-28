use std::collections::HashSet;

use tonic::Status;
use trogon_atlas_core::message_claims as claims;
use trogon_atlas_proto as pb;

use crate::validation::architecture_resources::ArchitectureContext;

pub(crate) fn unavailable() -> Status {
    Status::invalid_argument("selected message claim target is unavailable or invalid")
}

pub(crate) fn changed(previous: Option<&pb::Entity>, proposed: &pb::Entity) -> pb::Entity {
    let old = previous.and_then(claims::declarations);
    let mut view = proposed.clone();
    if let Some(v) = claims::declarations_mut(&mut view) {
        v.routes
            .retain(|r| old.is_none_or(|o| !o.routes.contains(r)));
        v.triggers
            .retain(|r| old.is_none_or(|o| !o.triggers.contains(r)));
        v.field_selections
            .retain(|r| old.is_none_or(|o| !o.field_selections.contains(r)));
        let selected: HashSet<_> = v
            .routes
            .iter()
            .filter_map(|r| r.claim_id.as_ref())
            .chain(v.triggers.iter().filter_map(|r| r.claim_id.as_ref()))
            .chain(
                v.field_selections
                    .iter()
                    .filter_map(|r| r.claim_id.as_ref()),
            )
            .cloned()
            .collect();
        v.claims.retain(|c| {
            c.connection_id
                .as_ref()
                .is_some_and(|id| selected.contains(id))
                || old.is_none_or(|o| !o.claims.contains(c))
        });
    }
    view
}

pub(crate) fn native_targets(
    owner: &pb::Entity,
    context: &ArchitectureContext<'_>,
) -> Result<Vec<(pb::EntityKind, pb::Id)>, Status> {
    let mut targets = Vec::new();
    for s in claims::declarations(owner)
        .into_iter()
        .flat_map(|v| &v.field_selections)
    {
        let reference = s
            .binding
            .as_ref()
            .and_then(|b| b.owner.as_ref())
            .ok_or_else(unavailable)?;
        let spec = context
            .find(
                pb::EntityKind::Specification,
                reference.id.as_ref().ok_or_else(unavailable)?,
            )
            .ok_or_else(unavailable)?;
        if let Some(schema) =
            claims::native_schema_target(owner, s, spec).map_err(|_| unavailable())?
        {
            if s.selectors.iter().any(|s| {
                matches!(
                    s.selector,
                    Some(pb::claim_field_selector::Selector::Path(_))
                )
            }) {
                targets.push((pb::EntityKind::Schema, schema.id.ok_or_else(unavailable)?));
            }
        }
    }
    Ok(targets)
}

pub(crate) fn validate(
    owner: &pb::Entity,
    context: &ArchitectureContext<'_>,
) -> Result<(), Status> {
    claims::validate_values(owner).map_err(|_| unavailable())?;
    let Some(v) = claims::declarations(owner) else {
        return Ok(());
    };
    for m in v
        .claims
        .iter()
        .filter_map(|v| v.message.as_ref())
        .chain(v.triggers.iter().filter_map(|v| v.message.as_ref()))
    {
        let spec = m
            .specification
            .as_ref()
            .and_then(|v| v.id.as_ref())
            .and_then(|id| context.find(pb::EntityKind::Specification, id))
            .ok_or_else(unavailable)?;
        trogon_atlas_core::specification::resolve_member(spec, m).map_err(|_| unavailable())?;
    }
    for r in &v.routes {
        let target = r
            .channel
            .as_ref()
            .and_then(|v| v.id.as_ref())
            .and_then(|id| context.find(pb::EntityKind::Channel, id))
            .ok_or_else(unavailable)?;
        let Some(pb::entity::Kind::Channel(channel)) = &target.kind else {
            return Err(unavailable());
        };
        claims::validate_route_target(r, channel).map_err(|_| unavailable())?;
    }
    for s in &v.field_selections {
        let spec = s
            .binding
            .as_ref()
            .and_then(|b| b.owner.as_ref())
            .and_then(|o| o.id.as_ref())
            .and_then(|id| context.find(pb::EntityKind::Specification, id))
            .ok_or_else(unavailable)?;
        let schema = claims::native_schema_target(owner, s, spec)
            .map_err(|_| unavailable())?
            .and_then(|s| s.id)
            .and_then(|id| context.find(pb::EntityKind::Schema, &id));
        claims::validate_field_selection(owner, s, spec, schema).map_err(|_| unavailable())?;
    }
    Ok(())
}

pub(crate) fn selected(
    subject: &pb::ResponsibilitySubject,
    owner: &pb::Entity,
) -> Option<pb::Entity> {
    let Some(pb::responsibility_subject::Selection::Connection(reference)) = &subject.selection
    else {
        return None;
    };
    let occurrence = trogon_atlas_core::connections::resolve_connection(owner, reference).ok()?;
    if !occurrence.path.contains(".message_claims.") {
        return None;
    }
    let mut view = owner.clone();
    let v = claims::declarations_mut(&mut view)?;
    v.routes.retain(|v| v.connection_id == reference.id);
    v.triggers.retain(|v| v.connection_id == reference.id);
    v.field_selections
        .retain(|v| v.connection_id == reference.id);
    let dependencies: HashSet<_> = v
        .routes
        .iter()
        .filter_map(|v| v.claim_id.as_ref())
        .chain(v.triggers.iter().filter_map(|v| v.claim_id.as_ref()))
        .chain(
            v.field_selections
                .iter()
                .filter_map(|v| v.claim_id.as_ref()),
        )
        .cloned()
        .collect();
    v.claims.retain(|c| {
        c.connection_id == reference.id
            || c.connection_id
                .as_ref()
                .is_some_and(|id| dependencies.contains(id))
    });
    Some(view)
}
pub(crate) fn charge_selections(
    owner: &pb::Entity,
    context: &ArchitectureContext<'_>,
    budget: &mut crate::decision_document_resources::CatalogBudget,
) -> Result<(), Status> {
    let Some(v) = claims::declarations(owner) else {
        return Ok(());
    };
    for m in v
        .claims
        .iter()
        .filter_map(|v| v.message.as_ref())
        .chain(v.triggers.iter().filter_map(|v| v.message.as_ref()))
    {
        if let Some(spec) = m
            .specification
            .as_ref()
            .and_then(|s| s.id.as_ref())
            .and_then(|id| context.find(pb::EntityKind::Specification, id))
        {
            budget.selection(
                &pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Member(m.clone())),
                },
                spec,
            )?;
        }
    }
    for s in &v.field_selections {
        if let Some(b) = &s.binding {
            if let Some(spec) = b
                .owner
                .as_ref()
                .and_then(|o| o.id.as_ref())
                .and_then(|id| context.find(pb::EntityKind::Specification, id))
            {
                budget.selection(
                    &pb::CatalogSubject {
                        selection: Some(pb::catalog_subject::Selection::Connection(b.clone())),
                    },
                    spec,
                )?;
            }
        }
    }
    Ok(())
}
