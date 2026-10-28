//! Remaining agent and shared metadata acceptance through the public stack:
//! a real tonic server behind `BearerAuth` and `AuthzLayer` over a real
//! JetStream store.
//!
//! Preview semantics: `validate_only` on `PutEntity` and `BatchMutate` is the
//! only preview the API offers. It returns no token and binds no revision,
//! owner UID or principal, so every guard is re-evaluated by the apply request.
//! The owner incarnation is observable only through the entity etag (the store
//! revision, unique per key across delete and recreate) and the server-issued
//! `system.uid`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use prost::Message;
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "metadata-acceptance-admin";
const WRITER: &str = "metadata-acceptance-writer";
const AGENT: &str = "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.AgentAnnotation";
const LIFECYCLE: &str = "type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation";
const BADGES: &str = "type.googleapis.com/trogonatlas.annotation.v1alpha1.BadgesAnnotation";
const GROUPS: &str = "type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation";
const NOTE: &str = "type.googleapis.com/trogonatlas.annotation.v1alpha1.NoteAnnotation";

type Client = EventModelServiceClient<Channel>;

struct World {
    client: Client,
    ns: String,
}

fn request<T>(body: T, token: &str) -> Request<T> {
    let mut req = Request::new(body);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    req
}

async fn world() -> World {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            r#"
[principals.admin]
role = "admin"
tokens = ["{ADMIN}"]
[principals.writer]
role = "writer"
parent = "visible"
tokens = ["{WRITER}"]
"#
        ))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
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
    let mut client = Client::connect(format!("http://{addr}")).await.unwrap();
    let ns = client
        .register_namespace(request(
            pb::RegisterNamespaceRequest {
                name: "acceptance".into(),
                parent: "visible".into(),
            },
            ADMIN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    World { client, ns }
}

fn any(url: &str, value: Vec<u8>) -> prost_types::Any {
    prost_types::Any {
        type_url: url.into(),
        value,
    }
}

fn id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn target(ns: &str, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Component as i32,
        id: Some(id(ns, slug)),
    }
}

fn component(ns: &str, slug: &str, metadata: Vec<prost_types::Any>) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Component(pb::Component {
            id: Some(id(ns, slug)),
            title: slug.into(),
            metadata,
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn metadata_of(entity: &pb::Entity) -> &Vec<prost_types::Any> {
    let Some(pb::entity::Kind::Component(value)) = entity.kind.as_ref() else {
        panic!("component")
    };
    &value.metadata
}

fn agent(model: pb::AgentModel, tools: Vec<pb::AgentTool>) -> prost_types::Any {
    any(
        AGENT,
        pb::AgentAnnotation {
            model: Some(model),
            tools,
        }
        .encode_to_vec(),
    )
}

fn lifecycle(value: pb::LifecycleAnnotation) -> prost_types::Any {
    any(LIFECYCLE, value.encode_to_vec())
}

fn badge(label: &str) -> pb::Badge {
    pb::Badge {
        content: Some(pb::BadgeText {
            value: label.into(),
        }),
        background_color: Some(pb::HexColor {
            value: "#aAbBcC".into(),
        }),
        text_color: Some(pb::HexColor {
            value: "#000000".into(),
        }),
        ..Default::default()
    }
}

fn badges(values: Vec<pb::Badge>) -> prost_types::Any {
    any(
        BADGES,
        pb::BadgesAnnotation { badges: values }.encode_to_vec(),
    )
}

fn item(target: pb::EntityRef) -> pb::ResourceGroupItem {
    pb::ResourceGroupItem {
        connection_id: None,
        target: Some(target),
    }
}

fn groups(values: Vec<pb::ResourceGroup>) -> prost_types::Any {
    any(
        GROUPS,
        pb::ResourceGroupsAnnotation { groups: values }.encode_to_vec(),
    )
}

fn provider_only() -> pb::AgentModel {
    pb::AgentModel {
        provider: Some(pb::ModelProvider {
            value: "example".into(),
        }),
        ..Default::default()
    }
}

fn tool(name: &str) -> pb::AgentTool {
    pb::AgentTool {
        name: Some(pb::ToolName { value: name.into() }),
        r#type: Some(pb::ToolKindText {
            value: "manual".into(),
        }),
        ..Default::default()
    }
}

