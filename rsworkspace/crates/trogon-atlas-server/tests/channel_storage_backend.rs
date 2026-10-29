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
fn with_claims(value: &pb::Entity, claims: Vec<pb::StorageAccessClaim>) -> pb::entity::Kind {
    let mut kind = kind_entity(value);
    match &mut kind {
        pb::entity::Kind::Component(v) => v.storage_claims = claims,
        pb::entity::Kind::ExternalSystem(v) => v.storage_claims = claims,
        pb::entity::Kind::DataProduct(v) => v.storage_claims = claims,
        _ => panic!("owner"),
    }
    kind
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
    claim_visible_namespace(&mut client).await;
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
    assert_ne!(hidden.code(), Code::PermissionDenied, "{hidden}");
}

async fn outgoing(client: &mut Client, kind: pb::EntityKind, slug: &str) -> Vec<pb::Reference> {
    client
        .get_outgoing_references(request(
            pb::GetReferencesRequest {
                kind: kind as i32,
                id: Some(id(slug)),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .references
}
async fn incoming(client: &mut Client, kind: pb::EntityKind, slug: &str) -> Vec<pb::Reference> {
    let mut all = Vec::new();
    let mut token = String::new();
    loop {
        let page = client
            .get_incoming_references(request(
                pb::GetReferencesRequest {
                    kind: kind as i32,
                    id: Some(id(slug)),
                    page_size: i32::MAX,
                    page_token: token,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        all.extend(page.references);
        if page.next_page_token.is_empty() {
            return all;
        }
        token = page.next_page_token;
    }
}
async fn delete_if_unreferenced(
    client: &mut Client,
    kind: pb::EntityKind,
    slug: &str,
) -> Result<(), tonic::Status> {
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: kind as i32,
                id: Some(id(slug)),
                mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .map(|_| ())
}
async fn retarget(
    client: &mut Client,
    kind: pb::EntityKind,
    from: &str,
    to: &str,
    filter: pb::EntityKind,
) -> u32 {
    client
        .retarget_references(request(
            pb::RetargetReferencesRequest {
                from: Some(pb::EntityRef {
                    kind: kind as i32,
                    id: Some(id(from)),
                }),
                to: Some(pb::EntityRef {
                    kind: kind as i32,
                    id: Some(id(to)),
                }),
                filter_kinds: vec![filter as i32],
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .rewritten
}

#[tokio::test(flavor = "multi_thread")]
async fn references_carry_occurrence_ids_and_guard_delete_and_retarget() {
    let mut client = client().await;
    let (_, _, auto) = anchors(&mut client).await;
    let pb::entity::Kind::AutomationSlice(slice) = auto.kind.as_ref().unwrap() else {
        panic!()
    };
    let observation = connection(&auto, slice.source_read_models[0].connection_id.clone());
    channel(&mut client, "plain", &observation, None, &[], vec![]).await;
    channel(&mut client, "plain-two", &observation, None, &[], vec![]).await;
    let routes = pb::ChannelLinkKind::RoutesTo;
    let hub = channel(
        &mut client,
        "hub",
        &observation,
        None,
        &[],
        vec![
            link(pb::ChannelLinkKind::Composes, "plain", &[]),
            link(routes, "plain", &[]),
        ],
    )
    .await;
    let links = channel_value(&hub).links;
    let refs = outgoing(&mut client, pb::EntityKind::Channel, "hub").await;
    for (index, stored) in links.iter().enumerate() {
        let field = format!("links[{index}].target");
        let hits: Vec<_> = refs.iter().filter(|r| r.from_field == field).collect();
        assert_eq!(hits.len(), 1, "{field}");
        assert_eq!(hits[0].connection_id, stored.connection_id, "{field}");
        assert_eq!(hits[0].to.as_ref().unwrap().id, Some(id("plain")));
    }
    let seen = incoming(&mut client, pb::EntityKind::Channel, "plain").await;
    let from_hub: Vec<_> = seen
        .iter()
        .filter(|r| r.from.as_ref().and_then(|f| f.id.as_ref()) == Some(&id("hub")))
        .collect();
    assert_eq!(from_hub.len(), 2);
    assert!(from_hub
        .iter()
        .all(|r| links.iter().any(|l| l.connection_id == r.connection_id)));
    assert_eq!(
        delete_if_unreferenced(&mut client, pb::EntityKind::Channel, "plain")
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert_eq!(
        retarget(
            &mut client,
            pb::EntityKind::Channel,
            "plain",
            "plain-two",
            pb::EntityKind::Channel
        )
        .await,
        1
    );
    let moved = channel_value(&saved_target(&mut client, "hub").await).links;
    for (before, after) in links.iter().zip(&moved) {
        assert_eq!(after.target.as_ref().unwrap().id, Some(id("plain-two")));
        assert_ne!(after.connection_id, before.connection_id);
    }
    delete_if_unreferenced(&mut client, pb::EntityKind::Channel, "plain")
        .await
        .unwrap();

    let storage = data_store(
        &mut client,
        "storage",
        pb::data_store::AccessMode::Read,
        &["result"],
    )
    .await;
    data_store(
        &mut client,
        "other-store",
        pb::data_store::AccessMode::Read,
        &["other"],
    )
    .await;
    let read = pb::StorageDirection::Read;
    let saved = put(
        &mut client,
        owner_kind(
            pb::EntityKind::Component,
            "svc",
            vec![claim(&storage, read, None), claim(&storage, read, Some(0))],
        ),
    )
    .await;
    let claims = claims_of(&saved);
    let refs = outgoing(&mut client, pb::EntityKind::Component, "svc").await;
    for (index, field) in [
        (0, "storage_claims[0].store"),
        (1, "storage_claims[1].store"),
        (1, "storage_claims[1].attachment.owner"),
    ] {
        let hits: Vec<_> = refs.iter().filter(|r| r.from_field == field).collect();
        assert_eq!(hits.len(), 1, "{field}");
        assert_eq!(
            hits[0].connection_id, claims[index].connection_id,
            "{field}"
        );
    }
    assert_eq!(
        delete_if_unreferenced(&mut client, pb::EntityKind::DataStore, "storage")
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert_eq!(
        retarget(
            &mut client,
            pb::EntityKind::DataStore,
            "storage",
            "other-store",
            pb::EntityKind::Component
        )
        .await,
        1
    );
    let after = claims_of(&put_get(&mut client, pb::EntityKind::Component, "svc").await);
    assert_eq!(after[0].store.as_ref().unwrap().id, Some(id("other-store")));
    assert_ne!(after[0].connection_id, claims[0].connection_id);
    assert_eq!(after[1], claims[1], "attachment claim stays pinned");
}
async fn put_get(client: &mut Client, kind: pb::EntityKind, slug: &str) -> pb::Entity {
    client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: kind as i32,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mixed_owner_inventories_share_the_occurrence_budget_across_links_and_claims() {
    let mut client = client().await;
    let (_, _, auto) = anchors(&mut client).await;
    let pb::entity::Kind::AutomationSlice(slice) = auto.kind.as_ref().unwrap() else {
        panic!()
    };
    let observation = connection(&auto, slice.source_read_models[0].connection_id.clone());
    channel(&mut client, "plain", &observation, None, &[], vec![]).await;
    let storage = data_store(
        &mut client,
        "storage",
        pb::data_store::AccessMode::Read,
        &["result"],
    )
    .await;
    let links = vec![link(pb::ChannelLinkKind::RoutesTo, "plain", &[]); 1024];
    let claims = vec![claim(&storage, pb::StorageDirection::Read, None); 1000];
    for index in 0..9 {
        if index % 2 == 0 {
            channel(
                &mut client,
                &format!("bulk-{index}"),
                &observation,
                None,
                &[],
                links.clone(),
            )
            .await;
        } else {
            put(
                &mut client,
                owner_kind(
                    pb::EntityKind::DataProduct,
                    &format!("bulk-{index}"),
                    claims.clone(),
                ),
            )
            .await;
        }
    }
    let within = incoming(&mut client, pb::EntityKind::DataStore, "storage").await;
    assert_eq!(within.len(), 4 * 1000);
    put(
        &mut client,
        owner_kind(pb::EntityKind::Component, "bulk-last", claims.clone()),
    )
    .await;
    for (kind, slug) in [
        (pb::EntityKind::DataStore, "storage"),
        (pb::EntityKind::Channel, "plain"),
    ] {
        let error = client
            .get_incoming_references(request(
                pb::GetReferencesRequest {
                    kind: kind as i32,
                    id: Some(id(slug)),
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::ResourceExhausted, "{slug}: {error}");
    }
}

async fn claim_visible_namespace(client: &mut Client) {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity(pb::entity::Kind::Event(pb::Event {
                    id: Some(visible_id("claim")),
                    title: "Claim".into(),
                    ..Default::default()
                }))),
                ..Default::default()
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap();
}
fn visible_id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "visible".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn delete_op(kind: pb::EntityKind, id: pb::Id) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
            kind: kind as i32,
            id: Some(id),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            ..Default::default()
        })),
    }
}
fn put_op(value: pb::entity::Kind) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
            entity: Some(entity(value)),
            ..Default::default()
        })),
    }
}
async fn batch(
    client: &mut Client,
    token: &str,
    ops: Vec<pb::BatchMutateOp>,
) -> Result<pb::BatchMutateResponse, tonic::Status> {
    client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops,
                ..Default::default()
            },
            token,
            None,
        ))
        .await
        .map(tonic::Response::into_inner)
}
async fn exists(client: &mut Client, kind: pb::EntityKind, id: pb::Id) -> bool {
    client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(id),
            },
            ADMIN,
            None,
        ))
        .await
        .is_ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_referrers_block_safe_delete_without_leaking_and_batch_overlay_is_final() {
    let mut client = client().await;
    claim_visible_namespace(&mut client).await;
    let (_, _, auto) = anchors(&mut client).await;
    let pb::entity::Kind::AutomationSlice(slice) = auto.kind.as_ref().unwrap() else {
        panic!("slice")
    };
    let observation = connection(&auto, slice.source_read_models[0].connection_id.clone());
    let shared_channel = put(
        &mut client,
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(visible_id("shared")),
            title: "Shared".into(),
            bindings: vec![pb::ChannelBinding {
                observation: Some(observation.clone()),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let shared_store = put(
        &mut client,
        pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(visible_id("shared-store")),
            title: "Shared store".into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Known(
                    pb::store_class::Known::Cache as i32,
                )),
            }),
            attachments: vec![pb::ReadModelAttachment {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("result")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let hidden_channel = |links: Vec<pb::ChannelLink>| {
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(id("hidden-hub")),
            title: "Hidden hub".into(),
            bindings: vec![pb::ChannelBinding {
                observation: Some(observation.clone()),
                ..Default::default()
            }],
            links,
            ..Default::default()
        })
    };
    let hidden_component = |claims: Vec<pb::StorageAccessClaim>| {
        pb::entity::Kind::Component(pb::Component {
            id: Some(id("hidden-app")),
            title: "Hidden app".into(),
            storage_claims: claims,
            ..Default::default()
        })
    };
    let link_to_shared = pb::ChannelLink {
        kind: pb::ChannelLinkKind::Composes as i32,
        target: Some(pb::ChannelRef {
            id: Some(visible_id("shared")),
        }),
        ..Default::default()
    };
    let claim_on_shared = claim(&shared_store, pb::StorageDirection::Read, None);
    let hub = put(&mut client, hidden_channel(vec![link_to_shared.clone()])).await;
    put(&mut client, hidden_component(vec![claim_on_shared.clone()])).await;
    assert!(shared_channel.system.is_some());

    for (kind, target) in [
        (pb::EntityKind::Channel, visible_id("shared")),
        (pb::EntityKind::DataStore, visible_id("shared-store")),
    ] {
        let error = client
            .delete_entity(request(
                pb::DeleteEntityRequest {
                    kind: kind as i32,
                    id: Some(target.clone()),
                    mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                    ..Default::default()
                },
                LIMITED,
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::FailedPrecondition, "{kind:?}: {error}");
        assert!(!error.message().contains("hidden"), "{kind:?}: {error}");
        assert!(exists(&mut client, kind, target.clone()).await);
        let refused = batch(&mut client, LIMITED, vec![delete_op(kind, target.clone())])
            .await
            .map_or_else(
                |status| status.code() != Code::Ok,
                |r| r.status != pb::batch_mutate_response::Status::Applied as i32,
            );
        assert!(refused, "{kind:?}");
        assert!(exists(&mut client, kind, target).await);
    }

    let removed = batch(
        &mut client,
        ADMIN,
        vec![
            delete_op(pb::EntityKind::Channel, visible_id("shared")),
            put_op(pb::entity::Kind::Channel(pb::Channel {
                links: vec![],
                ..channel_value(&hub)
            })),
            delete_op(pb::EntityKind::DataStore, visible_id("shared-store")),
            delete_op(pb::EntityKind::Component, id("hidden-app")),
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        removed.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{removed:?}"
    );
    assert!(!exists(&mut client, pb::EntityKind::Channel, visible_id("shared")).await);
    assert!(
        !exists(
            &mut client,
            pb::EntityKind::DataStore,
            visible_id("shared-store")
        )
        .await
    );
    assert!(!exists(&mut client, pb::EntityKind::Component, id("hidden-app")).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn port_removal_must_carry_its_dependent_claim_in_the_same_guarded_mutation() {
    let mut client = client().await;
    anchors(&mut client).await;
    let storage = data_store(
        &mut client,
        "storage",
        pb::data_store::AccessMode::Read,
        &["result"],
    )
    .await;
    let seeded = put(
        &mut client,
        pb::entity::Kind::DataProduct(pb::DataProduct {
            outputs: vec![output("Result"), output("Other")],
            ..product("offering")
        }),
    )
    .await;
    let pb::entity::Kind::DataProduct(stored) = seeded.kind.clone().unwrap() else {
        panic!("product")
    };
    let kept = stored.outputs[0].port_id.clone();
    let dropped = stored.outputs[1].port_id.clone();
    let with_claim = |port: Option<pb::DataPortId>| {
        let mut value = claim(&storage, pb::StorageDirection::Read, None);
        value.port_id = port;
        value
    };
    let revise = |outputs: Vec<pb::DataOutputPort>, claims: Vec<pb::StorageAccessClaim>| {
        pb::entity::Kind::DataProduct(pb::DataProduct {
            outputs,
            storage_claims: claims,
            ..stored.clone()
        })
    };
    put(
        &mut client,
        revise(stored.outputs.clone(), vec![with_claim(dropped.clone())]),
    )
    .await;
    let before = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::DataProduct as i32,
                id: Some(id("offering")),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();

    reject_all(
        &mut client,
        "port removed, claim kept",
        revise(
            vec![stored.outputs[0].clone()],
            vec![with_claim(dropped.clone())],
        ),
    )
    .await;
    let unchanged = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::DataProduct as i32,
                id: Some(id("offering")),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(unchanged, before, "a rejected removal publishes nothing");

    let replaced = put(
        &mut client,
        revise(
            vec![stored.outputs[0].clone()],
            vec![with_claim(kept.clone())],
        ),
    )
    .await;
    let pb::entity::Kind::DataProduct(replaced) = replaced.kind.unwrap() else {
        panic!("product")
    };
    assert_eq!(replaced.outputs.len(), 1);
    assert_eq!(replaced.storage_claims[0].port_id, kept);

    let removed = put(&mut client, revise(vec![stored.outputs[0].clone()], vec![])).await;
    let pb::entity::Kind::DataProduct(removed) = removed.kind.unwrap() else {
        panic!("product")
    };
    assert_eq!(removed.storage_claims.len(), 0);
}

fn versioned(slug: &str, version: u64) -> pb::Id {
    pb::Id {
        version,
        ..id(slug)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_links_select_another_exact_version_of_the_same_slug_but_never_the_same_channel() {
    let mut client = client().await;
    let (_, _, auto) = anchors(&mut client).await;
    let pb::entity::Kind::AutomationSlice(slice) = auto.kind.as_ref().unwrap() else {
        panic!()
    };
    let observation = connection(&auto, slice.source_read_models[0].connection_id.clone());
    let at = |version: u64, links: Vec<pb::ChannelLink>| pb::Channel {
        id: Some(versioned("versioned", version)),
        title: format!("versioned {version}"),
        bindings: vec![pb::ChannelBinding {
            observation: Some(observation.clone()),
            ..Default::default()
        }],
        links,
        ..Default::default()
    };
    let toward = |version: u64| pb::ChannelLink {
        kind: pb::ChannelLinkKind::RoutesTo as i32,
        target: Some(pb::ChannelRef {
            id: Some(versioned("versioned", version)),
        }),
        ..Default::default()
    };
    let newer = put(&mut client, pb::entity::Kind::Channel(at(2, vec![]))).await;
    let first = put(
        &mut client,
        pb::entity::Kind::Channel(at(1, vec![toward(2)])),
    )
    .await;
    assert_eq!(
        channel_value(&first).links[0].target.as_ref().unwrap().id,
        Some(versioned("versioned", 2)),
        "exact target version is retained"
    );
    let mut back = channel_value(&newer);
    back.links = vec![toward(1)];
    let second = put(&mut client, pb::entity::Kind::Channel(back)).await;
    assert_eq!(
        channel_value(&second).links[0].target.as_ref().unwrap().id,
        Some(versioned("versioned", 1))
    );
    let base = channel_value(&saved_target(&mut client, "versioned").await);
    let mut itself = base.clone();
    itself.links = vec![toward(1)];
    reject_all(
        &mut client,
        "exact same channel",
        pb::entity::Kind::Channel(itself),
    )
    .await;
}

async fn force_delete(client: &mut Client, kind: pb::EntityKind, slug: &str) {
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: kind as i32,
                id: Some(id(slug)),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
}
fn retitled(value: &pb::Entity, title: &str) -> pb::entity::Kind {
    let mut kind = kind_entity(value);
    match &mut kind {
        pb::entity::Kind::Channel(v) => v.title = title.into(),
        pb::entity::Kind::Component(v) => v.title = title.into(),
        pb::entity::Kind::ExternalSystem(v) => v.title = title.into(),
        pb::entity::Kind::DataProduct(v) => v.title = title.into(),
        _ => panic!("owner"),
    }
    kind
}

#[tokio::test(flavor = "multi_thread")]
async fn unchanged_dangling_channel_links_stay_editable_while_new_or_edited_unresolved_links_reject(
) {
    let mut client = client().await;
    let (_, _, auto) = anchors(&mut client).await;
    let pb::entity::Kind::AutomationSlice(slice) = auto.kind.as_ref().unwrap() else {
        panic!()
    };
    let observation = connection(&auto, slice.source_read_models[0].connection_id.clone());
    channel(&mut client, "gone", &observation, None, &[], vec![]).await;
    channel(&mut client, "kept", &observation, None, &[], vec![]).await;
    let routes = pb::ChannelLinkKind::RoutesTo;
    let hub = channel(
        &mut client,
        "hub",
        &observation,
        None,
        &[],
        vec![link(routes, "gone", &[]), link(routes, "kept", &[])],
    )
    .await;
    force_delete(&mut client, pb::EntityKind::Channel, "gone").await;

    let edited = put(&mut client, retitled(&hub, "renamed hub")).await;
    assert_eq!(
        channel_value(&edited).links,
        channel_value(&hub).links,
        "dangling link keeps its identity and bytes"
    );
    let mut grown = channel_value(&edited);
    grown.links.push(link(routes, "kept", &[]));
    let grown = put(&mut client, pb::entity::Kind::Channel(grown)).await;
    assert_eq!(channel_value(&grown).links.len(), 3);

    let base = channel_value(&grown);
    let mut new_missing = base.clone();
    new_missing.links.push(link(routes, "absent", &[]));
    reject_all(
        &mut client,
        "new unresolved link",
        pb::entity::Kind::Channel(new_missing),
    )
    .await;
    let mut retargeted = base.clone();
    retargeted.links[0] = pb::ChannelLink {
        connection_id: retargeted.links[0].connection_id.clone(),
        ..link(routes, "absent", &[])
    };
    reject_all(
        &mut client,
        "dangling link retargeted to another missing channel",
        pb::entity::Kind::Channel(retargeted),
    )
    .await;
    let mut edited_kind = base;
    edited_kind.links[0].doc = Some(pb::CatalogText {
        value: "edited".into(),
    });
    reject_all(
        &mut client,
        "dangling link edited in place",
        pb::entity::Kind::Channel(edited_kind),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unchanged_dangling_storage_claims_stay_editable_for_every_owner_kind() {
    let mut client = client().await;
    anchors(&mut client).await;
    let doomed = data_store(
        &mut client,
        "doomed",
        pb::data_store::AccessMode::Read,
        &["result"],
    )
    .await;
    let kept = data_store(
        &mut client,
        "survivor",
        pb::data_store::AccessMode::ReadWrite,
        &["other"],
    )
    .await;
    let read = pb::StorageDirection::Read;
    let write = pb::StorageDirection::Write;
    let mut saved = Vec::new();
    for owner in [
        pb::EntityKind::Component,
        pb::EntityKind::ExternalSystem,
        pb::EntityKind::DataProduct,
    ] {
        let slug = format!("holder-{}", owner as i32);
        let claims = vec![
            claim(&doomed, read, Some(0)),
            claim(&doomed, write, None),
            claim(&kept, read, None),
        ];
        saved.push((
            owner,
            slug.clone(),
            put(&mut client, owner_kind(owner, &slug, claims)).await,
        ));
    }
    force_delete(&mut client, pb::EntityKind::DataStore, "doomed").await;

    for (owner, _, original) in saved {
        let edited = put(&mut client, retitled(&original, "renamed")).await;
        assert_eq!(
            claims_of(&edited),
            claims_of(&original),
            "{owner:?}: dangling claims keep identity and bytes"
        );
        let mut grown = claims_of(&edited);
        grown.push(claim(&kept, write, Some(0)));
        let grown = put(&mut client, with_claims(&edited, grown)).await;
        assert_eq!(claims_of(&grown).len(), 4, "{owner:?}");

        let base = claims_of(&grown);
        let rebuilt = |claims: Vec<pb::StorageAccessClaim>| with_claims(&grown, claims);
        let mut documented = base.clone();
        documented[2].doc = Some(pb::CatalogText {
            value: "Visible store documentation".into(),
        });
        let documented = put(&mut client, rebuilt(documented)).await;
        assert_eq!(
            claims_of(&documented)[2].connection_id,
            base[2].connection_id
        );
        let mut dangling_doc = claims_of(&documented);
        dangling_doc[1].doc = Some(pb::CatalogText {
            value: "Dangling store documentation".into(),
        });
        reject_all(
            &mut client,
            &format!("{owner:?} doc-only dangling edit"),
            with_claims(&documented, dangling_doc),
        )
        .await;
        let mut new_missing = base.clone();
        let mut absent = claim(&kept, read, None);
        absent.store = Some(pb::DataStoreRef {
            id: Some(id("absent-store")),
        });
        new_missing.push(absent);
        reject_all(
            &mut client,
            &format!("{owner:?} new unresolved claim"),
            rebuilt(new_missing),
        )
        .await;
        let mut flipped = base.clone();
        flipped[1].direction = read as i32;
        reject_all(
            &mut client,
            &format!("{owner:?} dangling claim edited in place"),
            rebuilt(flipped),
        )
        .await;
        let mut retargeted = base;
        retargeted[0].connection_id = None;
        retargeted[0].store = Some(pb::DataStoreRef {
            id: Some(id("absent-store")),
        });
        reject_all(
            &mut client,
            &format!("{owner:?} dangling claim retargeted to another missing store"),
            rebuilt(retargeted),
        )
        .await;
    }
}

fn connection_subject(owner: &pb::Entity, id: Option<pb::ConnectionId>) -> pb::CatalogSubject {
    pb::CatalogSubject {
        selection: Some(pb::catalog_subject::Selection::Connection(connection(
            owner, id,
        ))),
    }
}
fn decision_on(slug: &str, subject: pb::CatalogSubject) -> pb::entity::Kind {
    let option_id = pb::LocalValueId {
        value: uuid::Uuid::now_v7().to_string(),
    };
    pb::entity::Kind::Decision(pb::Decision {
        id: Some(id(slug)),
        title: Some(pb::CatalogTitle { value: slug.into() }),
        subjects: vec![pb::SubjectAssociation {
            subject: Some(subject),
            ..Default::default()
        }],
        status: pb::DecisionStatus::Proposed as i32,
        context: Some(pb::CatalogText {
            value: "Choose".into(),
        }),
        rationale: Some(pb::CatalogText {
            value: "Explicit".into(),
        }),
        options: vec![pb::DecisionOption {
            option_id: Some(option_id),
            title: Some(pb::CatalogTitle {
                value: "Native".into(),
            }),
            description: Some(pb::CatalogText {
                value: "Keep".into(),
            }),
        }],
        ..Default::default()
    })
}
async fn subject_decisions(
    client: &mut Client,
    subject: &pb::CatalogSubject,
    token: &str,
    branch: Option<&str>,
) -> pb::ListSubjectDecisionsResponse {
    client
        .list_subject_decisions(request(
            pb::ListSubjectDecisionsRequest {
                subject: Some(subject.clone()),
                status: None,
            },
            token,
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}
async fn subject_assignments(
    client: &mut Client,
    subject: &pb::CatalogSubject,
) -> pb::ListSubjectAssignmentsResponse {
    let Some(pb::catalog_subject::Selection::Connection(connection)) = &subject.selection else {
        panic!("connection subject")
    };
    client
        .list_subject_assignments(request(
            pb::ListSubjectAssignmentsRequest {
                subject: Some(pb::ResponsibilitySubject {
                    selection: Some(pb::responsibility_subject::Selection::Connection(
                        connection.clone(),
                    )),
                }),
                kind: None,
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
}

fn occurrence_ids(value: &pb::Entity) -> Vec<pb::ConnectionId> {
    match value.kind.as_ref().unwrap() {
        pb::entity::Kind::Channel(v) => v
            .links
            .iter()
            .map(|item| item.connection_id.clone().unwrap())
            .collect(),
        pb::entity::Kind::Component(v) => v
            .storage_claims
            .iter()
            .map(|item| item.connection_id.clone().unwrap())
            .collect(),
        pb::entity::Kind::ExternalSystem(v) => v
            .storage_claims
            .iter()
            .map(|item| item.connection_id.clone().unwrap())
            .collect(),
        pb::entity::Kind::DataProduct(v) => v
            .storage_claims
            .iter()
            .map(|item| item.connection_id.clone().unwrap())
            .collect(),
        _ => panic!("owner"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_subject_listing_selects_exact_link_and_claim_occurrences_through_their_lifecycle()
{
    let mut client = client().await;
    let (_, _, auto) = anchors(&mut client).await;
    let pb::entity::Kind::AutomationSlice(slice) = auto.kind.as_ref().unwrap() else {
        panic!()
    };
    let observation = connection(&auto, slice.source_read_models[0].connection_id.clone());
    channel(&mut client, "inner", &observation, None, &[], vec![]).await;
    let composes = pb::ChannelLinkKind::Composes;
    let hub = channel(
        &mut client,
        "hub",
        &observation,
        None,
        &[],
        vec![
            link(composes, "inner", &[]),
            link(composes, "inner", &[]),
            link(pb::ChannelLinkKind::RoutesTo, "inner", &[]),
        ],
    )
    .await;
    let store = data_store(
        &mut client,
        "storage",
        pb::data_store::AccessMode::ReadWrite,
        &["result"],
    )
    .await;
    let read = pb::StorageDirection::Read;
    let mut owners = vec![hub];
    for owner in [
        pb::EntityKind::Component,
        pb::EntityKind::ExternalSystem,
        pb::EntityKind::DataProduct,
    ] {
        owners.push(
            put(
                &mut client,
                owner_kind(
                    owner,
                    &format!("holder-{}", owner as i32),
                    vec![
                        claim(&store, read, Some(0)),
                        claim(&store, read, Some(0)),
                        claim(&store, pb::StorageDirection::Write, None),
                    ],
                ),
            )
            .await,
        );
    }
    let mut assignees = Vec::new();
    for (position, owner) in owners.iter().enumerate() {
        let mut owner_assignees = Vec::new();
        put(
            &mut client,
            pb::entity::Kind::Team(pb::Team {
                id: Some(id(&format!("owner-assigned-{position}"))),
                title: Some(pb::DirectoryDisplayName {
                    value: "Owner maintainer".into(),
                }),
                responsibilities: vec![pb::Responsibility {
                    kind: pb::ResponsibilityKind::Maintainer as i32,
                    subject: Some(pb::ResponsibilitySubject {
                        selection: Some(pb::responsibility_subject::Selection::Entity(
                            trogon_atlas_server::conv::entity_to_ref(owner).unwrap(),
                        )),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        )
        .await;
        let ids = occurrence_ids(owner);
        assert_eq!(ids.len(), 3);
        for (index, connection_id) in ids.iter().enumerate() {
            let selected = connection_subject(owner, Some(connection_id.clone()));
            put(
                &mut client,
                decision_on(&format!("pick-{position}-{index}"), selected.clone()),
            )
            .await;
            let Some(pb::catalog_subject::Selection::Connection(connection)) = &selected.selection
            else {
                panic!("connection subject")
            };
            owner_assignees.push(
                put(
                    &mut client,
                    pb::entity::Kind::Team(pb::Team {
                        id: Some(id(&format!("assigned-{position}-{index}"))),
                        title: Some(pb::DirectoryDisplayName {
                            value: format!("Assignee {position}/{index}"),
                        }),
                        responsibilities: vec![pb::Responsibility {
                            kind: pb::ResponsibilityKind::Maintainer as i32,
                            subject: Some(pb::ResponsibilitySubject {
                                selection: Some(pb::responsibility_subject::Selection::Connection(
                                    connection.clone(),
                                )),
                            }),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                )
                .await,
            );
        }
        for (index, connection_id) in ids.iter().enumerate() {
            let selected = connection_subject(owner, Some(connection_id.clone()));
            let found = subject_decisions(&mut client, &selected, READER, None).await;
            assert_eq!(
                found.subject_state,
                pb::CatalogResolutionState::Resolved as i32,
                "{position}/{index}"
            );
            assert_eq!(
                found.declarations.len(),
                1,
                "{position}/{index}: equal siblings are never merged"
            );
            let Some(pb::catalog_declaration::Declaration::Subject(association)) =
                &found.declarations[0].declaration
            else {
                panic!("subject provenance");
            };
            assert_eq!(association.subject.as_ref(), Some(&selected));
            let assignments = subject_assignments(&mut client, &selected).await;
            assert_eq!(
                assignments.subject_state,
                pb::ResponsibilityResolutionState::Resolved as i32
            );
            assert_eq!(assignments.assignments.len(), 1);
            assert!(!assignments.incomplete);
            let expected = &owner_assignees[index];
            let pb::entity::Kind::Team(team) = expected.kind.as_ref().unwrap() else {
                panic!("team")
            };
            assert_eq!(
                assignments.assignments[0].assignee_uid,
                expected.system.as_ref().unwrap().uid
            );
            assert_eq!(
                assignments.assignments[0].assignment.as_ref(),
                Some(&team.responsibilities[0])
            );
            assert_eq!(
                assignments.assignments[0].state,
                pb::ResponsibilityResolutionState::Resolved as i32
            );
        }
        let owner_assignments = client
            .list_subject_assignments(request(
                pb::ListSubjectAssignmentsRequest {
                    subject: Some(pb::ResponsibilitySubject {
                        selection: Some(pb::responsibility_subject::Selection::Entity(
                            trogon_atlas_server::conv::entity_to_ref(owner).unwrap(),
                        )),
                    }),
                    kind: None,
                },
                READER,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(owner_assignments.assignments.len(), 1);
        assignees.push(owner_assignees);
    }

    for (position, owner) in owners.iter().enumerate() {
        let ids = occurrence_ids(owner);
        let mut shrunk = kind_entity(owner);
        match &mut shrunk {
            pb::entity::Kind::Channel(v) => {
                v.links.remove(0);
            }
            pb::entity::Kind::Component(v) => {
                v.storage_claims.remove(0);
            }
            pb::entity::Kind::ExternalSystem(v) => {
                v.storage_claims.remove(0);
            }
            pb::entity::Kind::DataProduct(v) => {
                v.storage_claims.remove(0);
            }
            _ => panic!("owner"),
        }
        let after = put(&mut client, shrunk).await;
        assert_eq!(occurrence_ids(&after), ids[1..], "{position}");
        let removed = connection_subject(&after, Some(ids[0].clone()));
        let found = subject_decisions(&mut client, &removed, READER, None).await;
        assert_eq!(
            found.subject_state,
            pb::CatalogResolutionState::Unresolved as i32,
            "{position}: a removed occurrence no longer resolves"
        );
        let assignments = subject_assignments(&mut client, &removed).await;
        assert_eq!(
            assignments.subject_state,
            pb::ResponsibilityResolutionState::Unresolved as i32
        );
        assert_eq!(assignments.assignments.len(), 1);
        assert_eq!(
            assignments.assignments[0].assignee_uid,
            assignees[position][0].system.as_ref().unwrap().uid
        );
        assert_eq!(
            assignments.assignments[0].state,
            pb::ResponsibilityResolutionState::Unresolved as i32
        );
        for connection_id in &ids[1..] {
            let kept = connection_subject(&after, Some(connection_id.clone()));
            let found = subject_decisions(&mut client, &kept, READER, None).await;
            assert_eq!(
                found.subject_state,
                pb::CatalogResolutionState::Resolved as i32,
                "{position}: surviving occurrences keep resolving"
            );
            assert_eq!(found.declarations.len(), 1, "{position}");
            let assignments = subject_assignments(&mut client, &kept).await;
            assert_eq!(
                assignments.subject_state,
                pb::ResponsibilityResolutionState::Resolved as i32
            );
            assert_eq!(assignments.assignments.len(), 1);
            assert_eq!(
                assignments.assignments[0].state,
                pb::ResponsibilityResolutionState::Resolved as i32
            );
        }
    }
}

async fn put_on(client: &mut Client, value: pb::entity::Kind, branch: &str) -> pb::Entity {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity(value)),
                ..Default::default()
            },
            ADMIN,
            Some(branch),
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}
async fn subject_outcome(
    client: &mut Client,
    subject: &pb::CatalogSubject,
    token: &str,
    branch: Option<&str>,
) -> Result<pb::ListSubjectDecisionsResponse, (Code, String)> {
    client
        .list_subject_decisions(request(
            pb::ListSubjectDecisionsRequest {
                subject: Some(subject.clone()),
                status: None,
            },
            token,
            branch,
        ))
        .await
        .map(tonic::Response::into_inner)
        .map_err(|status| (status.code(), status.message().to_string()))
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_authored_links_and_claims_resolve_only_on_the_branch_and_diff_as_real_changes() {
    let mut client = client().await;
    claim_visible_namespace(&mut client).await;
    let (_, _, auto) = anchors(&mut client).await;
    let pb::entity::Kind::AutomationSlice(slice) = auto.kind.as_ref().unwrap() else {
        panic!()
    };
    let observation = connection(&auto, slice.source_read_models[0].connection_id.clone());
    channel(&mut client, "inner", &observation, None, &[], vec![]).await;
    let composes = pb::ChannelLinkKind::Composes;
    let hub = channel(
        &mut client,
        "hub",
        &observation,
        None,
        &[],
        vec![link(composes, "inner", &[]), link(composes, "inner", &[])],
    )
    .await;
    let store = data_store(
        &mut client,
        "storage",
        pb::data_store::AccessMode::ReadWrite,
        &["result"],
    )
    .await;
    let read = pb::StorageDirection::Read;
    let holder = put(
        &mut client,
        owner_kind(
            pb::EntityKind::Component,
            "holder",
            vec![claim(&store, read, Some(0)), claim(&store, read, Some(0))],
        ),
    )
    .await;
    let main_ids = (occurrence_ids(&hub), occurrence_ids(&holder));

    let branch = format!("assoc-{}", uuid::Uuid::now_v7().simple());
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: branch.clone(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();

    let mut hub_edit = kind_entity(&hub);
    let pb::entity::Kind::Channel(value) = &mut hub_edit else {
        panic!()
    };
    value.links[0].doc = Some(pb::CatalogText {
        value: "edited on branch".into(),
    });
    value
        .links
        .push(link(pb::ChannelLinkKind::RoutesTo, "inner", &[]));
    let hub_branch = put_on(&mut client, hub_edit, &branch).await;
    let mut grown = claims_of(&holder);
    grown.push(claim(&store, pb::StorageDirection::Write, None));
    let holder_branch = put_on(&mut client, with_claims(&holder, grown), &branch).await;
    let hub_ids = occurrence_ids(&hub_branch);
    let holder_ids = occurrence_ids(&holder_branch);
    assert_eq!(hub_ids.len(), 3);
    assert_eq!(holder_ids.len(), 3);
    assert_eq!(hub_ids[..2], main_ids.0[..], "kept occurrences keep ids");
    assert_eq!(holder_ids[..2], main_ids.1[..], "kept occurrences keep ids");

    for (owner, ids) in [(&hub_branch, &hub_ids), (&holder_branch, &holder_ids)] {
        let added = connection_subject(owner, Some(ids[2].clone()));
        put_on(
            &mut client,
            decision_on(&format!("branch-pick-{}", ids[2].value), added.clone()),
            &branch,
        )
        .await;
        let on_branch = subject_decisions(&mut client, &added, READER, Some(&branch)).await;
        assert_eq!(
            on_branch.subject_state,
            pb::CatalogResolutionState::Resolved as i32
        );
        assert_eq!(on_branch.declarations.len(), 1);
        let on_main = subject_decisions(&mut client, &added, READER, None).await;
        assert_eq!(
            on_main.subject_state,
            pb::CatalogResolutionState::Unresolved as i32,
            "an occurrence authored on a branch is not visible on main"
        );
        assert_eq!(on_main.declarations.len(), 0);
        let unseen = pb::CatalogSubject {
            selection: Some(pb::catalog_subject::Selection::Connection(
                pb::ConnectionRef {
                    owner: Some(trogon_atlas_server::conv::entity_to_ref(owner).unwrap()),
                    owner_uid: "00000000-0000-7000-8000-000000000000".into(),
                    id: Some(ids[2].clone()),
                },
            )),
        };
        let hidden = subject_outcome(&mut client, &added, LIMITED, None).await;
        let missing = subject_outcome(&mut client, &unseen, LIMITED, None).await;
        assert_eq!(
            hidden, missing,
            "a hidden namespace answers exactly like a missing subject"
        );
        let refused = subject_outcome(&mut client, &added, LIMITED, Some(&branch)).await;
        let absent = subject_outcome(&mut client, &added, LIMITED, Some("no-such-branch")).await;
        assert_eq!(refused.as_ref().unwrap_err().0, Code::NotFound);
        assert_eq!(
            refused.unwrap_err(),
            absent.unwrap_err(),
            "a branch the caller may not see answers like one that does not exist"
        );
    }

    let diff = client
        .diff_branch(request(
            pb::DiffBranchRequest {
                name: branch.clone(),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    for (slug, base, ours) in [
        ("hub", &hub, &hub_branch),
        ("holder", &holder, &holder_branch),
    ] {
        let entry = diff
            .entries
            .iter()
            .find(|e| e.r#ref.as_ref().unwrap().id.as_ref().unwrap().slug == slug)
            .unwrap_or_else(|| panic!("diff entry for {slug}"));
        assert_eq!(
            entry.status,
            pb::branch_diff_entry::Status::Changed as i32,
            "{slug}"
        );
        assert_eq!(entry.base.as_ref(), Some(base), "{slug}");
        assert_eq!(
            occurrence_ids(entry.ours.as_ref().unwrap()),
            occurrence_ids(ours),
            "{slug}"
        );
        assert_eq!(
            occurrence_ids(entry.base.as_ref().unwrap()).len(),
            2,
            "{slug}"
        );
    }
}
