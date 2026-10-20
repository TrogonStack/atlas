//! Backend-owned connection identity continuity, including retired occurrences.

use std::collections::BTreeSet;

use trogon_atlas_proto::Entity;

use crate::{
    connections::{
        assign_connection_ids, connection_occurrences, equivalent_with_unassigned_ids,
        reconcile_connection_ids, validate_stored_connection_ids, ConnectionError, ConnectionId,
    },
    stamp_system,
};

/// Prepare an untrusted write against the value guarded by the store's CAS.
pub fn prepare_entity(
    previous: Option<&Entity>,
    proposed: &Entity,
) -> Result<Entity, ConnectionError> {
    if let Some(previous) = previous {
        if equivalent_with_unassigned_ids(previous, proposed)? {
            return prepare(Some(previous), previous, &[], false);
        }
    }
    prepare(previous, proposed, &[], false)
}

/// Land a trusted branch value after checking its baseline concurrency guard.
pub fn prepare_landing(
    previous: Option<&Entity>,
    branch: &Entity,
) -> Result<Entity, ConnectionError> {
    if let Some(previous) = previous {
        validate_stored_connection_ids(previous)?;
    }
    validate_stored_connection_ids(branch)?;
    let retired = branch
        .system
        .as_ref()
        .map_or(&[][..], |system| system.retired_connection_ids.as_slice());
    let previous = previous.filter(|previous| {
        let old_uid = previous.system.as_ref().map(|system| system.uid.as_str());
        let branch_uid = branch.system.as_ref().map(|system| system.uid.as_str());
        !matches!((old_uid, branch_uid), (Some(old), Some(new)) if !old.is_empty() && !new.is_empty() && old != new)
    });
    prepare(previous, branch, retired, true)
}