fn put_request(entity: pb::Entity, if_match: &str, validate_only: bool) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(entity),
        if_match: if_match.into(),
        validate_only,
        ..Default::default()
    }
}

impl World {
    async fn put_as(
        &mut self,
        token: &str,
        entity: pb::Entity,
        if_match: &str,
        validate_only: bool,
    ) -> Result<pb::PutEntityResponse, tonic::Status> {
        self.client
            .put_entity(request(put_request(entity, if_match, validate_only), token))
            .await
            .map(tonic::Response::into_inner)
    }

    async fn put(&mut self, entity: pb::Entity) -> pb::PutEntityResponse {
        self.put_as(ADMIN, entity, "", false).await.unwrap()
    }

    async fn get_kind(
        &mut self,
        kind: pb::EntityKind,
        slug: &str,
    ) -> Result<pb::GetEntityResponse, tonic::Status> {
        self.client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: kind as i32,
                    id: Some(id(&self.ns, slug)),
                },
                ADMIN,
            ))
            .await
            .map(tonic::Response::into_inner)
    }

    async fn get(&mut self, slug: &str) -> pb::GetEntityResponse {
        self.get_kind(pb::EntityKind::Component, slug)
            .await
            .unwrap()
    }

    async fn absent(&mut self, kind: pb::EntityKind, slug: &str) -> bool {
        self.get_kind(kind, slug)
            .await
            .is_err_and(|status| status.code() == Code::NotFound)
    }

    async fn incoming(&mut self, slug: &str) -> Vec<pb::Reference> {
        let ns = self.ns.clone();
        let mut all = Vec::new();
        let mut page_token = String::new();
        loop {
            let page = self
                .client
                .get_incoming_references(request(
                    pb::GetReferencesRequest {
                        kind: pb::EntityKind::Component as i32,
                        id: Some(id(&ns, slug)),
                        page_size: 500,
                        page_token: page_token.clone(),
                        ..Default::default()
                    },
                    ADMIN,
                ))
                .await
                .unwrap()
                .into_inner();
            all.extend(page.references);
            if page.next_page_token.is_empty() {
                return all;
            }
            page_token = page.next_page_token;
        }
    }

    async fn outgoing(&mut self, slug: &str) -> Vec<pb::Reference> {
        let ns = self.ns.clone();
        let mut all = Vec::new();
        let mut page_token = String::new();
        loop {
            let page = self
                .client
                .get_outgoing_references(request(
                    pb::GetReferencesRequest {
                        kind: pb::EntityKind::Component as i32,
                        id: Some(id(&ns, slug)),
                        page_size: 500,
                        page_token: page_token.clone(),
                        ..Default::default()
                    },
                    ADMIN,
                ))
                .await
                .unwrap()
                .into_inner();
            all.extend(page.references);
            if page.next_page_token.is_empty() {
                return all;
            }
            page_token = page.next_page_token;
        }
    }

    async fn delete(&mut self, slug: &str, mode: pb::delete_entity_request::Mode) {
        self.client
            .delete_entity(request(
                pb::DeleteEntityRequest {
                    kind: pb::EntityKind::Component as i32,
                    id: Some(id(&self.ns, slug)),
                    mode: mode as i32,
                    ..Default::default()
                },
                ADMIN,
            ))
            .await
            .unwrap();
    }
}

