#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::{Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "commerce".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn reference(kind: pb::EntityKind, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: kind as i32,
        id: Some(id(slug)),
    }
}

async fn put(service: &EventModelServiceImpl, kind: pb::entity::Kind) -> pb::Entity {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(kind),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

async fn get_store(service: &EventModelServiceImpl) -> pb::GetEntityResponse {
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::DataStore as i32,
            id: Some(id("order-cache")),
        }))
        .await
        .unwrap()
        .into_inner()
}

async fn incoming(service: &EventModelServiceImpl, slug: &str) -> Vec<pb::Reference> {
    service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id(slug)),
            filter_kinds: vec![pb::EntityKind::DataStore as i32],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references
}

async fn fixture() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default())
            .unwrap()
            .with_snapshot_cap(0);
    for slug in ["ready", "sent", "pending"] {
        put(
            &service,
            pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            }),
        )
        .await;
    }
    put(
        &service,
        pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(id("order-cache")),
            title: "Order cache".into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Known(
                    pb::store_class::Known::Cache as i32,
                )),
            }),
            attachments: ["ready", "sent"]
                .into_iter()
                .map(|slug| pb::ReadModelAttachment {
                    read_model: Some(pb::ReadModelRef { id: Some(id(slug)) }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }),
    )
    .await;
    service
}

fn retarget(to: &str, dry_run: bool) -> pb::RetargetReferencesRequest {
    pb::RetargetReferencesRequest {
        from: Some(reference(pb::EntityKind::ReadModel, "ready")),
        to: Some(reference(pb::EntityKind::ReadModel, to)),
        filter_kinds: vec![pb::EntityKind::DataStore as i32],
        dry_run,
    }
}

#[tokio::test]
async fn retarget_rejects_duplicate_store_attachments_in_preview_and_apply() {
    let service = fixture().await;
    let before = get_store(&service).await;
    let ready_refs = incoming(&service, "ready").await;
    let sent_refs = incoming(&service, "sent").await;
    assert_eq!(ready_refs.len(), 1);
    assert_eq!(sent_refs.len(), 1);
    for dry_run in [true, false] {
        let error = service
            .retarget_references(Request::new(retarget("sent", dry_run)))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
        assert_eq!(get_store(&service).await, before);
        assert_eq!(incoming(&service, "ready").await, ready_refs);
        assert_eq!(incoming(&service, "sent").await, sent_refs);
    }
}

#[tokio::test]
async fn valid_store_retarget_renews_changed_association_and_preserves_other_attachment() {
    let service = fixture().await;
    let before = get_store(&service).await;
    let Some(pb::entity::Kind::DataStore(old)) = before.entity.as_ref().unwrap().kind.as_ref()
    else {
        panic!("DataStore")
    };
    let old_ids: Vec<_> = old
        .attachments
        .iter()
        .map(|a| a.connection_id.clone())
        .collect();
    let preview = service
        .retarget_references(Request::new(retarget("pending", true)))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(preview.rewritten, 1);
    assert_eq!(preview.failed, 0);
    assert_eq!(get_store(&service).await, before);
    let applied = service
        .retarget_references(Request::new(retarget("pending", false)))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(applied.rewritten, 1);
    assert_eq!(applied.failed, 0);
    let after = get_store(&service).await.entity.unwrap();
    let Some(pb::entity::Kind::DataStore(new)) = after.kind else {
        panic!("DataStore")
    };
    assert_eq!(
        new.attachments[0].read_model.as_ref().unwrap().id,
        Some(id("pending"))
    );
    assert_ne!(new.attachments[0].connection_id, old_ids[0]);
    assert_eq!(new.attachments[1].read_model, old.attachments[1].read_model);
    assert_eq!(new.attachments[1].connection_id, old_ids[1]);
    assert_eq!(
        incoming(&service, "ready").await,
        Vec::<pb::Reference>::new()
    );
    assert_eq!(incoming(&service, "sent").await.len(), 1);
    assert_eq!(incoming(&service, "pending").await.len(), 1);
}

