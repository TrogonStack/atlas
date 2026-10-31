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

const ADMIN: &str = "catalog-admin";
const READER: &str = "catalog-reader";
const LIMITED: &str = "catalog-limited";
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
    client_with_cap(0).await
}
async fn client_with_cap(cap: usize) -> Client {
    client_with_store(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        cap,
    )
    .await
}
async fn client_with_store(store: Arc<dyn trogon_atlas_store::Store>, cap: usize) -> Client {
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
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(cap);
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
        namespace: "topology".into(),
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
fn reference(value: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(value).unwrap()
}
fn title(value: &str) -> pb::CatalogTitle {
    pb::CatalogTitle {
        value: value.into(),
    }
}
fn component(slug: &str) -> pb::Entity {
    entity(pb::entity::Kind::Component(pb::Component {
        id: Some(id(slug)),
        title: slug.into(),
        ..Default::default()
    }))
}
fn step(name: &str, target: Option<pb::topology_step_target::Selection>) -> pb::TopologyStep {
    pb::TopologyStep {
        title: Some(title(name)),
        target: target.map(|selection| pb::TopologyStepTarget {
            selection: Some(selection),
        }),
        ..Default::default()
    }
}
fn graph(slug: &str, steps: Vec<pb::TopologyStep>) -> pb::Entity {
    entity(pb::entity::Kind::DesignGraph(pb::DesignGraph {
        id: Some(id(slug)),
        title: Some(title(slug)),
        profile: pb::DesignGraphProfile::Flow as i32,
        steps,
        ..Default::default()
    }))
}
fn steps_of(value: &pb::Entity) -> &Vec<pb::TopologyStep> {
    match value.kind.as_ref() {
        Some(pb::entity::Kind::DesignGraph(graph)) => &graph.steps,
        _ => panic!("design graph"),
    }
}
fn graph_mut(value: &mut pb::Entity) -> &mut pb::DesignGraph {
    match value.kind.as_mut() {
        Some(pb::entity::Kind::DesignGraph(graph)) => graph,
        _ => panic!("design graph"),
    }
}
fn graph_ref(slug: &str) -> pb::DesignGraphRef {
    pb::DesignGraphRef { id: Some(id(slug)) }
}
fn transition(to: &pb::LocalValueId, label: &str) -> pb::TopologyTransition {
    pb::TopologyTransition {
        label: Some(title(label)),
        target_step_id: Some(to.clone()),
        ..Default::default()
    }
}
async fn try_put(
    client: &mut Client,
    value: pb::Entity,
    etag: String,
    token: &str,
) -> Result<pb::PutEntityResponse, tonic::Status> {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(value),
                if_match: etag,
                ..Default::default()
            },
            token,
            None,
        ))
        .await
        .map(tonic::Response::into_inner)
}
async fn put(client: &mut Client, value: pb::Entity) -> pb::PutEntityResponse {
    try_put(client, value, String::new(), ADMIN).await.unwrap()
}
async fn detail(
    client: &mut Client,
    slug: &str,
    token: &str,
) -> Result<pb::GetDesignGraphDetailResponse, tonic::Status> {
    client
        .get_design_graph_detail(request(
            pb::GetDesignGraphDetailRequest {
                graph: Some(graph_ref(slug)),
            },
            token,
            None,
        ))
        .await
        .map(tonic::Response::into_inner)
}
async fn traverse(
    client: &mut Client,
    slug: &str,
    follow: bool,
    max_depth: u32,
) -> Result<pb::TraverseDesignGraphTopologyResponse, tonic::Status> {
    client
        .traverse_design_graph_topology(request(
            pb::TraverseDesignGraphTopologyRequest {
                root: Some(graph_ref(slug)),
                follow_subflows: follow,
                max_depth,
            },
            READER,
            None,
        ))
        .await
        .map(tonic::Response::into_inner)
}

