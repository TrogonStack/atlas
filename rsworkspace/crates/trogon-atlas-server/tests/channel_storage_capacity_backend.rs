#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "association-admin";
const READER: &str = "association-reader";
const LIMITED: &str = "association-limited";
type Client = EventModelServiceClient<Channel>;
fn request<T>(body: T, token: &str, branch: Option<&str>) -> Request<T> {
    let mut req = Request::new(body);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    if let Some(branch) = branch {
        req.metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    req
}
async fn client() -> (Client, Arc<trogon_atlas_store::NatsStore>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            r#"
[principals.admin]
role = "admin"
tokens = ["{ADMIN}"]
[principals.reader]
role = "reader"
tokens = ["{READER}"]
[principals.limited]
role = "writer"
parent = "visible"
tokens = ["{LIMITED}"]
"#
        ))
        .unwrap(),
    )
    .unwrap();
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(0);
    let grpc = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(EventModelServiceServer::new(service)),
        BearerAuth::new(Arc::new(registry)),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    (
        Client::connect(format!("http://{addr}")).await.unwrap(),
        store,
    )
}
fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "offering".into(),
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
fn output(name: &str) -> pb::DataOutputPort {
    pb::DataOutputPort {
        name: Some(pb::PortName { value: name.into() }),
        data: Some(pb::OutputDataSelection {
            read_model: Some(pb::ReadModelRef {
                id: Some(id("result")),
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn product(slug: &str) -> pb::DataProduct {
    pb::DataProduct {
        id: Some(id(slug)),
        title: slug.into(),
        outputs: vec![output("Result")],
        ..Default::default()
    }
}
async fn put(client: &mut Client, value: pb::entity::Kind) -> pb::Entity {
    let label = std::mem::discriminant(&value);
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity(value)),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap_or_else(|error| panic!("{label:?}: {error}"))
        .into_inner()
        .entity
        .unwrap()
}

use prost::Message;
use trogon_atlas_store::Store;

fn owner(kind: pb::EntityKind, count: usize) -> pb::entity::Kind {
    let claims = vec![
        pb::StorageAccessClaim {
            store: Some(pb::DataStoreRef {
                id: Some(id("store"))
            }),
            direction: pb::StorageDirection::Read as i32,
            ..Default::default()
        };
        count
    ];
    match kind {
        pb::EntityKind::Channel => pb::entity::Kind::Channel(pb::Channel {
            id: Some(id("owner")),
            title: "Owner".into(),
            links: vec![
                pb::ChannelLink {
                    kind: pb::ChannelLinkKind::RoutesTo as i32,
                    target: Some(pb::ChannelRef {
                        id: Some(id("target"))
                    }),
                    ..Default::default()
                };
                count
            ],
            ..Default::default()
        }),
        pb::EntityKind::Component => pb::entity::Kind::Component(pb::Component {
            id: Some(id("owner")),
            title: "Owner".into(),
            storage_claims: claims,
            ..Default::default()
        }),
        pb::EntityKind::ExternalSystem => pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
            id: Some(id("owner")),
            title: "Owner".into(),
            storage_claims: claims,
            ..Default::default()
        }),
        pb::EntityKind::DataProduct => {
            let mut v = product("owner");
            v.storage_claims = claims;
            pb::entity::Kind::DataProduct(v)
        }
        _ => panic!(),
    }
}
fn doc(v: &mut pb::Entity) -> &mut String {
    match v.kind.as_mut().unwrap() {
        pb::entity::Kind::Channel(v) => &mut v.doc,
        pb::entity::Kind::Component(v) => &mut v.doc,
        pb::entity::Kind::ExternalSystem(v) => &mut v.doc,
        pb::entity::Kind::DataProduct(v) => v.doc.get_or_insert_default(),
        _ => panic!(),
    }
}
fn resize(v: &mut pb::Entity, target: usize) {
    loop {
        let size = v.encoded_len();
        if size == target {
            return;
        }
        let text = doc(v);
        if size < target {
            text.push_str(&"x".repeat(target - size));
        } else {
            text.truncate(text.len() - (size - target));
        }
    }
}
fn append(v: &mut pb::Entity) {
    match v.kind.as_mut().unwrap() {
        pb::entity::Kind::Channel(v) => {
            let mut x = v.links[0].clone();
            x.connection_id = None;
            v.links.push(x);
        }
        pb::entity::Kind::Component(v) => {
            let mut x = v.storage_claims[0].clone();
            x.connection_id = None;
            v.storage_claims.push(x);
        }
        pb::entity::Kind::ExternalSystem(v) => {
            let mut x = v.storage_claims[0].clone();
            x.connection_id = None;
            v.storage_claims.push(x);
        }
        pb::entity::Kind::DataProduct(v) => {
            let mut x = v.storage_claims[0].clone();
            x.connection_id = None;
            v.storage_claims.push(x);
        }
        _ => panic!(),
    }
}
async fn fixture(c: &mut Client, kind: pb::EntityKind, count: usize) -> pb::Entity {
    let (_, _, automation) = anchors(c).await;
    let Some(pb::entity::Kind::AutomationSlice(auto)) = &automation.kind else {
        panic!()
    };
    let binding = pb::ChannelBinding {
        observation: Some(connection(
            &automation,
            auto.source_read_models[0].connection_id.clone(),
        )),
        ..Default::default()
    };
    put(
        c,
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(id("target")),
            bindings: vec![binding.clone()],
            title: "Target".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        c,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id("result")),
            title: "Result".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        c,
        pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(id("store")),
            title: "Store".into(),
            attachments: vec![pb::ReadModelAttachment {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("result")),
                }),
                ..Default::default()
            }],
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Known(
                    pb::store_class::Known::Cache as i32,
                )),
            }),
            access_mode: Some(pb::data_store::AccessMode::ReadWrite as i32),
            ..Default::default()
        }),
    )
    .await;
    let mut value = owner(kind, count);
    if let pb::entity::Kind::Channel(v) = &mut value {
        v.bindings = vec![binding];
    }
    put(c, value).await
}
async fn refuse(
    c: &mut Client,
    s: &trogon_atlas_store::NatsStore,
    saved: &pb::Entity,
    proposed: pb::Entity,
    cause: &str,
) {
    let r = trogon_atlas_server::conv::entity_to_ref(saved).unwrap();
    let kind = pb::EntityKind::try_from(r.kind).unwrap();
    let key = r.id.unwrap();
    let before = s.get(kind, &key, None).await.unwrap();
    let seq = s.current_change_seq().await.unwrap();
    let before_inventory = inventory(&before.entity);
    let ids = trogon_atlas_core::connections::connection_occurrences(&before.entity).unwrap();
    for preview in [true, false] {
        for batch in [false, true] {
            let body = pb::PutEntityRequest {
                entity: Some(proposed.clone()),
                validate_only: preview,
                ..Default::default()
            };
            let error = if batch {
                c.batch_mutate(request(
                    pb::BatchMutateRequest {
                        ops: vec![pb::BatchMutateOp {
                            op: Some(pb::batch_mutate_op::Op::Put(body)),
                        }],
                        validate_only: preview,
                        ..Default::default()
                    },
                    ADMIN,
                    None,
                ))
                .await
                .expect_err("capacity mutation must be refused")
            } else {
                c.put_entity(request(body, ADMIN, None))
                    .await
                    .expect_err("capacity mutation must be refused")
            };
            assert_eq!(error.code(), Code::InvalidArgument, "{error}");
            assert!(error.message().contains(cause), "expected {cause}: {error}");
            let after = s.get(kind, &key, None).await.unwrap();
            assert_eq!(after.entity.encode_to_vec(), before.entity.encode_to_vec());
            assert_eq!(after.etag, before.etag);
            assert_eq!(after.entity.system, before.entity.system);
            assert_eq!(inventory(&after.entity), before_inventory);
            assert_eq!(
                trogon_atlas_core::connections::connection_occurrences(&after.entity).unwrap(),
                ids
            );
            assert_eq!(s.current_change_seq().await.unwrap(), seq);
            let public = c
                .get_entity(request(
                    pb::GetEntityRequest {
                        kind: kind as i32,
                        id: Some(key.clone()),
                    },
                    ADMIN,
                    None,
                ))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(public.entity, Some(before.entity.clone()));
            assert_eq!(public.etag, before.etag);
        }
    }
}
const KINDS: [pb::EntityKind; 4] = [
    pb::EntityKind::Channel,
    pb::EntityKind::Component,
    pb::EntityKind::ExternalSystem,
    pb::EntityKind::DataProduct,
];
#[tokio::test]
async fn collection_capacity() {
    for kind in KINDS {
        let (mut c, s) = client().await;
        let saved = fixture(&mut c, kind, 1024).await;
        let ids = trogon_atlas_core::connections::connection_occurrences(&saved).unwrap();
        assert!(ids.iter().all(|x| x.id.is_some()));
        assert_eq!(
            ids.iter()
                .map(|x| x.id.unwrap())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            ids.len()
        );
        assert_eq!(
            if kind == pb::EntityKind::Channel {
                trogon_atlas_core::channel_links::links(&saved).len()
            } else {
                trogon_atlas_core::storage_claims::claims(&saved).len()
            },
            1024
        );
        let mut overflow = saved.clone();
        append(&mut overflow);
        refuse(
            &mut c,
            &s,
            &saved,
            overflow,
            if kind == pb::EntityKind::Channel {
                "channel link limit exceeded"
            } else {
                "storage claim limit exceeded"
            },
        )
        .await;
    }
}
#[tokio::test]
async fn prepared_byte_capacity() {
    for kind in KINDS {
        let (mut c, s) = client().await;
        let mut saved = fixture(&mut c, kind, 64).await;
        match saved.kind.as_mut().unwrap() {
            pb::entity::Kind::Channel(v) => {
                for x in &mut v.links {
                    x.doc = Some(pb::CatalogText {
                        value: "x".repeat(15_500),
                    });
                }
            }
            pb::entity::Kind::Component(v) => {
                for x in &mut v.storage_claims {
                    x.doc = Some(pb::CatalogText {
                        value: "x".repeat(15_500),
                    });
                }
            }
            pb::entity::Kind::ExternalSystem(v) => {
                for x in &mut v.storage_claims {
                    x.doc = Some(pb::CatalogText {
                        value: "x".repeat(15_500),
                    });
                }
            }
            pb::entity::Kind::DataProduct(v) => {
                for x in &mut v.storage_claims {
                    x.doc = Some(pb::CatalogText {
                        value: "x".repeat(15_500),
                    });
                }
            }
            _ => panic!(),
        }
        resize(&mut saved, 1_048_576);
        let exact = put(&mut c, saved.kind.unwrap()).await;
        assert_eq!(exact.encoded_len(), 1_048_576, "{kind:?}");
        let mut overflow = exact.clone();
        resize(&mut overflow, 1_048_577);
        refuse(&mut c, &s, &exact, overflow.clone(), "byte").await;
        overflow.system = None;
        assert!(overflow.encoded_len() < 1_048_576);
        assert_eq!(
            trogon_atlas_core::stamp_system(Some(&exact), &overflow).encoded_len(),
            1_048_577
        );
        refuse(&mut c, &s, &exact, overflow, "byte").await;
        let mut growth = exact.clone();
        append(&mut growth);
        let mut prepared = growth.clone();
        trogon_atlas_core::connections::reconcile_connection_ids(&exact, &mut prepared).unwrap();
        let delta = prepared.encoded_len() - growth.encoded_len();
        resize(&mut growth, 1_048_577 - delta);
        growth.system = None;
        assert!(growth.encoded_len() < 1_048_576);
        assert!(trogon_atlas_core::stamp_system(Some(&exact), &growth).encoded_len() <= 1_048_576);
        let mut prepared = growth.clone();
        trogon_atlas_core::connections::reconcile_connection_ids(&exact, &mut prepared).unwrap();
        prepared = trogon_atlas_core::stamp_system(Some(&exact), &prepared);
        assert_eq!(prepared.encoded_len(), 1_048_577);
        refuse(&mut c, &s, &exact, growth, "byte").await;
    }
}
#[tokio::test]
async fn product_total_capacity() {
    let (mut c, s) = client().await;
    let saved = fixture(&mut c, pb::EntityKind::DataProduct, 1024).await;
    let pb::entity::Kind::DataProduct(mut v) = saved.kind.unwrap() else {
        panic!()
    };
    v.resource_associations = vec![
        pb::ProductResourceAssociation {
            direction: pb::DataPortDirection::Output as i32,
            target: Some(pb::product_resource_association::Target::DataStore(
                pb::DataStoreRef {
                    id: Some(id("store"))
                }
            )),
            ..Default::default()
        };
        3071
    ];
    let saved = put(&mut c, pb::entity::Kind::DataProduct(v)).await;
    assert_eq!(
        trogon_atlas_core::connections::connection_occurrences(&saved)
            .unwrap()
            .len(),
        4096
    );
    let occurrences = trogon_atlas_core::connections::connection_occurrences(&saved).unwrap();
    assert!(occurrences.iter().all(|x| x.id.is_some()));
    assert_eq!(
        occurrences
            .iter()
            .map(|x| x.id.unwrap())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        4096
    );
    let mut next = saved.clone();
    if let Some(pb::entity::Kind::DataProduct(v)) = &mut next.kind {
        v.resource_associations
            .push(pb::ProductResourceAssociation {
                direction: pb::DataPortDirection::Output as i32,
                target: Some(pb::product_resource_association::Target::DataStore(
                    pb::DataStoreRef {
                        id: Some(id("store")),
                    },
                )),
                ..Default::default()
            });
    }
    let mut identified = next.clone();
    trogon_atlas_core::connections::assign_connection_ids(&mut identified).unwrap();
    let Some(pb::entity::Kind::DataProduct(v)) = &identified.kind else {
        panic!()
    };
    assert!(trogon_atlas_core::data_product::validate_values(v)
        .unwrap_err()
        .to_string()
        .contains("association limit exceeded"));
    refuse(&mut c, &s, &saved, next, "DATA_PRODUCT_VALUE_INVALID").await;
}
fn connection(owner: &pb::Entity, id: Option<pb::ConnectionId>) -> pb::ConnectionRef {
    pb::ConnectionRef {
        owner: Some(trogon_atlas_server::conv::entity_to_ref(owner).unwrap()),
        owner_uid: owner.system.as_ref().unwrap().uid.clone(),
        id,
    }
}
async fn anchors(client: &mut Client) -> (pb::Entity, pb::Entity, pb::Entity) {
    for slug in ["result", "other"] {
        put(
            client,
            pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            }),
        )
        .await;
    }
    put(
        client,
        pb::entity::Kind::Event(pb::Event {
            id: Some(id("fact")),
            title: "Fact".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        client,
        pb::entity::Kind::Command(pb::Command {
            id: Some(id("produce")),
            title: "Produce".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        client,
        pb::entity::Kind::Processor(pb::Processor {
            id: Some(id("worker")),
            title: "Worker".into(),
            ..Default::default()
        }),
    )
    .await;
    let emission = put(
        client,
        pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id("emit")),
            title: "Emit".into(),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id("produce")),
                }),
                ..Default::default()
            }),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id("fact")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let projection = put(
        client,
        pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
            id: Some(id("project")),
            title: "Project".into(),
            source_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id("fact")),
                }),
                ..Default::default()
            }],
            read_model: Some(pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("result")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    let automation = put(
        client,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id("observe")),
            title: "Observe".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("result")),
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id("worker")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    (emission, projection, automation)
}

fn inventory(entity: &pb::Entity) -> Vec<(Option<pb::EntityKind>, pb::Id, String)> {
    trogon_atlas_core::refs::outbound_refs(entity)
        .into_iter()
        .map(|r| (r.to_kind, r.to_id, r.from_field))
        .collect()
}
