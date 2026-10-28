//! Backend-owned connection identity continuity against bounded current snapshots.

use std::collections::HashSet;

use prost::Message;
use trogon_atlas_proto::{entity, Entity};

use crate::{
    connections::{
        assign_connection_ids, connection_occurrences, equivalent_with_unassigned_ids,
        reconcile_branch_connection_ids, reconcile_connection_ids, renew_connection_ids,
        validate_stored_connection_ids, ConnectionError, ConnectionId,
    },
    data_product, decision_document, specification, stamp_system,
};

fn checked_prepared_owner(entity: Entity) -> Result<Entity, ConnectionError> {
    if matches!(
        entity.kind,
        Some(entity::Kind::Document(_) | entity::Kind::Decision(_))
    ) && entity.encoded_len() > decision_document::MAX_ENCODED_BYTES
    {
        return Err(ConnectionError::Catalog(
            "catalog entity byte limit exceeded".into(),
        ));
    }
    Ok(entity)
}

/// Prepare an untrusted write against the value guarded by the store's CAS.
pub fn prepare_entity(
    previous: Option<&Entity>,
    proposed: &Entity,
) -> Result<Entity, ConnectionError> {
    let mut prepared = proposed.clone();
    if let Some(previous) = previous {
        if decision_document::equivalent_manifest(previous, proposed)?
            || specification::equivalent_manifest(previous, proposed)?
            || data_product::equivalent_manifest(previous, proposed)?
        {
            return checked_prepared_owner(previous.clone());
        }
    }
    decision_document::prepare_values(previous, &mut prepared, false)?;
    specification::prepare_members(previous, &mut prepared, false)?;
    data_product::prepare_ports(previous, &mut prepared, false)?;
    match previous {
        Some(previous) => {
            if equivalent_with_unassigned_ids(previous, &prepared)? {
                prepared = previous.clone();
            } else {
                reconcile_connection_ids(previous, &mut prepared)?;
            }
        }
        None => {
            assign_connection_ids(&mut prepared)?;
        }
    }
    checked_prepared_owner(stamp_system(previous, &prepared))
}

/// Land a trusted branch value after checking its baseline concurrency guard.
pub fn prepare_landing(
    base: Option<&Entity>,
    previous: Option<&Entity>,
    branch: &Entity,
) -> Result<Entity, ConnectionError> {
    let removed_values = decision_document::removed_values(base, previous, branch)?;
    decision_document::reject_removed(branch, &removed_values)?;
    let removed_ports = data_product::removed_ports(base, previous, branch)?;
    data_product::reject_removed(branch, &removed_ports)?;
    let removed_members = specification::removed_members(base, previous, branch)?;
    specification::reject_removed(branch, &removed_members)?;
    let removed = removed_base_ids(base, previous, branch)?;
    if let Some(id) = connection_occurrences(branch)?
        .into_iter()
        .filter_map(|item| item.id)
        .find(|id| removed.contains(id))
    {
        return Err(ConnectionError::RemovedFromBranchBase(id));
    }
    let previous = previous.filter(|entity| same_incarnation(entity, branch));
    let mut prepared = branch.clone();
    decision_document::prepare_values(previous, &mut prepared, true)?;
    specification::prepare_members(previous, &mut prepared, true)?;
    data_product::prepare_ports(previous, &mut prepared, true)?;
    if let Some(previous) = previous {
        reconcile_branch_connection_ids(previous, &mut prepared)?;
    }
    prepared = stamp_system(previous, &prepared);
    if previous.is_none() {
        if let Some(system) = branch
            .system
            .as_ref()
            .filter(|system| !system.uid.is_empty())
        {
            prepared.system = Some(system.clone());
        }
    }
    checked_prepared_owner(prepared)
}

