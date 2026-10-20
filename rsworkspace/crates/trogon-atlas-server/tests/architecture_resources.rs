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

fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}

fn read_model(slug: &str) -> pb::Entity {
    entity(pb::entity::Kind::ReadModel(pb::ReadModel {
        id: Some(id(slug)),
        title: slug.into(),
        ..Default::default()
    }))
}

fn store(slug: &str, target: &str) -> pb::Entity {
    entity(pb::entity::Kind::DataStore(pb::DataStore {
        id: Some(id(slug)),
        title: slug.into(),
        store_class: Some(pb::StoreClass {
            kind: Some(pb::store_class::Kind::Known(
                pb::store_class::Known::Cache as i32,
            )),
        }),
        authoritative: Some(false),
        attachments: vec![pb::ReadModelAttachment {
            read_model: Some(pb::ReadModelRef {
                id: Some(id(target)),
            }),
            ..Default::default()
        }],
        ..Default::default()
    }))
}

fn automation() -> pb::Entity {
    entity(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
        id: Some(id("observe-orders")),
        title: "Observe orders".into(),
        source_read_models: vec![
            pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("order-feed"))
                }),
                ..Default::default()
            };
            2
        ],
        processor: Some(pb::ProcessorEdge {
            processor: Some(pb::ProcessorRef {
                id: Some(id("order-intake")),
            }),
            ..Default::default()
        }),
        ..Default::default()
    }))
}

async fn service() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(0)
}

async fn put(service: &EventModelServiceImpl, entity: pb::Entity) -> pb::PutEntityResponse {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
}

async fn get(
    service: &EventModelServiceImpl,
    kind: pb::EntityKind,
    slug: &str,
) -> pb::GetEntityResponse {
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(id(slug)),
        }))
        .await
        .unwrap()
        .into_inner()
}