#[tokio::test]
async fn reverting_store_edit_rejects_missing_anchor_in_preview_and_apply() {
    let service = fixture().await;
    let current = get_store(&service).await;
    let mut edited = current.entity.unwrap();
    let Some(pb::entity::Kind::DataStore(resource)) = edited.kind.as_mut() else {
        panic!("DataStore")
    };
    resource.doc = "Design rationale".into();
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(edited),
            if_match: current.etag,
            ..Default::default()
        }))
        .await
        .unwrap();
    let revision = service
        .get_entity_history(Request::new(pb::GetEntityHistoryRequest {
            r#ref: Some(reference(pb::EntityKind::DataStore, "order-cache")),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .revisions
        .remove(0);
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id("ready")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let before = get_store(&service).await;
    for dry_run in [true, false] {
        let error = service
            .revert_changeset(Request::new(pb::RevertChangesetRequest {
                id: revision.changeset_id.clone(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
        assert_eq!(get_store(&service).await, before);
    }
}

fn on_branch<T>(mut request: Request<T>) -> Request<T> {
    request.metadata_mut().insert(
        "x-trogon-atlas-branch",
        "resource-conflict".parse().unwrap(),
    );
    request
}

#[tokio::test]
async fn taking_structural_merge_rejects_incompatible_channel_parameter_declarations() {
    let service = fixture().await;
    put(
        &service,
        pb::entity::Kind::Processor(pb::Processor {
            id: Some(id("observe")),
            title: "Observe orders".into(),
            ..Default::default()
        }),
    )
    .await;
    let owner = put(
        &service,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id("observe-ready")),
            title: "Observe ready".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("ready")),
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id("observe")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    let Some(pb::entity::Kind::AutomationSlice(slice)) = owner.kind.as_ref() else {
        panic!("AutomationSlice")
    };
    let channel = put(
        &service,
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(id("delivery")),
            title: "Delivery".into(),
            address: Some(pb::ChannelAddress {
                template: "orders.{region}".into(),
                parameters: vec![pb::AddressParameter {
                    name: Some(pb::ParameterName {
                        value: "region".into(),
                    }),
                    allowed_values: Some(pb::StringValues {
                        values: vec!["east".into(), "west".into()],
                    }),
                    ..Default::default()
                }],
            }),
            bindings: vec![pb::ChannelBinding {
                observation: Some(pb::ConnectionRef {
                    owner: Some(reference(pb::EntityKind::AutomationSlice, "observe-ready")),
                    owner_uid: owner.system.as_ref().unwrap().uid.clone(),
                    id: slice.source_read_models[0].connection_id.clone(),
                }),
                parameter_values: [("region".into(), "east".into())].into_iter().collect(),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "resource-conflict".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut ours = channel.clone();
    let Some(pb::entity::Kind::Channel(value)) = ours.kind.as_mut() else {
        panic!("Channel")
    };
    value.address.as_mut().unwrap().parameters[0]
        .allowed_values
        .as_mut()
        .unwrap()
        .values = vec!["east".into()];
    service
        .put_entity(on_branch(Request::new(pb::PutEntityRequest {
            entity: Some(ours),
            ..Default::default()
        })))
        .await
        .unwrap();
    let mut theirs = channel;
    let Some(pb::entity::Kind::Channel(value)) = theirs.kind.as_mut() else {
        panic!("Channel")
    };
    value.bindings[0]
        .parameter_values
        .insert("region".into(), "west".into());
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(theirs),
            ..Default::default()
        }))
        .await
        .unwrap();
    let diff = service
        .diff_branch(Request::new(pb::DiffBranchRequest {
            name: "resource-conflict".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(diff.entries.len(), 1);
    assert!(diff.entries[0].auto_merged.is_some(), "{diff:?}");
    let before = service
        .get_entity(on_branch(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Channel as i32,
            id: Some(id("delivery")),
        })))
        .await
        .unwrap()
        .into_inner();
    let error = service
        .resolve_branch_entry(Request::new(pb::ResolveBranchEntryRequest {
            name: "resource-conflict".into(),
            r#ref: Some(reference(pb::EntityKind::Channel, "delivery")),
            resolution: pb::resolve_branch_entry_request::Resolution::TakeMerged as i32,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    let after = service
        .get_entity(on_branch(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Channel as i32,
            id: Some(id("delivery")),
        })))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(after, before);
    let after_diff = service
        .diff_branch(Request::new(pb::DiffBranchRequest {
            name: "resource-conflict".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(after_diff, diff);
}

#[tokio::test]
async fn resource_put_retry_replays_applied_receipt_after_anchor_is_force_deleted() {
    let service = fixture().await;
    let mut resource = get_store(&service).await.entity.unwrap();
    let Some(pb::entity::Kind::DataStore(value)) = resource.kind.as_mut() else {
        panic!("DataStore")
    };
    value.id = Some(id("retry-cache"));
    let request = pb::PutEntityRequest {
        operation_id: "resource-write-retry".into(),
        entity: Some(resource),
        create_only: true,
        ..Default::default()
    };
    let first = service
        .put_entity(Request::new(request.clone()))
        .await
        .unwrap()
        .into_inner();
    let original = first.operation_receipt.unwrap();
    assert!(!original.replayed);
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id("ready")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let before = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::DataStore as i32,
            id: Some(id("retry-cache")),
        }))
        .await
        .unwrap()
        .into_inner();
    let replayed = service
        .put_entity(Request::new(request))
        .await
        .unwrap()
        .into_inner()
        .operation_receipt
        .unwrap();
    assert!(replayed.replayed);
    assert_eq!(replayed.changeset_id, original.changeset_id);
    let after = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::DataStore as i32,
            id: Some(id("retry-cache")),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(after, before);
}

#[tokio::test]
async fn resource_batch_retry_replays_applied_receipt_after_anchor_is_force_deleted() {
    let service = fixture().await;
    let mut resource = get_store(&service).await.entity.unwrap();
    let Some(pb::entity::Kind::DataStore(value)) = resource.kind.as_mut() else {
        panic!("DataStore")
    };
    value.id = Some(id("retry-batch-cache"));
    let request = pb::BatchMutateRequest {
        operation_id: "resource-batch-retry".into(),
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                entity: Some(resource),
                create_only: true,
                ..Default::default()
            })),
        }],
        ..Default::default()
    };
    let first = service
        .batch_mutate(Request::new(request.clone()))
        .await
        .unwrap()
        .into_inner();
    let original = first.operation_receipt.unwrap();
    assert!(!original.replayed);
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id("ready")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let before = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::DataStore as i32,
            id: Some(id("retry-batch-cache")),
        }))
        .await
        .unwrap()
        .into_inner();
    let replayed = service
        .batch_mutate(Request::new(request))
        .await
        .unwrap()
        .into_inner()
        .operation_receipt
        .unwrap();
    assert!(replayed.replayed);
    assert_eq!(replayed.changeset_id, original.changeset_id);
    let after = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::DataStore as i32,
            id: Some(id("retry-batch-cache")),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(after, before);
}
