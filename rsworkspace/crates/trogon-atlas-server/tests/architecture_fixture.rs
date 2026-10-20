#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::HashMap, sync::Arc};

use pb::event_model_service_server::EventModelService;
use tonic::Request;
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

fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}

async fn put(service: &EventModelServiceImpl, kind: pb::entity::Kind) -> pb::Entity {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity(kind)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

async fn outgoing(
    service: &EventModelServiceImpl,
    kind: pb::EntityKind,
    slug: &str,
) -> Vec<pb::Reference> {
    service
        .get_outgoing_references(Request::new(pb::GetReferencesRequest {
            kind: kind as i32,
            id: Some(id(slug)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references
}

#[tokio::test(flavor = "multi_thread")]
async fn connected_transport_storage_fixture_is_queryable_and_exports_exact_associations() {
    let nats = trogon_atlas_testsupport::shared().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default())
            .unwrap();
    let mut expected = HashMap::new();
    for slug in ["order-feed", "order-view"] {
        let stored = put(
            &service,
            pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            }),
        )
        .await;
        expected.insert(
            pb::EntityKey::new(pb::EntityKind::ReadModel, &id(slug)),
            stored,
        );
    }
    for (kind, value) in [
        (
            pb::EntityKind::Processor,
            pb::entity::Kind::Processor(pb::Processor {
                id: Some(id("order-intake")),
                title: "Order intake".into(),
                ..Default::default()
            }),
        ),
        (
            pb::EntityKind::Command,
            pb::entity::Kind::Command(pb::Command {
                id: Some(id("import-order")),
                title: "Import order".into(),
                ..Default::default()
            }),
        ),
        (
            pb::EntityKind::Event,
            pb::entity::Kind::Event(pb::Event {
                id: Some(id("order-imported")),
                title: "Order imported".into(),
                ..Default::default()
            }),
        ),
    ] {
        let stored = put(&service, value).await;
        expected.insert(
            pb::EntityKey::new(kind, trogon_atlas_core::entity_id(&stored).unwrap()),
            stored,
        );
    }
    let automation = put(
        &service,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
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
            emitted_command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id("import-order")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    let Some(pb::entity::Kind::AutomationSlice(slice)) = automation.kind.as_ref() else {
        panic!()
    };
    let observations: Vec<_> = slice
        .source_read_models
        .iter()
        .map(|edge| pb::ConnectionRef {
            owner: Some(reference(pb::EntityKind::AutomationSlice, "observe-orders")),
            owner_uid: automation.system.as_ref().unwrap().uid.clone(),
            id: edge.connection_id.clone(),
        })
        .collect();
    assert_ne!(observations[0].id, observations[1].id);
    expected.insert(
        pb::EntityKey::new(pb::EntityKind::AutomationSlice, &id("observe-orders")),
        automation,
    );
    let handling = put(
        &service,
        pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id("handle-import")),
            title: "Handle import".into(),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id("import-order")),
                }),
                ..Default::default()
            }),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id("order-imported")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    expected.insert(
        pb::EntityKey::new(pb::EntityKind::CommandSlice, &id("handle-import")),
        handling,
    );
    let projection = put(
        &service,
        pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
            id: Some(id("project-order")),
            title: "Project order".into(),
            source_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id("order-imported")),
                }),
                ..Default::default()
            }],
            read_model: Some(pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("order-view")),
                }),
                ..Default::default()
            }),
            projection_role: pb::ProjectionRole::Upsert as i32,
            ..Default::default()
        }),
    )
    .await;
    expected.insert(
        pb::EntityKey::new(pb::EntityKind::ReadModelSlice, &id("project-order")),
        projection,
    );
    let channel = put(
        &service,
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(id("order-delivery")),
            title: "Order delivery".into(),
            address: Some(pb::ChannelAddress {
                template: "orders.received".into(),
                ..Default::default()
            }),
            protocols: vec![pb::ProtocolName {
                value: "nats".into(),
            }],
            delivery_guarantee: Some(pb::channel::DeliveryGuarantee::AtLeastOnce as i32),
            bindings: observations
                .iter()
                .map(|observation| pb::ChannelBinding {
                    observation: Some(observation.clone()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }),
    )
    .await;
    let Some(pb::entity::Kind::Channel(channel_value)) = channel.kind.as_ref() else {
        panic!()
    };
    let binding_ids: Vec<_> = channel_value
        .bindings
        .iter()
        .map(|binding| binding.connection_id.clone().unwrap())
        .collect();
    assert_ne!(binding_ids[0], binding_ids[1]);
    for (binding, observation) in channel_value.bindings.iter().zip(&observations) {
        assert_eq!(binding.observation.as_ref(), Some(observation));
    }
    expected.insert(
        pb::EntityKey::new(pb::EntityKind::Channel, &id("order-delivery")),
        channel,
    );
    for (slug, target, class, technology) in [
        (
            "feed-cache",
            "order-feed",
            pb::store_class::Known::Cache,
            "redis",
        ),
        (
            "order-index",
            "order-view",
            pb::store_class::Known::SearchIndex,
            "opensearch",
        ),
    ] {
        let stored = put(
            &service,
            pb::entity::Kind::DataStore(pb::DataStore {
                id: Some(id(slug)),
                title: slug.into(),
                store_class: Some(pb::StoreClass {
                    kind: Some(pb::store_class::Kind::Known(class as i32)),
                }),
                technology: Some(pb::TechnologyName {
                    value: technology.into(),
                }),
                attachments: vec![pb::ReadModelAttachment {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id(target)),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        )
        .await;
        expected.insert(
            pb::EntityKey::new(pb::EntityKind::DataStore, &id(slug)),
            stored,
        );
    }
    let members = expected
        .keys()
        .map(|key| pb::EntityRef {
            kind: key.kind as i32,
            id: Some(pb::Id {
                namespace: key.namespace.clone(),
                slug: key.slug.clone(),
                version: key.version,
            }),
        })
        .collect();
    let model = put(
        &service,
        pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id("order-integration")),
            title: "Order integration".into(),
            members,
            ..Default::default()
        }),
    )
    .await;
    expected.insert(
        pb::EntityKey::new(pb::EntityKind::EventModel, &id("order-integration")),
        model,
    );

    let incoming = service
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
    assert_eq!(incoming.len(), observations.len());
    assert_ne!(incoming[0].from_field, incoming[1].from_field);
    for target in ["order-feed", "order-view"] {
        let stores = service
            .get_incoming_references(Request::new(pb::GetReferencesRequest {
                kind: pb::EntityKind::ReadModel as i32,
                id: Some(id(target)),
                filter_kinds: vec![pb::EntityKind::DataStore as i32],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner()
            .references;
        assert_eq!(stores.len(), 1);
        assert_eq!(
            stores[0].from.as_ref().unwrap(),
            &reference(
                pb::EntityKind::DataStore,
                if target == "order-feed" {
                    "feed-cache"
                } else {
                    "order-index"
                }
            )
        );
    }
    for (kind, slug, endpoints) in [
        (
            pb::EntityKind::AutomationSlice,
            "observe-orders",
            vec![
                reference(pb::EntityKind::ReadModel, "order-feed"),
                reference(pb::EntityKind::Processor, "order-intake"),
                reference(pb::EntityKind::Command, "import-order"),
            ],
        ),
        (
            pb::EntityKind::CommandSlice,
            "handle-import",
            vec![
                reference(pb::EntityKind::Command, "import-order"),
                reference(pb::EntityKind::Event, "order-imported"),
            ],
        ),
        (
            pb::EntityKind::ReadModelSlice,
            "project-order",
            vec![
                reference(pb::EntityKind::Event, "order-imported"),
                reference(pb::EntityKind::ReadModel, "order-view"),
            ],
        ),
    ] {
        let refs = outgoing(&service, kind, slug).await;
        for endpoint in endpoints {
            assert!(
                refs.iter().any(|edge| edge.to.as_ref() == Some(&endpoint)),
                "{slug} missing {endpoint:?}"
            );
        }
    }
    let impact = service
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(reference(pb::EntityKind::ReadModel, "order-feed")),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    for slug in ["order-delivery", "feed-cache"] {
        assert!(
            impact.nodes.iter().any(|node| node
                .entity
                .as_ref()
                .and_then(|entity| entity.id.as_ref())
                .is_some_and(|id| id.slug == slug)),
            "missing {slug}"
        );
    }
    let snapshot = service
        .get_snapshot_id(Request::new(pb::GetSnapshotIdRequest {
            namespaces: vec!["commerce".into()],
            include_entries: true,
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!snapshot.truncated);
    assert_eq!(snapshot.entity_count as usize, expected.len());
    assert_eq!(snapshot.snapshot_id.len(), 64);
    eprintln!(
        "Fixture namespace=commerce snapshot={} entities={} truncated={}",
        snapshot.snapshot_id, snapshot.entity_count, snapshot.truncated
    );
    let exported = service
        .export_event_model_manifests(Request::new(pb::ExportEventModelManifestsRequest {
            id: Some(id("order-integration")),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!exported.truncated);
    assert_eq!(exported.skipped, [] as [pb::SkippedEntity; 0]);
    assert_eq!(exported.external_manifests, "");
    let parsed =
        trogon_atlas_client::manifest::parse_str(&exported.manifests, "connected-resource-export")
            .unwrap();
    assert_eq!(parsed.len(), expected.len());
    for manifest in parsed {
        assert_eq!(manifest.id.version, 1);
        assert_eq!(manifest.id.namespace, "commerce");
        let key = pb::EntityKey::new(manifest.kind, &manifest.id);
        let original = expected.get(&key).unwrap();
        assert_eq!(
            manifest.entity.kind, original.kind,
            "export changed {key:?}"
        );
    }
}