/// Resolve an explicit branch choice without reviving occurrences removed since its base.
pub fn prepare_resolution(
    base: Option<&Entity>,
    discarded: Option<&Entity>,
    selected: &Entity,
) -> Result<Entity, ConnectionError> {
    let removed = removed_base_ids(base, discarded, selected)?;
    let mut resolved = selected.clone();
    renew_connection_ids(&mut resolved, &removed)?;
    let removed_values = decision_document::removed_values(base, discarded, selected)?;
    decision_document::restore_values(&mut resolved, &removed_values)?;
    let removed_members = specification::removed_members(base, discarded, selected)?;
    specification::restore_members(&mut resolved, &removed_members)?;
    let removed_ports = data_product::removed_ports(base, discarded, selected)?;
    data_product::restore_ports(&mut resolved, &removed_ports)?;
    crate::connections::renew_retargeted_connection_ids(selected, &mut resolved)?;
    crate::connections::assign_connection_ids(&mut resolved)?;
    specification::rewrite_exposure_selections(selected, &mut resolved);
    crate::connections::renew_retargeted_connection_ids(selected, &mut resolved)?;
    crate::connections::assign_connection_ids(&mut resolved)?;
    prepare_landing(base, discarded, &resolved)
}

fn same_incarnation(left: &Entity, right: &Entity) -> bool {
    left.system
        .as_ref()
        .zip(right.system.as_ref())
        .is_some_and(|(left, right)| !left.uid.is_empty() && left.uid == right.uid)
}

