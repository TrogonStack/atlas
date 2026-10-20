#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::{Code, Request};
use trogon_atlas_core::connections::{connection_occurrences, resolve_connection};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "connection-ids".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn screen(slug: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Ui(pb::Ui {
            id: Some(id(slug)),
            transitions: vec![
                pb::ui::Transition {
                    to: Some(pb::UiRef {
                        id: Some(id("destination"))
                    }),
                    ..Default::default()
                };
                2
            ],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn ui(entity: &mut pb::Entity) -> &mut pb::Ui {
    let Some(pb::entity::Kind::Ui(ui)) = entity.kind.as_mut() else {
        panic!("expected UI")
    };
    ui
}

fn put(entity: pb::Entity) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(entity),
        ..Default::default()
    }
}

async fn service() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default()).unwrap()
}

async fn save(service: &EventModelServiceImpl, entity: pb::Entity) -> pb::Entity {
    service
        .put_entity(Request::new(put(entity)))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

async fn get(service: &EventModelServiceImpl, slug: &str) -> pb::Entity {
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id(slug)),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

fn reference(entity: &pb::Entity, index: usize) -> pb::ConnectionRef {
    pb::ConnectionRef {
        owner: Some(pb::EntityRef {
            kind: pb::EntityKind::Ui as i32,
            id: Some(trogon_atlas_core::entity_id(entity).unwrap().clone()),
        }),
        id: Some(
            connection_occurrences(entity).unwrap()[index]
                .id
                .unwrap()
                .into(),
        ),
        owner_uid: entity.system.as_ref().unwrap().uid.clone(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn public_writes_preserve_ids_on_reorder_and_noop_and_reject_retarget_or_stripping() {
    let service = service().await;
    let mut entity = save(&service, screen("reorder")).await;
    let selected = reference(&entity, 0);
    assert_ne!(reference(&entity, 1).id, selected.id);
    ui(&mut entity).transitions.reverse();
    ui(&mut entity).transitions[1].doc = "a renamed navigation affordance".into();
    let reordered = save(&service, entity).await;
    assert!(resolve_connection(&reordered, &selected)
        .unwrap()
        .path
        .ends_with("[1]"));
    let noop = service
        .put_entity(Request::new(put(reordered.clone())))
        .await
        .unwrap()
        .into_inner();
    assert!(noop.no_op);
    assert_eq!(noop.entity, Some(reordered.clone()));

    let mut changed = reordered.clone();
    ui(&mut changed).transitions[1].to = Some(pb::UiRef {
        id: Some(id("other")),
    });
    assert_eq!(
        service
            .put_entity(Request::new(put(changed)))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let mut stripped = reordered.clone();
    ui(&mut stripped).transitions[1].connection_id = None;
    assert_eq!(
        service
            .put_entity(Request::new(put(stripped)))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(get(&service, "reorder").await, reordered);
}

#[tokio::test(flavor = "multi_thread")]
async fn existing_owner_rejects_unknown_client_supplied_connection_ids() {
    let service = service().await;
    let original = save(&service, screen("unknown-id")).await;
    let mut proposed = original.clone();
    ui(&mut proposed).transitions.push(pb::ui::Transition {
        to: Some(pb::UiRef {
            id: Some(id("destination")),
        }),
        connection_id: Some(pb::ConnectionId {
            value: "01900000-0000-7000-8000-000000000001".into(),
        }),
        ..Default::default()
    });
    assert!(service
        .put_entity(Request::new(put(proposed)))
        .await
        .is_err());
    assert_eq!(get(&service, "unknown-id").await, original);
}

#[tokio::test(flavor = "multi_thread")]
async fn removed_ids_are_rejected_and_owner_recreation_does_not_revive_references() {
    let service = service().await;
    let mut original = save(&service, screen("removed-id")).await;
    let selected = reference(&original, 0);
    let removed = ui(&mut original).transitions.remove(0);
    let mut pruned = save(&service, original).await;
    assert!(resolve_connection(&pruned, &selected).is_err());
    let mut reused = pruned.clone();
    ui(&mut reused).transitions.push(removed.clone());
    assert_eq!(
        service
            .put_entity(Request::new(put(reused)))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let mut replacement = removed.clone();
    replacement.connection_id = None;
    ui(&mut pruned).transitions.push(replacement);
    let replaced = save(&service, pruned).await;
    assert!(resolve_connection(&replaced, &selected).is_err());
    assert_ne!(
        ui(&mut replaced.clone())
            .transitions
            .last()
            .unwrap()
            .connection_id,
        selected.id
    );
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("removed-id")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut recreated = screen("removed-id");
    ui(&mut recreated).transitions[0] = removed;
    let recreated = save(&service, recreated).await;
    assert!(resolve_connection(&recreated, &selected).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn operation_retry_uses_original_request_digest_before_server_generated_ids() {
    let service = service().await;
    let request = pb::PutEntityRequest {
        operation_id: "connection-id-retry".into(),
        create_only: true,
        ..put(screen("receipt"))
    };
    let first = service
        .put_entity(Request::new(request.clone()))
        .await
        .unwrap()
        .into_inner();
    let replay = service
        .put_entity(Request::new(request))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.entity, replay.entity);
    assert_eq!(first.etag, replay.etag);
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_validates_connection_lifecycle_without_persisting() {
    let service = service().await;
    let original = save(&service, screen("preview")).await;
    let mut proposed = original.clone();
    ui(&mut proposed).transitions.remove(0);
    let preview = service
        .put_entity(Request::new(pb::PutEntityRequest {
            validate_only: true,
            ..put(proposed.clone())
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert!(resolve_connection(&preview, &reference(&original, 0)).is_err());
    assert_eq!(get(&service, "preview").await, original);
    let mut reused = preview;
    let removed = ui(&mut original.clone()).transitions[0].clone();
    ui(&mut reused).transitions.push(removed);
    let batch = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            validate_only: true,
            ops: vec![proposed, reused]
                .into_iter()
                .map(|entity| pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(put(entity))),
                })
                .collect(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        batch.status,
        pb::batch_mutate_response::Status::Failed as i32
    );
    assert_eq!(batch.failed_op_index, 1);
    assert_eq!(get(&service, "preview").await, original);
}

fn on_branch<T>(value: T) -> Request<T> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert("x-trogon-atlas-branch", "identity-work".parse().unwrap());
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_structural_merge_preserves_baseline_added_connection_ids() {
    let service = service().await;
    let mut destination = screen("destination");
    ui(&mut destination).transitions.clear();
    save(&service, destination).await;
    let original = save(&service, screen("structural-owner")).await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut branch = original.clone();
    ui(&mut branch).doc = "branch description".into();
    service.put_entity(on_branch(put(branch))).await.unwrap();
    let mut baseline = original.clone();
    ui(&mut baseline).transitions.push(pb::ui::Transition {
        to: Some(pb::UiRef {
            id: Some(id("destination")),
        }),
        ..Default::default()
    });
    let baseline = save(&service, baseline).await;
    let references = (0..3)
        .map(|index| reference(&baseline, index))
        .collect::<Vec<_>>();
    service
        .resolve_branch_entry(Request::new(pb::ResolveBranchEntryRequest {
            name: "identity-work".into(),
            r#ref: Some(pb::EntityRef {
                kind: pb::EntityKind::Ui as i32,
                id: Some(id("structural-owner")),
            }),
            resolution: pb::resolve_branch_entry_request::Resolution::TakeMerged as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut selected = service
        .get_entity(on_branch(pb::GetEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("structural-owner")),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(ui(&mut selected).doc, "branch description");
    for reference in &references {
        assert!(resolve_connection(&selected, reference).is_ok());
    }
    for dry_run in [true, false] {
        let merged = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "identity-work".into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            merged.status,
            if dry_run {
                pb::merge_branch_response::Status::Validated
            } else {
                pb::merge_branch_response::Status::Applied
            } as i32,
            "{merged:?}"
        );
    }
    let mut landed = get(&service, "structural-owner").await;
    assert_eq!(ui(&mut landed).doc, "branch description");
    for reference in &references {
        assert!(resolve_connection(&landed, reference).is_ok());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn updating_branch_does_not_erase_removed_connection_provenance() {
    let service = service().await;
    let original = save(&service, screen("branch-rebase")).await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut branch = original.clone();
    ui(&mut branch).transitions[0].doc = "branch connection annotation".into();
    service.put_entity(on_branch(put(branch))).await.unwrap();
    let mut baseline = original.clone();
    ui(&mut baseline).transitions.remove(0);
    let baseline = save(&service, baseline).await;
    service
        .update_branch(Request::new(pb::UpdateBranchRequest {
            name: "identity-work".into(),
        }))
        .await
        .unwrap();
    let diff = service
        .diff_branch(Request::new(pb::DiffBranchRequest {
            name: "identity-work".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(diff.entries.len(), 1);
    assert_eq!(
        diff.entries[0].status,
        pb::branch_diff_entry::Status::ConflictEditEdit as i32
    );
    assert_eq!(diff.entries[0].base.as_ref(), Some(&original));
    assert!(diff.entries[0].auto_merged.is_none());
    assert_eq!(get(&service, "branch-rebase").await, baseline);
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_added_connections_land_with_their_assigned_ids_and_dry_run_parity() {
    let service = service().await;
    let mut destination = screen("destination");
    ui(&mut destination).transitions.clear();
    save(&service, destination).await;
    let baseline = save(&service, screen("branch-addition")).await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut proposed = baseline.clone();
    ui(&mut proposed).transitions.push(pb::ui::Transition {
        to: Some(pb::UiRef {
            id: Some(id("destination")),
        }),
        ..Default::default()
    });
    let branch = service
        .put_entity(on_branch(put(proposed)))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let selected = reference(&branch, 2);
    for dry_run in [true, false] {
        let merged = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "identity-work".into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            merged.status,
            if dry_run {
                pb::merge_branch_response::Status::Validated
            } else {
                pb::merge_branch_response::Status::Applied
            } as i32,
            "{merged:?}"
        );
        let stored = get(&service, "branch-addition").await;
        if dry_run {
            assert_eq!(stored, baseline);
        } else {
            assert!(resolve_connection(&stored, &selected).is_ok());
            assert_eq!(
                connection_occurrences(&stored).unwrap(),
                connection_occurrences(&branch).unwrap()
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_merge_rejects_removed_ids_and_preserves_incarnation() {
    let service = service().await;
    save(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::Ui(pb::Ui {
                id: Some(id("destination")),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let baseline = save(&service, screen("branch-owner")).await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut proposed = baseline.clone();
    ui(&mut proposed).transitions.push(pb::ui::Transition {
        to: Some(pb::UiRef {
            id: Some(id("destination")),
        }),
        ..Default::default()
    });
    let mut added = service
        .put_entity(on_branch(put(proposed)))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let removed = ui(&mut added).transitions.pop().unwrap();
    service
        .put_entity(on_branch(put(added)))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(get(&service, "branch-owner").await, baseline);
    let merged = service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merged.status,
        pb::merge_branch_response::Status::Applied as i32,
        "{merged:?}"
    );
    let mut landed = get(&service, "branch-owner").await;
    assert_eq!(
        landed.system.as_ref().unwrap().uid,
        baseline.system.as_ref().unwrap().uid
    );
    ui(&mut landed).transitions.push(removed);
    assert_eq!(
        service
            .put_entity(Request::new(put(landed)))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sequential_batch_rejects_reuse_and_rolls_back_and_retarget_renews_ids() {
    let service = service().await;
    let original = save(&service, screen("batch-owner")).await;
    let selected = reference(&original, 0);
    let mut pruned = original.clone();
    ui(&mut pruned).transitions.remove(0);
    let batch = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![pruned, original.clone()]
                .into_iter()
                .map(|entity| pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(put(entity))),
                })
                .collect(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        batch.status,
        pb::batch_mutate_response::Status::Failed as i32
    );
    assert_eq!(batch.failed_op_index, 1);
    assert_eq!(get(&service, "batch-owner").await, original);
    for slug in ["destination", "other"] {
        save(
            &service,
            pb::Entity {
                kind: Some(pb::entity::Kind::Ui(pb::Ui {
                    id: Some(id(slug)),
                    ..Default::default()
                })),
                ..Default::default()
            },
        )
        .await;
    }
    let result = service
        .retarget_references(Request::new(pb::RetargetReferencesRequest {
            from: Some(pb::EntityRef {
                kind: pb::EntityKind::Ui as i32,
                id: Some(id("destination")),
            }),
            to: Some(pb::EntityRef {
                kind: pb::EntityKind::Ui as i32,
                id: Some(id("other")),
            }),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(result.failed, 0);
    assert_eq!(result.rewritten, 1);
    let changed = get(&service, "batch-owner").await;
    assert!(resolve_connection(&changed, &selected).is_err());
    assert_ne!(reference(&changed, 0).id, selected.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_and_duplicate_ids_are_rejected_before_create() {
    let service = service().await;
    let mut malformed = screen("malformed");
    ui(&mut malformed).transitions[0].connection_id = Some(pb::ConnectionId {
        value: "not-an-id".into(),
    });
    assert_eq!(
        service
            .put_entity(Request::new(put(malformed)))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let mut duplicate = screen("duplicate");
    let same: pb::ConnectionId = trogon_atlas_core::connections::ConnectionId::new().into();
    for transition in &mut ui(&mut duplicate).transitions {
        transition.connection_id = Some(same.clone());
    }
    assert_eq!(
        service
            .put_entity(Request::new(put(duplicate)))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn guarded_update_and_batch_create_assign_ids_without_noop_churn() {
    let service = service().await;
    let batch = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    create_only: true,
                    ..put(screen("batch-created"))
                })),
            }],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        batch.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
    let stored = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("batch-created")),
        }))
        .await
        .unwrap()
        .into_inner();
    let original = stored.entity.unwrap();
    assert!(connection_occurrences(&original)
        .unwrap()
        .iter()
        .all(|item| item.id.is_some()));
    let response = service
        .put_entity(Request::new(pb::PutEntityRequest {
            if_match: stored.etag.clone(),
            ..put(original.clone())
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(response.no_op);
    assert_eq!(response.etag, stored.etag);
    assert_eq!(response.entity, Some(original));
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_create_and_guarded_update_keep_the_same_occurrence_identity() {
    let service = service().await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let created = service
        .put_entity(on_branch(pb::PutEntityRequest {
            create_only: true,
            ..put(screen("branch-created"))
        }))
        .await
        .unwrap()
        .into_inner();
    let mut entity = created.entity.unwrap();
    let selected = reference(&entity, 0);
    ui(&mut entity).transitions[0].doc = "updated description".into();
    let updated = service
        .put_entity(on_branch(pb::PutEntityRequest {
            if_match: created.etag,
            ..put(entity)
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert!(resolve_connection(&updated, &selected).is_ok());
    assert_eq!(reference(&updated, 0), selected);
    let baseline = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("branch-created")),
        }))
        .await
        .unwrap_err();
    assert_eq!(baseline.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_recreation_merges_with_dry_run_parity_and_a_new_owner_uid() {
    let service = service().await;
    save(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::Ui(pb::Ui {
                id: Some(id("destination")),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let original = save(&service, screen("recreated-on-branch")).await;
    let selected = reference(&original, 0);
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    service
        .delete_entity(on_branch(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("recreated-on-branch")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let recreated = service
        .put_entity(on_branch(pb::PutEntityRequest {
            create_only: true,
            ..put(original.clone())
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_ne!(recreated.system.as_ref().unwrap().uid, selected.owner_uid);
    let preview = service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: "identity-work".into(),
            dry_run: true,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        preview.status,
        pb::merge_branch_response::Status::Validated as i32,
        "{preview:?}"
    );
    assert_eq!(get(&service, "recreated-on-branch").await, original);
    let applied = service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        applied.status,
        pb::merge_branch_response::Status::Applied as i32,
        "{applied:?}"
    );
    let landed = get(&service, "recreated-on-branch").await;
    assert_eq!(landed, recreated);
    assert!(resolve_connection(&landed, &selected).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn recreated_baseline_owner_is_not_automatically_merged_with_a_stale_branch() {
    let service = service().await;
    let original = save(&service, screen("stale-owner")).await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut edit = original.clone();
    ui(&mut edit).doc = "branch edit".into();
    service.put_entity(on_branch(put(edit))).await.unwrap();
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("stale-owner")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut replacement = original;
    ui(&mut replacement).title = "baseline replacement".into();
    let replacement = save(&service, replacement).await;
    let merge = service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: "identity-work".into(),
            auto_merge: true,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merge.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    assert!(merge.conflicts[0].auto_merged.is_none());
    assert!(merge.conflicts[0]
        .conflict_field_paths
        .contains(&"system.uid".into()));
    assert_eq!(get(&service, "stale-owner").await, replacement);
}

#[tokio::test(flavor = "multi_thread")]
async fn independent_identical_creates_require_an_explicit_incarnation_choice() {
    let service = service().await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut entity = screen("independent-owner");
    ui(&mut entity).transitions.clear();
    let branch = service
        .put_entity(on_branch(put(entity.clone())))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let baseline = save(&service, entity).await;
    assert!(trogon_atlas_core::semantically_equal(&branch, &baseline));
    assert_ne!(
        branch.system.as_ref().unwrap().uid,
        baseline.system.as_ref().unwrap().uid
    );
    let diff = service
        .diff_branch(Request::new(pb::DiffBranchRequest {
            name: "identity-work".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(diff.entries.len(), 1);
    let entry = &diff.entries[0];
    assert_eq!(
        entry.status,
        pb::branch_diff_entry::Status::ConflictEditEdit as i32
    );
    assert!(entry.auto_merged.is_none());
    assert!(entry.conflict_field_paths.contains(&"system.uid".into()));
    let blocked = service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: "identity-work".into(),
            auto_merge: true,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        blocked.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    assert_eq!(get(&service, "independent-owner").await, baseline);
    service
        .resolve_branch_entry(Request::new(pb::ResolveBranchEntryRequest {
            name: "identity-work".into(),
            r#ref: entry.r#ref.clone(),
            resolution: pb::resolve_branch_entry_request::Resolution::TakeTheirs as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let selected = service
        .get_entity(on_branch(pb::GetEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("independent-owner")),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(selected, baseline);
    let merged = service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merged.status,
        pb::merge_branch_response::Status::Applied as i32
    );
    assert_eq!(merged.applied_count, 1);
    assert_eq!(get(&service, "independent-owner").await, baseline);
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_baseline_content_rejects_removed_branch_ids() {
    let service = service().await;
    let original = save(&service, screen("resolved-owner")).await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut branch = original.clone();
    ui(&mut branch).transitions.push(pb::ui::Transition {
        to: Some(pb::UiRef {
            id: Some(id("destination")),
        }),
        ..Default::default()
    });
    let mut branch = service
        .put_entity(on_branch(put(branch)))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let removed = ui(&mut branch).transitions.pop().unwrap();
    service.put_entity(on_branch(put(branch))).await.unwrap();
    let mut baseline = original;
    ui(&mut baseline).doc = "new baseline description".into();
    save(&service, baseline).await;
    service
        .resolve_branch_entry(Request::new(pb::ResolveBranchEntryRequest {
            name: "identity-work".into(),
            r#ref: Some(pb::EntityRef {
                kind: pb::EntityKind::Ui as i32,
                id: Some(id("resolved-owner")),
            }),
            resolution: pb::resolve_branch_entry_request::Resolution::TakeTheirs as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut resolved = service
        .get_entity(on_branch(pb::GetEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("resolved-owner")),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    ui(&mut resolved).transitions.push(removed);
    assert_eq!(
        service
            .put_entity(on_branch(put(resolved)))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
}

async fn resolve_with_removed_selected_connection(take_theirs: bool) {
    let service = service().await;
    let mut destination = screen("destination");
    ui(&mut destination).transitions.clear();
    save(&service, destination).await;
    let original = save(&service, screen("resolution-renewal")).await;
    let selected = reference(&original, 0);
    let unaffected = reference(&original, 1);
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut branch = original.clone();
    ui(&mut branch).doc = "branch choice".into();
    if take_theirs {
        ui(&mut branch).transitions.remove(0);
    }
    service.put_entity(on_branch(put(branch))).await.unwrap();
    let mut baseline = original.clone();
    ui(&mut baseline).doc = "baseline choice".into();
    if !take_theirs {
        ui(&mut baseline).transitions.remove(0);
    }
    let baseline = save(&service, baseline).await;
    service
        .resolve_branch_entry(Request::new(pb::ResolveBranchEntryRequest {
            name: "identity-work".into(),
            r#ref: selected.owner.clone(),
            resolution: if take_theirs {
                pb::resolve_branch_entry_request::Resolution::TakeTheirs
            } else {
                pb::resolve_branch_entry_request::Resolution::KeepOurs
            } as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut resolved = service
        .get_entity(on_branch(pb::GetEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("resolution-renewal")),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(ui(&mut resolved).transitions.len(), 2);
    assert_eq!(
        ui(&mut resolved).doc,
        if take_theirs {
            "baseline choice"
        } else {
            "branch choice"
        }
    );
    assert_eq!(
        resolved.system.as_ref().unwrap().uid,
        original.system.as_ref().unwrap().uid
    );
    assert!(resolve_connection(&resolved, &selected).is_err());
    assert!(resolve_connection(&resolved, &unaffected).is_ok());
    assert_eq!(get(&service, "resolution-renewal").await, baseline);
    let diff = service
        .diff_branch(Request::new(pb::DiffBranchRequest {
            name: "identity-work".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(diff.entries.len(), 1);
    assert_eq!(
        diff.entries[0].status,
        pb::branch_diff_entry::Status::Changed as i32
    );
    let merged = service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merged.status,
        pb::merge_branch_response::Status::Applied as i32,
        "{merged:?}"
    );
    let landed = get(&service, "resolution-renewal").await;
    assert!(resolve_connection(&landed, &selected).is_err());
    assert!(resolve_connection(&landed, &unaffected).is_ok());
    assert_eq!(
        connection_occurrences(&landed).unwrap(),
        connection_occurrences(&resolved).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn keep_ours_renews_connections_removed_by_baseline() {
    resolve_with_removed_selected_connection(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn take_theirs_renews_connections_removed_by_branch() {
    resolve_with_removed_selected_connection(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn keep_ours_after_baseline_owner_deletion_preserves_branch_connection_identity() {
    let service = service().await;
    let mut destination = screen("destination");
    ui(&mut destination).transitions.clear();
    save(&service, destination).await;
    let original = save(&service, screen("surviving-owner")).await;
    let references = [reference(&original, 0), reference(&original, 1)];
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut branch = original;
    ui(&mut branch).doc = "branch documentation edit".into();
    let branch = service
        .put_entity(on_branch(put(branch)))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("surviving-owner")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let diff = service
        .diff_branch(Request::new(pb::DiffBranchRequest {
            name: "identity-work".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(diff.entries.len(), 1);
    assert_eq!(
        diff.entries[0].status,
        pb::branch_diff_entry::Status::ConflictDeleteEdit as i32
    );
    let blocked = service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: "identity-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        blocked.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    service
        .resolve_branch_entry(Request::new(pb::ResolveBranchEntryRequest {
            name: "identity-work".into(),
            r#ref: references[0].owner.clone(),
            resolution: pb::resolve_branch_entry_request::Resolution::KeepOurs as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let resolved = service
        .get_entity(on_branch(pb::GetEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("surviving-owner")),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(
        resolved.system.as_ref().unwrap().uid,
        branch.system.as_ref().unwrap().uid
    );
    assert_eq!(
        connection_occurrences(&resolved).unwrap(),
        connection_occurrences(&branch).unwrap()
    );
    for reference in &references {
        assert!(resolve_connection(&resolved, reference).is_ok());
    }
    for dry_run in [true, false] {
        let merged = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "identity-work".into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            merged.status,
            if dry_run {
                pb::merge_branch_response::Status::Validated
            } else {
                pb::merge_branch_response::Status::Applied
            } as i32,
            "{merged:?}"
        );
    }
    let landed = get(&service, "surviving-owner").await;
    assert_eq!(
        landed.system.as_ref().unwrap().uid,
        branch.system.as_ref().unwrap().uid
    );
    assert_eq!(
        connection_occurrences(&landed).unwrap(),
        connection_occurrences(&branch).unwrap()
    );
    for reference in &references {
        assert!(resolve_connection(&landed, reference).is_ok());
    }
}
