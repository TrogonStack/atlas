#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use pb::event_model_service_server::EventModelService;
use serde_json::{json, Value};
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::{data_plane, mcp_router},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
    telemetry::{InflightLayer, RpcMetricsLayer},
};

const TOKEN: &str = "channel-storage-admin";
const INERT: &str = "<script>window.__atlasInert = true</script>\nwhy";

struct Fixture {
    service: EventModelServiceImpl,
    namespace: String,
}

fn reference(entity: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(entity).unwrap()
}

fn uid(entity: &pb::Entity) -> String {
    entity.system.as_ref().unwrap().uid.clone()
}

fn connection(entity: &pb::Entity, id: Option<pb::ConnectionId>) -> pb::ConnectionRef {
    assert!(id.is_some());
    pb::ConnectionRef {
        owner: Some(reference(entity)),
        owner_uid: uid(entity),
        id,
    }
}

fn text(value: &str) -> pb::CatalogText {
    pb::CatalogText {
        value: value.into(),
    }
}

fn values(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

impl Fixture {
    fn id(&self, slug: &str) -> pb::Id {
        pb::Id {
            namespace: self.namespace.clone(),
            slug: slug.into(),
            version: 1,
        }
    }

    async fn put(&self, kind: pb::entity::Kind) -> pb::Entity {
        let proposed = pb::Entity {
            kind: Some(kind),
            ..Default::default()
        };
        let owner = reference(&proposed);
        self.service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(proposed),
                ..Default::default()
            }))
            .await
            .unwrap_or_else(|error| panic!("fixture put failed for {owner:?}: {error}"))
            .into_inner()
            .entity
            .unwrap()
    }

    async fn get(&self, entity: &pb::Entity) -> pb::Entity {
        let owner = reference(entity);
        self.service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: owner.kind,
                id: owner.id,
            }))
            .await
            .unwrap()
            .into_inner()
            .entity
            .unwrap()
    }

    async fn force_delete(&self, entity: &pb::Entity) {
        let owner = reference(entity);
        self.service
            .delete_entity(Request::new(pb::DeleteEntityRequest {
                kind: owner.kind,
                id: owner.id,
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            }))
            .await
            .unwrap();
    }

    async fn read_model(&self, slug: &str) -> pb::Entity {
        self.put(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(self.id(slug)),
            title: slug.into(),
            ..Default::default()
        }))
        .await
    }

    async fn channel(
        &self,
        slug: &str,
        observation: &pb::ConnectionRef,
        address: Option<pb::ChannelAddress>,
        binding_values: &[(&str, &str)],
        links: Vec<pb::ChannelLink>,
    ) -> pb::Entity {
        self.put(pb::entity::Kind::Channel(pb::Channel {
            id: Some(self.id(slug)),
            title: slug.into(),
            address,
            bindings: vec![pb::ChannelBinding {
                observation: Some(observation.clone()),
                parameter_values: values(binding_values),
                ..Default::default()
            }],
            links,
            ..Default::default()
        }))
        .await
    }

    fn link(
        &self,
        kind: pb::ChannelLinkKind,
        target: &str,
        parameter_values: &[(&str, &str)],
        doc: Option<&str>,
    ) -> pb::ChannelLink {
        pb::ChannelLink {
            kind: kind as i32,
            target: Some(pb::ChannelRef {
                id: Some(self.id(target)),
            }),
            parameter_values: values(parameter_values),
            doc: doc.map(text),
            ..Default::default()
        }
    }

    async fn store(&self, slug: &str, read_models: &[&pb::Entity]) -> pb::Entity {
        self.put(pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(self.id(slug)),
            title: slug.into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Known(
                    pb::store_class::Known::Warehouse as i32,
                )),
            }),
            access_mode: Some(pb::data_store::AccessMode::Read as i32),
            attachments: read_models
                .iter()
                .map(|model| pb::ReadModelAttachment {
                    read_model: Some(pb::ReadModelRef {
                        id: reference(model).id,
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }))
        .await
    }
}

fn attachments(store: &pb::Entity) -> Vec<pb::ReadModelAttachment> {
    let Some(pb::entity::Kind::DataStore(store)) = &store.kind else {
        panic!("store")
    };
    store.attachments.clone()
}

fn claim(
    store: &pb::Entity,
    direction: pb::StorageDirection,
    attachment: Option<usize>,
    port_id: Option<pb::DataPortId>,
    doc: Option<&str>,
) -> pb::StorageAccessClaim {
    pb::StorageAccessClaim {
        store: Some(pb::DataStoreRef {
            id: reference(store).id,
        }),
        direction: direction as i32,
        attachment: attachment
            .map(|index| connection(store, attachments(store)[index].connection_id.clone())),
        port_id,
        doc: doc.map(text),
        ..Default::default()
    }
}

