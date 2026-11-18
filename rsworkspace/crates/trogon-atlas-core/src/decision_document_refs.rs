use trogon_atlas_proto::{self as pb, entity, Entity, EntityKind, Id};

use crate::{decision_document, refs::OutboundRef};

fn walk(entity: &mut Entity, visit: &mut impl FnMut(EntityKind, &mut Option<Id>, String, bool)) {
    let subjects = match &mut entity.kind {
        Some(entity::Kind::Decision(v)) => {
            if let Some(r) = v.replaced_by.as_mut().and_then(|r| r.decision.as_mut()) {
                visit(
                    EntityKind::Decision,
                    &mut r.id,
                    "replaced_by.decision".into(),
                    false,
                );
            }
            for (i, maker) in v.decision_makers.iter_mut().enumerate() {
                match &mut maker.assignee {
                    Some(pb::decision_maker::Assignee::Person(r)) => visit(
                        EntityKind::Person,
                        &mut r.id,
                        format!("decision_makers[{i}].person"),
                        false,
                    ),
                    Some(pb::decision_maker::Assignee::Team(r)) => visit(
                        EntityKind::Team,
                        &mut r.id,
                        format!("decision_makers[{i}].team"),
                        false,
                    ),
                    None => {}
                }
            }
            for (i, r) in v.amends.iter_mut().enumerate() {
                if let Some(r) = &mut r.decision {
                    visit(
                        EntityKind::Decision,
                        &mut r.id,
                        format!("amends[{i}].decision"),
                        false,
                    );
                }
            }
            for (i, r) in v.related_decisions.iter_mut().enumerate() {
                if let Some(r) = &mut r.decision {
                    visit(
                        EntityKind::Decision,
                        &mut r.id,
                        format!("related_decisions[{i}].decision"),
                        false,
                    );
                }
            }
            for (i, r) in v.documents.iter_mut().enumerate() {
                if let Some(r) = &mut r.document {
                    document(r, &format!("documents[{i}].document"), visit);
                }
            }
            &mut v.subjects
        }
        Some(entity::Kind::Document(v)) => {
            for (i, r) in v.navigation.iter_mut().enumerate() {
                if let Some(r) = &mut r.document {
                    document(r, &format!("navigation[{i}].document"), visit);
                }
            }
            for (i, r) in v.related_content.iter_mut().enumerate() {
                if let Some(r) = &mut r.document {
                    document(r, &format!("related_content[{i}].document"), visit);
                }
            }
            &mut v.subjects
        }
        _ => return,
    };
    walk_subjects(subjects, visit);
}
pub(crate) fn walk_subjects(
    subjects: &mut [pb::SubjectAssociation],
    visit: &mut impl FnMut(EntityKind, &mut Option<Id>, String, bool),
) {
    for (i, association) in subjects.iter_mut().enumerate() {
        if let Some(subject) = association.subject.as_mut() {
            walk_subject(subject, &format!("subjects[{i}].subject"), visit);
        }
    }
}
pub(crate) fn walk_subject(
    subject: &mut pb::CatalogSubject,
    path: &str,
    visit: &mut impl FnMut(EntityKind, &mut Option<Id>, String, bool),
) {
    match subject.selection.as_mut() {
        Some(pb::catalog_subject::Selection::Entity(r)) => {
            if let Ok(kind) = EntityKind::try_from(r.kind) {
                visit(kind, &mut r.id, format!("{path}.entity"), false);
            }
        }
        Some(pb::catalog_subject::Selection::Connection(r)) => {
            if let Some(r) = &mut r.owner {
                if let Ok(kind) = EntityKind::try_from(r.kind) {
                    visit(kind, &mut r.id, format!("{path}.connection.owner"), true);
                }
            }
        }
        Some(pb::catalog_subject::Selection::Member(r)) => {
            if let Some(r) = &mut r.specification {
                visit(
                    EntityKind::Specification,
                    &mut r.id,
                    format!("{path}.member.specification"),
                    true,
                );
            }
        }
        Some(pb::catalog_subject::Selection::Port(r)) => {
            if let Some(r) = &mut r.product {
                visit(
                    EntityKind::DataProduct,
                    &mut r.id,
                    format!("{path}.port.product"),
                    true,
                );
            }
        }
        None => {}
    }
}
pub(crate) fn document(
    selection: &mut pb::DocumentSelection,
    path: &str,
    visit: &mut impl FnMut(EntityKind, &mut Option<Id>, String, bool),
) {
    match &mut selection.selection {
        Some(pb::document_selection::Selection::Document(r)) => visit(
            EntityKind::Document,
            &mut r.id,
            format!("{path}.document"),
            false,
        ),
        Some(pb::document_selection::Selection::Anchor(r)) => {
            if let Some(r) = &mut r.document {
                visit(
                    EntityKind::Document,
                    &mut r.id,
                    format!("{path}.anchor.document"),
                    true,
                );
            }
        }
        None => {}
    }
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
    let field = reference.from_field.as_str();
    for (i, subject) in decision_document::subjects(entity).iter().enumerate() {
        if field.starts_with(&format!("subjects[{i}].subject.")) {
            reference.catalog_subject.clone_from(&subject.subject);
            reference.connection_id.clone_from(&subject.connection_id);
            return;
        }
    }
    match &entity.kind {
        Some(entity::Kind::Decision(v)) => {
            for (i, declaration) in v.decision_makers.iter().enumerate() {
                if field.starts_with(&format!("decision_makers[{i}].")) {
                    reference
                        .connection_id
                        .clone_from(&declaration.connection_id);
                }
            }
            for (i, declaration) in v.amends.iter().enumerate() {
                if field == format!("amends[{i}].decision") {
                    reference
                        .connection_id
                        .clone_from(&declaration.connection_id);
                }
            }
            for (i, declaration) in v.documents.iter().enumerate() {
                if field.starts_with(&format!("documents[{i}].document.")) {
                    reference
                        .document_selection
                        .clone_from(&declaration.document);
                    reference
                        .connection_id
                        .clone_from(&declaration.connection_id);
                }
            }
            for (i, declaration) in v.related_decisions.iter().enumerate() {
                if field == format!("related_decisions[{i}].decision") {
                    reference
                        .connection_id
                        .clone_from(&declaration.connection_id);
                }
            }
            if field == "replaced_by.decision" {
                reference.connection_id =
                    v.replaced_by.as_ref().and_then(|d| d.connection_id.clone());
            }
        }
        Some(entity::Kind::Document(v)) => {
            for (i, declaration) in v.navigation.iter().enumerate() {
                if field.starts_with(&format!("navigation[{i}].document.")) {
                    reference
                        .document_selection
                        .clone_from(&declaration.document);
                    reference
                        .connection_id
                        .clone_from(&declaration.connection_id);
                }
            }
            for (i, declaration) in v.related_content.iter().enumerate() {
                if field.starts_with(&format!("related_content[{i}].document.")) {
                    reference
                        .document_selection
                        .clone_from(&declaration.document);
                    reference
                        .connection_id
                        .clone_from(&declaration.connection_id);
                }
            }
        }
        _ => {}
    }
}
