//! Read paths report when the snapshot behind them was cut at the configured
//! cap, so a short or empty answer is never mistaken for a complete one, and
//! impact reports the hop bound it applied and whether that bound hid more.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tokio::net::TcpListener;
use tonic::{transport::Channel, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};
use trogon_atlas_store::{store::WriteContext, Store};

const CAP: usize = 6;
const NS: &str = "read-paths";
type Client = EventModelServiceClient<Channel>;

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: NS.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn screen(slug: &str, to: Option<&str>) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Ui(pb::Ui {
            id: Some(id(slug)),
            transitions: to
                .map(|to| pb::ui::Transition {
                    to: Some(pb::UiRef { id: Some(id(to)) }),
                    ..Default::default()
                })
                .into_iter()
                .collect(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn board() -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id("board")),
            members: vec![root("end")],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn slice() -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id("slice")),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn storyboard() -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
            id: Some(id("storyboard")),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// Holds exactly `CAP` entities plus one screen per `extra` slug. `start`
/// transitions to `middle`, which transitions to `end`.
async fn client(extra: &[&str]) -> Client {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let mut seeded = vec![
        (pb::EntityKind::Ui, screen("end", None)),
        (pb::EntityKind::Ui, screen("middle", Some("end"))),
        (pb::EntityKind::Ui, screen("start", Some("middle"))),
        (pb::EntityKind::EventModel, board()),
        (pb::EntityKind::CommandSlice, slice()),
        (pb::EntityKind::Storyboard, storyboard()),
    ];
    seeded.extend(
        extra
            .iter()
            .map(|slug| (pb::EntityKind::Ui, screen(slug, None))),
    );
    for (kind, entity) in &seeded {
        store
            .put(*kind, entity, false, WriteContext::baseline())
            .await
            .unwrap();
    }
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(CAP);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    Client::connect(format!("http://{addr}")).await.unwrap()
}

fn root(slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Ui as i32,
        id: Some(id(slug)),
    }
}

async fn impact(c: &mut Client, max_depth: u32) -> pb::GetImpactResponse {
    c.get_impact(Request::new(pb::GetImpactRequest {
        root: Some(root("end")),
        max_depth,
        ..Default::default()
    }))
    .await
    .unwrap()
    .into_inner()
}

/// Every read path the snapshot cap can cut, reduced to its truncation flag.
async fn read_path_flags(c: &mut Client) -> Vec<(&'static str, bool)> {
    let scope = || pb::AnalysisScope {
        scope: Some(pb::analysis_scope::Scope::Slice(root("end"))),
    };
    let list = c
        .list_entities(Request::new(pb::ListEntitiesRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let versions = c
        .list_versions(Request::new(pb::ListVersionsRequest {
            kind: pb::EntityKind::Ui as i32,
            namespace: NS.into(),
            slug: "end".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    let by_domain = c
        .list_entities_by_domain(Request::new(pb::ListEntitiesByDomainRequest {
            domain_id: Some(id("no-such-domain")),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    let flow = c
        .infer_data_flow(Request::new(pb::InferDataFlowRequest {
            scope: Some(scope()),
        }))
        .await
        .unwrap()
        .into_inner();
    let completeness = c
        .check_information_completeness(Request::new(pb::CheckInformationCompletenessRequest {
            scope: Some(scope()),
        }))
        .await
        .unwrap()
        .into_inner();
    let subgraph = c
        .extract_subgraph(Request::new(pb::ExtractSubgraphRequest {
            scope: Some(scope()),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    let board = c
        .get_event_model_projection(Request::new(pb::GetEventModelProjectionRequest {
            id: Some(id("board")),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    let slice = c
        .get_slice_projection(Request::new(pb::GetSliceProjectionRequest {
            kind: pb::EntityKind::CommandSlice as i32,
            id: Some(id("slice")),
        }))
        .await
        .unwrap()
        .into_inner();
    let storyboard = c
        .get_storyboard_projection(Request::new(pb::GetStoryboardProjectionRequest {
            id: Some(id("storyboard")),
        }))
        .await
        .unwrap()
        .into_inner();
    let impacted = impact(c, 0).await;
    vec![
        ("list_entities", list.snapshot_truncated),
        ("list_versions", versions.snapshot_truncated),
        ("list_entities_by_domain", by_domain.snapshot_truncated),
        ("infer_data_flow", flow.snapshot_truncated),
        (
            "check_information_completeness",
            completeness.snapshot_truncated,
        ),
        ("extract_subgraph", subgraph.truncated),
        ("get_event_model_projection", board.truncated),
        ("get_slice_projection", slice.truncated),
        ("get_storyboard_projection", storyboard.truncated),
        ("get_impact", impacted.snapshot_truncated),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn read_paths_are_complete_at_the_cap() {
    let mut c = client(&[]).await;
    for (path, truncated) in read_path_flags(&mut c).await {
        assert!(!truncated, "{path} flagged a load that fits the cap");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn read_paths_report_a_load_cut_at_the_cap() {
    let mut c = client(&["overflow"]).await;
    for (path, truncated) in read_path_flags(&mut c).await {
        assert!(truncated, "{path} hid that the load was cut");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn impact_reports_the_depth_it_applied() {
    let mut c = client(&[]).await;

    let bounded = impact(&mut c, 1).await;
    assert_eq!(bounded.effective_max_depth, 1);
    assert!(!bounded.depth_clamped);
    assert!(bounded.depth_limited, "start lies past the bound");

    let reaching = impact(&mut c, 2).await;
    assert!(reaching.nodes.len() >= 3);
    assert!(!reaching.depth_limited);

    for asked in [0, 1_000] {
        let clamped = impact(&mut c, asked).await;
        assert_eq!(clamped.effective_max_depth, 16);
        assert!(
            clamped.depth_clamped,
            "max_depth {asked} is the server's call"
        );
        assert!(!clamped.depth_limited);
    }
}
