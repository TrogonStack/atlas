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
async fn removed_ids_are_retired_and_owner_recreation_does_not_revive_references() {
    let service = service().await;
    let mut original = save(&service, screen("retirement")).await;
    let selected = reference(&original, 0);
    let removed = ui(&mut original).transitions.remove(0);
    let mut pruned = save(&service, original).await;
    assert!(resolve_connection(&pruned, &selected).is_err());
    ui(&mut pruned).transitions.push(removed.clone());
    assert_eq!(
        service
            .put_entity(Request::new(put(pruned)))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("retirement")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut recreated = screen("retirement");
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
async fn branch_merge_keeps_retired_ids_and_incarnation() {
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
    let branch = service
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
    assert_eq!(
        landed.system.as_ref().unwrap().retired_connection_ids,
        branch.system.as_ref().unwrap().retired_connection_ids
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
async fn taking_baseline_content_does_not_drop_branch_retirement_history() {
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
    assert!(resolved
        .system
        .as_ref()
        .unwrap()
        .retired_connection_ids
        .contains(removed.connection_id.as_ref().unwrap()));
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
