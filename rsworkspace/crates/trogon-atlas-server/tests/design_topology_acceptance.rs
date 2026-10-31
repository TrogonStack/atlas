#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::large_futures,
    clippy::too_many_lines
)]

use std::{collections::BTreeSet, sync::Arc};

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

const ADMIN: &str = "catalog-admin";
const READER: &str = "catalog-reader";
static HEAVY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
type Client = EventModelServiceClient<Channel>;
type Selection = pb::topology_step_target::Selection;

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

async fn serve(service: EventModelServiceImpl, scoped_parent: &str, scoped_token: &str) -> Client {
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
[principals.scoped]
role = "writer"
parent = "{scoped_parent}"
tokens = ["{scoped_token}"]
"#
        ))
        .unwrap(),
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
    Client::connect(format!("http://{addr}")).await.unwrap()
}

struct Scope {
    client: Client,
    ns: String,
    token: String,
}

async fn scope_over(store: Arc<dyn trogon_atlas_store::Store>, name: &str) -> Scope {
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    scope_service(service, name).await
}

async fn scope_service(service: EventModelServiceImpl, name: &str) -> Scope {
    let parent = format!("owner-{name}");
    let token = format!("scoped-{name}");
    let mut client = serve(service, &parent, &token).await;
    let ns = register(&mut client, &format!("acc-{name}"), &parent).await;
    Scope { client, ns, token }
}

async fn scope(name: &str) -> Scope {
    scope_over(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        name,
    )
    .await
}

async fn register(client: &mut Client, name: &str, parent: &str) -> String {
    client
        .register_namespace(request(
            pb::RegisterNamespaceRequest {
                name: name.into(),
                parent: parent.into(),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id
}

fn id_in(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}
fn title(value: &str) -> pb::CatalogTitle {
    pb::CatalogTitle {
        value: value.into(),
    }
}
fn text(value: &str) -> pb::CatalogText {
    pb::CatalogText {
        value: value.into(),
    }
}
fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}
fn reference(value: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(value).unwrap()
}
fn uid(value: &pb::Entity) -> String {
    value.system.as_ref().unwrap().uid.clone()
}
fn step(name: &str, target: Option<Selection>) -> pb::TopologyStep {
    pb::TopologyStep {
        title: Some(title(name)),
        target: target.map(|selection| pb::TopologyStepTarget {
            selection: Some(selection),
        }),
        ..Default::default()
    }
}
fn graph_in(
    ns: &str,
    slug: &str,
    profile: pb::DesignGraphProfile,
    steps: Vec<pb::TopologyStep>,
) -> pb::Entity {
    entity(pb::entity::Kind::DesignGraph(pb::DesignGraph {
        id: Some(id_in(ns, slug, 1)),
        title: Some(title(slug)),
        profile: profile as i32,
        steps,
        ..Default::default()
    }))
}
fn body(value: &pb::Entity) -> &pb::DesignGraph {
    match value.kind.as_ref() {
        Some(pb::entity::Kind::DesignGraph(graph)) => graph,
        _ => panic!("design graph"),
    }
}
fn body_mut(value: &mut pb::Entity) -> &mut pb::DesignGraph {
    match value.kind.as_mut() {
        Some(pb::entity::Kind::DesignGraph(graph)) => graph,
        _ => panic!("design graph"),
    }
}
fn link(to: &pb::LocalValueId, label: &str) -> pb::TopologyTransition {
    pb::TopologyTransition {
        label: Some(title(label)),
        target_step_id: Some(to.clone()),
        ..Default::default()
    }
}
fn step_id(value: &pb::Entity, index: usize) -> pb::LocalValueId {
    body(value).steps[index].step_id.clone().unwrap()
}
fn catalog_entity(reference: &pb::EntityRef) -> pb::CatalogSubject {
    pb::CatalogSubject {
        selection: Some(pb::catalog_subject::Selection::Entity(reference.clone())),
    }
}

impl Scope {
    fn id(&self, slug: &str) -> pb::Id {
        id_in(&self.ns, slug, 1)
    }
    fn graph(&self, slug: &str, steps: Vec<pb::TopologyStep>) -> pb::Entity {
        graph_in(&self.ns, slug, pb::DesignGraphProfile::Flow, steps)
    }
    fn graph_ref(&self, slug: &str) -> pb::DesignGraphRef {
        pb::DesignGraphRef {
            id: Some(self.id(slug)),
        }
    }
    fn subflow(&self, slug: &str) -> Selection {
        Selection::Subflow(self.graph_ref(slug))
    }
    async fn try_put(
        &mut self,
        value: pb::Entity,
        etag: &str,
        token: &str,
    ) -> Result<pb::PutEntityResponse, tonic::Status> {
        self.client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(value),
                    if_match: etag.into(),
                    ..Default::default()
                },
                token,
                None,
            ))
            .await
            .map(tonic::Response::into_inner)
    }
    async fn try_write(
        &mut self,
        value: pb::Entity,
        etag: &str,
    ) -> Result<pb::PutEntityResponse, tonic::Status> {
        let token = self.token.clone();
        self.try_put(value, etag, &token).await
    }
    async fn put(&mut self, value: pb::Entity) -> pb::PutEntityResponse {
        self.try_write(value, "").await.unwrap()
    }
    async fn guarded(&mut self, value: pb::Entity, etag: &str) -> pb::PutEntityResponse {
        self.try_write(value, etag).await.unwrap()
    }
    async fn validate(
        &mut self,
        value: pb::Entity,
    ) -> Result<pb::PutEntityResponse, tonic::Status> {
        let token = self.token.clone();
        self.client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(value),
                    validate_only: true,
                    ..Default::default()
                },
                &token,
                None,
            ))
            .await
            .map(tonic::Response::into_inner)
    }
    async fn get(&mut self, slug: &str) -> Result<pb::GetEntityResponse, tonic::Status> {
        let token = self.token.clone();
        self.client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::DesignGraph as i32,
                    id: Some(self.id(slug)),
                },
                &token,
                None,
            ))
            .await
            .map(tonic::Response::into_inner)
    }
    async fn detail(
        &mut self,
        slug: &str,
    ) -> Result<pb::GetDesignGraphDetailResponse, tonic::Status> {
        let token = self.token.clone();
        let graph = Some(self.graph_ref(slug));
        self.client
            .get_design_graph_detail(request(
                pb::GetDesignGraphDetailRequest { graph },
                &token,
                None,
            ))
            .await
            .map(tonic::Response::into_inner)
    }
    async fn traverse(
        &mut self,
        slug: &str,
        follow: bool,
        max_depth: u32,
    ) -> Result<pb::TraverseDesignGraphTopologyResponse, tonic::Status> {
        let token = self.token.clone();
        let root = Some(self.graph_ref(slug));
        self.client
            .traverse_design_graph_topology(request(
                pb::TraverseDesignGraphTopologyRequest {
                    root,
                    follow_subflows: follow,
                    max_depth,
                },
                &token,
                None,
            ))
            .await
            .map(tonic::Response::into_inner)
    }
    async fn incoming(
        &mut self,
        slug: &str,
    ) -> Result<pb::ListIncomingDesignGraphSubflowsResponse, tonic::Status> {
        let token = self.token.clone();
        let graph = Some(self.graph_ref(slug));
        self.client
            .list_incoming_design_graph_subflows(request(
                pb::ListIncomingDesignGraphSubflowsRequest { graph },
                &token,
                None,
            ))
            .await
            .map(tonic::Response::into_inner)
    }
    async fn subject_graphs(
        &mut self,
        subject: pb::CatalogSubject,
    ) -> Result<pb::ListSubjectDesignGraphsResponse, tonic::Status> {
        let token = self.token.clone();
        self.client
            .list_subject_design_graphs(request(
                pb::ListSubjectDesignGraphsRequest {
                    subject: Some(subject),
                },
                &token,
                None,
            ))
            .await
            .map(tonic::Response::into_inner)
    }
    async fn outgoing(&mut self, slug: &str) -> pb::GetReferencesResponse {
        let token = self.token.clone();
        let id = Some(self.id(slug));
        self.client
            .get_outgoing_references(request(
                pb::GetReferencesRequest {
                    kind: pb::EntityKind::DesignGraph as i32,
                    id,
                    ..Default::default()
                },
                &token,
                None,
            ))
            .await
            .unwrap()
            .into_inner()
    }
    async fn delete(
        &mut self,
        slug: &str,
        force: bool,
    ) -> Result<tonic::Response<pb::DeleteEntityResponse>, tonic::Status> {
        let token = self.token.clone();
        let id = Some(self.id(slug));
        self.client
            .delete_entity(request(
                pb::DeleteEntityRequest {
                    kind: pb::EntityKind::DesignGraph as i32,
                    id,
                    mode: if force {
                        pb::delete_entity_request::Mode::Force as i32
                    } else {
                        pb::delete_entity_request::Mode::Unspecified as i32
                    },
                    ..Default::default()
                },
                &token,
                None,
            ))
            .await
    }
}

/// Native catalog anchors mirroring the acceptance document's fixture table.
struct Anchors {
    component: pb::Entity,
    channel: pb::Entity,
    data_store: pb::Entity,
    data_product: pb::Entity,
    specification: pb::Entity,
    event: pb::Entity,
    command: pb::Entity,
    read_model: pb::Entity,
    document: pb::Entity,
}

impl Anchors {
    fn member(&self) -> pb::CatalogSubject {
        let Some(pb::entity::Kind::Specification(spec)) = &self.specification.kind else {
            panic!("specification")
        };
        pb::CatalogSubject {
            selection: Some(pb::catalog_subject::Selection::Member(
                pb::ContractMemberRef {
                    specification: Some(pb::SpecificationRef {
                        id: reference(&self.specification).id,
                    }),
                    owner_uid: uid(&self.specification),
                    kind: pb::ContractMemberKind::Message as i32,
                    member_id: spec.messages[0].member_id.clone(),
                    expected_digest: None,
                },
            )),
        }
    }
}

