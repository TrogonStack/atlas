#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::BTreeSet, sync::Arc};

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "catalog-admin";
const BRANCH: &str = "topology-work";
type Client = EventModelServiceClient<Channel>;

fn request<T>(body: T, branch: bool) -> Request<T> {
    let mut req = Request::new(body);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {ADMIN}").parse().unwrap());
    if branch {
        req.metadata_mut()
            .insert("x-trogon-atlas-branch", BRANCH.parse().unwrap());
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
"#
        ))
        .unwrap(),
    )
    .unwrap();
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
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

fn title(value: &str) -> pb::CatalogTitle {
    pb::CatalogTitle {
        value: value.into(),
    }
}

fn graph_mut(value: &mut pb::Entity) -> &mut pb::DesignGraph {
    match value.kind.as_mut() {
        Some(pb::entity::Kind::DesignGraph(graph)) => graph,
        _ => panic!("design graph"),
    }
}

fn graph(value: &pb::Entity) -> &pb::DesignGraph {
    match value.kind.as_ref() {
        Some(pb::entity::Kind::DesignGraph(graph)) => graph,
        _ => panic!("design graph"),
    }
}

fn step(name: &str) -> pb::TopologyStep {
    pb::TopologyStep {
        title: Some(title(name)),
        ..Default::default()
    }
}

fn transition(to: &pb::LocalValueId, label: &str) -> pb::TopologyTransition {
    pb::TopologyTransition {
        label: Some(title(label)),
        target_step_id: Some(to.clone()),
        ..Default::default()
    }
}

async fn put(client: &mut Client, value: pb::Entity, branch: bool) -> pb::Entity {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(value),
                ..Default::default()
            },
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

async fn get(client: &mut Client, slug: &str, branch: bool) -> pb::Entity {
    client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::DesignGraph as i32,
                id: Some(id(slug)),
            },
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

#[derive(Clone)]
struct Identities {
    steps: [pb::LocalValueId; 3],
    forward: pb::ConnectionId,
    skip: pb::ConnectionId,
    onward: pb::ConnectionId,
    back: pb::ConnectionId,
}

fn identities(value: &pb::Entity) -> Identities {
    let graph = graph(value);
    let id = |index: usize| graph.steps[index].step_id.clone().unwrap();
    let connection = |step: usize, index: usize| {
        graph.steps[step].transitions[index]
            .connection_id
            .clone()
            .unwrap()
    };
    Identities {
        steps: [id(0), id(1), id(2)],
        forward: connection(0, 0),
        skip: connection(0, 1),
        onward: connection(1, 0),
        back: connection(2, 0),
    }
}

async fn baseline_flow(client: &mut Client, slug: &str) -> pb::Entity {
    let first = put(
        client,
        pb::Entity {
            kind: Some(pb::entity::Kind::DesignGraph(pb::DesignGraph {
                id: Some(id(slug)),
                title: Some(title("original")),
                profile: pb::DesignGraphProfile::Flow as i32,
                steps: vec![step("A"), step("B"), step("C")],
                ..Default::default()
            })),
            ..Default::default()
        },
        false,
    )
    .await;
    let mut entity = first.clone();
    let ids: Vec<_> = graph(&first)
        .steps
        .iter()
        .map(|value| value.step_id.clone().unwrap())
        .collect();
    let value = graph_mut(&mut entity);
    value.entry_steps = vec![ids[0].clone(), ids[1].clone()];
    value.steps[0].transitions = vec![transition(&ids[1], "forward"), transition(&ids[2], "skip")];
    value.steps[1].transitions = vec![transition(&ids[2], "onward")];
    value.steps[2].transitions = vec![transition(&ids[0], "back")];
    put(client, entity, false).await
}

fn without_b_and_back(value: &pb::Entity) -> pb::Entity {
    let mut entity = value.clone();
    let removed = graph(value).steps[1].step_id.clone().unwrap();
    let value = graph_mut(&mut entity);
    value.steps.remove(1);
    value.entry_steps.retain(|entry| entry != &removed);
    value.steps[0]
        .transitions
        .retain(|transition| transition.target_step_id.as_ref() != Some(&removed));
    value.steps[1].transitions.clear();
    value.title = Some(title("removal side"));
    entity
}

async fn create_branch(client: &mut Client) {
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: BRANCH.into(),
                ..Default::default()
            },
            false,
        ))
        .await
        .unwrap();
}