fn removed_base_ids(
    base: Option<&Entity>,
    opposing: Option<&Entity>,
    selected: &Entity,
) -> Result<HashSet<ConnectionId>, ConnectionError> {
    for entity in base
        .into_iter()
        .chain(opposing)
        .chain(std::iter::once(selected))
    {
        validate_stored_connection_ids(entity)?;
    }
    let Some(base) = base.filter(|base| same_incarnation(base, selected)) else {
        return Ok(HashSet::new());
    };
    let Some(opposing) = opposing.filter(|entity| same_incarnation(entity, selected)) else {
        return Ok(HashSet::new());
    };
    let active = connection_occurrences(opposing)?
        .into_iter()
        .filter_map(|item| item.id)
        .collect::<HashSet<_>>();
    Ok(connection_occurrences(base)?
        .into_iter()
        .filter_map(|item| item.id)
        .filter(|id| !active.contains(id))
        .collect())
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
    fn existing_owner_rejects_client_supplied_unknown_connection_id() {
        let stored = prepare_entity(None, &ui()).unwrap();
        let mut proposed = stored.clone();
        let Some(entity::Kind::Ui(screen)) = proposed.kind.as_mut() else {
            panic!("expected UI");
        };
        screen.transitions.push(trogon_atlas_proto::ui::Transition {
            to: Some(UiRef {
                id: Some(Id {
                    namespace: "shop".into(),
                    slug: "checkout".into(),
                    version: 1,
                }),
            }),
            connection_id: Some(ConnectionId::new().into()),
            ..Default::default()
        });
        assert!(prepare_entity(Some(&stored), &proposed).is_err());
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
    fn removed_ids_cannot_be_supplied_again_but_idless_additions_are_fresh() {
        let original = prepare_entity(None, &ui()).unwrap();
        let old = connection_occurrences(&original).unwrap()[0].id.unwrap();
        let mut removed = original.clone();
        clear_connections(&mut removed);
        let removed = prepare_entity(Some(&original), &removed).unwrap();
        assert_eq!(
            prepare_entity(Some(&removed), &original),
            Err(ConnectionError::UnknownId(old))
        );
        let restored = prepare_entity(Some(&removed), &ui()).unwrap();
        assert_ne!(
            connection_occurrences(&restored).unwrap()[0].id.unwrap(),
            old
        );
    }

    #[test]
    fn fresh_incarnation_can_accept_supplied_ids() {
        let original = prepare_entity(None, &ui()).unwrap();
        let replacement = prepare_entity(None, &original).unwrap();
        assert_eq!(
            connection_occurrences(&original).unwrap(),
            connection_occurrences(&replacement).unwrap()
        );
        assert_ne!(
            original.system.unwrap().uid,
            replacement.system.unwrap().uid
        );
    }

    #[test]
    fn common_base_removal_blocks_landing_and_explicit_resolution_renews_only_removed_ids() {
        let mut authored = ui();
        let Some(entity::Kind::Ui(screen)) = authored.kind.as_mut() else {
            panic!("expected UI")
        };
        screen.transitions.push(screen.transitions[0].clone());
        let base = prepare_entity(None, &authored).unwrap();
        let mut pruned = base.clone();
        let Some(entity::Kind::Ui(screen)) = pruned.kind.as_mut() else {
            panic!("expected UI")
        };
        screen.transitions.remove(0);
        let pruned = prepare_entity(Some(&base), &pruned).unwrap();
        let ids = connection_occurrences(&base).unwrap();
        let removed = ids[0].id.unwrap();
        assert_eq!(
            prepare_landing(Some(&base), Some(&pruned), &base),
            Err(ConnectionError::RemovedFromBranchBase(removed))
        );
        let resolved = prepare_resolution(Some(&base), Some(&pruned), &base).unwrap();
        let resolved_ids = connection_occurrences(&resolved).unwrap();
        assert_ne!(resolved_ids[0].id, ids[0].id);
        assert_eq!(resolved_ids[1].id, ids[1].id);
        assert_eq!(
            resolved.system.as_ref().unwrap().uid,
            base.system.as_ref().unwrap().uid
        );
        assert!(prepare_landing(Some(&pruned), Some(&pruned), &resolved).is_ok());
    }

    #[test]
    fn absent_opposing_owner_preserves_surviving_branch_occurrence_identity() {
        let branch = prepare_entity(None, &ui()).unwrap();
        assert_eq!(
            prepare_resolution(Some(&branch), None, &branch).unwrap(),
            branch
        );
        assert_eq!(
            prepare_landing(Some(&branch), None, &branch).unwrap(),
            branch
        );
    }

    #[test]
    fn trusted_landing_accepts_branch_only_additions() {
        let mut authored = ui();
        clear_connections(&mut authored);
        let base = prepare_entity(None, &authored).unwrap();
        let branch = prepare_entity(Some(&base), &ui()).unwrap();
        let landed = prepare_landing(Some(&base), Some(&base), &branch).unwrap();
        assert_eq!(
            connection_occurrences(&landed).unwrap(),
            connection_occurrences(&branch).unwrap()
        );
    }

    #[test]
    fn distinct_incarnations_do_not_share_removed_base_ids() {
        let base = prepare_entity(None, &ui()).unwrap();
        let mut authored = ui();
        clear_connections(&mut authored);
        let other = prepare_entity(None, &authored).unwrap();
        let landed = prepare_landing(Some(&base), Some(&other), &base).unwrap();
        assert_eq!(
            landed.system.as_ref().unwrap().uid,
            base.system.as_ref().unwrap().uid
        );
        assert_eq!(
            connection_occurrences(&landed).unwrap(),
            connection_occurrences(&base).unwrap()
        );
        let resolved = prepare_resolution(Some(&base), Some(&other), &base).unwrap();
        assert_eq!(
            connection_occurrences(&resolved).unwrap(),
            connection_occurrences(&base).unwrap()
        );
    }

    #[test]
    fn branch_preparation_validates_all_snapshots_before_incarnation_filtering() {
        let stored = prepare_entity(None, &ui()).unwrap();
        let mut incomplete_other_incarnation = prepare_entity(None, &ui()).unwrap();
        let Some(entity::Kind::Ui(screen)) = incomplete_other_incarnation.kind.as_mut() else {
            panic!("expected UI")
        };
        screen.transitions[0].connection_id = None;
        for (base, previous, selected) in [
            (
                Some(stored.clone()),
                Some(incomplete_other_incarnation),
                stored.clone(),
            ),
            (Some(ui()), None, stored.clone()),
            (Some(stored.clone()), None, ui()),
            (Some(ui()), Some(stored.clone()), stored.clone()),
            (Some(stored.clone()), Some(ui()), stored.clone()),
            (Some(stored.clone()), Some(stored.clone()), ui()),
        ] {
            assert!(matches!(
                prepare_landing(base.as_ref(), previous.as_ref(), &selected),
                Err(ConnectionError::MissingStoredId(_))
            ));
            assert!(matches!(
                prepare_resolution(base.as_ref(), previous.as_ref(), &selected),
                Err(ConnectionError::MissingStoredId(_))
            ));
        }
    }

    #[test]
    fn guarded_landing_preserves_new_or_replaced_branch_incarnation() {
        let branch = prepare_entity(None, &ui()).unwrap();
        assert_eq!(
            prepare_landing(None, None, &branch).unwrap().system,
            branch.system
        );
        let replacement = prepare_entity(None, &ui()).unwrap();
        assert_eq!(
            prepare_landing(None, Some(&replacement), &branch)
                .unwrap()
                .system,
            branch.system
        );
    }
}