fn owner_claims(owner: &pb::Entity) -> &[pb::StorageAccessClaim] {
    match owner.kind.as_ref().unwrap() {
        pb::entity::Kind::Component(value) => &value.storage_claims,
        pb::entity::Kind::ExternalSystem(value) => &value.storage_claims,
        pb::entity::Kind::DataProduct(value) => &value.storage_claims,
        _ => panic!("storage owner"),
    }
}

fn link_ids(channel: &pb::Entity) -> Vec<String> {
    let Some(pb::entity::Kind::Channel(channel)) = &channel.kind else {
        panic!("channel")
    };
    channel
        .links
        .iter()
        .map(|link| link.connection_id.clone().unwrap().value)
        .collect()
}

fn claim_ids(owner: &pb::Entity) -> Vec<String> {
    owner_claims(owner)
        .iter()
        .map(|claim| claim.connection_id.clone().unwrap().value)
        .collect()
}

struct Built {
    fixture: Fixture,
    receipt: Value,
}

fn slug_of(entity: &pb::Entity) -> String {
    reference(entity).id.unwrap().slug
}

fn unique(ids: &[String]) {
    let distinct: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(distinct.len(), ids.len(), "occurrence ids must be distinct");
}

async fn build() -> Built {
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
    let namespace = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "channel-storage-browser".into(),
            parent: "fixture-owner".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let fixture = Fixture { service, namespace };
    let f = &fixture;

    let receipt = f.read_model("receipt").await;
    let ledger = f.read_model("ledger").await;
    let notifier = f
        .put(pb::entity::Kind::Processor(pb::Processor {
            id: Some(f.id("notifier")),
            title: "Notifier".into(),
            ..Default::default()
        }))
        .await;
    let observed = f
        .put(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(f.id("observe")),
            title: "Observe receipts".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: reference(&receipt).id,
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: reference(&notifier).id,
                }),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await;
    let Some(pb::entity::Kind::AutomationSlice(slice)) = &observed.kind else {
        panic!("slice")
    };
    let observation = connection(&observed, slice.source_read_models[0].connection_id.clone());

    let address = pb::ChannelAddress {
        template: "orders.{tenant}".into(),
        parameters: vec![pb::AddressParameter {
            name: Some(pb::ParameterName {
                value: "tenant".into(),
            }),
            allowed_values: Some(pb::StringValues {
                values: vec!["acme".into(), "globex".into()],
            }),
            ..Default::default()
        }],
    };
    let addressed = f
        .channel(
            "addressed",
            &observation,
            Some(address),
            &[("tenant", "acme")],
            vec![],
        )
        .await;
    let gone = f.channel("gone", &observation, None, &[], vec![]).await;
    let unloaded = f.channel("unloaded", &observation, None, &[], vec![]).await;
    let composes = pb::ChannelLinkKind::Composes;
    let routes = pb::ChannelLinkKind::RoutesTo;
    let hub = f
        .channel(
            "hub",
            &observation,
            None,
            &[],
            vec![
                f.link(composes, "addressed", &[("tenant", "acme")], Some(INERT)),
                f.link(composes, "addressed", &[("tenant", "acme")], Some(INERT)),
                f.link(routes, "addressed", &[("tenant", "globex")], None),
                f.link(routes, "gone", &[], None),
                f.link(routes, "unloaded", &[], None),
            ],
        )
        .await;

    let warehouse = f.store("warehouse", &[&receipt, &ledger]).await;
    let replaced = f.store("replaced", &[&receipt]).await;
    let doomed = f.store("doomed", &[&receipt]).await;
    let read = pb::StorageDirection::Read;
    let write = pb::StorageDirection::Write;
    let checkout = f
        .put(pb::entity::Kind::Component(pb::Component {
            id: Some(f.id("checkout")),
            title: "Checkout".into(),
            storage_claims: vec![
                claim(&warehouse, read, Some(1), None, Some(INERT)),
                claim(&warehouse, write, None, None, Some(INERT)),
                claim(&warehouse, write, None, None, Some(INERT)),
            ],
            ..Default::default()
        }))
        .await;
    let stale_holder = f
        .put(pb::entity::Kind::Component(pb::Component {
            id: Some(f.id("stale-holder")),
            title: "Stale holder".into(),
            storage_claims: vec![claim(&replaced, read, Some(0), None, None)],
            ..Default::default()
        }))
        .await;
    let dangling_holder = f
        .put(pb::entity::Kind::Component(pb::Component {
            id: Some(f.id("dangling-holder")),
            title: "Dangling holder".into(),
            storage_claims: vec![
                claim(&doomed, read, Some(0), None, None),
                claim(&doomed, write, None, None, None),
            ],
            ..Default::default()
        }))
        .await;
    let payments = f
        .put(pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
            id: Some(f.id("payments")),
            title: "Payments".into(),
            storage_claims: vec![claim(&warehouse, read, None, None, None)],
            ..Default::default()
        }))
        .await;
    let outside = f
        .put(pb::entity::Kind::Component(pb::Component {
            id: Some(f.id("outside-claimant")),
            title: "Outside claimant".into(),
            storage_claims: vec![claim(&warehouse, read, None, None, None)],
            ..Default::default()
        }))
        .await;
    let offering = f
        .put(pb::entity::Kind::DataProduct(pb::DataProduct {
            id: Some(f.id("offering")),
            title: "Offering".into(),
            outputs: vec![pb::DataOutputPort {
                name: Some(pb::PortName {
                    value: "Receipts".into(),
                }),
                data: Some(pb::OutputDataSelection {
                    read_model: Some(pb::ReadModelRef {
                        id: reference(&receipt).id,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }))
        .await;
    let Some(pb::entity::Kind::DataProduct(mut product)) = offering.kind.clone() else {
        panic!("product")
    };
    let port = product.outputs[0].port_id.clone();
    assert!(port.is_some());
    product.storage_claims = vec![claim(&warehouse, read, None, port.clone(), None)];
    let offering = f.put(pb::entity::Kind::DataProduct(product)).await;

    let selected = f
        .put(pb::entity::Kind::Decision(pb::Decision {
            id: Some(f.id("pick-sibling")),
            title: Some(pb::CatalogTitle {
                value: "Pick siblings".into(),
            }),
            subjects: [
                pb::catalog_subject::Selection::Connection(connection(
                    &hub,
                    Some(pb::ConnectionId {
                        value: link_ids(&hub)[1].clone(),
                    }),
                )),
                pb::catalog_subject::Selection::Connection(connection(
                    &checkout,
                    Some(pb::ConnectionId {
                        value: claim_ids(&checkout)[2].clone(),
                    }),
                )),
            ]
            .into_iter()
            .map(|selection| pb::SubjectAssociation {
                subject: Some(pb::CatalogSubject {
                    selection: Some(selection),
                }),
                ..Default::default()
            })
            .collect(),
            status: pb::DecisionStatus::Proposed as i32,
            context: Some(text("Choose exact occurrences.")),
            rationale: Some(text("Equal siblings stay distinct.")),
            options: vec![pb::DecisionOption {
                title: Some(pb::CatalogTitle {
                    value: "Declared".into(),
                }),
                description: Some(text("Use the selected occurrence.")),
                ..Default::default()
            }],
            ..Default::default()
        }))
        .await;

    f.force_delete(&gone).await;
    f.force_delete(&doomed).await;
    f.force_delete(&replaced).await;
    let replaced_again = f.store("replaced", &[&receipt]).await;
    assert_ne!(uid(&replaced), uid(&replaced_again));

    let loaded = [
        &receipt,
        &ledger,
        &notifier,
        &observed,
        &addressed,
        &hub,
        &warehouse,
        &replaced_again,
        &checkout,
        &stale_holder,
        &dangling_holder,
        &payments,
        &offering,
        &selected,
    ];
    let model = f
        .put(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(f.id("model")),
            title: "Channel storage browser acceptance".into(),
            members: loaded.iter().map(|entity| reference(entity)).collect(),
            ..Default::default()
        }))
        .await;

    unique(&link_ids(&hub));
    unique(&claim_ids(&checkout));
    for entity in [&hub, &checkout, &stale_holder, &dangling_holder] {
        let stored = f.get(entity).await;
        assert_eq!(&stored, entity, "persisted identity of {}", slug_of(entity));
    }
    assert_eq!(f.get(&warehouse).await, warehouse);
    assert_eq!(f.get(&unloaded).await, unloaded);
    assert_eq!(f.get(&outside).await, outside);
    assert_eq!(f.get(&offering).await, offering);
    assert_eq!(f.get(&payments).await, payments);
    assert_eq!(f.get(&selected).await, selected);
    for removed in [&gone, &doomed] {
        let owner = reference(removed);
        let missing = f
            .service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: owner.kind,
                id: owner.id,
            }))
            .await;
        assert_eq!(missing.unwrap_err().code(), tonic::Code::NotFound);
    }

    let ids = |entity: &pb::Entity| json!({ "slug": slug_of(entity), "uid": uid(entity) });
    let receipt = json!({
        "namespace": f.namespace,
        "model": slug_of(&model),
        "inert": INERT,
        "hub": { "entity": ids(&hub), "links": link_ids(&hub) },
        "checkout": { "entity": ids(&checkout), "claims": claim_ids(&checkout) },
        "staleHolder": { "entity": ids(&stale_holder), "claims": claim_ids(&stale_holder) },
        "danglingHolder": { "entity": ids(&dangling_holder), "claims": claim_ids(&dangling_holder) },
        "payments": { "entity": ids(&payments), "claims": claim_ids(&payments) },
        "offering": {
            "entity": ids(&offering),
            "claims": claim_ids(&offering),
            "port": port.unwrap().value,
        },
        "outside": ids(&outside),
        "warehouse": {
            "entity": ids(&warehouse),
            "attachments": attachments(&warehouse)
                .iter()
                .map(|item| item.connection_id.clone().unwrap().value)
                .collect::<Vec<_>>(),
        },
        "replaced": { "stale": uid(&replaced), "current": uid(&replaced_again) },
        "decision": ids(&selected),
        "addressed": ids(&addressed),
        "gone": slug_of(&gone),
        "unloaded": slug_of(&unloaded),
        "doomed": slug_of(&doomed),
    });
    Built { fixture, receipt }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_storage_fixture_builds_with_persisted_identities() {
    let built = Box::pin(build()).await;
    assert_eq!(built.receipt["hub"]["links"].as_array().unwrap().len(), 5);
}

fn port(variable: &str) -> u16 {
    std::env::var(variable).map_or(0, |value| {
        value.parse().expect("fixture port must be a u16")
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual browser acceptance; serves loopback gRPC and MCP for at most ten minutes"]
async fn serve_channel_storage_fixture() {
    let Built { fixture, receipt } = Box::pin(build()).await;
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "fixture-admin": {"role": "admin", "tokens": [TOKEN]}
        }}))
        .unwrap(),
    )
    .unwrap();
    let grpc_listener = tokio::net::TcpListener::bind((
        std::net::Ipv4Addr::LOCALHOST,
        port("ATLAS_CHANNEL_STORAGE_GRPC_PORT"),
    ))
    .await
    .unwrap();
    let mcp_listener = tokio::net::TcpListener::bind((
        std::net::Ipv4Addr::LOCALHOST,
        port("ATLAS_CHANNEL_STORAGE_MCP_PORT"),
    ))
    .await
    .unwrap();
    let grpc_address = grpc_listener.local_addr().unwrap();
    let service = pb::event_model_service_server::EventModelServiceServer::new(fixture.service)
        .max_decoding_message_size(36 * 1024 * 1024)
        .max_encoding_message_size(36 * 1024 * 1024);
    let grpc = data_plane(service.clone(), BearerAuth::new(registry.clone()));
    let (drain, _) = tokio::sync::watch::channel(0i64);
    let app = mcp_router(
        data_plane(service, BearerAuth::new(registry)),
        RpcMetricsLayer::new(ServerMetrics::default()),
        InflightLayer::new(Arc::new(AtomicI64::new(0)), Arc::new(drain)),
        Some(Duration::from_secs(30)),
        36 * 1024 * 1024,
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    let seconds = std::env::var("ATLAS_CHANNEL_STORAGE_SECONDS")
        .map_or(600, |value| {
            value
                .parse::<u64>()
                .expect("fixture lifetime must be seconds")
        })
        .clamp(1, 600);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut servers = tokio::task::JoinSet::new();
    servers.spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                grpc_listener,
            ))
            .await
            .unwrap();
    });
    servers.spawn(async move {
        axum::serve(mcp_listener, app).await.unwrap();
    });
    let grpc_endpoint = format!("http://{grpc_address}");
    let mut client =
        pb::event_model_service_client::EventModelServiceClient::connect(grpc_endpoint.clone())
            .await
            .unwrap();
    let denied = client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::EventModel as i32,
            id: Some(pb::Id {
                namespace: receipt["namespace"].as_str().unwrap().into(),
                slug: "model".into(),
                version: 1,
            }),
        })
        .await;
    assert_eq!(denied.unwrap_err().code(), tonic::Code::Unauthenticated);
    let mut published = receipt;
    published["grpc"] = json!(grpc_endpoint);
    if let Ok(path) = std::env::var("ATLAS_BROWSER_FIXTURE_FILE") {
        std::fs::write(path, serde_json::to_vec_pretty(&published).unwrap()).unwrap();
    }
    println!(
        "READY grpc={grpc_endpoint} namespace={} model=model token={TOKEN} seconds={seconds}",
        published["namespace"].as_str().unwrap()
    );
    let outcome = tokio::select! {
        () = tokio::time::sleep_until(deadline) => Ok(()),
        result = servers.join_next() => Err(format!("fixture server exited before deadline: {result:?}")),
        result = tokio::signal::ctrl_c() => result.map_err(|error| error.to_string()),
    };
    servers.shutdown().await;
    outcome.unwrap_or_else(|error| panic!("{error}"));
}
