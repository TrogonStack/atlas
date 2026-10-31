#![allow(clippy::unwrap_used, clippy::expect_used)]

// `trogon-atlas export-attested` and `verify` against an in-process gRPC
// server, spawning the compiled binary like validate_cli_integration.rs.
// A blocking Command::output on the current_thread runtime deadlocks the
// in-process server, so every test here uses the multi_thread flavor.

use std::{path::Path, process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn start_server(snapshot_cap: Option<usize>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let mut svc =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    if let Some(cap) = snapshot_cap {
        svc = svc.with_snapshot_cap(cap);
    }
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

async fn seed_model(endpoint: &str, ns: &str) {
    let mut client = trogon_atlas_client::client::connect(endpoint, None, 30)
        .await
        .unwrap();
    put(
        &mut client,
        pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, "order.placed")),
            title: "Order placed".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
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

fn trogon_atlas(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args(args)
        .output()
        .expect("trogon-atlas must spawn")
}

fn outcome(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap()
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exported_bundle_verifies_offline_until_it_is_altered() {
    let endpoint = start_server(None).await;
    seed_model(&endpoint, "cli-attested").await;
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("shop.bundle.json");

    let export = trogon_atlas(&[
        "--endpoint",
        &endpoint,
        "--format",
        "json",
        "export-attested",
        "cli-attested/shop",
        "-o",
        path_str(&bundle),
    ]);
    assert_eq!(export.status.code(), Some(0), "{export:?}");
    let exported = outcome(&export);
    assert_eq!(exported["data"]["complete"], true, "{exported}");
    assert_eq!(
        exported["data"]["unresolved"],
        serde_json::json!([]),
        "{exported}"
    );

    let verify = trogon_atlas(&["--format", "json", "verify", path_str(&bundle)]);
    assert_eq!(verify.status.code(), Some(0), "{verify:?}");
    assert_eq!(
        outcome(&verify)["data"]["bundle_digest"],
        exported["data"]["bundle_digest"]
    );

    let saved = std::fs::read_to_string(&bundle).unwrap();
    assert!(saved.contains("Order placed"), "{saved}");
    std::fs::write(&bundle, saved.replace("Order placed", "Order shipped")).unwrap();
    let tampered = trogon_atlas(&["verify", path_str(&bundle)]);
    assert_eq!(tampered.status.code(), Some(5), "{tampered:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incomplete_export_is_refused_unless_allowed_and_then_exits_incomplete() {
    let endpoint = start_server(Some(1)).await;
    seed_model(&endpoint, "cli-attested-partial").await;
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("partial.bundle.json");
    let export = |extra: &[&str]| {
        let mut args = vec![
            "--endpoint",
            &endpoint,
            "export-attested",
            "cli-attested-partial/shop",
            "-o",
            path_str(&bundle),
        ];
        args.extend_from_slice(extra);
        trogon_atlas(&args)
    };

    let refused = export(&[]);
    assert_eq!(refused.status.code(), Some(4), "{refused:?}");
    assert!(!bundle.exists());

    let allowed = export(&["--allow-incomplete"]);
    assert_eq!(allowed.status.code(), Some(11), "{allowed:?}");

    let verify = trogon_atlas(&["--format", "json", "verify", path_str(&bundle)]);
    assert_eq!(verify.status.code(), Some(11), "{verify:?}");
    let verified = outcome(&verify);
    assert_eq!(verified["data"]["complete"], false, "{verified}");
    assert!(
        verified["data"]["incomplete_reasons"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("REASON_SNAPSHOT_TRUNCATED")),
        "{verified}"
    );
    assert!(
        verified["data"]["unresolved"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["cause"].as_str().is_some_and(|c| c.starts_with("CAUSE_"))),
        "{verified}"
    );
}

#[test]
fn verifying_a_missing_bundle_is_invalid_input() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("absent.bundle.json");
    let verify = trogon_atlas(&["verify", path_str(&missing)]);
    assert_eq!(verify.status.code(), Some(5), "{verify:?}");
}
