use trogon_atlas_proto::{entity, responsibility_subject::Selection, Entity, EntityKind, Id};

use crate::refs::OutboundRef;

fn walk(entity: &mut Entity, visit: &mut impl FnMut(EntityKind, &mut Option<Id>, String, bool)) {
    let assignments = match entity.kind.as_mut() {
        Some(entity::Kind::Team(team)) => {
            for (i, member) in team.members.iter_mut().enumerate() {
                if let Some(person) = &mut member.person {
                    visit(
                        EntityKind::Person,
                        &mut person.id,
                        format!("members[{i}].person"),
                        false,
                    );
                }
            }
            &mut team.responsibilities
        }
        Some(entity::Kind::Person(person)) => &mut person.responsibilities,
        _ => return,
    };
    for (i, assignment) in assignments.iter_mut().enumerate() {
        let path = format!("responsibilities[{i}].subject");
        match assignment
            .subject
            .as_mut()
            .and_then(|s| s.selection.as_mut())
        {
            Some(Selection::Entity(r)) => {
                if let Ok(kind) = EntityKind::try_from(r.kind) {
                    visit(kind, &mut r.id, format!("{path}.entity"), false);
                }
            }
            Some(Selection::Connection(r)) => {
                if let Some(owner) = &mut r.owner {
                    if let Ok(kind) = EntityKind::try_from(owner.kind) {
                        visit(
                            kind,
                            &mut owner.id,
                            format!("{path}.connection.owner"),
                            true,
                        );
                    }
                }
            }
            Some(Selection::Member(r)) => {
                if let Some(owner) = &mut r.specification {
                    visit(
                        EntityKind::Specification,
                        &mut owner.id,
                        format!("{path}.member.specification"),
                        true,
                    );
                }
            }
            Some(Selection::Port(r)) => {
                if let Some(owner) = &mut r.product {
                    visit(
                        EntityKind::DataProduct,
                        &mut owner.id,
                        format!("{path}.port.product"),
                        true,
                    );
                }
            }
            None => {}
        }
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