async fn author_flow(client: &mut Client) -> pb::Entity {
    let component = put(client, component("checkout")).await.entity.unwrap();
    let first = put(
        client,
        graph(
            "order-flow",
            vec![
                step(
                    "Receive order",
                    Some(pb::topology_step_target::Selection::Component(reference(
                        &component,
                    ))),
                ),
                step(
                    "Customer",
                    Some(pb::topology_step_target::Selection::Actor(
                        pb::TopologyActor {
                            name: Some(title("Customer")),
                            summary: None,
                        },
                    )),
                ),
                step("Fulfil", None),
            ],
        ),
    )
    .await;
    let mut entity = first.entity.unwrap();
    let ids: Vec<_> = steps_of(&entity)
        .iter()
        .map(|s| s.step_id.clone().unwrap())
        .collect();
    assert!(ids.iter().all(|value| !value.value.is_empty()));
    assert_eq!(
        ids.iter()
            .map(|value| value.value.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    let value = graph_mut(&mut entity);
    value.entry_steps = vec![ids[0].clone()];
    value.steps[0].transitions = vec![
        transition(&ids[2], "paid"),
        transition(&ids[2], "paid"),
        transition(&ids[1], "notify"),
    ];
    try_put(client, entity, first.etag, ADMIN)
        .await
        .unwrap()
        .entity
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn two_phase_authoring_returns_ids_and_allows_equal_parallel_transitions() {
    let mut client = client().await;
    let authored = author_flow(&mut client).await;
    let steps = steps_of(&authored);
    let ids: Vec<_> = steps[0]
        .transitions
        .iter()
        .map(|t| t.connection_id.clone().unwrap().value)
        .collect();
    assert!(ids.iter().all(|value| !value.is_empty()));
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        3
    );
    let read = detail(&mut client, "order-flow", READER).await.unwrap();
    assert_eq!(read.entity.as_ref().unwrap().kind, authored.kind);
    assert!(read
        .targets
        .iter()
        .all(|t| t.state == pb::CatalogResolutionState::Resolved as i32));
    assert!(!read.incomplete);
}

#[tokio::test(flavor = "multi_thread")]
async fn outgoing_references_surface_same_owner_transitions_with_internal_descriptors() {
    let mut client = client().await;
    let authored = author_flow(&mut client).await;
    let steps = steps_of(&authored);
    let response = client
        .get_outgoing_references(request(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::DesignGraph as i32,
                id: Some(id("order-flow")),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let internal: Vec<_> = response
        .references
        .iter()
        .filter(|r| r.target_step_id.is_some())
        .collect();
    assert_eq!(internal.len(), 3);
    for (found, transition) in internal.iter().zip(&steps[0].transitions) {
        assert_eq!(found.from, found.to);
        assert_eq!(found.step_id, steps[0].step_id);
        assert_eq!(found.target_step_id, transition.target_step_id);
        assert_eq!(found.connection_id, transition.connection_id);
    }
    assert_eq!(
        internal
            .iter()
            .map(|r| r.from_field.as_str())
            .collect::<Vec<_>>(),
        [
            "steps[0].transitions[0]",
            "steps[0].transitions[1]",
            "steps[0].transitions[2]"
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn authoring_rejects_unknown_step_ids_self_subflow_and_reader_writes() {
    let mut client = client().await;
    let mut bad = graph("bad", vec![step("A", None)]);
    graph_mut(&mut bad).steps[0].step_id = Some(pb::LocalValueId {
        value: "predicted".into(),
    });
    assert_eq!(
        try_put(&mut client, bad, String::new(), ADMIN)
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let looped = graph(
        "self",
        vec![step(
            "Self",
            Some(pb::topology_step_target::Selection::Subflow(graph_ref(
                "self",
            ))),
        )],
    );
    assert_eq!(
        try_put(&mut client, looped, String::new(), ADMIN)
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        try_put(&mut client, graph("denied", vec![]), String::new(), READER)
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn traversal_counts_subflow_depth_reports_cycles_and_budgets() {
    let mut client = client().await;
    let leaf = put(&mut client, graph("leaf", vec![step("Leaf step", None)])).await;
    let _ = leaf;
    let mid = put(
        &mut client,
        graph(
            "mid",
            vec![step(
                "Call leaf",
                Some(pb::topology_step_target::Selection::Subflow(graph_ref(
                    "leaf",
                ))),
            )],
        ),
    )
    .await;
    let _ = mid;
    put(
        &mut client,
        graph(
            "root",
            vec![
                step("Start", None),
                step(
                    "Call mid",
                    Some(pb::topology_step_target::Selection::Subflow(graph_ref(
                        "mid",
                    ))),
                ),
                step("Disconnected", None),
            ],
        ),
    )
    .await;
    let full = traverse(&mut client, "root", true, 0).await.unwrap();
    assert_eq!(full.applied_max_depth, 8);
    let depths: Vec<_> = full.nodes.iter().map(|n| n.depth).collect();
    assert_eq!(depths.iter().filter(|d| **d == 0).count(), 3);
    assert!(depths.contains(&1) && depths.contains(&2));
    assert!(!full.incomplete);
    let shallow = traverse(&mut client, "root", true, 1).await.unwrap();
    assert!(shallow.incomplete);
    assert!(shallow
        .subflows
        .iter()
        .any(|m| m.state == pb::TopologySubflowState::DepthLimit as i32));
    let unfollowed = traverse(&mut client, "root", false, 8).await.unwrap();
    assert!(unfollowed
        .subflows
        .iter()
        .all(|m| m.state == pb::TopologySubflowState::Unfollowed as i32));
    assert!(unfollowed.nodes.iter().all(|n| n.depth == 0));
    let capped = traverse(&mut client, "root", true, 99).await.unwrap();
    assert_eq!(capped.applied_max_depth, 32);

    let incoming = client
        .list_incoming_design_graph_subflows(request(
            pb::ListIncomingDesignGraphSubflowsRequest {
                graph: Some(graph_ref("mid")),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(incoming.references.len(), 1);
    assert_eq!(
        incoming.graph_state,
        pb::CatalogResolutionState::Resolved as i32
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn subflow_cycles_give_a_diagnostic_without_infinite_expansion() {
    let mut client = client().await;
    let call = |slug: &str| {
        step(
            "Call",
            Some(pb::topology_step_target::Selection::Subflow(graph_ref(
                slug,
            ))),
        )
    };
    let a = put(
        &mut client,
        graph("cycle-a", vec![step("Placeholder", None)]),
    )
    .await;
    put(&mut client, graph("cycle-b", vec![call("cycle-a")])).await;
    let mut entity = a.entity.unwrap();
    graph_mut(&mut entity).steps = vec![call("cycle-b")];
    try_put(&mut client, entity, a.etag, ADMIN).await.unwrap();
    let result = traverse(&mut client, "cycle-a", true, 32).await.unwrap();
    assert_ne!(result.cycles, Vec::new());
    assert!(result.nodes.len() <= 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_and_missing_targets_share_one_generic_state() {
    let mut client = client().await;
    let mut namespaces = Vec::new();
    for (name, parent) in [
        ("topology-visible", "visible"),
        ("topology-private", "hidden"),
    ] {
        namespaces.push(
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
                .id,
        );
    }
    let in_namespace = |namespace: &str, mut value: pb::Entity| {
        match value.kind.as_mut() {
            Some(pb::entity::Kind::Component(c)) => {
                c.id.as_mut().unwrap().namespace = namespace.into();
            }
            Some(pb::entity::Kind::DesignGraph(g)) => {
                g.id.as_mut().unwrap().namespace = namespace.into();
            }
            _ => panic!("kind"),
        }
        value
    };
    let mut private = component("secret");
    if let Some(pb::entity::Kind::Component(value)) = private.kind.as_mut() {
        value.title = "Private retained title".into();
    }
    let hidden = put(&mut client, in_namespace(&namespaces[1], private))
        .await
        .entity
        .unwrap();
    let hidden_ref = reference(&hidden);
    let missing = put(
        &mut client,
        in_namespace(&namespaces[0], component("later-deleted")),
    )
    .await
    .entity
    .unwrap();
    let missing_ref = reference(&missing);
    let visible = put(
        &mut client,
        in_namespace(
            &namespaces[0],
            graph(
                "gateway",
                vec![
                    step(
                        "Secret",
                        Some(pb::topology_step_target::Selection::Component(hidden_ref)),
                    ),
                    step(
                        "Later deleted",
                        Some(pb::topology_step_target::Selection::Component(
                            missing_ref.clone(),
                        )),
                    ),
                ],
            ),
        ),
    )
    .await
    .entity
    .unwrap();
    let query = pb::GetDesignGraphDetailRequest {
        graph: Some(pb::DesignGraphRef {
            id: reference(&visible).id,
        }),
    };
    let baseline = client
        .get_design_graph_detail(request(query.clone(), ADMIN, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(baseline.targets.len(), 2);
    for target in &baseline.targets {
        assert_eq!(target.state, pb::CatalogResolutionState::Resolved as i32);
        assert!(target.owner.is_some());
    }
    let before = client
        .get_design_graph_detail(request(query.clone(), LIMITED, None))
        .await
        .unwrap()
        .into_inner();
    let hidden_step = &steps_of(&visible)[0].step_id;
    let missing_step = &steps_of(&visible)[1].step_id;
    let find = |response: &pb::GetDesignGraphDetailResponse, step_id: &Option<pb::LocalValueId>| {
        response
            .targets
            .iter()
            .find(|target| &target.step_id == step_id)
            .unwrap()
            .clone()
    };
    assert_eq!(
        find(&before, hidden_step).state,
        pb::CatalogResolutionState::Unavailable as i32
    );
    assert_eq!(
        find(&before, missing_step).state,
        pb::CatalogResolutionState::Resolved as i32
    );
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: missing_ref.kind,
                id: missing_ref.id,
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let response = client
        .get_design_graph_detail(request(query, LIMITED, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response.targets.len(), 2);
    let hidden_target = find(&response, hidden_step);
    let missing_target = find(&response, missing_step);
    assert_eq!(
        hidden_target.state,
        pb::CatalogResolutionState::Unavailable as i32
    );
    assert_eq!(missing_target.state, hidden_target.state);
    assert!(hidden_target.owner.is_none());
    assert_eq!(missing_target.owner, hidden_target.owner);
    assert!(!response.incomplete);
    assert_eq!(response.failures, Vec::new());
    assert!(!format!("{response:?}").contains("Private retained title"));
}

#[tokio::test(flavor = "multi_thread")]
async fn safe_delete_protects_subflow_targets_and_force_delete_leaves_exact_refs() {
    let mut client = client().await;
    put(&mut client, graph("callee", vec![step("Work", None)])).await;
    put(
        &mut client,
        graph(
            "caller",
            vec![step(
                "Call",
                Some(pb::topology_step_target::Selection::Subflow(graph_ref(
                    "callee",
                ))),
            )],
        ),
    )
    .await;
    let delete = |force| {
        request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::DesignGraph as i32,
                id: Some(id("callee")),
                mode: if force {
                    pb::delete_entity_request::Mode::Force as i32
                } else {
                    pb::delete_entity_request::Mode::Unspecified as i32
                },
                ..Default::default()
            },
            ADMIN,
            None,
        )
    };
    assert_eq!(
        client
            .delete_entity(delete(false))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    client.delete_entity(delete(true)).await.unwrap();
    let caller = detail(&mut client, "caller", READER).await.unwrap();
    assert_eq!(
        caller.targets[0].state,
        pb::CatalogResolutionState::Unavailable as i32
    );
}

fn lifecycle(status: &str) -> prost_types::Any {
    use prost::Message;
    prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation".into(),
        value: pb::LifecycleAnnotation {
            status: status.into(),
            ..Default::default()
        }
        .encode_to_vec(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn stripped_step_id_is_rejected_while_changed_content_and_separate_writes_are_new_nodes() {
    let mut client = client().await;
    let created = put(
        &mut client,
        graph("stripping", vec![step("Keep", None), step("Drop", None)]),
    )
    .await;
    let current = created.entity.unwrap();
    let dropped = steps_of(&current)[1].clone();

    let mut stripped = current.clone();
    graph_mut(&mut stripped).steps[1].step_id = None;
    let rejected = try_put(&mut client, stripped, created.etag.clone(), ADMIN)
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument);

    let mut changed = current.clone();
    graph_mut(&mut changed).steps[1] = step("Drop renamed", None);
    let renamed = try_put(&mut client, changed, created.etag.clone(), ADMIN)
        .await
        .unwrap();
    let renamed_id = steps_of(renamed.entity.as_ref().unwrap())[1]
        .step_id
        .clone()
        .unwrap();
    assert_ne!(Some(renamed_id), dropped.step_id.clone());

    let mut removed = renamed.entity.clone().unwrap();
    graph_mut(&mut removed).steps.truncate(1);
    let trimmed = try_put(&mut client, removed, renamed.etag, ADMIN)
        .await
        .unwrap();
    let mut readded = trimmed.entity.clone().unwrap();
    graph_mut(&mut readded).steps.push(pb::TopologyStep {
        step_id: None,
        ..dropped.clone()
    });
    let accepted = try_put(&mut client, readded, trimmed.etag, ADMIN)
        .await
        .unwrap();
    assert_ne!(
        steps_of(accepted.entity.as_ref().unwrap())[1].step_id,
        dropped.step_id
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn equal_sibling_steps_keep_distinct_ids_across_resubmission() {
    let mut client = client().await;
    let created = put(
        &mut client,
        graph("siblings", vec![step("Same", None), step("Same", None)]),
    )
    .await;
    let entity = created.entity.unwrap();
    let before: Vec<_> = steps_of(&entity)
        .iter()
        .map(|s| s.step_id.clone())
        .collect();
    assert_ne!(before[0], before[1]);
    let again = try_put(&mut client, entity.clone(), created.etag, ADMIN)
        .await
        .unwrap()
        .entity
        .unwrap_or(entity);
    let after: Vec<_> = steps_of(&again).iter().map(|s| s.step_id.clone()).collect();
    assert_eq!(before, after);
}

#[tokio::test(flavor = "multi_thread")]
async fn incoming_subflows_preserve_incident_order_and_self_loop_identity() {
    let mut client = client().await;
    put(
        &mut client,
        graph("incident-target", vec![step("Target", None)]),
    )
    .await;
    let created = put(
        &mut client,
        graph(
            "incident-owner",
            vec![
                step(
                    "Subflow",
                    Some(pb::topology_step_target::Selection::Subflow(graph_ref(
                        "incident-target",
                    ))),
                ),
                step("Left", None),
                step("Right", None),
            ],
        ),
    )
    .await;
    let mut value = created.entity.unwrap();
    let body = graph_mut(&mut value);
    let selected = body.steps[0].step_id.clone().unwrap();
    let right = body.steps[2].step_id.clone().unwrap();
    body.steps[0].transitions = vec![
        transition(&selected, "Loop"),
        transition(&right, "Outgoing"),
    ];
    body.steps[1].transitions = vec![
        transition(&selected, "Incoming"),
        transition(&right, "Unrelated"),
    ];
    let stored = try_put(&mut client, value, created.etag, ADMIN)
        .await
        .unwrap();
    let body = steps_of(stored.entity.as_ref().unwrap());
    let expected: Vec<_> = [(&body[0], 0), (&body[0], 1), (&body[1], 0)]
        .into_iter()
        .map(|(step, index)| {
            (
                step.step_id.clone(),
                step.transitions[index].connection_id.clone(),
                step.transitions[index].target_step_id.clone(),
                step.transitions[index].label.clone(),
            )
        })
        .collect();
    let incoming = client
        .list_incoming_design_graph_subflows(request(
            pb::ListIncomingDesignGraphSubflowsRequest {
                graph: Some(graph_ref("incident-target")),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(incoming.references.len(), 1);
    assert_eq!(incoming.references[0].step_id, Some(selected));
    let actual: Vec<_> = incoming.references[0]
        .transitions
        .iter()
        .map(|edge| {
            (
                edge.from_step_id.clone(),
                edge.connection_id.clone(),
                edge.to_step_id.clone(),
                edge.label.clone(),
            )
        })
        .collect();
    assert_eq!(actual, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn incoming_subflows_charge_transition_index_work_once_per_owner() {
    let mut client = client().await;
    put(
        &mut client,
        graph("budget-target", vec![step("Target", None)]),
    )
    .await;
    for owner in 0..3 {
        let mut steps: Vec<_> = (0..400)
            .map(|index| {
                step(
                    &format!("Subflow {index}"),
                    Some(pb::topology_step_target::Selection::Subflow(graph_ref(
                        "budget-target",
                    ))),
                )
            })
            .collect();
        steps.extend((0..25).map(|index| step(&format!("Generic {index}"), None)));
        let created = put(&mut client, graph(&format!("budget-owner-{owner}"), steps)).await;
        let mut value = created.entity.unwrap();
        let body = graph_mut(&mut value);
        let destination = body.steps[400].step_id.clone().unwrap();
        for generic in &mut body.steps[400..] {
            generic.transitions = (0..64)
                .map(|_| transition(&destination, "Generic transition"))
                .collect();
        }
        try_put(&mut client, value, created.etag, ADMIN)
            .await
            .unwrap();
        if owner == 1 {
            let within_budget = client
                .list_incoming_design_graph_subflows(request(
                    pb::ListIncomingDesignGraphSubflowsRequest {
                        graph: Some(graph_ref("budget-target")),
                    },
                    READER,
                    None,
                ))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(within_budget.references.len(), 800);
            assert!(within_budget
                .references
                .iter()
                .all(|reference| reference.transitions.is_empty()));
        }
    }
    let exhausted = client
        .list_incoming_design_graph_subflows(request(
            pb::ListIncomingDesignGraphSubflowsRequest {
                graph: Some(graph_ref("budget-target")),
            },
            READER,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(exhausted.code(), Code::ResourceExhausted);
    assert_eq!(exhausted.message(), "catalog work budget exceeded");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_limits_reject_oversized_step_and_transition_collections() {
    let mut client = client().await;
    let many = (0..513).map(|i| step(&format!("s{i}"), None)).collect();
    assert_eq!(
        try_put(
            &mut client,
            graph("too-many-steps", many),
            String::new(),
            ADMIN
        )
        .await
        .unwrap_err()
        .code(),
        Code::InvalidArgument
    );
    let created = put(
        &mut client,
        graph("fan-out", vec![step("A", None), step("B", None)]),
    )
    .await;
    let mut entity = created.entity.unwrap();
    let target = steps_of(&entity)[1].step_id.clone().unwrap();
    graph_mut(&mut entity).steps[0].transitions =
        (0..65).map(|_| transition(&target, "go")).collect();
    assert_eq!(
        try_put(&mut client, entity, created.etag, ADMIN)
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn lifecycle_metadata_is_supported_on_the_design_graph_kind() {
    let mut client = client().await;
    let mut value = graph("annotated", vec![step("A", None)]);
    graph_mut(&mut value).metadata.push(lifecycle("draft"));
    let stored = put(&mut client, value).await.entity.unwrap();
    match stored.kind.as_ref() {
        Some(pb::entity::Kind::DesignGraph(graph)) => assert_eq!(graph.metadata.len(), 1),
        _ => panic!("design graph"),
    }
    let mut bad = graph("bad-annotation", vec![]);
    graph_mut(&mut bad).metadata.push(prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.AgentAnnotation".into(),
        value: Vec::new(),
    });
    assert_eq!(
        try_put(&mut client, bad, String::new(), ADMIN)
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
}

fn padded_graph(slug: &str, filler: usize, last: usize) -> pb::Entity {
    let text = |length: usize| pb::CatalogText {
        value: "x".repeat(length),
    };
    let mut steps: Vec<_> = (0..filler)
        .map(|index| pb::TopologyStep {
            title: Some(title(&format!("Filler {index}"))),
            summary: Some(text(16_000)),
            ..Default::default()
        })
        .collect();
    steps.push(pb::TopologyStep {
        title: Some(title("Tail")),
        summary: Some(text(last)),
        ..Default::default()
    });
    graph(slug, steps)
}

async fn validate_only(
    client: &mut Client,
    value: pb::Entity,
) -> Result<pb::PutEntityResponse, tonic::Status> {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(value),
                validate_only: true,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn prepared_owner_accepts_exactly_one_mebibyte_and_rejects_one_byte_more() {
    use prost::Message;
    const CEILING: usize = 1_048_576;
    let mut client = client().await;
    let probe = put(&mut client, padded_graph("bound-a", 65, 300)).await;
    let probe_size = probe.entity.as_ref().unwrap().encoded_len();
    assert!(probe_size < CEILING);
    let tail = 300 + (CEILING - probe_size);
    assert!(tail < 16_384, "tail {tail}");

    validate_only(&mut client, padded_graph("bound-b", 65, tail))
        .await
        .unwrap();
    let over = validate_only(&mut client, padded_graph("bound-b", 65, tail + 1))
        .await
        .unwrap_err();
    assert_eq!(over.code(), Code::InvalidArgument, "{over}");
    assert!(over.message().contains("byte limit"), "{over}");

    let refused = try_put(
        &mut client,
        padded_graph("bound-c", 65, tail + 1),
        String::new(),
        ADMIN,
    )
    .await
    .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument, "{refused}");
    for slug in ["bound-b", "bound-c"] {
        let missing = client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::DesignGraph as i32,
                    id: Some(id(slug)),
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(missing.code(), Code::NotFound, "{slug}");
    }

    let mut grown = probe.entity.clone().unwrap();
    graph_mut(&mut grown).steps[65].summary = Some(pb::CatalogText {
        value: "x".repeat(300 + (CEILING - probe_size) + 1),
    });
    let rejected = try_put(&mut client, grown, probe.etag.clone(), ADMIN)
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument, "{rejected}");
    let stored = detail(&mut client, "bound-a", READER).await.unwrap();
    assert_eq!(
        steps_of(stored.entity.as_ref().unwrap())[65]
            .summary
            .as_ref()
            .unwrap()
            .value
            .len(),
        300,
        "rejected growth must not change the stored owner"
    );
}

async fn list_incoming(
    client: &mut Client,
) -> Result<tonic::Response<pb::ListIncomingDesignGraphSubflowsResponse>, tonic::Status> {
    client
        .list_incoming_design_graph_subflows(request(
            pb::ListIncomingDesignGraphSubflowsRequest {
                graph: Some(graph_ref("emit-target")),
            },
            READER,
            None,
        ))
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn incoming_subflow_output_stops_at_the_emitted_record_ceiling() {
    let mut client = client().await;
    put(
        &mut client,
        graph("emit-target", vec![step("Target", None)]),
    )
    .await;
    let subflows = |count: usize| -> Vec<pb::TopologyStep> {
        (0..count)
            .map(|index| {
                step(
                    &format!("Subflow {index}"),
                    Some(pb::topology_step_target::Selection::Subflow(graph_ref(
                        "emit-target",
                    ))),
                )
            })
            .collect()
    };
    let mut last = None;
    for owner in 0..8 {
        last = Some(
            put(
                &mut client,
                graph(
                    &format!("emit-owner-{owner}"),
                    subflows(if owner == 7 { 503 } else { 512 }),
                ),
            )
            .await
            .entity
            .unwrap(),
        );
    }
    let at_ceiling = list_incoming(&mut client).await.unwrap().into_inner();
    assert_eq!(at_ceiling.references.len(), 4_087);
    let scope = at_ceiling.scope.unwrap();
    assert_eq!(
        at_ceiling.references.len()
            + scope.visited.len()
            + scope.visited_subjects.len()
            + scope.visited_documents.len(),
        4_096
    );

    let mut overflow = last.unwrap();
    graph_mut(&mut overflow).steps.push(step(
        "One more",
        Some(pb::topology_step_target::Selection::Subflow(graph_ref(
            "emit-target",
        ))),
    ));
    put(&mut client, overflow).await;
    let exhausted = list_incoming(&mut client).await.unwrap_err();
    assert_eq!(exhausted.code(), Code::ResourceExhausted);
    assert_eq!(exhausted.message(), "catalog output budget exceeded");
}