fn prepare(
    previous: Option<&Entity>,
    proposed: &Entity,
    branch_retired: &[trogon_atlas_proto::ConnectionId],
    landing: bool,
) -> Result<Entity, ConnectionError> {
    let mut prepared = proposed.clone();
    match previous {
        Some(previous) => {
            reconcile_connection_ids(previous, &mut prepared)?;
        }
        None => {
            assign_connection_ids(&mut prepared)?;
        }
    }
    let active: BTreeSet<String> = connection_occurrences(&prepared)?
        .into_iter()
        .filter_map(|item| item.id.map(|id| id.to_string()))
        .collect();
    let mut retired = BTreeSet::new();
    for id in previous
        .and_then(|entity| entity.system.as_ref())
        .into_iter()
        .flat_map(|system| &system.retired_connection_ids)
        .chain(branch_retired)
    {
        let id: ConnectionId = id.value.parse()?;
        retired.insert(id.to_string());
    }
    for id in &active {
        if retired.contains(id) {
            return Err(ConnectionError::RetiredId(id.parse()?));
        }
    }
    if let Some(previous) = previous {
        for id in connection_occurrences(previous)?
            .into_iter()
            .filter_map(|item| item.id)
        {
            let id = id.to_string();
            if !active.contains(&id) {
                retired.insert(id);
            }
        }
    }
    prepared = stamp_system(previous, &prepared);
    if landing && previous.is_none() {
        if let Some(system) = proposed
            .system
            .as_ref()
            .filter(|system| !system.uid.is_empty())
        {
            prepared.system = Some(system.clone());
        }
    }
    if let Some(system) = prepared.system.as_mut() {
        system.retired_connection_ids = retired
            .into_iter()
            .map(|value| trogon_atlas_proto::ConnectionId { value })
            .collect();
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use trogon_atlas_proto::{entity, ui, Id, SystemMeta, Ui, UiRef};

    use super::*;

    fn ui() -> Entity {
        Entity {
            kind: Some(entity::Kind::Ui(Ui {
                id: Some(Id {
                    namespace: "shop".into(),
                    slug: "home".into(),
                    version: 1,
                }),
                transitions: vec![ui::Transition {
                    to: Some(UiRef {
                        id: Some(Id {
                            namespace: "shop".into(),
                            slug: "cart".into(),
                            version: 1,
                        }),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            system: None,
        }
    }

    fn clear_connections(entity: &mut Entity) {
        let Some(entity::Kind::Ui(ui)) = entity.kind.as_mut() else {
            panic!("expected UI")
        };
        ui.transitions.clear();
    }

    #[test]
    fn creation_ignores_client_system_and_assigns_ids() {
        let mut proposed = ui();
        proposed.system = Some(SystemMeta {
            uid: "untrusted".into(),
            ..Default::default()
        });
        let stored = prepare_entity(None, &proposed).unwrap();
        assert_ne!(stored.system.as_ref().unwrap().uid, "untrusted");
        assert!(connection_occurrences(&stored).unwrap()[0].id.is_some());
        assert_eq!(prepare_entity(Some(&stored), &stored).unwrap(), stored);
        assert_eq!(prepare_entity(Some(&stored), &ui()).unwrap(), stored);
    }

    #[test]
    fn idless_stored_owner_is_rejected_before_no_op_substitution() {
        let unsupported = ui();
        assert_eq!(
            prepare_entity(Some(&unsupported), &unsupported),
            Err(ConnectionError::MissingStoredId("ui.transitions[0]".into()))
        );
        let mut identified_proposal = unsupported.clone();
        assign_connection_ids(&mut identified_proposal).unwrap();
        assert!(matches!(
            prepare_entity(Some(&unsupported), &identified_proposal),
            Err(ConnectionError::MissingStoredId(_))
        ));
    }

    #[test]
    fn landing_requires_complete_stored_baseline_and_branch_ids() {
        let stored = prepare_entity(None, &ui()).unwrap();
        assert!(matches!(
            prepare_landing(Some(&ui()), &stored),
            Err(ConnectionError::MissingStoredId(_))
        ));
        assert!(matches!(
            prepare_landing(None, &ui()),
            Err(ConnectionError::MissingStoredId(_))
        ));
        assert!(matches!(
            prepare_landing(Some(&stored), &ui()),
            Err(ConnectionError::MissingStoredId(_))
        ));
    }

    #[test]
    fn removal_retires_id_and_client_cannot_clear_history() {
        let original = prepare_entity(None, &ui()).unwrap();
        let id = connection_occurrences(&original).unwrap()[0].id.unwrap();
        let mut removed = original.clone();
        clear_connections(&mut removed);
        let removed = prepare_entity(Some(&original), &removed).unwrap();
        assert_eq!(
            removed.system.as_ref().unwrap().retired_connection_ids,
            vec![id.into()]
        );
        let mut reuse = original;
        reuse.system = None;
        assert_eq!(
            prepare_entity(Some(&removed), &reuse),
            Err(ConnectionError::RetiredId(id))
        );
    }

    #[test]
    fn malformed_retirement_history_is_not_silently_dropped() {
        let mut original = prepare_entity(None, &ui()).unwrap();
        original
            .system
            .as_mut()
            .unwrap()
            .retired_connection_ids
            .push(trogon_atlas_proto::ConnectionId {
                value: "invalid".into(),
            });
        assert!(matches!(
            prepare_entity(Some(&original), &original),
            Err(ConnectionError::InvalidId(_))
        ));
    }

    #[test]
    fn landing_preserves_branch_only_retirement_history() {
        let mut baseline = ui();
        clear_connections(&mut baseline);
        let baseline = prepare_entity(None, &baseline).unwrap();
        let branch = prepare_entity(Some(&baseline), &ui()).unwrap();
        let id = connection_occurrences(&branch).unwrap()[0].id.unwrap();
        let mut removed = branch.clone();
        clear_connections(&mut removed);
        let removed = prepare_entity(Some(&branch), &removed).unwrap();
        let landed = prepare_landing(Some(&baseline), &removed).unwrap();
        assert_eq!(
            landed.system.as_ref().unwrap().retired_connection_ids,
            vec![id.into()]
        );
        assert_eq!(
            prepare_entity(Some(&landed), &branch),
            Err(ConnectionError::RetiredId(id))
        );
    }

    #[test]
    fn guarded_landing_preserves_new_or_replaced_branch_incarnation() {
        let branch = prepare_entity(None, &ui()).unwrap();
        assert_eq!(
            prepare_landing(None, &branch).unwrap().system,
            branch.system
        );
        let replacement = prepare_entity(None, &ui()).unwrap();
        assert_eq!(
            prepare_landing(Some(&replacement), &branch).unwrap().system,
            branch.system
        );
    }
}
