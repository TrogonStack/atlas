use std::collections::{HashMap, HashSet};

use trogon_atlas_proto as pb;
use trogon_atlas_store::StoredEntity;

#[derive(Default)]
pub(super) struct EventConfirmation {
    rows: HashSet<pb::EntityKey>,
    examinations: usize,
}

impl EventConfirmation {
    pub(super) fn confirm(
        &mut self,
        event: &pb::Id,
        world: &[StoredEntity],
        discovery_complete: bool,
    ) -> Result<Vec<pb::ConnectionRef>, tonic::Status> {
        self.charge(pb::EntityKind::Event, event)?;
        let mut witnesses = Vec::new();
        for stored in world {
            let Some(pb::entity::Kind::CommandSlice(slice)) = &stored.entity.kind else {
                continue;
            };
            if let Some(id) = &slice.id {
                self.charge(pb::EntityKind::CommandSlice, id)?;
            }
            let valid_owner = stored
                .entity
                .system
                .as_ref()
                .is_some_and(|system| !system.uid.is_empty())
                && trogon_atlas_core::connections::validate_stored_connection_ids(&stored.entity)
                    .is_ok();
            for edge in &slice.emitted_events {
                self.examinations += 1;
                if self.examinations > 5_000 {
                    return Err(exhausted());
                }
                if valid_owner && edge.event.as_ref().and_then(|r| r.id.as_ref()) == Some(event) {
                    let reference = pb::ConnectionRef {
                        owner: crate::conv::entity_to_ref(&stored.entity).ok(),
                        owner_uid: stored
                            .entity
                            .system
                            .as_ref()
                            .map_or_else(String::new, |system| system.uid.clone()),
                        id: edge.connection_id.clone(),
                    };
                    witnesses.push(reference);
                }
            }
        }
        if !discovery_complete {
            return Err(exhausted());
        }
        Ok(witnesses)
    }

    fn charge(&mut self, kind: pb::EntityKind, id: &pb::Id) -> Result<(), tonic::Status> {
        self.rows.insert(pb::EntityKey::new(kind, id));
        if self.rows.len() > 5_000 {
            return Err(exhausted());
        }
        Ok(())
    }
}

fn exhausted() -> tonic::Status {
    tonic::Status::resource_exhausted("Data product Event confirmation exceeds its request budget")
}

pub(super) fn inputs(product: &pb::DataProduct) -> impl Iterator<Item = (usize, &pb::Id)> {
    product
        .inputs
        .iter()
        .enumerate()
        .filter_map(|(index, port)| {
            match port.data.as_ref().and_then(|data| data.target.as_ref()) {
                Some(pb::input_data_selection::Target::Event(reference)) => {
                    reference.id.as_ref().map(|id| (index, id))
                }
                _ => None,
            }
        })
}

pub(super) fn merge_rows(
    rows: Vec<StoredEntity>,
    overlay: impl IntoIterator<Item = (pb::EntityKey, Option<StoredEntity>)>,
) -> Vec<StoredEntity> {
    let mut merged: HashMap<_, _> = rows
        .into_iter()
        .filter_map(|stored| {
            Some((
                pb::EntityKey::new(
                    crate::conv::entity_kind(&stored.entity).ok()?,
                    crate::conv::entity_id(&stored.entity).ok()?,
                ),
                stored,
            ))
        })
        .collect();
    for (key, stored) in overlay {
        match stored {
            Some(stored) => {
                merged.insert(key, stored);
            }
            None => {
                merged.remove(&key);
            }
        }
    }
    let mut rows: Vec<_> = merged.into_values().collect();
    rows.sort_by_key(|stored| {
        crate::conv::entity_id(&stored.entity)
            .ok()
            .map(|id| (id.namespace.clone(), id.slug.clone(), id.version))
    });
    rows
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn distinct_row_boundary_counts_events_and_slices_and_reuses_cached_rows() {
        let id = |slug: String| pb::Id {
            namespace: "budget".into(),
            slug,
            version: 1,
        };
        let rows: Vec<_> = (0..4_999)
            .map(|index| StoredEntity {
                entity: pb::Entity {
                    kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                        id: Some(id(format!("candidate-{index}"))),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                etag: String::new(),
            })
            .collect();
        let mut budget = EventConfirmation::default();
        assert_eq!(
            budget.confirm(&id("first".into()), &rows, true).unwrap(),
            Vec::<pb::ConnectionRef>::new()
        );
        assert_eq!(
            budget.confirm(&id("first".into()), &rows, true).unwrap(),
            Vec::<pb::ConnectionRef>::new()
        );
        assert_eq!(
            budget
                .confirm(&id("other".into()), &rows, true)
                .unwrap_err()
                .code(),
            tonic::Code::ResourceExhausted
        );
    }
    #[test]
    fn occurrence_work_is_charged_again_but_rows_are_not() {
        let event = pb::Id {
            namespace: "budget".into(),
            slug: "fact".into(),
            version: 1,
        };
        let rows = vec![StoredEntity {
            entity: pb::Entity {
                kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                    id: Some(pb::Id {
                        slug: "emitter".into(),
                        ..event.clone()
                    }),
                    emitted_events: vec![pb::EventEdge::default(); 2_500],
                    ..Default::default()
                })),
                ..Default::default()
            },
            etag: String::new(),
        }];
        let mut budget = EventConfirmation::default();
        assert!(budget.confirm(&event, &rows, true).is_ok());
        assert!(budget.confirm(&event, &rows, true).is_ok());
        assert_eq!(budget.rows.len(), 2);
        assert_eq!(
            budget.confirm(&event, &rows, true).unwrap_err().code(),
            tonic::Code::ResourceExhausted
        );
        assert!(EventConfirmation::default()
            .confirm(&event, &rows, true)
            .is_ok());
    }
}