async fn resolve(client: &mut Client, slug: &str, take_theirs: bool) {
    client
        .resolve_branch_entry(request(
            pb::ResolveBranchEntryRequest {
                name: BRANCH.into(),
                r#ref: Some(pb::EntityRef {
                    kind: pb::EntityKind::DesignGraph as i32,
                    id: Some(id(slug)),
                }),
                resolution: if take_theirs {
                    pb::resolve_branch_entry_request::Resolution::TakeTheirs
                } else {
                    pb::resolve_branch_entry_request::Resolution::KeepOurs
                } as i32,
                ..Default::default()
            },
            false,
        ))
        .await
        .unwrap();
}

async fn merge(client: &mut Client, dry_run: bool) -> pb::MergeBranchResponse {
    client
        .merge_branch(request(
            pb::MergeBranchRequest {
                name: BRANCH.into(),
                dry_run,
                ..Default::default()
            },
            false,
        ))
        .await
        .unwrap()
        .into_inner()
}

async fn restoration_renews_only_restored_identities(take_theirs: bool) {
    let slug = if take_theirs {
        "take-theirs"
    } else {
        "keep-ours"
    };
    let mut client = client().await;
    let original = baseline_flow(&mut client, slug).await;
    let before = identities(&original);
    create_branch(&mut client).await;

    let removal = without_b_and_back(&original);
    let mut retained = original.clone();
    graph_mut(&mut retained).title = Some(title("retaining side"));
    let (branch_side, baseline_side) = if take_theirs {
        (removal, retained)
    } else {
        (retained, removal)
    };
    put(&mut client, branch_side, true).await;
    let baseline = put(&mut client, baseline_side, false).await;

    let conflict = merge(&mut client, true).await;
    assert_eq!(
        conflict.status,
        pb::merge_branch_response::Status::Conflicts as i32,
        "{conflict:?}"
    );

    resolve(&mut client, slug, take_theirs).await;
    let resolved = get(&mut client, slug, true).await;
    let after = graph(&resolved);
    assert_eq!(after.steps.len(), 3);
    assert_eq!(after.title, Some(title("retaining side")));
    let now = identities(&resolved);
    assert_eq!(now.steps[0], before.steps[0]);
    assert_eq!(now.steps[2], before.steps[2]);
    assert_ne!(now.steps[1], before.steps[1]);
    assert_eq!(now.skip, before.skip);
    for renewed in [
        (&now.forward, &before.forward),
        (&now.onward, &before.onward),
        (&now.back, &before.back),
    ] {
        assert_ne!(renewed.0, renewed.1);
    }
    let fresh = BTreeSet::from([
        now.steps[1].value.clone(),
        now.forward.value.clone(),
        now.onward.value.clone(),
        now.back.value.clone(),
    ]);
    let stale = BTreeSet::from([
        before.steps[1].value.clone(),
        before.forward.value.clone(),
        before.onward.value.clone(),
        before.back.value.clone(),
    ]);
    assert_eq!(fresh.len(), 4);
    assert!(fresh.is_disjoint(&stale));

    assert_eq!(
        after.entry_steps,
        vec![now.steps[0].clone(), now.steps[1].clone()]
    );
    assert_eq!(
        after.steps[0].transitions[0].target_step_id,
        Some(now.steps[1].clone())
    );
    assert_eq!(
        after.steps[0].transitions[1].target_step_id,
        Some(now.steps[2].clone())
    );
    assert_eq!(
        after.steps[1].transitions[0].target_step_id,
        Some(now.steps[2].clone())
    );
    assert_eq!(
        after.steps[2].transitions[0].target_step_id,
        Some(now.steps[0].clone())
    );
    assert_eq!(get(&mut client, slug, false).await, baseline);

    let validated = merge(&mut client, true).await;
    assert_eq!(
        validated.status,
        pb::merge_branch_response::Status::Validated as i32,
        "{validated:?}"
    );
    let applied = merge(&mut client, false).await;
    assert_eq!(
        applied.status,
        pb::merge_branch_response::Status::Applied as i32,
        "{applied:?}"
    );
    let landed = get(&mut client, slug, false).await;
    assert_eq!(graph(&landed).steps, after.steps);
    assert_eq!(graph(&landed).entry_steps, after.entry_steps);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keep_ours_restoration_renews_only_the_restored_step_and_transitions() {
    restoration_renews_only_restored_identities(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn take_theirs_restoration_renews_only_the_restored_step_and_transitions() {
    restoration_renews_only_restored_identities(true).await;
}