async fn fixture(service: &EventModelServiceImpl) -> (pb::Entity, pb::Entity) {
    put(service, read_model("order-feed")).await;
    put(
        service,
        entity(pb::entity::Kind::Processor(pb::Processor {
            id: Some(id("order-intake")),
            title: "Order intake".into(),
            ..Default::default()
        })),
    )
    .await;
    let owner = put(service, automation()).await.entity.unwrap();
    let pb::entity::Kind::AutomationSlice(slice) = owner.kind.as_ref().unwrap() else {
        panic!()
    };
    let uid = owner.system.as_ref().unwrap().uid.clone();
    let channel = entity(pb::entity::Kind::Channel(pb::Channel {
        id: Some(id("order-delivery")),
        title: "Order delivery".into(),
        protocols: vec![pb::ProtocolName {
            value: "nats".into(),
        }],
        delivery_guarantee: Some(pb::channel::DeliveryGuarantee::AtLeastOnce as i32),
        bindings: slice
            .source_read_models
            .iter()
            .map(|edge| pb::ChannelBinding {
                observation: Some(pb::ConnectionRef {
                    owner: Some(pb::EntityRef {
                        kind: pb::EntityKind::AutomationSlice as i32,
                        id: slice.id.clone(),
                    }),
                    owner_uid: uid.clone(),
                    id: edge.connection_id.clone(),
                }),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }));
    (owner, channel)
}

#[tokio::test(flavor = "multi_thread")]
async fn resources_preserve_occurrence_identity_and_optional_false() {
    let service = service().await;
    let (_, channel) = fixture(&service).await;
    let channel = put(&service, channel).await;
    let mut edited = channel.entity.clone().unwrap();
    let pb::entity::Kind::Channel(value) = edited.kind.as_mut().unwrap() else {
        panic!()
    };
    let ids: Vec<_> = value
        .bindings
        .iter()
        .map(|binding| binding.connection_id.clone().unwrap())
        .collect();
    assert_ne!(ids[0], ids[1]);
    value.bindings.reverse();
    value.doc = "Transport documentation".into();
    let result = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(edited),
            if_match: channel.etag,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    let pb::entity::Kind::Channel(value) = result.entity.unwrap().kind.unwrap() else {
        panic!()
    };
    assert_eq!(value.bindings[0].connection_id.as_ref(), Some(&ids[1]));
    assert_eq!(value.bindings[1].connection_id.as_ref(), Some(&ids[0]));
    let mut storage = store("feed-cache", "order-feed");
    if let Some(pb::entity::Kind::DataStore(value)) = storage.kind.as_mut() {
        value.retention = Some(pb::RetentionPolicy {
            policy: Some(pb::PolicyText {
                value: "Retain according to business policy".into(),
            }),
            duration: None,
        });
        value.residency = Some(pb::ResidencyPolicy {
            policy: Some(pb::PolicyText {
                value: "Keep in approved regions".into(),
            }),
            regions: None,
        });
    }
    let stored = put(&service, storage).await.entity.unwrap();
    let pb::entity::Kind::DataStore(value) = stored.kind.unwrap() else {
        panic!()
    };
    assert_eq!(value.authoritative, Some(false));
    assert!(value.attachments[0].connection_id.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn native_resource_value_and_anchor_failures_reject_public_writes() {
    let service = service().await;
    let (owner, channel) = fixture(&service).await;
    let mut invalid = Vec::new();
    let mut wrong_edge = channel.clone();
    if let (
        Some(pb::entity::Kind::Channel(channel)),
        Some(pb::entity::Kind::AutomationSlice(owner)),
    ) = (wrong_edge.kind.as_mut(), owner.kind.as_ref())
    {
        channel.bindings[0].observation.as_mut().unwrap().id =
            owner.processor.as_ref().unwrap().connection_id.clone();
    }
    invalid.push(wrong_edge);
    let mut wrong_kind = channel.clone();
    if let Some(pb::entity::Kind::Channel(channel)) = wrong_kind.kind.as_mut() {
        channel.bindings[0]
            .observation
            .as_mut()
            .unwrap()
            .owner
            .as_mut()
            .unwrap()
            .kind = pb::EntityKind::ReadModelSlice as i32;
    }
    invalid.push(wrong_kind);
    let mut unknown_occurrence = channel.clone();
    if let Some(pb::entity::Kind::Channel(channel)) = unknown_occurrence.kind.as_mut() {
        channel.bindings[0].observation.as_mut().unwrap().id =
            Some(trogon_atlas_core::connections::ConnectionId::new().into());
    }
    invalid.push(unknown_occurrence);
    let mut value = channel.clone();
    if let Some(pb::entity::Kind::Channel(channel)) = value.kind.as_mut() {
        channel.bindings.push(channel.bindings[0].clone());
    }
    invalid.push(value);
    let mut value = channel.clone();
    if let Some(pb::entity::Kind::Channel(channel)) = value.kind.as_mut() {
        channel.bindings[0].observation.as_mut().unwrap().owner_uid = "guessed-incarnation".into();
    }
    invalid.push(value);
    let mut value = channel.clone();
    if let Some(pb::entity::Kind::Channel(channel)) = value.kind.as_mut() {
        channel.protocols[0].value = "NATS".into();
    }
    invalid.push(value);
    let mut value = channel.clone();
    if let Some(pb::entity::Kind::Channel(channel)) = value.kind.as_mut() {
        channel.delivery_guarantee = Some(999);
    }
    invalid.push(value);
    let mut value = channel.clone();
    if let Some(pb::entity::Kind::Channel(channel)) = value.kind.as_mut() {
        channel.bindings.clear();
    }
    invalid.push(value);
    let mut value = store("feed-cache", "order-feed");
    if let Some(pb::entity::Kind::DataStore(store)) = value.kind.as_mut() {
        store.attachments.push(store.attachments[0].clone());
    }
    invalid.push(value);
    let mut value = store("feed-cache", "order-feed");
    if let Some(pb::entity::Kind::DataStore(store)) = value.kind.as_mut() {
        store.store_class = None;
    }
    invalid.push(value);
    let mut value = store("feed-cache", "order-feed");
    if let Some(pb::entity::Kind::DataStore(store)) = value.kind.as_mut() {
        store.retention = Some(pb::RetentionPolicy {
            policy: Some(pb::PolicyText {
                value: "Retain briefly".into(),
            }),
            duration: Some(pb::PositiveDuration { seconds: 0 }),
        });
    }
    invalid.push(value);
    for case in 0..7 {
        let mut value = store("feed-cache", "order-feed");
        let Some(pb::entity::Kind::DataStore(store)) = value.kind.as_mut() else {
            panic!()
        };
        match case {
            0 => {
                store.technology = Some(pb::TechnologyName { value: " ".into() });
            }
            1 => {
                store.store_class = Some(pb::StoreClass {
                    kind: Some(pb::store_class::Kind::Other(" ".into())),
                });
            }
            2 => {
                store.access_mode = Some(999);
            }
            3 => {
                store.classification = Some(999);
            }
            4 => {
                store.residency = Some(pb::ResidencyPolicy {
                    policy: Some(pb::PolicyText {
                        value: "Keep local".into(),
                    }),
                    regions: Some(pb::RegionCodes { values: vec![] }),
                });
            }
            5 => {
                store.residency = Some(pb::ResidencyPolicy {
                    policy: Some(pb::PolicyText {
                        value: "Keep local".into(),
                    }),
                    regions: Some(pb::RegionCodes {
                        values: vec![pb::RegionCode { value: " ".into() }],
                    }),
                });
            }
            _ => {
                store.attachments.clear();
            }
        }
        invalid.push(value);
    }
    for (index, entity) in invalid.into_iter().enumerate() {
        let error = service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "case {index}: {error}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn guarded_batch_resolves_new_read_model_but_rejects_guessed_owner_uid_atomically() {
    let service = service().await;
    let ops = [read_model("batch-feed"), store("batch-cache", "batch-feed")]
        .into_iter()
        .map(|entity| pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            })),
        })
        .collect();
    let response = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        response.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
    get(&service, pb::EntityKind::DataStore, "batch-cache").await;
    let (_, mut channel) = fixture(&service).await;
    let mut owner = automation();
    if let Some(pb::entity::Kind::AutomationSlice(slice)) = owner.kind.as_mut() {
        slice.id = Some(id("new-owner"));
    }
    if let Some(pb::entity::Kind::Channel(value)) = channel.kind.as_mut() {
        for binding in &mut value.bindings {
            let reference = binding.observation.as_mut().unwrap();
            reference.owner.as_mut().unwrap().id = Some(id("new-owner"));
            reference.owner_uid = "predicted-uid".into();
        }
    }
    let ops = [owner, channel]
        .into_iter()
        .map(|entity| pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            })),
        })
        .collect();
    let error = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::AutomationSlice as i32,
                id: Some(id("new-owner"))
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn safe_delete_blocks_anchors_and_recreation_obeys_reference_kind() {
    let service = service().await;
    let (owner, channel) = fixture(&service).await;
    let channel = put(&service, channel).await.entity.unwrap();
    let storage = put(&service, store("feed-cache", "order-feed"))
        .await
        .entity
        .unwrap();
    for (kind, slug) in [
        (pb::EntityKind::AutomationSlice, "observe-orders"),
        (pb::EntityKind::ReadModel, "order-feed"),
    ] {
        let error = service
            .delete_entity(Request::new(pb::DeleteEntityRequest {
                kind: kind as i32,
                id: Some(id(slug)),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::FailedPrecondition);
        for validate_only in [true, false] {
            let response = service
                .batch_mutate(Request::new(pb::BatchMutateRequest {
                    ops: vec![pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                            kind: kind as i32,
                            id: Some(id(slug)),
                            ..Default::default()
                        })),
                    }],
                    validate_only,
                    ..Default::default()
                }))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(
                response.status,
                pb::batch_mutate_response::Status::Failed as i32
            );
        }
    }
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id("order-feed")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(storage.clone()),
            validate_only: true,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    put(&service, read_model("order-feed")).await;
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(storage),
            validate_only: true,
            ..Default::default()
        }))
        .await
        .unwrap();
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::AutomationSlice as i32,
            id: Some(id("observe-orders")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut replacement = owner;
    replacement.system = None;
    put(&service, replacement).await;
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(channel),
            validate_only: true,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn address_defaults_and_explicit_values_are_checked() {
    let service = service().await;
    let (_, channel) = fixture(&service).await;
    let mut channel = channel;
    let Some(pb::entity::Kind::Channel(value)) = channel.kind.as_mut() else {
        panic!()
    };
    value.address = Some(pb::ChannelAddress {
        template: "orders.{region}".into(),
        parameters: vec![pb::AddressParameter {
            name: Some(pb::ParameterName {
                value: "region".into(),
            }),
            allowed_values: Some(pb::StringValues {
                values: vec!["east".into(), "west".into()],
            }),
            default: Some("east".into()),
            ..Default::default()
        }],
    });
    value.bindings[1]
        .parameter_values
        .insert("region".into(), "west".into());
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(channel.clone()),
            validate_only: true,
            ..Default::default()
        }))
        .await
        .unwrap();
    for case in 0..6 {
        let mut invalid = channel.clone();
        let Some(pb::entity::Kind::Channel(value)) = invalid.kind.as_mut() else {
            panic!()
        };
        match case {
            0 => {
                value.bindings[0]
                    .parameter_values
                    .insert("region".into(), "north".into());
            }
            1 => {
                value.bindings[0]
                    .parameter_values
                    .insert("unknown".into(), "east".into());
            }
            2 => {
                value.address.as_mut().unwrap().template = "orders.{region".into();
            }
            3 => {
                value.address.as_mut().unwrap().parameters[0].default = None;
            }
            4 => {
                value.address.as_mut().unwrap().parameters[0].examples = vec!["north".into()];
            }
            _ => {
                value.address = None;
            }
        }
        let error = service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(invalid),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "case {case}: {error}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_preview_and_land_preserve_resource_association_ids() {
    let service = service().await;
    let (_, channel) = fixture(&service).await;
    let created = put(&service, channel).await.entity.unwrap();
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "resource-edit".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut read = Request::new(pb::GetEntityRequest {
        kind: pb::EntityKind::Channel as i32,
        id: Some(id("order-delivery")),
    });
    read.metadata_mut()
        .insert("x-trogon-atlas-branch", "resource-edit".parse().unwrap());
    let current = service.get_entity(read).await.unwrap().into_inner();
    let mut owner_read = Request::new(pb::GetEntityRequest {
        kind: pb::EntityKind::AutomationSlice as i32,
        id: Some(id("observe-orders")),
    });
    owner_read
        .metadata_mut()
        .insert("x-trogon-atlas-branch", "resource-edit".parse().unwrap());
    let owner_current = service.get_entity(owner_read).await.unwrap().into_inner();
    let mut owner_changed = owner_current.entity.unwrap();
    if let Some(pb::entity::Kind::AutomationSlice(owner)) = owner_changed.kind.as_mut() {
        owner.source_read_models.push(pb::ReadModelEdge {
            read_model: Some(pb::ReadModelRef {
                id: Some(id("order-feed")),
            }),
            ..Default::default()
        });
    }
    let mut owner_write = Request::new(pb::PutEntityRequest {
        entity: Some(owner_changed),
        if_match: owner_current.etag,
        ..Default::default()
    });
    owner_write
        .metadata_mut()
        .insert("x-trogon-atlas-branch", "resource-edit".parse().unwrap());
    let owner_changed = service
        .put_entity(owner_write)
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let Some(pb::entity::Kind::AutomationSlice(owner)) = owner_changed.kind.as_ref() else {
        panic!()
    };
    let new_observation = pb::ConnectionRef {
        owner: Some(pb::EntityRef {
            kind: pb::EntityKind::AutomationSlice as i32,
            id: owner.id.clone(),
        }),
        owner_uid: owner_changed.system.as_ref().unwrap().uid.clone(),
        id: owner
            .source_read_models
            .last()
            .unwrap()
            .connection_id
            .clone(),
    };
    let mut changed = current.entity.unwrap();
    if let Some(pb::entity::Kind::Channel(channel)) = changed.kind.as_mut() {
        channel.doc = "Branch transport declaration".into();
        channel.bindings.reverse();
        channel.bindings.push(pb::ChannelBinding {
            observation: Some(new_observation),
            ..Default::default()
        });
    }
    let mut write = Request::new(pb::PutEntityRequest {
        entity: Some(changed.clone()),
        if_match: current.etag,
        ..Default::default()
    });
    write
        .metadata_mut()
        .insert("x-trogon-atlas-branch", "resource-edit".parse().unwrap());
    let changed = service
        .put_entity(write)
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    for dry_run in [true, false] {
        let merged = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "resource-edit".into(),
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
        let baseline = get(&service, pb::EntityKind::Channel, "order-delivery")
            .await
            .entity
            .unwrap();
        assert_eq!(
            baseline.kind,
            if dry_run {
                created.kind.clone()
            } else {
                changed.kind.clone()
            }
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_observation_owners_and_store_attachments_are_discoverable() {
    let nats = trogon_atlas_testsupport::shared().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default())
            .unwrap()
            .with_snapshot_cap(0);
    let (_, channel) = fixture(&service).await;
    put(&service, channel).await;
    put(&service, store("feed-cache", "order-feed")).await;
    let observations = service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::AutomationSlice as i32,
            id: Some(id("observe-orders")),
            filter_kinds: vec![pb::EntityKind::Channel as i32],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references;
    assert_eq!(observations.len(), 2);
    assert_ne!(observations[0].from_field, observations[1].from_field);
    assert!(observations.iter().all(|reference| reference
        .from
        .as_ref()
        .and_then(|owner| owner.id.as_ref())
        == Some(&id("order-delivery"))));
    let stores = service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id("order-feed")),
            filter_kinds: vec![pb::EntityKind::DataStore as i32],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references;
    assert_eq!(stores.len(), 1);
    assert_eq!(
        stores[0].from.as_ref().and_then(|owner| owner.id.as_ref()),
        Some(&id("feed-cache"))
    );
    let impact = service
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(pb::EntityRef {
                kind: pb::EntityKind::AutomationSlice as i32,
                id: Some(id("observe-orders")),
            }),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(impact.nodes.iter().any(|node| node
        .entity
        .as_ref()
        .and_then(|entity| entity.id.as_ref())
        == Some(&id("order-delivery"))));
}

#[tokio::test(flavor = "multi_thread")]
async fn authorized_resource_writes_check_derived_endpoint_visibility() {
    use pb::{
        event_model_service_client::EventModelServiceClient,
        event_model_service_server::EventModelServiceServer,
    };
    use tonic::service::interceptor::InterceptedService;
    use trogon_atlas_server::auth::{AuthzLayer, BearerAuth, TokenRegistry};

    fn authenticated<T>(body: T) -> Request<T> {
        let mut request = Request::new(body);
        request.metadata_mut().insert(
            "authorization",
            "Bearer resource-scope-writer".parse().unwrap(),
        );
        request
    }
    fn scoped_id(namespace: &str, slug: &str) -> pb::Id {
        pb::Id {
            namespace: namespace.into(),
            slug: slug.into(),
            version: 1,
        }
    }
    let nats = trogon_atlas_testsupport::shared().await;
    let store_backend = Arc::new(nats.store().await);
    let seed =
        EventModelServiceImpl::try_new(store_backend.clone(), ServerMetrics::default()).unwrap();
    let mut namespace_ids = Vec::new();
    for (name, parent) in [
        ("commerce", "acme"),
        ("shared", "acme"),
        ("restricted", "beta"),
    ] {
        namespace_ids.push(
            seed.register_namespace(Request::new(pb::RegisterNamespaceRequest {
                name: name.into(),
                parent: parent.into(),
            }))
            .await
            .unwrap()
            .into_inner()
            .namespace
            .unwrap()
            .id,
        );
    }
    let commerce = &namespace_ids[0];
    let shared = &namespace_ids[1];
    let restricted = &namespace_ids[2];
    for namespace in [shared, restricted] {
        put(
            &seed,
            entity(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(scoped_id(namespace, "feed")),
                title: "Private target display text".into(),
                ..Default::default()
            })),
        )
        .await;
        put(
            &seed,
            entity(pb::entity::Kind::Processor(pb::Processor {
                id: Some(scoped_id(namespace, "processor")),
                title: "Private processor display text".into(),
                ..Default::default()
            })),
        )
        .await;
    }
    let mut channels = Vec::new();
    for (index, (read_namespace, processor_namespace)) in
        [(restricted, shared), (shared, restricted), (shared, shared)]
            .into_iter()
            .enumerate()
    {
        let owner = put(
            &seed,
            entity(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
                id: Some(scoped_id(commerce, &format!("observation-{index}"))),
                title: "Observed input".into(),
                source_read_models: vec![pb::ReadModelEdge {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(scoped_id(read_namespace, "feed")),
                    }),
                    ..Default::default()
                }],
                processor: Some(pb::ProcessorEdge {
                    processor: Some(pb::ProcessorRef {
                        id: Some(scoped_id(processor_namespace, "processor")),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        )
        .await
        .entity
        .unwrap();
        let Some(pb::entity::Kind::AutomationSlice(slice)) = owner.kind.as_ref() else {
            panic!()
        };
        channels.push(entity(pb::entity::Kind::Channel(pb::Channel {
            id: Some(scoped_id(commerce, &format!("channel-{index}"))),
            title: "Declared transport".into(),
            bindings: vec![pb::ChannelBinding {
                observation: Some(pb::ConnectionRef {
                    owner: Some(pb::EntityRef {
                        kind: pb::EntityKind::AutomationSlice as i32,
                        id: slice.id.clone(),
                    }),
                    owner_uid: owner.system.as_ref().unwrap().uid.clone(),
                    id: slice.source_read_models[0].connection_id.clone(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })));
    }
    let registry_text = format!(
        r#"
[principals.resource-writer]
role = "writer"
tokens = ["resource-scope-writer"]
parent = "acme"
namespaces = ["{commerce}"]
"#
    );
    let registry =
        toml::from_str::<trogon_atlas_server::auth::RegistryFile>(&registry_text).unwrap();
    let svc = EventModelServiceImpl::try_new(store_backend, ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(0);
    let grpc = EventModelServiceServer::new(svc);
    let data_plane = InterceptedService::new(
        tower::ServiceBuilder::new().layer(AuthzLayer).service(grpc),
        BearerAuth::new(Arc::new(
            TokenRegistry::from_file_contents(registry).unwrap(),
        )),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let mut client = EventModelServiceClient::connect(format!("http://{address}"))
        .await
        .unwrap();
    for (index, channel) in channels.into_iter().enumerate() {
        let result = client
            .put_entity(authenticated(pb::PutEntityRequest {
                entity: Some(channel),
                ..Default::default()
            }))
            .await;
        if index < 2 {
            let error = result.unwrap_err();
            assert_eq!(error.code(), Code::NotFound);
            assert!(!error.message().contains("Private target display text"));
            assert!(!error.message().contains("Private processor display text"));
        } else {
            result.unwrap();
        }
    }
    client
        .get_entity(authenticated(pb::GetEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(scoped_id(shared, "feed")),
        }))
        .await
        .unwrap();
    for (index, target_namespace) in [restricted, shared].into_iter().enumerate() {
        let mut storage = store(&format!("storage-{index}"), "feed");
        let Some(pb::entity::Kind::DataStore(value)) = storage.kind.as_mut() else {
            panic!()
        };
        value.id = Some(scoped_id(commerce, &format!("storage-{index}")));
        value.attachments[0].read_model.as_mut().unwrap().id =
            Some(scoped_id(target_namespace, "feed"));
        let result = client
            .put_entity(authenticated(pb::PutEntityRequest {
                entity: Some(storage),
                ..Default::default()
            }))
            .await;
        if index == 0 {
            assert_eq!(result.unwrap_err().code(), Code::NotFound);
        } else {
            result.unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn resource_supersession_requires_an_existing_prior_exact_family_version() {
    let service = service().await;
    put(&service, read_model("order-feed")).await;
    put(&service, store("feed-cache", "order-feed")).await;
    let mut next = store("feed-cache", "order-feed");
    if let Some(pb::entity::Kind::DataStore(store)) = next.kind.as_mut() {
        store.id.as_mut().unwrap().version = 2;
        store.supersedes = Some(id("feed-cache"));
    }
    for case in 0..4 {
        let mut invalid = next.clone();
        let Some(pb::entity::Kind::DataStore(store)) = invalid.kind.as_mut() else {
            panic!()
        };
        let prior = store.supersedes.as_mut().unwrap();
        match case {
            0 => {
                prior.slug = "another-store".into();
            }
            1 => {
                prior.version = 3;
            }
            2 => {
                prior.version = 0;
            }
            _ => {
                prior.namespace = "another-namespace".into();
            }
        }
        let error = service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(invalid),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
    }
    put(&service, next).await;
    let mut missing = store("missing-family", "order-feed");
    if let Some(pb::entity::Kind::DataStore(store)) = missing.kind.as_mut() {
        store.id.as_mut().unwrap().version = 2;
        store.supersedes = Some(id("missing-family"));
    }
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(missing),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
}