fn full_owner(ns: &str, slug: &str) -> pb::Entity {
    component(
        ns,
        slug,
        vec![
            agent(provider_only(), vec![tool("lookup")]),
            lifecycle(pb::LifecycleAnnotation {
                status: "draft".into(),
                draft: Some(pb::DraftNotice {
                    title: None,
                    message: Some(pb::MetadataText {
                        value: "Review".into(),
                    }),
                }),
                ..Default::default()
            }),
            badges(vec![badge("Preview")]),
            groups(vec![pb::ResourceGroup {
                items: vec![item(target(ns, "target")), item(target(ns, "target"))],
                ..Default::default()
            }]),
        ],
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn partial_model_roundtrips_exactly_through_the_public_api() {
    let mut world = world().await;
    let ns = world.ns.clone();
    for (slug, model) in [
        ("provider-only", provider_only()),
        (
            "name-only",
            pb::AgentModel {
                name: Some(pb::ModelName {
                    value: "any-name".into(),
                }),
                ..Default::default()
            },
        ),
        (
            "version-only",
            pb::AgentModel {
                version: Some(pb::ModelVersion {
                    value: "not-semver 1.x preview".into(),
                }),
                ..Default::default()
            },
        ),
    ] {
        let sent = agent(model.clone(), vec![]);
        world.put(component(&ns, slug, vec![sent.clone()])).await;
        let stored = world.get(slug).await.entity.unwrap();
        assert_eq!(metadata_of(&stored), &vec![sent], "{slug}");
        let decoded =
            pb::AgentAnnotation::decode(metadata_of(&stored)[0].value.as_slice()).unwrap();
        assert_eq!(decoded.model, Some(model), "{slug}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_field_and_scenario_scope_is_refused_by_the_public_put() {
    let mut world = world().await;
    let ns = world.ns.clone();
    let note = any(
        NOTE,
        pb::NoteAnnotation {
            text: "kept".into(),
            ..Default::default()
        }
        .encode_to_vec(),
    );
    let proposed = [
        agent(provider_only(), vec![]),
        lifecycle(pb::LifecycleAnnotation {
            status: "draft".into(),
            ..Default::default()
        }),
        badges(vec![badge("x")]),
        groups(vec![]),
    ];
    let with_field = |slug: &str, annotation: prost_types::Any| pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(&ns, slug)),
            title: slug.into(),
            schema: Some(any(
                "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema",
                pb::Schema {
                    fields: vec![pb::FieldSpec {
                        name: "field".into(),
                        metadata: vec![annotation],
                        ..Default::default()
                    }],
                    ..Default::default()
                }
                .encode_to_vec(),
            )),
            ..Default::default()
        })),
        ..Default::default()
    };
    for (kind, slug) in [
        (
            pb::entity::Kind::Event(pb::Event {
                id: Some(id(&ns, "placed")),
                title: "placed".into(),
                ..Default::default()
            }),
            "event",
        ),
        (
            pb::entity::Kind::Command(pb::Command {
                id: Some(id(&ns, "place")),
                title: "place".into(),
                ..Default::default()
            }),
            "command",
        ),
        (
            pb::entity::Kind::Persona(pb::Persona {
                id: Some(id(&ns, "buyer")),
                title: "buyer".into(),
                role: "tester".into(),
                ..Default::default()
            }),
            "persona",
        ),
        (
            pb::entity::Kind::Ui(pb::Ui {
                id: Some(id(&ns, "checkout")),
                title: "checkout".into(),
                ..Default::default()
            }),
            "ui",
        ),
    ] {
        let _ = slug;
        world
            .put(pb::Entity {
                kind: Some(kind),
                ..Default::default()
            })
            .await;
    }
    let with_scenario = |slug: &str, annotation: prost_types::Any| pb::Entity {
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(&ns, slug)),
            title: slug.into(),
            persona: Some(pb::PersonaEdge {
                persona: Some(pb::PersonaRef {
                    id: Some(id(&ns, "buyer")),
                }),
                ..Default::default()
            }),
            ui: Some(pb::UiEdge {
                ui: Some(pb::UiRef {
                    id: Some(id(&ns, "checkout")),
                }),
                ..Default::default()
            }),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id(&ns, "place")),
                }),
                ..Default::default()
            }),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id(&ns, "placed")),
                }),
                ..Default::default()
            }],
            scenarios: vec![pb::CommandScenario {
                id: "scenario".into(),
                title: "Scenario".into(),
                when: Some(pb::CommandExample {
                    command: Some(pb::CommandRef {
                        id: Some(id(&ns, "place")),
                    }),
                    payload: None,
                }),
                then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
                    events: vec![pb::EventExample {
                        event: Some(pb::EventRef {
                            id: Some(id(&ns, "placed")),
                        }),
                        payload: None,
                    }],
                })),
                metadata: vec![annotation],
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    world.put(with_field("control-field", note.clone())).await;
    world
        .put(with_scenario("control-scenario", note.clone()))
        .await;
    for (index, annotation) in proposed.iter().enumerate() {
        let field = world
            .put_as(
                ADMIN,
                with_field(&format!("field-{index}"), annotation.clone()),
                "",
                false,
            )
            .await;
        let scenario = world
            .put_as(
                ADMIN,
                with_scenario(&format!("scenario-{index}"), annotation.clone()),
                "",
                false,
            )
            .await;
        assert_eq!(
            field.as_ref().err().map(tonic::Status::code),
            Some(Code::InvalidArgument),
            "{} on a nested FieldSpec (index {index}): {field:?}",
            annotation.type_url
        );
        assert_eq!(
            scenario.as_ref().err().map(tonic::Status::code),
            Some(Code::InvalidArgument),
            "{} on a scenario: {scenario:?}",
            annotation.type_url
        );
        assert!(
            world
                .absent(pb::EntityKind::Event, &format!("field-{index}"))
                .await
        );
        assert!(
            world
                .absent(pb::EntityKind::CommandSlice, &format!("scenario-{index}"))
                .await
        );
    }
}

