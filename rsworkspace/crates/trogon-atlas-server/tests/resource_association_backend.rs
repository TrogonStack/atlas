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
async fn client() -> Client {
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
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
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
    Client::connect(format!("http://{addr}")).await.unwrap()
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
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
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

fn kind_entity(value: &pb::Entity) -> pb::entity::Kind {
    value.kind.clone().unwrap()
}
async fn reject_all(client: &mut Client, label: &str, value: pb::entity::Kind) {
    for preview in [false, true] {
        let body = pb::PutEntityRequest {
            entity: Some(entity(value.clone())),
            validate_only: preview,
            ..Default::default()
        };
        let error = client
            .put_entity(request(body.clone(), ADMIN, None))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{label}: {error}");
        let error = client
            .batch_mutate(request(
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
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{label}: {error}");
    }
}
fn parameter(name: &str, allowed: &[&str], default: Option<&str>) -> pb::AddressParameter {
    pb::AddressParameter {
        name: Some(pb::ParameterName { value: name.into() }),
        allowed_values: Some(pb::StringValues {
            values: allowed.iter().map(ToString::to_string).collect(),
        }),
        default: default.map(ToString::to_string),
        ..Default::default()
    }
}
async fn channel(
    client: &mut Client,
    slug: &str,
    observation: &pb::ConnectionRef,
    address: Option<(&str, Vec<pb::AddressParameter>)>,
    values: &[(&str, &str)],
    links: Vec<pb::ChannelLink>,
) -> pb::Entity {
    put(
        client,
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(id(slug)),
            title: slug.into(),
            address: address.map(|(template, parameters)| pb::ChannelAddress {
                template: template.into(),
                parameters,
            }),
            bindings: vec![pb::ChannelBinding {
                observation: Some(observation.clone()),
                parameter_values: values
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                ..Default::default()
            }],
            links,
            ..Default::default()
        }),
    )
    .await
}
fn link(kind: pb::ChannelLinkKind, target: &str, values: &[(&str, &str)]) -> pb::ChannelLink {
    pb::ChannelLink {
        kind: kind as i32,
        target: Some(pb::ChannelRef {
            id: Some(id(target)),
        }),
        parameter_values: values
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        ..Default::default()
    }
}
fn channel_value(value: &pb::Entity) -> pb::Channel {
    let pb::entity::Kind::Channel(channel) = value.kind.clone().unwrap() else {
        panic!("channel")
    };
    channel
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_links_admit_exact_targets_and_reject_invalid_parameter_shapes() {
    let mut client = client().await;
    let (_, _, auto) = anchors(&mut client).await;
    let pb::entity::Kind::AutomationSlice(slice) = auto.kind.as_ref().unwrap() else {
        panic!()
    };
    let observation = connection(&auto, slice.source_read_models[0].connection_id.clone());
    channel(&mut client, "plain", &observation, None, &[], vec![]).await;
    channel(
        &mut client,
        "addressed",
        &observation,
        Some((
            "orders.{tenant}",
            vec![parameter("tenant", &["acme", "globex"], None)],
        )),
        &[("tenant", "acme")],
        vec![],
    )
    .await;
    channel(
        &mut client,
        "defaulted",
        &observation,
        Some((
            "orders.{region}",
            vec![parameter("region", &["eu", "us"], Some("eu"))],
        )),
        &[],
        vec![],
    )
    .await;
    let composes = pb::ChannelLinkKind::Composes;
    let routes = pb::ChannelLinkKind::RoutesTo;
    let links = vec![
        link(composes, "plain", &[]),
        link(routes, "plain", &[]),
        link(composes, "plain", &[]),
        link(routes, "addressed", &[("tenant", "acme")]),
        link(routes, "defaulted", &[]),
        link(routes, "defaulted", &[("region", "us")]),
    ];
    let saved = channel(&mut client, "hub", &observation, None, &[], links.clone()).await;
    let stored = channel_value(&saved).links;
    assert_eq!(stored.len(), 6);
    let ids: std::collections::HashSet<_> = stored
        .iter()
        .map(|l| l.connection_id.clone().unwrap().value)
        .collect();
    assert_eq!(ids.len(), 6, "equal links keep distinct IDs");
    for (stored, authored) in stored.iter().zip(&links) {
        assert_eq!(stored.kind, authored.kind);
        assert_eq!(stored.target, authored.target);
        assert_eq!(stored.parameter_values, authored.parameter_values);
    }

    let mut reapplied = channel_value(&saved);
    let again = put(&mut client, pb::entity::Kind::Channel(reapplied.clone())).await;
    assert_eq!(
        channel_value(&again).links,
        stored,
        "no-op reapply keeps IDs"
    );

    reapplied.links[3]
        .parameter_values
        .insert("tenant".into(), "globex".into());
    let edited = put(&mut client, pb::entity::Kind::Channel(reapplied)).await;
    assert_eq!(
        channel_value(&edited).links[3].connection_id,
        stored[3].connection_id,
        "descriptive parameter values keep the occurrence ID"
    );

    let mut cycle = channel_value(&saved_target(&mut client, "plain").await);
    cycle.links = vec![link(routes, "hub", &[])];
    put(&mut client, pb::entity::Kind::Channel(cycle)).await;

    let base = channel_value(&saved_target(&mut client, "hub").await);
    for (name, links) in [
        ("self key", vec![link(routes, "hub", &[])]),
        ("missing target", vec![link(routes, "absent", &[])]),
        (
            "values for target without address",
            vec![link(routes, "plain", &[("tenant", "acme")])],
        ),
        (
            "required parameter omitted",
            vec![link(routes, "addressed", &[])],
        ),
        (
            "value outside allowed set",
            vec![link(routes, "addressed", &[("tenant", "initech")])],
        ),
        (
            "undeclared parameter",
            vec![link(
                routes,
                "addressed",
                &[("tenant", "acme"), ("region", "eu")],
            )],
        ),
        (
            "unspecified kind",
            vec![link(pb::ChannelLinkKind::Unspecified, "plain", &[])],
        ),
    ] {
        let mut proposed = base.clone();
        proposed.links = links;
        reject_all(&mut client, name, pb::entity::Kind::Channel(proposed)).await;
    }
}
async fn saved_target(client: &mut Client, slug: &str) -> pb::Entity {
    client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Channel as i32,
                id: Some(id(slug)),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

fn claim(
    store: &pb::Entity,
    direction: pb::StorageDirection,
    attachment: Option<usize>,
) -> pb::StorageAccessClaim {
    let pb::entity::Kind::DataStore(value) = store.kind.as_ref().unwrap() else {
        panic!("store")
    };
    pb::StorageAccessClaim {
        store: Some(pb::DataStoreRef {
            id: value.id.clone(),
        }),
        direction: direction as i32,
        attachment: attachment
            .map(|index| connection(store, value.attachments[index].connection_id.clone())),
        ..Default::default()
    }
}
async fn data_store(
    client: &mut Client,
    slug: &str,
    mode: pb::data_store::AccessMode,
    read_models: &[&str],
) -> pb::Entity {
    put(
        client,
        pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(id(slug)),
            title: slug.into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Known(
                    pb::store_class::Known::Cache as i32,
                )),
            }),
            access_mode: Some(mode as i32),
            attachments: read_models
                .iter()
                .map(|slug| pb::ReadModelAttachment {
                    read_model: Some(pb::ReadModelRef { id: Some(id(slug)) }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }),
    )
    .await
}
fn owner_kind(
    owner: pb::EntityKind,
    slug: &str,
    claims: Vec<pb::StorageAccessClaim>,
) -> pb::entity::Kind {
    match owner {
        pb::EntityKind::Component => pb::entity::Kind::Component(pb::Component {
            id: Some(id(slug)),
            title: slug.into(),
            storage_claims: claims,
            ..Default::default()
        }),
        pb::EntityKind::ExternalSystem => pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
            id: Some(id(slug)),
            title: slug.into(),
            storage_claims: claims,
            ..Default::default()
        }),
        pb::EntityKind::DataProduct => {
            let mut value = product(slug);
            value.storage_claims = claims;
            pb::entity::Kind::DataProduct(value)
        }
        _ => panic!("owner"),
    }
}
fn claims_of(value: &pb::Entity) -> Vec<pb::StorageAccessClaim> {
    match value.kind.clone().unwrap() {
        pb::entity::Kind::Component(v) => v.storage_claims,
        pb::entity::Kind::ExternalSystem(v) => v.storage_claims,
        pb::entity::Kind::DataProduct(v) => v.storage_claims,
        _ => panic!("owner"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_claims_admit_exact_stores_attachments_and_ports_for_every_owner_kind() {
    let mut client = client().await;
    anchors(&mut client).await;
    let storage = data_store(
        &mut client,
        "storage",
        pb::data_store::AccessMode::Read,
        &["result"],
    )
    .await;
    let other = data_store(
        &mut client,
        "other-store",
        pb::data_store::AccessMode::ReadWrite,
        &["other"],
    )
    .await;
    let read = pb::StorageDirection::Read;
    let write = pb::StorageDirection::Write;
    for owner in [
        pb::EntityKind::Component,
        pb::EntityKind::ExternalSystem,
        pb::EntityKind::DataProduct,
    ] {
        let slug = format!("owner-{}", owner as i32);
        let claims = vec![
            claim(&storage, read, None),
            claim(&storage, write, None),
            claim(&storage, read, Some(0)),
            claim(&storage, write, Some(0)),
            claim(&storage, read, None),
            claim(&other, write, Some(0)),
        ];
        let saved = put(&mut client, owner_kind(owner, &slug, claims.clone())).await;
        let stored = claims_of(&saved);
        assert_eq!(stored.len(), claims.len(), "{owner:?}");
        let ids: std::collections::HashSet<_> = stored
            .iter()
            .map(|c| c.connection_id.clone().unwrap().value)
            .collect();
        assert_eq!(
            ids.len(),
            claims.len(),
            "{owner:?}: equal claims keep distinct IDs"
        );
        for (stored, authored) in stored.iter().zip(&claims) {
            assert_eq!(stored.store, authored.store, "{owner:?}");
            assert_eq!(stored.direction, authored.direction, "{owner:?}");
            assert_eq!(stored.attachment, authored.attachment, "{owner:?}");
        }
        let again = put(&mut client, kind_entity(&saved)).await;
        assert_eq!(
            claims_of(&again),
            stored,
            "{owner:?}: no-op reapply keeps IDs"
        );

        let rejected: Vec<(&str, Vec<pb::StorageAccessClaim>)> = vec![
            ("missing store", {
                let mut c = claim(&storage, read, None);
                c.store = Some(pb::DataStoreRef {
                    id: Some(id("absent")),
                });
                vec![c]
            }),
            (
                "unspecified direction",
                vec![claim(&storage, pb::StorageDirection::Unspecified, None)],
            ),
            ("attachment of another store", {
                let mut c = claim(&other, read, Some(0));
                c.store = claim(&storage, read, None).store;
                vec![c]
            }),
            ("attachment owner uid of another incarnation", {
                let mut c = claim(&storage, read, Some(0));
                c.attachment.as_mut().unwrap().owner_uid =
                    "018f0000-0000-7000-8000-000000000000".into();
                vec![c]
            }),
            ("attachment id not on the store", {
                let mut c = claim(&storage, read, Some(0));
                c.attachment.as_mut().unwrap().id = Some(pb::ConnectionId {
                    value: "018f0000-0000-7000-8000-000000000001".into(),
                });
                vec![c]
            }),
        ];
        for (name, claims) in rejected {
            reject_all(
                &mut client,
                &format!("{owner:?} {name}"),
                owner_kind(owner, &slug, claims),
            )
            .await;
        }
        let mut with_port = claim(&storage, read, None);
        with_port.port_id = Some(pb::DataPortId {
            value: "018f0000-0000-7000-8000-000000000002".into(),
        });
        reject_all(
            &mut client,
            &format!("{owner:?} port on non-port owner or unknown port"),
            owner_kind(owner, &slug, vec![with_port]),
        )
        .await;
    }

    let mut seeded = product("ported");
    seeded.inputs.push(pb::DataInputPort {
        name: Some(pb::PortName {
            value: "Fact".into(),
        }),
        data: Some(pb::InputDataSelection {
            target: Some(pb::input_data_selection::Target::Event(pb::EventRef {
                id: Some(id("fact")),
            })),
            ..Default::default()
        }),
        ..Default::default()
    });
    let saved = put(&mut client, pb::entity::Kind::DataProduct(seeded)).await;
    let pb::entity::Kind::DataProduct(mut ported) = saved.kind.clone().unwrap() else {
        panic!()
    };
    let output_port = ported.outputs[0].port_id.clone().unwrap();
    let input_port = ported.inputs[0].port_id.clone().unwrap();
    ported.storage_claims = [(write, &output_port), (read, &input_port)]
        .into_iter()
        .map(|(direction, port)| {
            let mut on_port = claim(&storage, direction, Some(0));
            on_port.port_id = Some(port.clone());
            on_port
        })
        .collect();
    let saved = put(&mut client, pb::entity::Kind::DataProduct(ported)).await;
    let claims = claims_of(&saved);
    assert_eq!(claims[0].port_id, Some(output_port));
    assert_eq!(claims[1].port_id, Some(input_port));
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_and_missing_storage_targets_fail_identically() {
    let mut client = client().await;
    anchors(&mut client).await;
    let storage = data_store(
        &mut client,
        "storage",
        pb::data_store::AccessMode::Read,
        &["result"],
    )
    .await;
    let probe = |store: pb::DataStoreRef| {
        let mut c = claim(&storage, pb::StorageDirection::Read, None);
        c.store = Some(store);
        pb::entity::Kind::Component(pb::Component {
            id: Some(pb::Id {
                namespace: "visible".into(),
                slug: "probe".into(),
                version: 1,
            }),
            title: "Probe".into(),
            storage_claims: vec![c],
            ..Default::default()
        })
    };
    let attempt = |client: &mut Client, store: pb::DataStoreRef| {
        let body = pb::PutEntityRequest {
            entity: Some(entity(probe(store))),
            ..Default::default()
        };
        let request = request(body, LIMITED, None);
        let mut client = client.clone();
        async move { client.put_entity(request).await.unwrap_err() }
    };
    let hidden = attempt(
        &mut client,
        pb::DataStoreRef {
            id: Some(id("storage")),
        },
    )
    .await;
    let missing = attempt(
        &mut client,
        pb::DataStoreRef {
            id: Some(id("never-existed")),
        },
    )
    .await;
    assert_eq!(hidden.code(), missing.code());
    assert_eq!(hidden.message(), missing.message());
    assert_ne!(hidden.code(), Code::Ok);
}
