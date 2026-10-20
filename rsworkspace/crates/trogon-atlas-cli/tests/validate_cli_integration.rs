#![allow(clippy::unwrap_used, clippy::expect_used)]

// `trogon-atlas validate` against an in-process gRPC server, spawning the
// compiled binary like views_cli_integration.rs. A blocking
// Command::output on the current_thread runtime deadlocks the in-process
// server, so every test here uses the multi_thread flavor.

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    let grpc = EventModelServiceServer::new(svc);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

fn id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

async fn put(client: &mut trogon_atlas_client::client::Client, kind: pb::entity::Kind) {
    client
        .put_entity(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                system: None,
                kind: Some(kind),
            }),
            create_only: false,
            if_match: String::new(),
            validate_only: false,
            force: false,
            operation_id: String::new(),
        })
        .await
        .unwrap();
}

/// A model whose only member is an event with no swimlane: per-entity
/// checks accept it, but the model-level rule reports an Error.
async fn seed_model_with_laneless_event(
    client: &mut trogon_atlas_client::client::Client,
    ns: &str,
) {
    put(
        client,
        pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, "order.placed")),
            title: "Order placed".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        client,
        pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id(ns, "shop")),
            title: "Shop".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id(ns, "order.placed")),
            }],
            ..Default::default()
        }),
    )
    .await;
}

fn trogon_atlas(endpoint: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(endpoint)
        .args(args)
        .output()
        .expect("trogon-atlas must spawn")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn validate_reports_model_level_errors_and_exits_with_findings() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    seed_model_with_laneless_event(&mut client, "cli-validate-err").await;

    let text = trogon_atlas(&endpoint, &["validate", "cli-validate-err/shop"]);
    assert_eq!(text.status.code(), Some(1), "{text:?}");
    let out = stdout(&text);
    assert!(out.contains("error EVENT_NO_SWIMLANE"), "{out}");
    assert!(out.contains("1 errors"), "{out}");

    let json = trogon_atlas(
        &endpoint,
        &["--format", "json", "validate", "cli-validate-err/shop@1"],
    );
    assert_eq!(json.status.code(), Some(1), "{json:?}");
    let outcome: serde_json::Value = serde_json::from_str(&stdout(&json)).unwrap();
    assert_eq!(outcome["ok"], true, "{outcome}");
    assert_eq!(
        outcome["data"]["event_model"],
        "event_model:cli-validate-err/shop@1"
    );
    assert_eq!(
        outcome["data"]["validation_summary"]["errors"], 1,
        "{outcome}"
    );
    let codes: Vec<_> = outcome["data"]["validation"]
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| issue["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"EVENT_NO_SWIMLANE"), "{outcome}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn validate_exits_zero_for_a_clean_model() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    put(
        &mut client,
        pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id("cli-validate-ok", "empty")),
            title: "Empty".into(),
            ..Default::default()
        }),
    )
    .await;

    let out = trogon_atlas(&endpoint, &["validate", "cli-validate-ok/empty"]);
    assert!(out.status.success(), "{out:?}");
    assert!(stdout(&out).contains("0 errors"), "{}", stdout(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn validate_reads_the_model_from_the_given_branch() {
    let endpoint = start_server().await;
    let branch = format!(
        "cli-validate-{}",
        trogon_atlas_testsupport::unique_suffix_for_test()
    );
    let created = trogon_atlas(&endpoint, &["branch", "create", &branch]);
    assert!(created.status.success(), "{created:?}");
    let mut branched = trogon_atlas_client::client::connect_with_options(
        &endpoint,
        None,
        30,
        trogon_atlas_client::client::ConnectOptions {
            branch: Some(branch.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    seed_model_with_laneless_event(&mut branched, "cli-validate-br").await;

    let on_branch = trogon_atlas(
        &endpoint,
        &["--branch", &branch, "validate", "cli-validate-br/shop"],
    );
    assert_eq!(on_branch.status.code(), Some(1), "{on_branch:?}");
    assert!(
        stdout(&on_branch).contains("EVENT_NO_SWIMLANE"),
        "{}",
        stdout(&on_branch)
    );

    // Baseline never saw the branch's model.
    let on_baseline = trogon_atlas(&endpoint, &["validate", "cli-validate-br/shop"]);
    assert_eq!(on_baseline.status.code(), Some(6), "{on_baseline:?}");
}
