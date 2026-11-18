use trogon_atlas_proto::{self as pb, entity, Entity, EntityKind, Id};

use crate::{decision_document_refs, refs::OutboundRef};

type Visit<'a> = dyn FnMut(EntityKind, &mut Option<Id>, String, bool) + 'a;

fn walk(entity: &mut Entity, visit: &mut Visit<'_>) {
    let Some(entity::Kind::DesignGraph(graph)) = &mut entity.kind else {
        return;
    };
    for (i, step) in graph.steps.iter_mut().enumerate() {
        let path = format!("steps[{i}].target");
        match step.target.as_mut().and_then(|t| t.selection.as_mut()) {
            Some(pb::topology_step_target::Selection::CatalogMessage(subject)) => {
                decision_document_refs::walk_subject(
                    subject,
                    &format!("{path}.catalog_message"),
                    &mut |k, id, p, pinned| visit(k, id, p, pinned),
                );
            }
            Some(pb::topology_step_target::Selection::Component(r)) => {
                visit(
                    EntityKind::Component,
                    &mut r.id,
                    format!("{path}.component"),
                    false,
                );
            }
            Some(pb::topology_step_target::Selection::Channel(r)) => {
                visit(
                    EntityKind::Channel,
                    &mut r.id,
                    format!("{path}.channel"),
                    false,
                );
            }
            Some(pb::topology_step_target::Selection::DataStore(r)) => {
                visit(
                    EntityKind::DataStore,
                    &mut r.id,
                    format!("{path}.data_store"),
                    false,
                );
            }
            Some(pb::topology_step_target::Selection::DataProduct(r)) => {
                visit(
                    EntityKind::DataProduct,
                    &mut r.id,
                    format!("{path}.data_product"),
                    false,
                );
            }
            Some(pb::topology_step_target::Selection::SystemGroup(r)) => {
                visit(
                    EntityKind::SystemGroup,
                    &mut r.id,
                    format!("{path}.system_group"),
                    false,
                );
            }
            Some(pb::topology_step_target::Selection::Subflow(r)) => {
                visit(
                    EntityKind::DesignGraph,
                    &mut r.id,
                    format!("{path}.subflow"),
                    false,
                );
            }
            _ => {}
        }
    }
    if let Some(layout) = graph.layout_document.as_mut() {
        visit(
            EntityKind::Document,
            &mut layout.id,
            "layout_document".into(),
            false,
        );
    }
    decision_document_refs::walk_subjects(&mut graph.subjects, &mut |k, id, p, pinned| {
        visit(k, id, p, pinned);
    });
}

pub(crate) fn outbound(entity: &Entity, out: &mut Vec<OutboundRef>) {
    walk(&mut entity.clone(), &mut |kind, id, path, _| {
        if let Some(id) = id {
            out.push(OutboundRef {
                to_kind: Some(kind),
                to_id: id.clone(),
                from_field: path,
            });
        }
    });
}

pub(crate) fn retarget(
    entity: &mut Entity,
    kind: EntityKind,
    from: &Id,
    to: &Id,
    rewritten: &mut Vec<String>,
) {
    walk(entity, &mut |target, id, path, pinned| {
        if !pinned && target == kind && id.as_ref() == Some(from) {
            *id = Some(to.clone());
            rewritten.push(path);
        }
    });
}

pub(crate) fn decorate_reference(entity: &Entity, reference: &mut pb::Reference) {
    let Some(entity::Kind::DesignGraph(graph)) = &entity.kind else {
        return;
    };
    let field = reference.from_field.as_str();
    for (i, subject) in graph.subjects.iter().enumerate() {
        if field.starts_with(&format!("subjects[{i}].subject.")) {
            reference.catalog_subject.clone_from(&subject.subject);
            reference.connection_id.clone_from(&subject.connection_id);
            return;
        }
    }
    for (i, step) in graph.steps.iter().enumerate() {
        if field.starts_with(&format!("steps[{i}].target.")) {
            reference.step_id.clone_from(&step.step_id);
            if let Some(pb::topology_step_target::Selection::CatalogMessage(subject)) =
                step.target.as_ref().and_then(|t| t.selection.as_ref())
            {
                reference.catalog_subject = Some(subject.clone());
            }
            return;
        }
    }
}