async fn anchors(scope: &mut Scope) -> Anchors {
    let ns = scope.ns.clone();
    let component = scope
        .put(entity(pb::entity::Kind::Component(pb::Component {
            id: Some(id_in(&ns, "checkout-api", 1)),
            title: "Checkout API".into(),
            ..Default::default()
        })))
        .await
        .entity
        .unwrap();
    let read_model = scope
        .put(entity(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id_in(&ns, "order-view", 1)),
            title: "Order view".into(),
            ..Default::default()
        })))
        .await
        .entity
        .unwrap();
    let data_store = scope
        .put(entity(pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(id_in(&ns, "order-db", 1)),
            title: "Order database".into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Known(
                    pb::store_class::Known::Cache as i32,
                )),
            }),
            attachments: vec![pb::ReadModelAttachment {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id_in(&ns, "order-view", 1)),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })))
        .await
        .entity
        .unwrap();
    scope
        .put(entity(pb::entity::Kind::Processor(pb::Processor {
            id: Some(id_in(&ns, "order-notifier", 1)),
            title: "Order notifier".into(),
            ..Default::default()
        })))
        .await;
    let slice = scope
        .put(entity(pb::entity::Kind::AutomationSlice(
            pb::AutomationSlice {
                id: Some(id_in(&ns, "observe-order", 1)),
                title: "Observe order".into(),
                source_read_models: vec![pb::ReadModelEdge {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id_in(&ns, "order-view", 1)),
                    }),
                    ..Default::default()
                }],
                processor: Some(pb::ProcessorEdge {
                    processor: Some(pb::ProcessorRef {
                        id: Some(id_in(&ns, "order-notifier", 1)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )))
        .await
        .entity
        .unwrap();
    let Some(pb::entity::Kind::AutomationSlice(slice_body)) = &slice.kind else {
        panic!("slice")
    };
    let channel = scope
        .put(entity(pb::entity::Kind::Channel(pb::Channel {
            id: Some(id_in(&ns, "order-events", 1)),
            title: "Order events".into(),
            bindings: vec![pb::ChannelBinding {
                observation: Some(pb::ConnectionRef {
                    owner: Some(reference(&slice)),
                    owner_uid: uid(&slice),
                    id: slice_body.source_read_models[0].connection_id.clone(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })))
        .await
        .entity
        .unwrap();
    let data_product = scope
        .put(entity(pb::entity::Kind::DataProduct(pb::DataProduct {
            id: Some(id_in(&ns, "orders-product", 1)),
            title: "Orders product".into(),
            outputs: vec![pb::DataOutputPort {
                name: Some(pb::PortName {
                    value: "Orders".into(),
                }),
                data: Some(pb::OutputDataSelection {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id_in(&ns, "order-view", 1)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        })))
        .await
        .entity
        .unwrap();
    let specification = scope
        .put(entity(pb::entity::Kind::Specification(pb::Specification {
            id: Some(id_in(&ns, "order-contract", 1)),
            title: "Order contract".into(),
            messages: vec![pb::ContractMessage {
                name: Some(pb::MemberName {
                    value: "OrderPlaced".into(),
                }),
                mapping: Some(pb::MessageReadModelMapping {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id_in(&ns, "order-view", 1)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        })))
        .await
        .entity
        .unwrap();
    let event = scope
        .put(entity(pb::entity::Kind::Event(pb::Event {
            id: Some(id_in(&ns, "order-placed", 1)),
            title: "Order placed".into(),
            ..Default::default()
        })))
        .await
        .entity
        .unwrap();
    let command = scope
        .put(entity(pb::entity::Kind::Command(pb::Command {
            id: Some(id_in(&ns, "place-order", 1)),
            title: "Place order".into(),
            ..Default::default()
        })))
        .await
        .entity
        .unwrap();
    let document = scope
        .put(entity(pb::entity::Kind::Document(pb::Document {
            id: Some(id_in(&ns, "checkout-layout", 1)),
            title: Some(title("Checkout layout")),
            subjects: vec![pb::SubjectAssociation {
                subject: Some(catalog_entity(&reference(&component))),
                ..Default::default()
            }],
            body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                value: "https://example.com/layout".into(),
            })),
            ..Default::default()
        })))
        .await
        .entity
        .unwrap();
    Anchors {
        component,
        channel,
        data_store,
        data_product,
        specification,
        event,
        command,
        read_model,
        document,
    }
}

fn custom(name: &str) -> Selection {
    Selection::Custom(pb::TopologyCustomStep {
        title: Some(title(name)),
        ..Default::default()
    })
}

fn state(value: i32) -> pb::CatalogResolutionState {
    pb::CatalogResolutionState::try_from(value).unwrap()
}

fn ids_of(value: &pb::Entity) -> Vec<String> {
    body(value)
        .steps
        .iter()
        .map(|s| s.step_id.clone().unwrap().value)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_fixture_preserves_unicode_order_presence_and_distinct_ids_for_equal_catalog_selections(
) {
    let mut scope = scope("complete-fixture").await;
    let anchors = anchors(&mut scope).await;
    let component = Selection::Component(reference(&anchors.component));
    let unicode = "Vérifier la commande 注文 \u{1F6D2}";
    scope
        .put(scope.graph(
            "payment-flow",
            vec![step("Authorize", None), step("Capture", None)],
        ))
        .await;
    let steps = vec![
        step(unicode, Some(custom("Custom check"))),
        step(
            "Message member",
            Some(Selection::CatalogMessage(anchors.member())),
        ),
        step(
            "Message event",
            Some(Selection::CatalogMessage(catalog_entity(&reference(
                &anchors.event,
            )))),
        ),
        step(
            "Message command",
            Some(Selection::CatalogMessage(catalog_entity(&reference(
                &anchors.command,
            )))),
        ),
        step("Component", Some(component.clone())),
        step("Component again", Some(component)),
        step(
            "Channel",
            Some(Selection::Channel(reference(&anchors.channel))),
        ),
        step(
            "Data store",
            Some(Selection::DataStore(reference(&anchors.data_store))),
        ),
        step(
            "Data product",
            Some(Selection::DataProduct(reference(&anchors.data_product))),
        ),
        step("Payments", Some(scope.subflow("payment-flow"))),
        step("Generic", None),
    ];
    let created = scope.put(scope.graph("checkout-flow", steps)).await;
    let first = created.entity.unwrap();
    let ids = ids_of(&first);
    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), ids.len());
    assert!(body(&first).steps.iter().all(|s| s.transitions.is_empty()));

    let mut authored = first.clone();
    let ids: Vec<_> = (0..11).map(|index| step_id(&first, index)).collect();
    let graph = body_mut(&mut authored);
    graph.entry_steps = vec![ids[0].clone(), ids[4].clone()];
    graph.steps[0].transitions = vec![link(&ids[4], "ready")];
    graph.steps[4].transitions = vec![
        link(&ids[6], "publish"),
        link(&ids[7], "persist"),
        link(&ids[0], "retry"),
    ];
    graph.steps[5].transitions = vec![link(&ids[9], "pay"), link(&ids[9], "pay")];
    let authored = scope.guarded(authored, &created.etag).await.entity.unwrap();
    assert_eq!(ids_of(&authored), ids_of(&first));

    let read = scope.detail("checkout-flow").await.unwrap();
    assert_eq!(read.entity.as_ref().unwrap().kind, authored.kind);
    assert!(!read.incomplete && read.failures.is_empty());
    assert_eq!(
        state(read.layout_state),
        pb::CatalogResolutionState::Unspecified
    );
    let graph = body(read.entity.as_ref().unwrap());
    assert_eq!(graph.profile, pb::DesignGraphProfile::Flow as i32);
    assert_eq!(graph.steps[0].title.as_ref().unwrap().value, unicode);
    assert_eq!(
        graph
            .steps
            .iter()
            .map(|s| s.title.clone().unwrap().value)
            .collect::<Vec<_>>()[1..4],
        ["Message member", "Message event", "Message command"]
    );
    assert_eq!(
        graph.steps[4]
            .transitions
            .iter()
            .map(|t| t.label.clone().unwrap().value)
            .collect::<Vec<_>>(),
        ["publish", "persist", "retry"]
    );
    assert_eq!(graph.entry_steps, vec![ids[0].clone(), ids[4].clone()]);
    let connection_ids: BTreeSet<_> = graph
        .steps
        .iter()
        .flat_map(|s| {
            s.transitions
                .iter()
                .map(|t| t.connection_id.clone().unwrap().value)
        })
        .collect();
    assert_eq!(
        connection_ids.len(),
        6,
        "equal parallel transitions stay distinct"
    );

    assert_eq!(read.targets.len(), 10);
    for target in &read.targets {
        assert_eq!(state(target.state), pb::CatalogResolutionState::Resolved);
    }
    let by_field: std::collections::BTreeMap<_, _> =
        read.targets.iter().map(|t| (t.field.as_str(), t)).collect();
    assert!(by_field["steps[0].target"].owner.is_none());
    for (field, expected) in [
        ("steps[4].target", &anchors.component),
        ("steps[5].target", &anchors.component),
        ("steps[6].target", &anchors.channel),
        ("steps[7].target", &anchors.data_store),
        ("steps[8].target", &anchors.data_product),
    ] {
        let owner = by_field[field].owner.as_ref().unwrap();
        assert_eq!(owner.uid, uid(expected), "{field}");
    }
    assert_eq!(by_field["steps[4].target"].step_id, Some(ids[4].clone()));
    assert_eq!(by_field["steps[5].target"].step_id, Some(ids[5].clone()));
    assert_ne!(ids[4], ids[5]);

    scope
        .put(scope.graph("system-context", vec![step("S", None)]))
        .await;
}

fn rejected(status: &tonic::Status, code: Code, fragment: &str) {
    assert_eq!(status.code(), code, "{status}");
    assert!(
        status.message().contains(fragment),
        "expected {fragment:?} in {:?}",
        status.message()
    );
}

async fn stored(scope: &mut Scope, slug: &str) -> (pb::Entity, String) {
    let read = scope.get(slug).await.unwrap();
    (read.entity.unwrap(), read.etag)
}

async fn staged(scope: &mut Scope, slug: &str, titles: &[&str]) -> (pb::Entity, String) {
    let created = scope
        .put(scope.graph(slug, titles.iter().map(|t| step(t, None)).collect()))
        .await;
    (created.entity.unwrap(), created.etag)
}

#[tokio::test(flavor = "multi_thread")]
async fn every_adopted_target_arm_admits_exact_selections_and_rejects_wrong_kind_version_and_dangling_refs(
) {
    let mut scope = scope("target-arms").await;
    let a = anchors(&mut scope).await;
    let ns = scope.ns.clone();
    let mut wrong_version = reference(&a.component);
    wrong_version.id.as_mut().unwrap().version = 2;
    let mut dangling = reference(&a.component);
    dangling.id.as_mut().unwrap().slug = "missing".into();
    let unresolved = "CATALOG_REFERENCE_UNRESOLVED";

    let positives: Vec<(&str, Selection)> = vec![
        ("member", Selection::CatalogMessage(a.member())),
        (
            "event",
            Selection::CatalogMessage(catalog_entity(&reference(&a.event))),
        ),
        (
            "command",
            Selection::CatalogMessage(catalog_entity(&reference(&a.command))),
        ),
        ("component", Selection::Component(reference(&a.component))),
        ("channel", Selection::Channel(reference(&a.channel))),
        ("data store", Selection::DataStore(reference(&a.data_store))),
        (
            "data product",
            Selection::DataProduct(reference(&a.data_product)),
        ),
        (
            "actor",
            Selection::Actor(pb::TopologyActor {
                name: Some(title("Customer")),
                summary: Some(text("Places orders")),
            }),
        ),
        (
            "custom",
            Selection::Custom(pb::TopologyCustomStep {
                title: Some(title("Manual review")),
                summary: Some(text("Back office")),
                icon: "box".into(),
                url: "https://example.com/runbook".into(),
                properties: (0..32)
                    .map(|i| (format!("k{i}"), format!("v{i}")))
                    .collect(),
            }),
        ),
        (
            "external system",
            Selection::ExternalSystem(pb::TopologyExternalSystem {
                name: Some(title("Payments gateway")),
                summary: None,
                url: "https://example.com/psp".into(),
            }),
        ),
    ];
    for (name, selection) in &positives {
        scope
            .validate(scope.graph("arm", vec![step(name, Some(selection.clone()))]))
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }

    let negatives: Vec<(&str, Selection, &str)> = vec![
        (
            "component given a read model",
            Selection::Component(reference(&a.read_model)),
            "component target kind",
        ),
        (
            "component given a channel",
            Selection::Component(reference(&a.channel)),
            "component target kind",
        ),
        (
            "wrong exact version",
            Selection::Component(wrong_version),
            "absent, stale or unavailable",
        ),
        (
            "dangling component",
            Selection::Component(dangling),
            "absent, stale or unavailable",
        ),
        (
            "message given a read model",
            Selection::CatalogMessage(catalog_entity(&reference(&a.read_model))),
            "must be an Event or Command",
        ),
        (
            "message given a component",
            Selection::CatalogMessage(catalog_entity(&reference(&a.component))),
            "must be an Event or Command",
        ),
        (
            "missing subflow",
            Selection::Subflow(pb::DesignGraphRef {
                id: Some(id_in(&ns, "nowhere", 1)),
            }),
            "absent, stale or unavailable",
        ),
        (
            "subflow wrong version",
            Selection::Subflow(pb::DesignGraphRef {
                id: Some(id_in(&ns, "arm", 7)),
            }),
            "absent, stale or unavailable",
        ),
        (
            "custom plain http url",
            Selection::Custom(pb::TopologyCustomStep {
                title: Some(title("c")),
                url: "http://example.com".into(),
                ..Default::default()
            }),
            "requires HTTPS",
        ),
        (
            "custom icon with capitals",
            Selection::Custom(pb::TopologyCustomStep {
                title: Some(title("c")),
                icon: "Bad Icon".into(),
                ..Default::default()
            }),
            "lowercase token",
        ),
        (
            "custom beyond property limit",
            Selection::Custom(pb::TopologyCustomStep {
                title: Some(title("c")),
                properties: (0..33).map(|i| (format!("k{i}"), "v".into())).collect(),
                ..Default::default()
            }),
            "custom property limit exceeded",
        ),
        (
            "actor without name",
            Selection::Actor(pb::TopologyActor::default()),
            "catalog title required",
        ),
        (
            "external system with ftp url",
            Selection::ExternalSystem(pb::TopologyExternalSystem {
                name: Some(title("e")),
                url: "ftp://example.com".into(),
                ..Default::default()
            }),
            "requires HTTPS",
        ),
    ];
    for (name, selection, fragment) in negatives {
        let status = scope
            .validate(scope.graph("arm", vec![step(name, Some(selection.clone()))]))
            .await
            .unwrap_err();
        rejected(&status, Code::InvalidArgument, unresolved);
        rejected(&status, Code::InvalidArgument, fragment);
        let status = scope
            .try_write(
                scope.graph("arm-write", vec![step(name, Some(selection))]),
                "",
            )
            .await
            .unwrap_err();
        rejected(&status, Code::InvalidArgument, fragment);
    }
    assert_eq!(
        scope.get("arm-write").await.unwrap_err().code(),
        Code::NotFound,
        "rejected writes leave no owner"
    );

    let created = scope
        .put(
            scope.graph(
                "inline-arms",
                positives
                    .iter()
                    .skip(7)
                    .map(|(name, selection)| step(name, Some(selection.clone())))
                    .collect(),
            ),
        )
        .await;
    let read = scope.detail("inline-arms").await.unwrap();
    assert_eq!(read.entity.unwrap().kind, created.entity.unwrap().kind);
    assert_eq!(read.targets.len(), 3);
    assert!(read
        .targets
        .iter()
        .all(|t| state(t.state) == pb::CatalogResolutionState::Resolved && t.owner.is_none()));
    let Some(pb::entity::Kind::DesignGraph(stored)) =
        scope.get("inline-arms").await.unwrap().entity.unwrap().kind
    else {
        panic!("design graph")
    };
    let Some(Selection::Custom(custom)) = stored.steps[1].target.clone().unwrap().selection else {
        panic!("custom")
    };
    assert_eq!(custom.properties.len(), 32);
    assert_eq!(custom.properties["k31"], "v31");
    assert_eq!(custom.icon, "box");
}

#[tokio::test(flavor = "multi_thread")]
async fn layout_document_state_is_absent_resolved_or_unavailable_and_missing_layouts_are_rejected()
{
    let mut scope = scope("layout").await;
    let a = anchors(&mut scope).await;
    let ns = scope.ns.clone();
    scope
        .put(scope.graph("no-layout", vec![step("s", None)]))
        .await;
    assert_eq!(
        state(scope.detail("no-layout").await.unwrap().layout_state),
        pb::CatalogResolutionState::Unspecified
    );

    let mut with_layout = scope.graph("with-layout", vec![step("s", None)]);
    body_mut(&mut with_layout).layout_document = Some(pb::DocumentRef {
        id: Some(id_in(&ns, "checkout-layout", 1)),
    });
    scope.put(with_layout).await;
    assert_eq!(
        state(scope.detail("with-layout").await.unwrap().layout_state),
        pb::CatalogResolutionState::Resolved
    );

    let mut missing = scope.graph("missing-layout", vec![step("s", None)]);
    body_mut(&mut missing).layout_document = Some(pb::DocumentRef {
        id: Some(id_in(&ns, "nolayout", 1)),
    });
    let status = scope.try_write(missing, "").await.unwrap_err();
    rejected(
        &status,
        Code::InvalidArgument,
        "absent, stale or unavailable",
    );
    assert_eq!(
        scope.get("missing-layout").await.unwrap_err().code(),
        Code::NotFound
    );

    let token = scope.token.clone();
    scope
        .client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: a
                    .document
                    .kind
                    .as_ref()
                    .map(|_| id_in(&ns, "checkout-layout", 1)),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            &token,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        state(scope.detail("with-layout").await.unwrap().layout_state),
        pb::CatalogResolutionState::Unavailable
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_local_ids_transitions_entries_and_same_write_unknowns_leave_stored_state_unchanged(
) {
    let mut scope = scope("invalid-ids").await;
    let (created, etag) = staged(&mut scope, "flow", &["First", "Second"]).await;
    let ids: Vec<_> = (0..2).map(|i| step_id(&created, i)).collect();
    let unknown = pb::LocalValueId {
        value: "01a1851a-ffae-716c-a560-df3f6a391f99".into(),
    };
    let malformed = pb::LocalValueId {
        value: "not a valid id".into(),
    };
    let empty = pb::LocalValueId {
        value: String::new(),
    };

    type Mutation = Box<dyn Fn(&mut pb::Entity)>;
    let cases: Vec<(&str, Mutation)> = vec![
        (
            "transition to unknown step",
            Box::new({
                let u = unknown.clone();
                let from = ids[0].clone();
                move |e| {
                    let _ = &from;
                    body_mut(e).steps[0].transitions = vec![link(&u, "x")];
                }
            }),
        ),
        (
            "transition to malformed step",
            Box::new({
                let m = malformed.clone();
                move |e| body_mut(e).steps[0].transitions = vec![link(&m, "x")]
            }),
        ),
        (
            "transition to empty step",
            Box::new({
                let m = empty.clone();
                move |e| body_mut(e).steps[0].transitions = vec![link(&m, "x")]
            }),
        ),
        (
            "entry to unknown step",
            Box::new({
                let u = unknown.clone();
                move |e| body_mut(e).entry_steps = vec![u.clone()]
            }),
        ),
        (
            "entry to malformed step",
            Box::new({
                let m = malformed.clone();
                move |e| body_mut(e).entry_steps = vec![m.clone()]
            }),
        ),
        (
            "explicit empty step id",
            Box::new({
                let m = empty.clone();
                move |e| body_mut(e).steps[1].step_id = Some(m.clone())
            }),
        ),
        (
            "explicit malformed step id",
            Box::new({
                let m = malformed.clone();
                move |e| body_mut(e).steps[1].step_id = Some(m.clone())
            }),
        ),
        (
            "duplicate step id",
            Box::new({
                let first = ids[0].clone();
                move |e| body_mut(e).steps[1].step_id = Some(first.clone())
            }),
        ),
        (
            "unknown supplied step id",
            Box::new({
                let u = unknown.clone();
                move |e| body_mut(e).steps[1].step_id = Some(u.clone())
            }),
        ),
        (
            "unknown supplied connection id",
            Box::new({
                let to = ids[1].clone();
                move |e| {
                    let mut t = link(&to, "x");
                    t.connection_id = Some(pb::ConnectionId {
                        value: "01a1851a-ffae-716c-a560-df3f6a391f98".into(),
                    });
                    body_mut(e).steps[0].transitions = vec![t];
                }
            }),
        ),
        (
            "same write new step referenced by unknown id",
            Box::new({
                let u = unknown.clone();
                move |e| {
                    let g = body_mut(e);
                    g.steps.push(step("Third", None));
                    g.steps[0].transitions = vec![link(&u, "to-new")];
                }
            }),
        ),
        (
            "self subflow",
            Box::new(|e| {
                let g = body_mut(e);
                let me = g.id.clone();
                g.steps[0].target = Some(pb::TopologyStepTarget {
                    selection: Some(Selection::Subflow(pb::DesignGraphRef { id: me })),
                });
            }),
        ),
    ];
    for (name, mutate) in cases {
        let mut attempt = created.clone();
        mutate(&mut attempt);
        let status = scope.try_write(attempt, &etag).await.unwrap_err();
        let fragment = match name {
            "transition to unknown step" | "same write new step referenced by unknown id" => {
                "transition target step missing from graph"
            }
            "entry to unknown step" => "entry step missing from graph",
            "transition to malformed step" | "entry to malformed step" => "invalid character",
            "transition to empty step" | "explicit empty step id" => "invalid length",
            "explicit malformed step id" => "non-nil lowercase hyphenated UUID",
            "duplicate step id" => "duplicate connection ID",
            "unknown supplied step id" => "step ID not active; omit IDs for additions",
            "unknown supplied connection id" => "is not active in this owner",
            "self subflow" => "self subflow forbidden",
            other => panic!("unmapped case {other}"),
        };
        rejected(&status, Code::InvalidArgument, fragment);
        let (after, after_etag) = stored(&mut scope, "flow").await;
        assert_eq!(after, created, "{name}: entity changed");
        assert_eq!(after_etag, etag, "{name}: etag changed");
    }
    scope
        .try_write(created.clone(), &etag)
        .await
        .expect("an unchanged resubmission with the same ids stays valid");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_write_staging_returns_ids_first_then_accepts_transitions_and_rejects_stale_etags() {
    let mut scope = scope("two-write").await;
    let (created, etag) = staged(&mut scope, "flow", &["Intake", "Review", "Done"]).await;
    let ids: Vec<_> = (0..3).map(|i| step_id(&created, i)).collect();
    assert!(body(&created)
        .steps
        .iter()
        .all(|s| s.transitions.is_empty()));

    let mut wired = created.clone();
    let graph = body_mut(&mut wired);
    graph.entry_steps = vec![ids[0].clone()];
    graph.steps[0].transitions = vec![link(&ids[1], "submit")];
    graph.steps[1].transitions = vec![link(&ids[2], "approve"), link(&ids[0], "reject")];
    let wired_response = scope.guarded(wired.clone(), &etag).await;
    assert_ne!(wired_response.etag, etag);
    let wired = wired_response.entity.unwrap();
    assert_eq!(ids_of(&wired), ids_of(&created));
    assert_eq!(body(&wired).entry_steps, vec![ids[0].clone()]);
    let connection_ids: Vec<_> = body(&wired)
        .steps
        .iter()
        .flat_map(|s| {
            s.transitions
                .iter()
                .map(|t| t.connection_id.clone().unwrap().value)
        })
        .collect();
    assert_eq!(connection_ids.len(), 3);
    assert!(connection_ids.iter().all(|v| !v.is_empty()));
    assert_eq!(connection_ids.iter().collect::<BTreeSet<_>>().len(), 3);

    let stale = scope.try_write(wired.clone(), &etag).await.unwrap_err();
    assert!(
        matches!(stale.code(), Code::FailedPrecondition | Code::Aborted),
        "{stale}"
    );
    let (after, after_etag) = stored(&mut scope, "flow").await;
    assert_eq!(after, wired);
    assert_eq!(after_etag, wired_response.etag);
}

async fn wire(
    scope: &mut Scope,
    slug: &str,
    titles: &[&str],
    entries: &[usize],
    edges: &[(usize, usize, &str)],
) -> (pb::Entity, String) {
    let (created, etag) = staged(scope, slug, titles).await;
    let ids: Vec<_> = (0..titles.len()).map(|i| step_id(&created, i)).collect();
    let mut wired = created;
    let graph = body_mut(&mut wired);
    graph.entry_steps = entries.iter().map(|i| ids[*i].clone()).collect();
    for (from, to, label) in edges {
        graph.steps[*from].transitions.push(link(&ids[*to], label));
    }
    let response = scope.guarded(wired, &etag).await;
    (response.entity.unwrap(), response.etag)
}

fn connection_ids(value: &pb::Entity) -> Vec<Vec<String>> {
    body(value)
        .steps
        .iter()
        .map(|s| {
            s.transitions
                .iter()
                .map(|t| t.connection_id.clone().unwrap().value)
                .collect()
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn transition_identity_follows_label_retention_endpoint_renewal_reorder_and_remove_readd() {
    let mut scope = scope("identity").await;
    let (wired, etag) = wire(
        &mut scope,
        "flow",
        &["A", "B", "C"],
        &[0],
        &[(0, 1, "go"), (0, 2, "skip"), (1, 2, "next")],
    )
    .await;
    let ids: Vec<_> = (0..3).map(|i| step_id(&wired, i)).collect();
    let original = connection_ids(&wired);

    let mut relabeled = wired.clone();
    body_mut(&mut relabeled).steps[0].transitions[0].label = Some(title("renamed"));
    body_mut(&mut relabeled).steps[0].title = Some(title("A renamed"));
    let response = scope.guarded(relabeled, &etag).await;
    let (relabeled, etag) = (response.entity.unwrap(), response.etag);
    assert_eq!(
        connection_ids(&relabeled),
        original,
        "label edits keep identities"
    );
    assert_eq!(ids_of(&relabeled), ids_of(&wired));
    assert_eq!(
        body(&relabeled).steps[0].transitions[0]
            .label
            .as_ref()
            .unwrap()
            .value,
        "renamed"
    );

    let mut reordered = relabeled.clone();
    body_mut(&mut reordered).steps[0].transitions.reverse();
    body_mut(&mut reordered).steps.swap(1, 2);
    let response = scope.guarded(reordered, &etag).await;
    let (reordered, etag) = (response.entity.unwrap(), response.etag);
    assert_eq!(
        ids_of(&reordered),
        vec![
            ids[0].value.clone(),
            ids[2].value.clone(),
            ids[1].value.clone()
        ]
    );
    assert_eq!(
        connection_ids(&reordered)[0],
        vec![original[0][1].clone(), original[0][0].clone()]
    );
    assert_eq!(connection_ids(&reordered)[2], original[1]);

    let mut retargeted = reordered.clone();
    body_mut(&mut retargeted).steps[0].transitions[1].target_step_id = Some(ids[2].clone());
    let status = scope
        .try_write(retargeted.clone(), &etag)
        .await
        .unwrap_err();
    rejected(
        &status,
        Code::InvalidArgument,
        "changed role or endpoint; use a new ID",
    );
    body_mut(&mut retargeted).steps[0].transitions[1].connection_id = None;
    let response = scope.guarded(retargeted, &etag).await;
    let (retargeted, etag) = (response.entity.unwrap(), response.etag);
    let renewed = &connection_ids(&retargeted)[0];
    assert_eq!(
        renewed[0], original[0][1],
        "unchanged endpoints keep their identity"
    );
    assert_ne!(
        renewed[1], original[0][0],
        "a new target renews the identity"
    );
    let stale = renewed[1].clone();

    let mut moved = retargeted.clone();
    let carried = body_mut(&mut moved).steps[0].transitions.remove(0);
    body_mut(&mut moved).steps[2].transitions.push(carried);
    let status = scope.try_write(moved.clone(), &etag).await.unwrap_err();
    rejected(
        &status,
        Code::InvalidArgument,
        "changed role or endpoint; use a new ID",
    );
    body_mut(&mut moved).steps[2].transitions[1].connection_id = None;
    let response = scope.guarded(moved, &etag).await;
    let (moved, etag) = (response.entity.unwrap(), response.etag);
    let after_move = connection_ids(&moved);
    assert_eq!(after_move[0], vec![stale.clone()]);
    assert_eq!(after_move[2].len(), 2);
    assert_ne!(
        after_move[2][1], original[0][1],
        "a new source renews the identity"
    );
    assert_eq!(after_move[2][0], original[1][0], "siblings keep theirs");

    let mut revived = moved.clone();
    body_mut(&mut revived).steps[0].transitions[0].connection_id = Some(pb::ConnectionId {
        value: original[0][0].clone(),
    });
    let status = scope.try_write(revived, &etag).await.unwrap_err();
    rejected(
        &status,
        Code::InvalidArgument,
        "is not active in this owner",
    );

    let mut dropped = moved.clone();
    body_mut(&mut dropped).steps[0].transitions.clear();
    let response = scope.guarded(dropped, &etag).await;
    let (dropped, etag) = (response.entity.unwrap(), response.etag);
    let mut readded = dropped.clone();
    body_mut(&mut readded).steps[0].transitions = vec![link(&ids[2], "skip")];
    let response = scope.guarded(readded, &etag).await;
    let (readded, etag) = (response.entity.unwrap(), response.etag);
    assert_ne!(
        connection_ids(&readded)[0][0],
        stale,
        "remove then re-add renews"
    );

    let mut without = readded.clone();
    let doomed = body(&without).steps[1].step_id.clone().unwrap();
    assert_eq!(doomed, ids[2]);
    body_mut(&mut without).steps[0].transitions.clear();
    body_mut(&mut without).steps[1].transitions.clear();
    body_mut(&mut without).steps[2].transitions.clear();
    body_mut(&mut without).steps.remove(1);
    let response = scope.guarded(without, &etag).await;
    let (without, etag) = (response.entity.unwrap(), response.etag);
    let mut again = without.clone();
    body_mut(&mut again).steps.push(step("C", None));
    let response = scope.guarded(again, &etag).await;
    let (again, etag) = (response.entity.unwrap(), response.etag);
    let fresh = step_id(&again, 2);
    assert_ne!(
        fresh, doomed,
        "a removed step's id is retired for equal content"
    );
    assert_ne!(fresh, ids[1]);

    let mut revived = again.clone();
    body_mut(&mut revived).steps[2].step_id = Some(doomed);
    let status = scope.try_write(revived, &etag).await.unwrap_err();
    rejected(&status, Code::InvalidArgument, "step ID not active");
    let (after, after_etag) = stored(&mut scope, "flow").await;
    assert_eq!(after, again);
    assert_eq!(after_etag, etag);
}

fn node_row(node: &pb::TopologyTraversalNode) -> (String, u32, bool) {
    (
        node.title.clone().unwrap().value,
        node.depth,
        node.authored_entry,
    )
}
fn edge_row(
    edge: &pb::TopologyTraversalEdge,
    names: &std::collections::BTreeMap<String, String>,
) -> (String, String, String) {
    (
        names[&edge.from_step_id.clone().unwrap().value].clone(),
        names[&edge.to_step_id.clone().unwrap().value].clone(),
        edge.label.clone().map(|l| l.value).unwrap_or_default(),
    )
}
fn names_of(value: &pb::Entity) -> std::collections::BTreeMap<String, String> {
    body(value)
        .steps
        .iter()
        .map(|s| {
            (
                s.step_id.clone().unwrap().value,
                s.title.clone().unwrap().value,
            )
        })
        .collect()
}

async fn chain(scope: &mut Scope, prefix: &str, length: usize) {
    for index in (0..length).rev() {
        let target = (index + 1 < length).then(|| scope.subflow(&format!("{prefix}{}", index + 1)));
        scope
            .put(scope.graph(
                &format!("{prefix}{index}"),
                vec![step(&format!("{prefix}{index} step"), target)],
            ))
            .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn linear_branch_and_loop_traversals_return_exact_nodes_edges_entries_and_cycles() {
    let mut scope = scope("receipts").await;
    let (linear, _) = wire(
        &mut scope,
        "linear",
        &["A", "B", "C"],
        &[0],
        &[(0, 1, "one"), (1, 2, "two")],
    )
    .await;
    let (branch, _) = wire(
        &mut scope,
        "branch",
        &["A", "B", "C", "D"],
        &[0],
        &[(0, 1, "yes"), (0, 2, "no"), (1, 3, "left"), (2, 3, "right")],
    )
    .await;
    let (looped, _) = wire(
        &mut scope,
        "loop",
        &["A", "B", "C"],
        &[0, 2],
        &[(0, 1, "in"), (1, 2, "on"), (2, 1, "again")],
    )
    .await;
    let row = |a: &str, b: &str, label: &str| (a.to_string(), b.to_string(), label.to_string());
    let node = |t: &str, entry: bool| (t.to_string(), 0u32, entry);

    let received = scope.traverse("linear", true, 0).await.unwrap();
    let names = names_of(&linear);
    assert_eq!(
        received.nodes.iter().map(node_row).collect::<Vec<_>>(),
        [node("A", true), node("B", false), node("C", false)]
    );
    assert_eq!(
        received
            .edges
            .iter()
            .map(|e| edge_row(e, &names))
            .collect::<Vec<_>>(),
        [row("A", "B", "one"), row("B", "C", "two")]
    );
    let authored: Vec<_> = body(&linear)
        .steps
        .iter()
        .flat_map(|s| {
            s.transitions
                .iter()
                .map(|t| t.connection_id.clone().unwrap())
        })
        .collect();
    assert_eq!(
        received
            .edges
            .iter()
            .map(|e| e.connection_id.clone().unwrap())
            .collect::<Vec<_>>(),
        authored
    );
    assert!(received
        .edges
        .iter()
        .all(|e| e.kind == pb::topology_traversal_edge::Kind::Transition as i32));
    assert!(received.cycles.is_empty() && received.subflows.is_empty());
    assert!(!received.incomplete && received.failures.is_empty());
    assert_eq!(received.applied_max_depth, 8);
    assert!(received.follow_subflows);
    assert_eq!(received.nodes[0].step_id, Some(step_id(&linear, 0)));

    let received = scope.traverse("branch", true, 0).await.unwrap();
    let names = names_of(&branch);
    assert_eq!(
        received.nodes.iter().map(node_row).collect::<Vec<_>>(),
        [
            node("A", true),
            node("B", false),
            node("C", false),
            node("D", false)
        ]
    );
    assert_eq!(
        received
            .edges
            .iter()
            .map(|e| edge_row(e, &names))
            .collect::<Vec<_>>(),
        [
            row("A", "B", "yes"),
            row("A", "C", "no"),
            row("B", "D", "left"),
            row("C", "D", "right")
        ]
    );
    assert!(
        received.cycles.is_empty(),
        "converging branches are not cycles"
    );

    let received = scope.traverse("loop", true, 0).await.unwrap();
    let names = names_of(&looped);
    assert_eq!(
        received.nodes.iter().map(node_row).collect::<Vec<_>>(),
        [node("A", true), node("B", false), node("C", true)]
    );
    assert_eq!(
        received
            .edges
            .iter()
            .map(|e| edge_row(e, &names))
            .collect::<Vec<_>>(),
        [
            row("A", "B", "in"),
            row("B", "C", "on"),
            row("C", "B", "again")
        ]
    );
    assert_eq!(received.cycles.len(), 1);
    let cycle = &received.cycles[0];
    assert_eq!(cycle.scope, pb::topology_cycle::Scope::Transition as i32);
    assert_eq!(
        cycle
            .step_path
            .iter()
            .map(|s| names[&s.value].clone())
            .collect::<Vec<_>>(),
        ["B", "C"]
    );
    assert_eq!(
        cycle.closing_connection_id,
        body(&looped).steps[2].transitions[0].connection_id
    );
    assert!(!received.incomplete);
    assert_eq!(received.scope.as_ref().unwrap().visited.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn subflow_depth_boundaries_default_cap_and_exact_completion_are_reported() {
    let mut scope = scope("depth").await;
    chain(&mut scope, "c", 12).await;
    let depths = |r: &pb::TraverseDesignGraphTopologyResponse| {
        r.nodes.iter().map(|n| n.depth).collect::<Vec<_>>()
    };
    let marker_states = |r: &pb::TraverseDesignGraphTopologyResponse| {
        r.subflows
            .iter()
            .map(|m| (m.depth, m.state))
            .collect::<Vec<_>>()
    };
    let expanded = pb::TopologySubflowState::Expanded as i32;
    let limit = pb::TopologySubflowState::DepthLimit as i32;

    let defaulted = scope.traverse("c0", true, 0).await.unwrap();
    let explicit_eight = scope.traverse("c0", true, 8).await.unwrap();
    assert_eq!(
        defaulted, explicit_eight,
        "zero selects the default of eight"
    );
    assert_eq!(defaulted.applied_max_depth, 8);
    assert_eq!(depths(&defaulted), (0..=8).collect::<Vec<_>>());
    let mut expected: Vec<_> = (0..8).map(|d| (d, expanded)).collect();
    expected.push((8, limit));
    assert_eq!(marker_states(&defaulted), expected);
    assert!(defaulted.incomplete);
    assert_eq!(defaulted.failures.len(), 1);
    assert_eq!(defaulted.failures[0].code, Code::FailedPrecondition as i32);
    assert_eq!(defaulted.failures[0].field, "steps[0].target.subflow");
    assert_eq!(
        defaulted.failures[0].message,
        "Depth limit reached before expanding this subflow"
    );

    for depth in [1u32, 2] {
        let r = scope.traverse("c0", true, depth).await.unwrap();
        assert_eq!(r.applied_max_depth, depth);
        assert_eq!(depths(&r), (0..=depth).collect::<Vec<_>>());
        assert_eq!(*marker_states(&r).last().unwrap(), (depth, limit));
        assert!(r.incomplete);
    }

    let exact = scope.traverse("c0", true, 11).await.unwrap();
    assert_eq!(depths(&exact), (0..=11).collect::<Vec<_>>());
    assert!(marker_states(&exact).iter().all(|(_, s)| *s == expanded));
    assert!(!exact.incomplete && exact.failures.is_empty());

    for requested in [32u32, 33, 500] {
        let r = scope.traverse("c0", true, requested).await.unwrap();
        assert_eq!(r.applied_max_depth, 32, "{requested}");
        assert_eq!(depths(&r), (0..=11).collect::<Vec<_>>());
        assert!(!r.incomplete);
    }

    let unfollowed = scope.traverse("c0", false, 0).await.unwrap();
    assert!(!unfollowed.follow_subflows);
    assert_eq!(depths(&unfollowed), [0]);
    assert_eq!(
        marker_states(&unfollowed),
        [(0, pb::TopologySubflowState::Unfollowed as i32)]
    );
    assert_eq!(unfollowed.edges.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn subflow_cycles_report_their_graph_path_and_both_edges() {
    let mut scope = scope("subflow-cycle").await;
    let (a, a_etag) = staged(&mut scope, "cycle-a", &["Placeholder"]).await;
    scope
        .put(scope.graph(
            "cycle-b",
            vec![step("Call a", Some(scope.subflow("cycle-a")))],
        ))
        .await;
    let mut closing = a;
    body_mut(&mut closing).steps[0].target = Some(pb::TopologyStepTarget {
        selection: Some(scope.subflow("cycle-b")),
    });
    scope.guarded(closing, &a_etag).await;
    let r = scope.traverse("cycle-a", true, 32).await.unwrap();
    assert_eq!(r.cycles.len(), 1);
    let cycle = &r.cycles[0];
    assert_eq!(cycle.scope, pb::topology_cycle::Scope::Subflow as i32);
    assert_eq!(
        cycle
            .graph_path
            .iter()
            .map(|g| g.id.clone().unwrap().slug)
            .collect::<Vec<_>>(),
        ["cycle-a", "cycle-b", "cycle-a"]
    );
    assert_eq!(r.nodes.len(), 2);
    assert!(r
        .subflows
        .iter()
        .any(|m| m.state == pb::TopologySubflowState::Cycle as i32));
    assert!(!r.incomplete, "a cycle is reported, not a budget failure");
}

#[derive(Debug)]
struct Faulty {
    inner: trogon_atlas_server::ownership::RegistryAuthorizer,
    target: std::sync::Mutex<Option<trogon_atlas_core::NamespaceId>>,
    lens_fault: std::sync::Mutex<Option<tonic::Status>>,
}

#[async_trait::async_trait]
impl trogon_atlas_server::ownership::NamespaceAuthorizer for Faulty {
    async fn admits(
        &self,
        visibility: &trogon_atlas_server::ownership::Visibility,
        namespace: &trogon_atlas_core::NamespaceId,
    ) -> Result<bool, tonic::Status> {
        if self.target.lock().unwrap().as_ref() == Some(namespace) {
            return Err(tonic::Status::unavailable(
                "transitive authorization outage",
            ));
        }
        self.inner.admits(visibility, namespace).await
    }
    async fn lens(
        &self,
        visibility: &trogon_atlas_server::ownership::Visibility,
    ) -> Result<trogon_atlas_server::ownership::Lens, tonic::Status> {
        if let Some(failure) = self.lens_fault.lock().unwrap().clone() {
            return Err(failure);
        }
        self.inner.lens(visibility).await
    }
    async fn may_write(
        &self,
        visibility: &trogon_atlas_server::ownership::Visibility,
        namespace: &trogon_atlas_core::NamespaceId,
    ) -> Result<trogon_atlas_server::ownership::WriteVerdict, tonic::Status> {
        self.inner.may_write(visibility, namespace).await
    }
    async fn on_registered(
        &self,
        record: &trogon_atlas_store::NamespaceRecord,
    ) -> Result<(), tonic::Status> {
        self.inner.on_registered(record).await
    }
    async fn on_moved(
        &self,
        record: &trogon_atlas_store::NamespaceRecord,
        from: &trogon_atlas_core::OwnerId,
    ) -> Result<(), tonic::Status> {
        self.inner.on_moved(record, from).await
    }
    async fn on_released(
        &self,
        record: &trogon_atlas_store::NamespaceRecord,
    ) -> Result<(), tonic::Status> {
        self.inner.on_released(record).await
    }
}

async fn faulty_scope(name: &str) -> (Scope, Arc<Faulty>) {
    let store: Arc<dyn trogon_atlas_store::Store> =
        Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let authorizer = Arc::new(Faulty {
        inner: trogon_atlas_server::ownership::RegistryAuthorizer::new(service.directory(), store),
        target: std::sync::Mutex::new(None),
        lens_fault: std::sync::Mutex::new(None),
    });
    let scope = scope_service(service.with_authorizer(authorizer.clone()), name).await;
    (scope, authorizer)
}

enum Rpc {
    Detail,
    Subject,
    Traverse,
    Incoming,
}

async fn call(
    scope: &mut Scope,
    rpc: &Rpc,
    slug: &str,
    subject: &pb::CatalogSubject,
) -> Result<(), tonic::Status> {
    match rpc {
        Rpc::Detail => scope.detail(slug).await.map(|_| ()),
        Rpc::Subject => scope.subject_graphs(subject.clone()).await.map(|_| ()),
        Rpc::Traverse => scope.traverse(slug, true, 0).await.map(|_| ()),
        Rpc::Incoming => scope.incoming(slug).await.map(|_| ()),
    }
}

async fn hidden_namespace(scope: &mut Scope, name: &str) -> String {
    register(
        &mut scope.client,
        &format!("hid-{name}"),
        &format!("stranger-{name}"),
    )
    .await
}

async fn admin_put(scope: &mut Scope, value: pb::Entity) -> pb::PutEntityResponse {
    scope.try_put(value, "", ADMIN).await.unwrap()
}

async fn force_delete(scope: &mut Scope, kind: pb::EntityKind, id: pb::Id) {
    scope
        .client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: kind as i32,
                id: Some(id),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn authorization_lens_outage_fails_all_four_queries_instead_of_returning_empty_graphs() {
    let (mut scope, authorizer) = faulty_scope("lens-outage").await;
    let a = anchors(&mut scope).await;
    let subject = catalog_entity(&reference(&a.component));
    scope
        .put(scope.graph("leaf", vec![step("Leaf", None)]))
        .await;
    scope
        .put(scope.graph(
            "root",
            vec![
                step("Root", Some(Selection::Component(reference(&a.component)))),
                step("Call", Some(scope.subflow("leaf"))),
            ],
        ))
        .await;
    for (name, rpc) in [
        ("detail", Rpc::Detail),
        ("subject", Rpc::Subject),
        ("traverse", Rpc::Traverse),
        ("incoming", Rpc::Incoming),
    ] {
        *authorizer.lens_fault.lock().unwrap() =
            Some(tonic::Status::unavailable("authorized scope outage"));
        let failure = call(&mut scope, &rpc, "root", &subject).await.unwrap_err();
        assert_eq!(failure.code(), Code::Unavailable, "{name}");
        assert_eq!(failure.message(), "authorized scope outage", "{name}");
        *authorizer.lens_fault.lock().unwrap() = None;
        call(&mut scope, &rpc, "root", &subject)
            .await
            .unwrap_or_else(|e| panic!("{name} recovers: {e}"));
    }
}

fn graph_ref_in(ns: &str, slug: &str) -> pb::DesignGraphRef {
    pb::DesignGraphRef {
        id: Some(id_in(ns, slug, 1)),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_and_missing_endpoints_share_one_state_and_one_diagnostic_across_every_read() {
    let (mut scope, authorizer) = faulty_scope("parity").await;
    let other = hidden_namespace(&mut scope, "parity").await;
    admin_put(
        &mut scope,
        entity(pb::entity::Kind::Component(pb::Component {
            id: Some(id_in(&other, "secret", 1)),
            title: "Private retained title".into(),
            ..Default::default()
        })),
    )
    .await;
    admin_put(
        &mut scope,
        graph_in(
            &other,
            "hidden-flow",
            pb::DesignGraphProfile::Flow,
            vec![step("Private flow title", None)],
        ),
    )
    .await;
    let secret = pb::EntityRef {
        kind: pb::EntityKind::Component as i32,
        id: Some(id_in(&other, "secret", 1)),
    };
    scope
        .put(scope.graph("doomed-flow", vec![step("Doomed", None)]))
        .await;
    let gone = scope
        .put(entity(pb::entity::Kind::Component(pb::Component {
            id: Some(id_in(&scope.ns, "gone", 1)),
            title: "Gone".into(),
            ..Default::default()
        })))
        .await
        .entity
        .unwrap();
    let bridge = scope.graph(
        "bridge",
        vec![
            step(
                "hidden component",
                Some(Selection::Component(secret.clone())),
            ),
            step(
                "missing component",
                Some(Selection::Component(reference(&gone))),
            ),
            step(
                "hidden subflow",
                Some(Selection::Subflow(graph_ref_in(&other, "hidden-flow"))),
            ),
            step("missing subflow", Some(scope.subflow("doomed-flow"))),
        ],
    );
    admin_put(&mut scope, bridge).await;
    let ns = scope.ns.clone();
    force_delete(&mut scope, pb::EntityKind::Component, id_in(&ns, "gone", 1)).await;
    force_delete(
        &mut scope,
        pb::EntityKind::DesignGraph,
        id_in(&ns, "doomed-flow", 1),
    )
    .await;

    let detail = scope.detail("bridge").await.unwrap();
    let traced = scope.traverse("bridge", true, 0).await.unwrap();
    let shape = |t: &pb::TopologyTargetState| (t.state, t.owner.clone(), t.target.is_some());
    assert_eq!(detail.targets.len(), 4);
    assert_eq!(shape(&detail.targets[0]), shape(&detail.targets[1]));
    assert_eq!(shape(&detail.targets[2]), shape(&detail.targets[3]));
    for target in &detail.targets {
        assert_eq!(state(target.state), pb::CatalogResolutionState::Unavailable);
        assert!(target.owner.is_none());
    }
    assert!(!detail.incomplete && detail.failures.is_empty());
    assert_eq!(traced.subflows.len(), 2);
    assert_eq!(
        traced.subflows[0].state,
        pb::TopologySubflowState::Unavailable as i32
    );
    assert_eq!(traced.subflows[1].state, traced.subflows[0].state);
    assert_eq!(traced.failures.len(), 2);
    assert_eq!(traced.failures[0].code, traced.failures[1].code);
    assert_eq!(traced.failures[0].message, traced.failures[1].message);
    assert_eq!(traced.failures[0].message, "Subflow graph is unavailable");
    assert!(traced.incomplete);
    assert_eq!(
        traced
            .nodes
            .iter()
            .map(|n| n.target_state)
            .collect::<Vec<_>>(),
        vec![pb::CatalogResolutionState::Unavailable as i32; 4]
    );
    let blob = format!("{detail:?}{traced:?}");
    assert!(!blob.contains("Private retained title") && !blob.contains("Private flow title"));

    let token = scope.token.clone();
    let probes = [
        graph_ref_in(&other, "hidden-flow"),
        graph_ref_in(&other, "never-existed"),
        graph_ref_in("ns_does_not_exist_0000", "never-existed"),
    ];
    let mut seen = Vec::new();
    for graph in &probes {
        let detail = scope
            .client
            .get_design_graph_detail(request(
                pb::GetDesignGraphDetailRequest {
                    graph: Some(graph.clone()),
                },
                &token,
                None,
            ))
            .await
            .unwrap_err();
        let traverse = scope
            .client
            .traverse_design_graph_topology(request(
                pb::TraverseDesignGraphTopologyRequest {
                    root: Some(graph.clone()),
                    follow_subflows: true,
                    max_depth: 0,
                },
                &token,
                None,
            ))
            .await
            .unwrap_err();
        let incoming = scope
            .client
            .list_incoming_design_graph_subflows(request(
                pb::ListIncomingDesignGraphSubflowsRequest {
                    graph: Some(graph.clone()),
                },
                &token,
                None,
            ))
            .await
            .unwrap_err();
        let write = scope
            .try_write(
                scope.graph(
                    "hidden-write",
                    vec![step("s", Some(Selection::Subflow(graph.clone())))],
                ),
                "",
            )
            .await
            .unwrap_err();
        seen.push([detail, traverse, incoming, write].map(|e| (e.code(), e.message().to_string())));
    }
    assert_eq!(
        seen[0], seen[1],
        "an existing hidden graph and a missing one in a hidden namespace"
    );
    assert_eq!(
        seen[0], seen[2],
        "a hidden namespace and one that does not exist"
    );
    assert_eq!(seen[0][0], (Code::NotFound, "target not found".into()));
    assert_eq!(seen[0][1], (Code::NotFound, "target not found".into()));
    assert_eq!(seen[0][2], (Code::NotFound, "entity not found".into()));
    assert_eq!(seen[0][3], (Code::NotFound, "entity not found".into()));
    let component_write = scope
        .try_write(
            scope.graph(
                "hidden-write",
                vec![step("s", Some(Selection::Component(secret.clone())))],
            ),
            "",
        )
        .await
        .unwrap_err();
    assert_eq!(
        (component_write.code(), component_write.message()),
        (Code::NotFound, "entity not found")
    );
    assert_eq!(
        scope.get("hidden-write").await.unwrap_err().code(),
        Code::NotFound
    );

    *authorizer.target.lock().unwrap() =
        Some(trogon_atlas_core::NamespaceId::parse(&other).unwrap());
    for selection in [
        Selection::Component(secret),
        Selection::Subflow(graph_ref_in(&other, "hidden-flow")),
    ] {
        let outage = scope
            .try_write(
                scope.graph("outage-write", vec![step("s", Some(selection))]),
                "",
            )
            .await
            .unwrap_err();
        assert_eq!(outage.code(), Code::Unavailable);
        assert_eq!(outage.message(), "transitive authorization outage");
    }
    assert_eq!(
        scope.get("outage-write").await.unwrap_err().code(),
        Code::NotFound
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn store_decode_faults_and_outages_are_operational_failures_not_empty_or_unavailable_targets()
{
    let nats = trogon_atlas_testsupport::TestNats::start().await;
    let config = trogon_atlas_store::NatsStoreConfig::new(
        &nats.url,
        trogon_atlas_store::StoreMetrics::default(),
    );
    let bucket = config.bucket.clone();
    let store = Arc::new(
        trogon_atlas_store::NatsStore::connect_with(config)
            .await
            .unwrap(),
    );
    let mut scope = scope_over(store, "storage").await;
    scope
        .put(scope.graph("child", vec![step("Child", None)]))
        .await;
    let parent = scope.graph("parent", vec![step("Call", Some(scope.subflow("child")))]);
    let parent_put = scope.put(parent).await;
    let subject = catalog_entity(&pb::EntityRef {
        kind: pb::EntityKind::DesignGraph as i32,
        id: Some(id_in(&scope.ns, "child", 1)),
    });
    let rpcs = || {
        [
            ("detail", Rpc::Detail),
            ("subject", Rpc::Subject),
            ("traverse", Rpc::Traverse),
            ("incoming", Rpc::Incoming),
        ]
    };
    for (name, rpc) in rpcs() {
        call(&mut scope, &rpc, "parent", &subject)
            .await
            .unwrap_or_else(|e| panic!("{name} healthy: {e}"));
    }

    let raw = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    let kv = raw.get_key_value(bucket).await.unwrap();
    let key = trogon_atlas_store::key::entity_key(
        pb::EntityKind::DesignGraph,
        &id_in(&scope.ns, "child", 1),
    );
    kv.put(&key, b"not a valid protobuf Entity".to_vec().into())
        .await
        .unwrap();
    for (name, rpc) in rpcs() {
        for slug in ["parent", "child"] {
            let failure = call(&mut scope, &rpc, slug, &subject).await.unwrap_err();
            assert_eq!(
                (failure.code(), failure.message()),
                (Code::Internal, "storage error"),
                "{name} {slug}"
            );
        }
    }
    let mut edit = parent_put.entity.unwrap();
    body_mut(&mut edit).title = Some(title("Must not publish on decode failure"));
    let failure = scope.try_write(edit, &parent_put.etag).await.unwrap_err();
    assert_eq!(
        (failure.code(), failure.message()),
        (Code::Internal, "storage error")
    );

    nats.stop().await.unwrap();
    for (name, rpc) in rpcs() {
        let failure = call(&mut scope, &rpc, "parent", &subject)
            .await
            .unwrap_err();
        assert!(
            matches!(failure.code(), Code::Unavailable | Code::Internal),
            "{name}: {failure}"
        );
    }
}

async fn many_message_subjects(scope: &mut Scope, count: usize) -> Vec<pb::CatalogSubject> {
    let ns = scope.ns.clone();
    scope
        .put(entity(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id_in(&ns, "bulk-view", 1)),
            title: "Bulk view".into(),
            ..Default::default()
        })))
        .await;
    let specification = scope
        .put(entity(pb::entity::Kind::Specification(pb::Specification {
            id: Some(id_in(&ns, "bulk-contract", 1)),
            title: "Bulk contract".into(),
            messages: (0..count)
                .map(|i| pb::ContractMessage {
                    name: Some(pb::MemberName {
                        value: format!("Message{i}"),
                    }),
                    mapping: Some(pb::MessageReadModelMapping {
                        read_model: Some(pb::ReadModelRef {
                            id: Some(id_in(&ns, "bulk-view", 1)),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })))
        .await
        .entity
        .unwrap();
    let Some(pb::entity::Kind::Specification(body)) = &specification.kind else {
        panic!("specification")
    };
    body.messages
        .iter()
        .map(|m| pb::CatalogSubject {
            selection: Some(pb::catalog_subject::Selection::Member(
                pb::ContractMemberRef {
                    specification: Some(pb::SpecificationRef {
                        id: Some(id_in(&ns, "bulk-contract", 1)),
                    }),
                    owner_uid: uid(&specification),
                    kind: pb::ContractMemberKind::Message as i32,
                    member_id: m.member_id.clone(),
                    expected_digest: None,
                },
            )),
        })
        .collect()
}

fn associate(subjects: &[pb::CatalogSubject]) -> Vec<pb::SubjectAssociation> {
    subjects
        .iter()
        .map(|subject| pb::SubjectAssociation {
            subject: Some(subject.clone()),
            ..Default::default()
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn step_transition_and_local_identity_maxima_are_accepted_then_overflow_by_one() {
    let mut scope = scope("maxima").await;
    let titles: Vec<String> = (0..512).map(|i| format!("S{i}")).collect();
    let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
    let (created, etag) = staged(&mut scope, "wide", &refs).await;
    assert_eq!(body(&created).steps.len(), 512);
    let ids: Vec<_> = (0..512).map(|i| step_id(&created, i)).collect();

    let mut over_steps = scope.graph("wide-over", titles.iter().map(|t| step(t, None)).collect());
    body_mut(&mut over_steps).steps.push(step("S512", None));
    let status = scope.try_write(over_steps, "").await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument, "{status}");

    let mut full = created.clone();
    for source in 0..24 {
        body_mut(&mut full).steps[source].transitions = (0..64)
            .map(|n| link(&ids[(source + n + 1) % 512], "x"))
            .collect();
    }
    let response = scope.guarded(full, &etag).await;
    let (full, etag) = (response.entity.unwrap(), response.etag);
    let transitions: usize = body(&full).steps.iter().map(|s| s.transitions.len()).sum();
    assert_eq!(
        512 + transitions,
        2048,
        "exactly the local identity ceiling"
    );
    assert_eq!(
        connection_ids(&full)
            .into_iter()
            .flatten()
            .collect::<BTreeSet<_>>()
            .len(),
        1536
    );

    let mut overflow = full.clone();
    body_mut(&mut overflow).steps[24].transitions = vec![link(&ids[0], "one too many")];
    let status = scope.try_write(overflow, &etag).await.unwrap_err();
    rejected(
        &status,
        Code::InvalidArgument,
        "local identity limit exceeded",
    );

    let mut sixty_five = full.clone();
    body_mut(&mut sixty_five).steps[0]
        .transitions
        .push(link(&ids[0], "65th"));
    let status = scope.try_write(sixty_five, &etag).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument, "{status}");

    let (after, after_etag) = stored(&mut scope, "wide").await;
    assert_eq!(after, full);
    assert_eq!(after_etag, etag);
}

#[tokio::test(flavor = "multi_thread")]
async fn subject_maximum_counts_toward_the_local_identity_ceiling_and_overflows_by_one() {
    let _serial = HEAVY.lock().await;
    let mut scope = scope("subject-maxima").await;
    let subjects = many_message_subjects(&mut scope, 1024).await;
    let extra = {
        let a = anchors(&mut scope).await;
        catalog_entity(&reference(&a.component))
    };

    let mut maximal = scope.graph("subjects", vec![step("Only", None)]);
    body_mut(&mut maximal).subjects = associate(&subjects);
    let created = scope.put(maximal).await;
    let created_body = body(created.entity.as_ref().unwrap());
    assert_eq!(created_body.subjects.len(), 1024);
    assert_eq!(
        created_body
            .subjects
            .iter()
            .map(|s| s.connection_id.clone().unwrap().value)
            .collect::<BTreeSet<_>>()
            .len(),
        1024
    );

    let mut one_more = scope.graph("subjects-over", vec![step("Only", None)]);
    let mut with_extra = subjects.clone();
    with_extra.push(extra);
    body_mut(&mut one_more).subjects = associate(&with_extra);
    let status = scope.validate(one_more).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument, "{status}");

    let titles: Vec<String> = (0..512).map(|i| format!("S{i}")).collect();
    let mut shared = scope.graph(
        "subjects-and-steps",
        titles.iter().map(|t| step(t, None)).collect(),
    );
    body_mut(&mut shared).subjects = associate(&subjects);
    let response = scope.put(shared).await;
    let (stored_entity, etag) = (response.entity.unwrap(), response.etag);
    let ids: Vec<_> = (0..512).map(|i| step_id(&stored_entity, i)).collect();
    let mut wired = stored_entity.clone();
    for source in 0..8 {
        body_mut(&mut wired).steps[source].transitions = (0..64)
            .map(|n| link(&ids[(source + n + 1) % 512], "x"))
            .collect();
    }
    let response = scope.guarded(wired, &etag).await;
    let (wired, etag) = (response.entity.unwrap(), response.etag);
    let transitions: usize = body(&wired).steps.iter().map(|s| s.transitions.len()).sum();
    assert_eq!(512 + transitions + 1024, 2048);
    let mut overflow = wired.clone();
    body_mut(&mut overflow).steps[8].transitions = vec![link(&ids[0], "overflow")];
    let status = scope.try_write(overflow, &etag).await.unwrap_err();
    rejected(
        &status,
        Code::InvalidArgument,
        "local identity limit exceeded",
    );
    assert_eq!(stored(&mut scope, "subjects-and-steps").await.1, etag);
}

fn fan(scope: &Scope, root: &str, children: &[&str]) -> pb::Entity {
    scope.graph(
        root,
        children
            .iter()
            .enumerate()
            .map(|(i, child)| step(&format!("Call {i}"), Some(scope.subflow(child))))
            .collect(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn traversal_work_budget_charges_each_distinct_owner_once_and_exhausts_on_the_tenth() {
    let _serial = HEAVY.lock().await;
    let mut scope = scope("work-budget").await;
    let subjects = many_message_subjects(&mut scope, 1024).await;
    let names: Vec<String> = (0..10).map(|i| format!("heavy-{i}")).collect();
    for name in &names {
        let mut heavy = scope.graph(name, vec![step("Heavy", None)]);
        body_mut(&mut heavy).subjects = associate(&subjects);
        scope.put(heavy).await;
    }
    let all: Vec<&str> = names.iter().map(String::as_str).collect();

    let status = scope
        .try_write(fan(&scope, "direct", &all), "")
        .await
        .unwrap_err();
    rejected(
        &status,
        Code::ResourceExhausted,
        "catalog work budget exceeded",
    );

    for mid in 0..5 {
        let children: Vec<&str> = all[mid * 2..mid * 2 + 2].to_vec();
        scope
            .put(fan(&scope, &format!("mid-{mid}"), &children))
            .await;
    }
    let mids: Vec<String> = (0..5).map(|i| format!("mid-{i}")).collect();
    let mids: Vec<&str> = mids.iter().map(String::as_str).collect();
    scope.put(fan(&scope, "root", &mids)).await;
    let status = scope.traverse("root", true, 0).await.unwrap_err();
    rejected(
        &status,
        Code::ResourceExhausted,
        "catalog work budget exceeded",
    );

    let (mid4, etag) = stored(&mut scope, "mid-4").await;
    let mut trimmed = mid4.clone();
    body_mut(&mut trimmed).steps.truncate(1);
    scope.guarded(trimmed, &etag).await;
    let ok = scope.traverse("root", true, 0).await.unwrap();
    assert!(!ok.incomplete, "{:?}", ok.failures);
    assert_eq!(
        ok.nodes.len(),
        5 + 9 + 9,
        "five mids, nine heavy calls, nine heavy bodies"
    );
    assert_eq!(ok.edges.len(), 5 + 9);
    assert_eq!(ok.subflows.len(), 5 + 9);

    let unfollowed = scope.traverse("root", false, 0).await.unwrap();
    assert_eq!(unfollowed.nodes.len(), 5);
    assert_eq!(unfollowed.edges.len(), 0);

    let repeated = vec!["heavy-0"; 40];
    scope.put(fan(&scope, "repeat", &repeated)).await;
    let shared = scope.traverse("repeat", true, 0).await.unwrap();
    assert!(!shared.incomplete, "{:?}", shared.failures);
    assert_eq!(
        shared.nodes.len(),
        40 + 40,
        "each call expands the one repeated child"
    );
    assert_eq!(shared.edges.len(), 40);
    assert!(shared
        .subflows
        .iter()
        .all(|m| m.state == pb::TopologySubflowState::Expanded as i32));
}

#[tokio::test(flavor = "multi_thread")]
async fn traversal_output_budget_exhausts_on_the_expansion_that_crosses_4096_rows() {
    let mut scope = scope("output-budget").await;
    let titles: Vec<String> = (0..512).map(|i| format!("Wide {i}")).collect();
    scope
        .put(scope.graph("wide", titles.iter().map(|t| step(t, None)).collect()))
        .await;
    scope.put(fan(&scope, "seven", &["wide"; 7])).await;
    let ok = scope.traverse("seven", true, 0).await.unwrap();
    assert_eq!(ok.nodes.len(), 7 + 7 * 512);
    assert_eq!(
        ok.nodes.len() + ok.edges.len() + ok.subflows.len(),
        7 * 512 + 21
    );

    scope.put(fan(&scope, "eight", &["wide"; 8])).await;
    let status = scope.traverse("eight", true, 0).await.unwrap_err();
    rejected(
        &status,
        Code::ResourceExhausted,
        "catalog output budget exceeded",
    );
    let bounded = scope.traverse("eight", false, 0).await.unwrap();
    assert_eq!(bounded.nodes.len(), 8);
}

#[tokio::test(flavor = "multi_thread")]
async fn subject_listing_matches_explicit_subjects_only_and_readers_query_but_cannot_mutate() {
    let mut scope = scope("subjects").await;
    let a = anchors(&mut scope).await;
    let event = catalog_entity(&reference(&a.event));
    let command = catalog_entity(&reference(&a.command));
    let mut explicit = scope.graph(
        "explicit",
        vec![step(
            "Sends",
            Some(Selection::CatalogMessage(command.clone())),
        )],
    );
    body_mut(&mut explicit).subjects = associate(std::slice::from_ref(&event));
    let explicit = scope.put(explicit).await.entity.unwrap();
    scope
        .put(scope.graph(
            "target-only",
            vec![step(
                "Observes",
                Some(Selection::CatalogMessage(event.clone())),
            )],
        ))
        .await;

    let listed = scope.subject_graphs(event.clone()).await.unwrap();
    assert_eq!(
        state(listed.subject_state),
        pb::CatalogResolutionState::Resolved
    );
    assert!(!listed.incomplete && listed.failures.is_empty());
    assert_eq!(
        listed.declarations.len(),
        1,
        "a step target is not a subject association"
    );
    let found = &listed.declarations[0];
    assert_eq!(
        found
            .owner
            .as_ref()
            .unwrap()
            .owner
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .slug,
        "explicit"
    );
    assert_eq!(found.field, "subjects[0].subject");
    assert_eq!(state(found.state), pb::CatalogResolutionState::Resolved);
    let Some(pb::catalog_declaration::Declaration::Subject(association)) = &found.declaration
    else {
        panic!("subject declaration")
    };
    assert_eq!(
        association.connection_id,
        body(&explicit).subjects[0].connection_id
    );
    assert_eq!(association.subject.as_ref().unwrap(), &event);

    let only_target = scope.subject_graphs(command).await.unwrap();
    assert!(
        only_target.declarations.is_empty(),
        "a target-only selection never lists the graph"
    );
    assert_eq!(
        state(only_target.subject_state),
        pb::CatalogResolutionState::Resolved
    );

    let (_, etag) = stored(&mut scope, "explicit").await;
    scope.token = READER.into();
    assert_eq!(scope.detail("explicit").await.unwrap().etag, etag);
    assert_eq!(
        scope
            .traverse("explicit", true, 0)
            .await
            .unwrap()
            .nodes
            .len(),
        1
    );
    assert_eq!(
        scope.incoming("explicit").await.unwrap().references.len(),
        0
    );
    assert_eq!(
        scope
            .subject_graphs(event)
            .await
            .unwrap()
            .declarations
            .len(),
        1
    );
    let denied = scope
        .try_write(scope.graph("reader-write", vec![]), "")
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied);
    let denied = scope.delete("explicit", true).await.unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied);
    assert_eq!(stored(&mut scope, "explicit").await.1, etag);
}

#[tokio::test(flavor = "multi_thread")]
async fn references_keep_parallel_multiplicity_hidden_referrers_block_delete_and_recreation_is_stale(
) {
    let mut scope = scope("lifecycle").await;
    let a = anchors(&mut scope).await;
    scope
        .put(scope.graph("callee", vec![step("Work", None)]))
        .await;
    scope
        .put(scope.graph("replacement", vec![step("Other work", None)]))
        .await;
    let event = catalog_entity(&reference(&a.event));
    let mut caller = scope.graph(
        "caller",
        vec![
            step("C1", Some(Selection::Component(reference(&a.component)))),
            step("C2", Some(Selection::Component(reference(&a.component)))),
            step("Call", Some(scope.subflow("callee"))),
            step("Evt", Some(Selection::CatalogMessage(event.clone()))),
            step("Member", Some(Selection::CatalogMessage(a.member()))),
        ],
    );
    body_mut(&mut caller).subjects = associate(&[event]);
    let caller = scope.put(caller).await.entity.unwrap();
    let fields: Vec<_> = scope
        .outgoing("caller")
        .await
        .references
        .iter()
        .map(|r| (r.from_field.clone(), r.step_id.clone()))
        .collect();
    let steps = &body(&caller).steps;
    assert_eq!(
        fields,
        vec![
            (
                "steps[0].target.component".to_string(),
                steps[0].step_id.clone()
            ),
            (
                "steps[1].target.component".to_string(),
                steps[1].step_id.clone()
            ),
            (
                "steps[2].target.subflow".to_string(),
                steps[2].step_id.clone()
            ),
            (
                "steps[3].target.catalog_message.entity".to_string(),
                steps[3].step_id.clone()
            ),
            (
                "steps[4].target.catalog_message.member.specification".to_string(),
                steps[4].step_id.clone()
            ),
            ("subjects[0].subject.entity".to_string(), None),
        ],
        "two steps naming one Component stay two references"
    );

    let other = hidden_namespace(&mut scope, "lifecycle").await;
    let private = graph_in(
        &other,
        "private-caller",
        pb::DesignGraphProfile::Flow,
        vec![step(
            "Private referrer title",
            Some(scope.subflow("replacement")),
        )],
    );
    admin_put(&mut scope, private).await;
    let blocked = scope.delete("replacement", false).await.unwrap_err();
    assert_eq!(blocked.code(), Code::FailedPrecondition, "{blocked}");
    assert!(!format!("{blocked:?}").contains("Private referrer title"));
    assert!(scope.get("replacement").await.is_ok());

    let blocked = scope.delete("callee", false).await.unwrap_err();
    assert_eq!(blocked.code(), Code::FailedPrecondition, "{blocked}");

    let (_, etag) = stored(&mut scope, "caller").await;
    let mut retargeted = caller.clone();
    body_mut(&mut retargeted).steps[2].target = Some(pb::TopologyStepTarget {
        selection: Some(scope.subflow("replacement")),
    });
    let retargeted = scope.guarded(retargeted, &etag).await.entity.unwrap();
    assert_eq!(
        body(&retargeted).steps[2].step_id,
        body(&caller).steps[2].step_id
    );
    assert_eq!(ids_of(&retargeted), ids_of(&caller));
    assert_eq!(scope.incoming("callee").await.unwrap().references.len(), 0);
    assert_eq!(
        scope
            .incoming("replacement")
            .await
            .unwrap()
            .references
            .len(),
        1
    );
    scope.delete("callee", false).await.unwrap();

    let before = scope.detail("caller").await.unwrap();
    let member_before = before.targets[4].clone();
    assert_eq!(
        state(member_before.state),
        pb::CatalogResolutionState::Resolved
    );
    let ns = scope.ns.clone();
    let old_uid = uid(&a.specification);
    force_delete(
        &mut scope,
        pb::EntityKind::Specification,
        id_in(&ns, "order-contract", 1),
    )
    .await;
    let reborn = scope.put(a.specification.clone()).await.entity.unwrap();
    assert_ne!(
        uid(&reborn),
        old_uid,
        "recreation assigns a new owner incarnation"
    );
    let stale = scope.detail("caller").await.unwrap();
    assert_ne!(
        state(stale.targets[4].state),
        pb::CatalogResolutionState::Resolved
    );
    assert_eq!(
        state(stale.targets[3].state),
        pb::CatalogResolutionState::Resolved
    );
    let rejected_write = scope
        .try_write(
            scope.graph(
                "stale-selector",
                vec![step("Stale", Some(Selection::CatalogMessage(a.member())))],
            ),
            "",
        )
        .await
        .unwrap_err();
    assert_eq!(
        rejected_write.code(),
        Code::InvalidArgument,
        "{rejected_write}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storyboard_slices_and_traces_stay_unchanged_when_design_graphs_are_added() {
    let mut scope = scope("storyboard").await;
    let a = anchors(&mut scope).await;
    let ns = scope.ns.clone();
    let slice = id_in(&ns, "observe-order", 1);
    let storyboard = scope
        .put(entity(pb::entity::Kind::Storyboard(pb::Storyboard {
            id: Some(id_in(&ns, "checkout-story", 1)),
            title: "Checkout story".into(),
            slices: vec![
                pb::SliceRef {
                    id: Some(slice.clone()),
                },
                pb::SliceRef {
                    id: Some(slice.clone()),
                },
            ],
            traces: vec![pb::StoryboardTrace {
                id: "revisit-order".into(),
                title: "Revisit order".into(),
                steps: vec![
                    pb::TraceStep {
                        slice: Some(pb::SliceRef {
                            id: Some(slice.clone()),
                        }),
                        execution: Some(pb::trace_step::Execution::Automation(
                            pb::AutomationStepExec::default(),
                        )),
                    },
                    pb::TraceStep {
                        slice: Some(pb::SliceRef {
                            id: Some(slice.clone()),
                        }),
                        execution: Some(pb::trace_step::Execution::Automation(
                            pb::AutomationStepExec::default(),
                        )),
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        })))
        .await;
    let before = (storyboard.entity.clone().unwrap(), storyboard.etag.clone());
    let refs_before = {
        let id = Some(id_in(&ns, "checkout-story", 1));
        scope
            .client
            .get_outgoing_references(request(
                pb::GetReferencesRequest {
                    kind: pb::EntityKind::Storyboard as i32,
                    id,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap()
            .into_inner()
    };

    let mut flow = scope.graph(
        "checkout-flow",
        vec![step(
            "Place order",
            Some(Selection::CatalogMessage(catalog_entity(&reference(
                &a.command,
            )))),
        )],
    );
    body_mut(&mut flow).subjects = associate(&[catalog_entity(&reference(&a.event))]);
    scope.put(flow).await;
    scope
        .put(scope.graph(
            "second-flow",
            vec![step("Again", Some(scope.subflow("checkout-flow")))],
        ))
        .await;

    let id = Some(id_in(&ns, "checkout-story", 1));
    let after = scope
        .client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Storyboard as i32,
                id: id.clone(),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!((after.entity.clone().unwrap(), after.etag.clone()), before);
    let Some(pb::entity::Kind::Storyboard(body)) = &after.entity.unwrap().kind else {
        panic!("storyboard")
    };
    assert_eq!(
        body.slices.len(),
        2,
        "a revisited slice keeps both positions"
    );
    assert_eq!(body.slices[0], body.slices[1]);
    assert_eq!(body.traces.len(), 1, "the persisted trace is untouched");
    assert_eq!(body.traces[0].steps.len(), 2);
    assert_eq!(
        body.traces[0].steps[0].slice, body.traces[0].steps[1].slice,
        "a trace revisiting one slice keeps both steps"
    );
    let refs_after = scope
        .client
        .get_outgoing_references(request(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Storyboard as i32,
                id,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(refs_after, refs_before);
    assert!(refs_after
        .references
        .iter()
        .all(|r| r.to.as_ref().unwrap().kind != pb::EntityKind::DesignGraph as i32));
    let flows = scope.incoming("checkout-flow").await.unwrap();
    assert_eq!(flows.references.len(), 1);
    assert_eq!(
        flows.references[0]
            .owner
            .as_ref()
            .unwrap()
            .owner
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .slug,
        "second-flow"
    );
}

async fn impact_of(scope: &mut Scope, root: &pb::Entity) -> pb::GetImpactResponse {
    let token = scope.token.clone();
    scope
        .client
        .get_impact(request(
            pb::GetImpactRequest {
                root: Some(reference(root)),
                max_depth: 4,
                ..Default::default()
            },
            &token,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
}

fn graph_node<'a>(impact: &'a pb::GetImpactResponse, slug: &str) -> Vec<&'a pb::ImpactNode> {
    impact
        .nodes
        .iter()
        .filter(|node| {
            let found = node.entity.as_ref().unwrap();
            found.kind == pb::EntityKind::DesignGraph as i32
                && found.id.as_ref().unwrap().slug == slug
        })
        .collect()
}

fn layout_of(scope: &Scope, slug: &str) -> pb::DocumentRef {
    pb::DocumentRef {
        id: Some(id_in(&scope.ns, slug, 1)),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn outgoing_references_and_impact_name_every_external_kind_with_uid_field_step_and_multiplicity(
) {
    let mut scope = scope("kinds").await;
    let a = anchors(&mut scope).await;
    scope
        .put(scope.graph("callee", vec![step("Work", None)]))
        .await;
    let mut graph = scope.graph(
        "all-kinds",
        vec![
            step("Api", Some(Selection::Component(reference(&a.component)))),
            step("Events", Some(Selection::Channel(reference(&a.channel)))),
            step(
                "Events again",
                Some(Selection::Channel(reference(&a.channel))),
            ),
            step(
                "Store",
                Some(Selection::DataStore(reference(&a.data_store))),
            ),
            step(
                "Product",
                Some(Selection::DataProduct(reference(&a.data_product))),
            ),
            step("Member", Some(Selection::CatalogMessage(a.member()))),
            step("Call", Some(scope.subflow("callee"))),
        ],
    );
    body_mut(&mut graph).layout_document = Some(layout_of(&scope, "checkout-layout"));
    let created = scope.put(graph).await;
    let etag = created.etag;
    let created = created.entity.unwrap();
    let ids: Vec<_> = (0..7).map(|i| step_id(&created, i)).collect();
    let mut wired = created.clone();
    body_mut(&mut wired).steps[0]
        .transitions
        .push(link(&ids[1], "first"));
    body_mut(&mut wired).steps[0]
        .transitions
        .push(link(&ids[2], "second"));
    let wired = scope.guarded(wired, &etag).await.entity.unwrap();
    let owner_uid = uid(&wired);
    assert_eq!(owner_uid, uid(&created), "wiring keeps the incarnation");

    let graph_ref = reference(&wired);
    let rows = scope.outgoing("all-kinds").await.references;
    assert!(rows
        .iter()
        .all(|row| { row.from.as_ref() == Some(&graph_ref) && row.from_uid == owner_uid }));
    let external: Vec<_> = rows
        .iter()
        .filter(|row| row.target_step_id.is_none())
        .map(|row| {
            (
                row.from_field.clone(),
                row.step_id.clone(),
                row.to.clone().unwrap(),
            )
        })
        .collect();
    let steps = &body(&wired).steps;
    let sid = |index: usize| steps[index].step_id.clone();
    assert_eq!(
        external,
        vec![
            (
                "steps[0].target.component".to_string(),
                sid(0),
                reference(&a.component)
            ),
            (
                "steps[1].target.channel".to_string(),
                sid(1),
                reference(&a.channel)
            ),
            (
                "steps[2].target.channel".to_string(),
                sid(2),
                reference(&a.channel)
            ),
            (
                "steps[3].target.data_store".to_string(),
                sid(3),
                reference(&a.data_store)
            ),
            (
                "steps[4].target.data_product".to_string(),
                sid(4),
                reference(&a.data_product)
            ),
            (
                "steps[5].target.catalog_message.member.specification".to_string(),
                sid(5),
                reference(&a.specification)
            ),
            (
                "steps[6].target.subflow".to_string(),
                sid(6),
                pb::EntityRef {
                    kind: pb::EntityKind::DesignGraph as i32,
                    id: Some(scope.id("callee")),
                }
            ),
            ("layout_document".to_string(), None, reference(&a.document)),
        ],
        "two steps naming one Channel stay two rows; the layout row has no step"
    );
    let internal: Vec<_> = rows
        .iter()
        .filter(|row| row.target_step_id.is_some())
        .collect();
    assert_eq!(internal.len(), 2);
    for (index, row) in internal.iter().enumerate() {
        assert_eq!(row.to, row.from, "transitions stay inside the owner");
        assert_eq!(row.step_id, sid(0));
        assert_eq!(row.from_field, format!("steps[0].transitions[{index}]"));
        assert_eq!(row.connection_id, steps[0].transitions[index].connection_id);
        assert_eq!(
            row.target_step_id,
            steps[0].transitions[index].target_step_id
        );
    }

    let roots: Vec<(&pb::Entity, &str, Option<pb::LocalValueId>)> = vec![
        (&a.component, "steps[0].target.component", sid(0)),
        (&a.channel, "steps[1].target.channel", sid(1)),
        (&a.data_store, "steps[3].target.data_store", sid(3)),
        (&a.data_product, "steps[4].target.data_product", sid(4)),
        (
            &a.specification,
            "steps[5].target.catalog_message.member.specification",
            sid(5),
        ),
        (&a.document, "layout_document", None),
    ];
    for (root, field, step) in roots {
        let impact = impact_of(&mut scope, root).await;
        let found = graph_node(&impact, "all-kinds");
        assert_eq!(found.len(), 1, "{field}: one node however many steps refer");
        assert_eq!(found[0].depth, 1, "{field}");
        assert_eq!(found[0].path.len(), 1, "{field}");
        let hop = &found[0].path[0];
        assert_eq!(hop.from, Some(graph_ref.clone()), "{field}");
        assert_eq!(hop.to, Some(reference(root)), "{field}");
        assert_eq!(hop.from_field, field);
        assert_eq!(hop.step_id, step, "{field}");
        assert_eq!(hop.from_uid, owner_uid, "{field}");
    }
    let callee = scope.get("callee").await.unwrap().entity.unwrap();
    let impact = impact_of(&mut scope, &callee).await;
    let found = graph_node(&impact, "all-kinds");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path[0].from_field, "steps[6].target.subflow");
    assert_eq!(found[0].path[0].step_id, sid(6));
    let closure = impact_of(&mut scope, &wired).await;
    let kinds: Vec<_> = closure
        .nodes
        .iter()
        .filter(|node| node.depth == 1)
        .map(|node| reference_key(node.entity.as_ref().unwrap()))
        .collect();
    for expected in [
        &a.component,
        &a.channel,
        &a.data_store,
        &a.data_product,
        &a.specification,
        &a.document,
    ] {
        assert_eq!(
            kinds
                .iter()
                .filter(|key| **key == reference_key(&reference(expected)))
                .count(),
            1,
            "{expected:?} appears once however many steps name it"
        );
    }
    assert_eq!(
        graph_node(&closure, "all-kinds").len(),
        1,
        "transitions inside the owner add no second root"
    );
}

fn reference_key(value: &pb::EntityRef) -> (i32, String) {
    (value.kind, value.id.as_ref().unwrap().slug.clone())
}

#[tokio::test(flavor = "multi_thread")]
async fn recreating_a_graph_owner_stales_step_and_transition_selectors_while_exact_subflows_follow()
{
    let mut scope = scope("recreate").await;
    let (created, etag) = staged(&mut scope, "callee", &["Start", "Finish"]).await;
    let old_steps: Vec<_> = (0..2).map(|i| step_id(&created, i)).collect();
    let mut wired = created.clone();
    body_mut(&mut wired).steps[0]
        .transitions
        .push(link(&old_steps[1], "next"));
    let wired = scope.guarded(wired, &etag).await;
    let old_etag = wired.etag;
    let wired = wired.entity.unwrap();
    let old_uid = uid(&wired);
    let old_connection = body(&wired).steps[0].transitions[0]
        .connection_id
        .clone()
        .unwrap();
    scope
        .put(scope.graph("caller", vec![step("Call", Some(scope.subflow("callee")))]))
        .await;
    let before = scope.detail("caller").await.unwrap();
    let owner = before.targets[0].owner.clone().unwrap();
    assert_eq!(owner.uid, old_uid);
    assert_eq!(owner.etag, old_etag);

    let ns = scope.ns.clone();
    force_delete(
        &mut scope,
        pb::EntityKind::DesignGraph,
        id_in(&ns, "callee", 1),
    )
    .await;
    let (recreated, new_etag) = staged(&mut scope, "callee", &["Start", "Finish"]).await;
    let new_steps: Vec<_> = (0..2).map(|i| step_id(&recreated, i)).collect();
    let new_uid = uid(&recreated);
    assert_ne!(new_uid, old_uid, "recreation assigns a new incarnation");
    assert_ne!(new_steps, old_steps, "step identities are not reused");
    assert_ne!(new_etag, old_etag);

    let mut stale_transition = recreated.clone();
    body_mut(&mut stale_transition).steps[0]
        .transitions
        .push(link(&old_steps[1], "stale"));
    let status = scope
        .try_write(stale_transition, &new_etag)
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument, "{status}");

    let mut stale_connection = recreated.clone();
    let mut reused = link(&new_steps[1], "reused");
    reused.connection_id = Some(old_connection.clone());
    body_mut(&mut stale_connection).steps[0]
        .transitions
        .push(reused);
    let status = scope
        .try_write(stale_connection, &new_etag)
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument, "{status}");
    let selector = pb::CatalogSubject {
        selection: Some(pb::catalog_subject::Selection::Connection(
            pb::ConnectionRef {
                owner: Some(reference(&recreated)),
                id: Some(old_connection.clone()),
                owner_uid: new_uid.clone(),
            },
        )),
    };
    let mut annotated = scope.graph("annotated", vec![step("Note", None)]);
    body_mut(&mut annotated).subjects = associate(&[selector]);
    let status = scope.try_write(annotated, "").await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument, "{status}");

    let wired_again = {
        let mut next = recreated.clone();
        body_mut(&mut next).steps[0]
            .transitions
            .push(link(&new_steps[1], "next"));
        scope.guarded(next, &new_etag).await
    };
    let new_connection = body(wired_again.entity.as_ref().unwrap()).steps[0].transitions[0]
        .connection_id
        .clone()
        .unwrap();
    assert_ne!(new_connection, old_connection);
    let after = scope.detail("caller").await.unwrap();
    assert_eq!(
        state(after.targets[0].state),
        pb::CatalogResolutionState::Resolved
    );
    let owner = after.targets[0].owner.clone().unwrap();
    assert_eq!(owner.uid, new_uid, "the exact-key subflow follows the key");
    assert_eq!(owner.etag, wired_again.etag);
    let walked = scope.traverse("caller", true, 8).await.unwrap();
    let callee_nodes: Vec<_> = walked.nodes.iter().filter(|node| node.depth == 1).collect();
    assert_eq!(callee_nodes.len(), 2);
    assert!(callee_nodes
        .iter()
        .all(|node| node.graph.as_ref().unwrap().uid == new_uid));
    assert_eq!(
        walked.nodes[1..]
            .iter()
            .map(|node| node.step_id.clone().unwrap())
            .collect::<Vec<_>>(),
        new_steps
    );
    let incoming = scope.incoming("callee").await.unwrap();
    assert_eq!(incoming.references.len(), 1);
    assert_eq!(
        state(incoming.graph_state),
        pb::CatalogResolutionState::Resolved
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn diagram_profile_fixture_keeps_inline_actor_and_external_targets_with_layout_document() {
    let mut scope = scope("diagram").await;
    let a = anchors(&mut scope).await;
    let mut diagram = graph_in(
        &scope.ns,
        "system-context",
        pb::DesignGraphProfile::Diagram,
        vec![
            step(
                "Customer",
                Some(Selection::Actor(pb::TopologyActor {
                    name: Some(title("Customer")),
                    summary: Some(text("Places orders")),
                })),
            ),
            step(
                "Checkout",
                Some(Selection::Component(reference(&a.component))),
            ),
            step(
                "Payments",
                Some(Selection::ExternalSystem(pb::TopologyExternalSystem {
                    name: Some(title("Payments gateway")),
                    summary: None,
                    url: "https://example.com/psp".into(),
                })),
            ),
        ],
    );
    body_mut(&mut diagram).layout_document = Some(layout_of(&scope, "checkout-layout"));
    let created = scope.put(diagram).await;
    let etag = created.etag;
    let created = created.entity.unwrap();
    let ids: Vec<_> = (0..3).map(|i| step_id(&created, i)).collect();
    let mut wired = created.clone();
    body_mut(&mut wired).entry_steps = vec![ids[0].clone()];
    body_mut(&mut wired).steps[0]
        .transitions
        .push(link(&ids[1], "orders"));
    body_mut(&mut wired).steps[1]
        .transitions
        .push(link(&ids[2], "charges"));
    let wired = scope.guarded(wired, &etag).await.entity.unwrap();
    assert_eq!(body(&wired).profile, pb::DesignGraphProfile::Diagram as i32);

    let stored = scope.get("system-context").await.unwrap().entity.unwrap();
    assert_eq!(stored.kind, wired.kind);
    let read = scope.detail("system-context").await.unwrap();
    assert_eq!(
        body(read.entity.as_ref().unwrap()).profile,
        pb::DesignGraphProfile::Diagram as i32
    );
    assert_eq!(
        state(read.layout_state),
        pb::CatalogResolutionState::Resolved
    );
    assert_eq!(read.targets.len(), 3);
    assert!(read
        .targets
        .iter()
        .all(|t| state(t.state) == pb::CatalogResolutionState::Resolved));
    assert!(read.targets[0].owner.is_none() && read.targets[2].owner.is_none());
    assert_eq!(
        read.targets[1].owner.as_ref().unwrap().uid,
        uid(&a.component)
    );
    assert!(matches!(
        read.targets[0].target.as_ref().unwrap().selection,
        Some(Selection::Actor(_))
    ));
    assert_eq!(
        read.targets[2].target,
        body(&wired).steps[2].target,
        "the external system is returned exactly as authored"
    );

    let fields: Vec<_> = scope
        .outgoing("system-context")
        .await
        .references
        .into_iter()
        .map(|row| row.from_field)
        .collect();
    assert_eq!(
        fields,
        vec![
            "steps[1].target.component",
            "layout_document",
            "steps[0].transitions[0]",
            "steps[1].transitions[0]",
        ],
        "inline actor and external targets create no catalog references"
    );
    let walked = scope.traverse("system-context", false, 0).await.unwrap();
    assert_eq!(walked.nodes.len(), 3);
    assert_eq!(walked.edges.len(), 2);
    let impact = impact_of(&mut scope, &a.document).await;
    assert_eq!(graph_node(&impact, "system-context").len(), 1);
}