/// Every rejected update leaves content, revision and both reference indexes
/// exactly as they were.
#[tokio::test(flavor = "multi_thread")]
async fn invalid_updates_are_atomic_and_leave_the_index_untouched() {
    let mut world = world().await;
    let ns = world.ns.clone();
    world.put(component(&ns, "target", vec![])).await;
    let created = world.put(full_owner(&ns, "owner")).await;
    let before = world.get("owner").await;
    let incoming = world.incoming("target").await;
    let outgoing = world.outgoing("owner").await;
    assert_eq!(incoming.len(), 2);

    let valid = created.entity.clone().unwrap();
    let edit = |change: &dyn Fn(&mut Vec<prost_types::Any>)| {
        let mut entity = valid.clone();
        let Some(pb::entity::Kind::Component(body)) = entity.kind.as_mut() else {
            panic!("component")
        };
        change(&mut body.metadata);
        entity
    };
    let replace = |url: &'static str, with: prost_types::Any| {
        move |metadata: &mut Vec<prost_types::Any>| {
            for any in metadata.iter_mut().filter(|any| any.type_url == url) {
                *any = with.clone();
            }
        }
    };
    let draft = |title: Option<&str>, message: Option<&str>, status: &str| {
        lifecycle(pb::LifecycleAnnotation {
            status: status.into(),
            draft: Some(pb::DraftNotice {
                title: title.map(|value| pb::MetadataTitle {
                    value: value.into(),
                }),
                message: message.map(|value| pb::MetadataText {
                    value: value.into(),
                }),
            }),
            ..Default::default()
        })
    };
    let dated = |status: &str, date: &str| {
        lifecycle(pb::LifecycleAnnotation {
            status: status.into(),
            deprecated_on: Some(pb::CalendarDate { value: date.into() }),
            ..Default::default()
        })
    };
    let one_group = |group: pb::ResourceGroup| groups(vec![group]);
    let good_item = || item(target(&ns, "target"));
    let identified = |value: &str| pb::ConnectionId {
        value: value.into(),
    };
    let first = "10000000-0000-4000-8000-0000000000a1";
    let second = "10000000-0000-4000-8000-0000000000a2";
    let cases: Vec<(&str, pb::Entity)> = vec![
        (
            "blank draft message",
            edit(&replace(LIFECYCLE, draft(None, Some("  "), "draft"))),
        ),
        (
            "draft notice without a message",
            edit(&replace(LIFECYCLE, draft(None, None, "draft"))),
        ),
        (
            "blank optional draft title",
            edit(&replace(
                LIFECYCLE,
                draft(Some(" "), Some("Review"), "draft"),
            )),
        ),
        (
            "draft notice under accepted",
            edit(&replace(LIFECYCLE, draft(None, Some("Review"), "accepted"))),
        ),
        (
            "date under draft",
            edit(&replace(LIFECYCLE, dated("draft", "2030-01-01"))),
        ),
        (
            "impossible date",
            edit(&replace(LIFECYCLE, dated("deprecated", "2026-04-31"))),
        ),
        (
            "zero display limit",
            edit(&replace(
                GROUPS,
                one_group(pb::ResourceGroup {
                    items: vec![good_item()],
                    display_limit: Some(pb::PositiveDisplayLimit { value: 0 }),
                    ..Default::default()
                }),
            )),
        ),
        (
            "latest-version selector",
            edit(&replace(
                GROUPS,
                one_group(pb::ResourceGroup {
                    items: vec![pb::ResourceGroupItem {
                        connection_id: None,
                        target: Some(pb::EntityRef {
                            kind: pb::EntityKind::Component as i32,
                            id: Some(pb::Id {
                                version: 0,
                                ..id(&ns, "target")
                            }),
                        }),
                    }],
                    ..Default::default()
                }),
            )),
        ),
        (
            "duplicate item ID",
            edit(&replace(
                GROUPS,
                one_group(pb::ResourceGroup {
                    items: vec![
                        pb::ResourceGroupItem {
                            connection_id: Some(identified(first)),
                            target: Some(target(&ns, "target")),
                        },
                        pb::ResourceGroupItem {
                            connection_id: Some(identified(first)),
                            target: Some(target(&ns, "target")),
                        },
                    ],
                    ..Default::default()
                }),
            )),
        ),
        (
            "duplicate group ID",
            edit(&replace(
                GROUPS,
                groups(vec![
                    pb::ResourceGroup {
                        group_id: Some(identified(second)),
                        ..Default::default()
                    },
                    pb::ResourceGroup {
                        group_id: Some(identified(second)),
                        ..Default::default()
                    },
                ]),
            )),
        ),
        (
            "conflicting singleton",
            edit(&|metadata| {
                metadata.push(agent(
                    pb::AgentModel {
                        provider: Some(pb::ModelProvider {
                            value: "other".into(),
                        }),
                        ..Default::default()
                    },
                    vec![],
                ));
            }),
        ),
        (
            "identical singleton",
            edit(&|metadata| {
                let copy = metadata
                    .iter()
                    .find(|any| any.type_url == BADGES)
                    .unwrap()
                    .clone();
                metadata.push(copy);
            }),
        ),
        (
            "empty present model",
            edit(&replace(AGENT, agent(pb::AgentModel::default(), vec![]))),
        ),
        (
            "schema-valid malformed bytes",
            edit(&replace(BADGES, any(BADGES, vec![0x0a, 0x05, 0x01]))),
        ),
    ];
    for (name, proposal) in cases {
        let rejected = world
            .put_as(ADMIN, proposal, &created.etag, false)
            .await
            .expect_err(name);
        assert_eq!(rejected.code(), Code::InvalidArgument, "{name}: {rejected}");
        assert_eq!(world.get("owner").await, before, "{name}");
        assert_eq!(world.incoming("target").await, incoming, "{name}");
        assert_eq!(world.outgoing("owner").await, outgoing, "{name}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn backend_budgets_accept_the_maximum_and_refuse_one_more_atomically() {
    let mut world = world().await;
    let ns = world.ns.clone();
    world.put(component(&ns, "target", vec![])).await;

    let many_groups = |count: usize, per_group: usize| {
        groups(
            (0..count)
                .map(|_| pb::ResourceGroup {
                    items: vec![item(target(&ns, "target")); per_group],
                    ..Default::default()
                })
                .collect(),
        )
    };
    let limit = |value| {
        groups(vec![pb::ResourceGroup {
            display_limit: Some(pb::PositiveDisplayLimit { value }),
            items: vec![item(target(&ns, "target"))],
            ..Default::default()
        }])
    };
    let badge_list = |count: usize| {
        badges(
            (0..count)
                .map(|index| badge(&format!("badge-{index}")))
                .collect(),
        )
    };
    let tool_list = |count: usize| {
        agent(
            provider_only(),
            (0..count)
                .map(|index| tool(&format!("tool-{index}")))
                .collect(),
        )
    };
    let multibyte = |count: usize| {
        let mut one = tool("lookup");
        one.description = Some(pb::AgentDescription {
            value: "é".repeat(count),
        });
        agent(provider_only(), vec![one])
    };
    let boundaries: Vec<(&str, prost_types::Any, prost_types::Any)> = vec![
        ("tools", tool_list(64), tool_list(65)),
        ("badges", badge_list(32), badge_list(33)),
        ("groups", many_groups(32, 0), many_groups(33, 0)),
        ("items", many_groups(1, 256), many_groups(1, 257)),
        ("display limit", limit(500), limit(501)),
        ("multibyte description", multibyte(2048), multibyte(2049)),
    ];
    for (index, (name, maximum, over)) in boundaries.into_iter().enumerate() {
        let slug = format!("owner-{index}");
        let etag = world.put(component(&ns, &slug, vec![])).await.etag;
        let rejected = world
            .put_as(ADMIN, component(&ns, &slug, vec![over]), &etag, false)
            .await
            .expect_err(name);
        assert_eq!(rejected.code(), Code::InvalidArgument, "{name}: {rejected}");
        assert_eq!(world.get(&slug).await.etag, etag, "{name} refused");
        let accepted = world
            .put_as(
                ADMIN,
                component(&ns, &slug, vec![maximum.clone()]),
                &etag,
                false,
            )
            .await
            .unwrap_or_else(|status| panic!("{name} at the maximum: {status}"));
        let stored = world.get(&slug).await.entity.unwrap();
        if maximum.type_url == GROUPS {
            let sent = pb::ResourceGroupsAnnotation::decode(maximum.value.as_slice()).unwrap();
            let kept =
                pb::ResourceGroupsAnnotation::decode(metadata_of(&stored)[0].value.as_slice())
                    .unwrap();
            let shape = |value: &pb::ResourceGroupsAnnotation| {
                value
                    .groups
                    .iter()
                    .map(|group| group.items.len())
                    .collect::<Vec<_>>()
            };
            assert_eq!(shape(&kept), shape(&sent), "{name}");
        } else {
            assert_eq!(metadata_of(&stored), &vec![maximum], "{name}");
        }
        let _ = accepted;
    }
    let others = world.incoming("target").await.len();
    let etag = world.put(component(&ns, "owner-all", vec![])).await.etag;
    let all_items = many_groups(32, 8);
    let accepted = world
        .put_as(
            ADMIN,
            component(&ns, "owner-all", vec![all_items]),
            &etag,
            false,
        )
        .await
        .unwrap();
    assert_eq!(world.incoming("target").await.len(), others + 256);
    let over = many_groups(32, 9);
    let rejected = world
        .put_as(
            ADMIN,
            component(&ns, "owner-all", vec![over]),
            &accepted.etag,
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument);
    assert_eq!(world.incoming("target").await.len(), others + 256);
}

#[tokio::test(flavor = "multi_thread")]
async fn future_dates_never_change_status_and_note_attribution_is_untouched() {
    let mut world = world().await;
    let ns = world.ns.clone();
    let note = any(
        NOTE,
        pb::NoteAnnotation {
            text: "Decision".into(),
            author: "someone else".into(),
            created_at: "1999-01-01T00:00:00Z".into(),
        }
        .encode_to_vec(),
    );
    let future = lifecycle(pb::LifecycleAnnotation {
        status: "deprecated".into(),
        deprecated_on: Some(pb::CalendarDate {
            value: "9999-12-31".into(),
        }),
        ..Default::default()
    });
    let metadata = vec![note.clone(), future.clone()];
    let created = world
        .put(component(&ns, "dated", metadata.clone()))
        .await
        .entity
        .unwrap();
    let created_at = created.system.clone().unwrap().created_at;
    assert_ne!(created_at, "");
    assert_ne!(created_at, "1999-01-01T00:00:00Z");
    assert_eq!(metadata_of(&created), &metadata);

    let mut retitled = created.clone();
    let Some(pb::entity::Kind::Component(body)) = retitled.kind.as_mut() else {
        panic!("component")
    };
    body.title = "retitled".into();
    let etag = world.get("dated").await.etag;
    world.put_as(ADMIN, retitled, &etag, false).await.unwrap();
    let stored = world.get("dated").await.entity.unwrap();
    assert_eq!(metadata_of(&stored), &metadata, "note and lifecycle bytes");
    assert_eq!(stored.system.unwrap().created_at, created_at);
    let listed = world
        .client
        .list_entities(request(
            pb::ListEntitiesRequest {
                namespaces: vec![ns.clone()],
                kinds: vec![pb::EntityKind::Component as i32],
                lifecycle_status_in: vec!["deprecated".into()],
                ..Default::default()
            },
            ADMIN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.entities.len(), 1, "status stays as authored");
}

#[tokio::test(flavor = "multi_thread")]
async fn responsibility_resolves_the_component_and_refuses_local_selectors() {
    let mut world = world().await;
    let ns = world.ns.clone();
    world.put(component(&ns, "target", vec![])).await;
    let owner = world.put(full_owner(&ns, "owner")).await.entity.unwrap();
    let owner_uid = owner.system.clone().unwrap().uid;
    let groups_annotation = metadata_of(&owner)
        .iter()
        .find(|any| any.type_url == GROUPS)
        .unwrap();
    let stored_groups =
        pb::ResourceGroupsAnnotation::decode(groups_annotation.value.as_slice()).unwrap();
    let group_id = stored_groups.groups[0].group_id.clone().unwrap();
    let item_id = stored_groups.groups[0].items[0]
        .connection_id
        .clone()
        .unwrap();

    let team = |slug: &str, subject: pb::ResponsibilitySubject| pb::Entity {
        kind: Some(pb::entity::Kind::Team(pb::Team {
            id: Some(id(&ns, slug)),
            title: Some(pb::DirectoryDisplayName { value: slug.into() }),
            responsibilities: vec![pb::Responsibility {
                kind: pb::ResponsibilityKind::Owner as i32,
                subject: Some(subject),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    let entity_subject = pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Entity(target(
            &ns, "owner",
        ))),
    };
    world.put(team("owners", entity_subject.clone())).await;
    let listed = world
        .client
        .list_subject_assignments(request(
            pb::ListSubjectAssignmentsRequest {
                subject: Some(entity_subject),
                kind: None,
            },
            ADMIN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.assignments.len(), 1);
    assert_eq!(
        listed.subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );

    let tool_or_badge_stand_in = pb::ConnectionId {
        value: "10000000-0000-4000-8000-0000000000f1".into(),
    };
    for (name, connection) in [
        ("group ID", group_id),
        ("item ID", item_id),
        (
            "tool or badge without an identifier",
            tool_or_badge_stand_in,
        ),
    ] {
        let subject = pb::ResponsibilitySubject {
            selection: Some(pb::responsibility_subject::Selection::Connection(
                pb::ConnectionRef {
                    owner: Some(target(&ns, "owner")),
                    owner_uid: owner_uid.clone(),
                    id: Some(connection),
                },
            )),
        };
        let slug = format!("refused-{}", name.replace(' ', "-"));
        let rejected = world
            .put_as(ADMIN, team(&slug, subject), "", false)
            .await
            .expect_err(name);
        assert_eq!(rejected.code(), Code::InvalidArgument, "{name}: {rejected}");
        assert!(world.absent(pb::EntityKind::Team, &slug).await, "{name}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_looking_strings_and_unregistered_types_create_no_edges() {
    let mut world = world().await;
    let ns = world.ns.clone();
    world.put(component(&ns, "target", vec![])).await;
    let looking = format!(
        r#"{{"groups":[{{"items":[{{"target":{{"kind":"ENTITY_KIND_COMPONENT","id":{{"namespace":"{ns}","slug":"target","version":"1"}}}}}}]}}]}} {GROUPS}"#
    );
    let note = any(
        NOTE,
        pb::NoteAnnotation {
            text: looking.clone(),
            author: looking.clone(),
            created_at: looking.clone(),
        }
        .encode_to_vec(),
    );
    world
        .put(component(&ns, "mentions", vec![note.clone()]))
        .await;
    assert_eq!(world.incoming("target").await, []);
    assert_eq!(world.outgoing("mentions").await, []);
    assert_eq!(
        metadata_of(&world.get("mentions").await.entity.unwrap()),
        &vec![note]
    );

    let unregistered = world
        .put_as(
            ADMIN,
            component(
                &ns,
                "unregistered",
                vec![any(
                    "type.googleapis.com/catalog.unregistered.Annotation",
                    looking.into_bytes(),
                )],
            ),
            "",
            false,
        )
        .await;
    assert_eq!(world.incoming("target").await, []);
    if let Ok(response) = unregistered {
        assert_eq!(
            metadata_of(&response.entity.unwrap())[0].type_url,
            "type.googleapis.com/catalog.unregistered.Annotation",
            "an unknown type is retained verbatim and never reinterpreted"
        );
        assert_eq!(world.outgoing("unregistered").await, []);
    }
}

/// The apply request re-evaluates every guard: the owner incarnation as the
/// etag (the store revision of a recreated key never repeats), the target as
/// a live native entity, and the caller's authorization.
#[tokio::test(flavor = "multi_thread")]
async fn apply_rejects_stale_incarnation_deleted_target_and_revoked_authorization() {
    let mut world = world().await;
    let ns = world.ns.clone();
    world.put(component(&ns, "target", vec![])).await;
    let first = world.put(full_owner(&ns, "owner")).await;
    let first_uid = first.entity.clone().unwrap().system.unwrap().uid;

    world
        .delete("owner", pb::delete_entity_request::Mode::Force)
        .await;
    let mut recreated = full_owner(&ns, "owner");
    recreated.system = None;
    let recreated = world.put(recreated).await;
    let recreated_entity = recreated.entity.clone().unwrap();
    assert_ne!(recreated_entity.system.clone().unwrap().uid, first_uid);
    assert_ne!(recreated.etag, first.etag);

    let mut edit = recreated_entity.clone();
    let Some(pb::entity::Kind::Component(body)) = edit.kind.as_mut() else {
        panic!("component")
    };
    body.title = "stale edit".into();
    let before = world.get("owner").await;
    let incoming = world.incoming("target").await;
    let stale = world
        .put_as(WRITER, edit.clone(), &first.etag, false)
        .await
        .unwrap_err();
    assert_eq!(stale.code(), Code::Aborted, "{stale}");
    assert_eq!(world.get("owner").await, before);
    assert_eq!(world.incoming("target").await, incoming);

    let preview = world
        .put_as(WRITER, edit.clone(), &recreated.etag, true)
        .await
        .unwrap();
    assert!(preview.entity.is_some());
    assert_eq!(
        world.get("owner").await,
        before,
        "a preview publishes nothing"
    );
    let batch = |validate_only: bool| pb::BatchMutateRequest {
        validate_only,
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(put_request(
                edit.clone(),
                &recreated.etag,
                false,
            ))),
        }],
        ..Default::default()
    };
    let validated = world
        .client
        .batch_mutate(request(batch(true), WRITER))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        validated.status,
        pb::batch_mutate_response::Status::Validated as i32
    );

    world
        .delete("target", pb::delete_entity_request::Mode::Force)
        .await;
    let dangling = world
        .client
        .batch_mutate(request(batch(false), WRITER))
        .await;
    assert!(
        dangling.is_err()
            || dangling.as_ref().unwrap().get_ref().status
                != pb::batch_mutate_response::Status::Applied as i32,
        "{dangling:?}"
    );
    assert_eq!(
        world.get("owner").await,
        before,
        "deleted target publishes nothing"
    );
    world.put(component(&ns, "target", vec![])).await;

    let revoked_edit = edit.clone();
    let validated = world
        .put_as(WRITER, revoked_edit.clone(), &recreated.etag, true)
        .await;
    assert!(validated.is_ok(), "{validated:?}");
    let moved = world
        .client
        .move_namespace(request(
            pb::MoveNamespaceRequest {
                id: ns.clone(),
                parent: "hidden".into(),
            },
            ADMIN,
        ))
        .await;
    assert!(moved.is_ok(), "{moved:?}");
    for apply in [
        world
            .put_as(WRITER, revoked_edit.clone(), &recreated.etag, false)
            .await
            .map(|_| ()),
        world
            .client
            .batch_mutate(request(batch(false), WRITER))
            .await
            .map(|_| ()),
    ] {
        let denied = apply.unwrap_err();
        assert!(
            matches!(denied.code(), Code::PermissionDenied | Code::NotFound),
            "{denied}"
        );
    }
    assert_eq!(
        world.get("owner").await,
        before,
        "revoked apply publishes nothing"
    );
    assert_eq!(world.incoming("target").await.len(), incoming.len());
}
