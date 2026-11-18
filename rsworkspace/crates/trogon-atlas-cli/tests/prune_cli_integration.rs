#![allow(clippy::unwrap_used, clippy::expect_used)]

// `trogon-atlas diff --prune` / `apply --prune` through the compiled binary:
// the flags reach the client, deletes show in the diff, and the exit codes
// follow kubectl semantics. The behavior matrix lives in
// trogon-atlas-client/tests/prune_integration.rs.

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
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

#[derive(Debug)]
struct Output {
    code: Option<i32>,
    stdout: String,
}

fn trogon_atlas(endpoint: &str, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args(["--endpoint", endpoint])
        .args(args)
        .output()
        .expect("trogon-atlas must spawn");
    Output {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    }
}

fn event(slug: &str) -> String {
    format!(
        "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Event\nmetadata: {{ namespace: prune-cli, name: {slug} }}\nspec: {{ title: {slug} }}\n"
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diff_and_apply_prune_remove_a_dropped_manifest() {
    let endpoint = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("kept.yaml"), event("kept")).unwrap();
    std::fs::write(dir.path().join("dropped.yaml"), event("dropped")).unwrap();
    let path = dir.path().to_str().unwrap();

    let out = trogon_atlas(&endpoint, &["apply", "-f", path]);
    assert_eq!(out.code, Some(0), "{out:?}");

    std::fs::remove_file(dir.path().join("dropped.yaml")).unwrap();

    // Without --prune the dropped entity is not drift.
    let out = trogon_atlas(&endpoint, &["diff", "-f", path]);
    assert_eq!(out.code, Some(0), "{out:?}");

    let out = trogon_atlas(&endpoint, &["diff", "--prune", "-f", path]);
    assert_eq!(out.code, Some(1), "{out:?}");
    assert!(
        out.stdout.contains("diff event:prune-cli/dropped@1"),
        "{out:?}"
    );

    let out = trogon_atlas(&endpoint, &["apply", "--prune", "--dry-run", "-f", path]);
    assert_eq!(out.code, Some(0), "{out:?}");
    assert!(
        out.stdout
            .contains("event:prune-cli/dropped@1 pruned (server dry run)"),
        "{out:?}"
    );

    let out = trogon_atlas(&endpoint, &["apply", "--prune", "-f", path]);
    assert_eq!(out.code, Some(0), "{out:?}");
    assert!(
        out.stdout.contains("event:prune-cli/kept@1 unchanged"),
        "{out:?}"
    );
    assert!(
        out.stdout.contains("event:prune-cli/dropped@1 pruned"),
        "{out:?}"
    );

    let out = trogon_atlas(&endpoint, &["diff", "--prune", "-f", path]);
    assert_eq!(out.code, Some(0), "{out:?}");
}
