#![allow(clippy::unwrap_used, clippy::expect_used)]

// `--operation-id` / `trogon-atlas operation get` / `CommandOutcome.operation_id`,
// spawning the compiled trogon-atlas binary the same way branch_cli_integration.rs
// and format_cli_integration.rs do. Complements the client-crate-level
// coverage in trogon-atlas-client/tests/apply_integration.rs, which drives
// the same idempotency behavior through the library wrappers directly.

use std::{process::Command, sync::Arc};

use serde_json::Value;
use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::service::EventModelServiceImpl;

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
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
struct TrogonAtlasOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn trogon_atlas(endpoint: &str, args: &[&str]) -> TrogonAtlasOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(endpoint)
        .args(args)
        .output()
        .expect("trogon-atlas must spawn");
    TrogonAtlasOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn one_outcome_line(stdout: &str) -> Value {
    let mut lines = stdout.lines();
    let first = lines.next().unwrap_or_else(|| panic!("empty stdout"));
    assert!(
        lines.next().is_none(),
        "expected exactly one stdout line, got: {stdout:?}"
    );
    serde_json::from_str(first).unwrap_or_else(|e| panic!("{first}: not valid JSON: {e}"))
}

fn write_manifest(dir: &std::path::Path, ns: &str, slug: &str) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("event.yaml");
    std::fs::write(
        &path,
        format!(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Event\nmetadata: {{ namespace: {ns}, name: {slug} }}\nspec: {{ title: Thing happened }}\n"
        ),
    )
    .unwrap();
    path
}

fn tempfile_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "trogon-atlas-operation-cli-test-{tag}-{}",
        std::process::id()
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_apply_without_operation_id_mints_one_and_prints_it_to_stderr() {
    let endpoint = start_server().await;
    let dir = tempfile_dir("mint");
    let path = write_manifest(&dir, "opcli", "minted.happened");

    let out = trogon_atlas(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    assert!(out.status.success(), "{out:?}");
    let line = out
        .stderr
        .lines()
        .find(|l| l.starts_with("operation_id: "))
        .unwrap_or_else(|| panic!("{out:?}"));
    let id = line.trim_start_matches("operation_id: ");
    assert!(
        trogon_atlas_core::validate_operation_id(id).is_ok(),
        "minted id {id:?} must itself be a valid operation_id"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_apply_with_an_explicit_operation_id_echoes_it_to_stderr() {
    let endpoint = start_server().await;
    let dir = tempfile_dir("explicit");
    let path = write_manifest(&dir, "opcli", "explicit.happened");

    let out = trogon_atlas(
        &endpoint,
        &[
            "apply",
            "-f",
            path.to_str().unwrap(),
            "--operation-id",
            "test-op-explicit-1",
        ],
    );
    assert!(out.status.success(), "{out:?}");
    assert!(
        out.stderr.contains("operation_id: test-op-explicit-1"),
        "{out:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_apply_rejects_a_malformed_operation_id_before_connecting() {
    // An unreachable endpoint must not mask the local validation error: if
    // it did, this call would fail with a connection error instead.
    let out = trogon_atlas(
        "http://127.0.0.1:1",
        &[
            "apply",
            "-f",
            "/does/not/matter.yaml",
            "--operation-id",
            "not a valid id!",
        ],
    );
    assert!(!out.status.success(), "{out:?}");
    assert!(
        out.stderr.contains("operation_id"),
        "expected the operation_id validation error, got: {out:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_apply_format_json_carries_the_operation_id_in_the_outcome() {
    let endpoint = start_server().await;
    let dir = tempfile_dir("json-outcome");
    let path = write_manifest(&dir, "opcli", "jsonoutcome.happened");

    let out = trogon_atlas(
        &endpoint,
        &[
            "--format",
            "json",
            "apply",
            "-f",
            path.to_str().unwrap(),
            "--operation-id",
            "test-op-json-outcome-1",
        ],
    );
    assert!(out.status.success(), "{out:?}");
    let value = one_outcome_line(&out.stdout);
    assert_eq!(value["operation_id"], "test-op-json-outcome-1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_operation_get_reports_applied_after_a_real_apply() {
    let endpoint = start_server().await;
    let dir = tempfile_dir("get-applied");
    let path = write_manifest(&dir, "opcli", "getapplied.happened");
    let op_id = "test-op-get-applied-1";

    let applied = trogon_atlas(
        &endpoint,
        &[
            "apply",
            "-f",
            path.to_str().unwrap(),
            "--operation-id",
            op_id,
        ],
    );
    assert!(applied.status.success(), "{applied:?}");

    let looked_up = trogon_atlas(&endpoint, &["operation", "get", op_id]);
    assert!(looked_up.status.success(), "{looked_up:?}");
    assert!(
        looked_up.stdout.contains(&format!("{op_id}: applied")),
        "{looked_up:?}"
    );
    assert!(looked_up.stdout.contains("changeset_id:"), "{looked_up:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_operation_get_reports_unknown_for_an_id_nothing_ever_claimed() {
    let endpoint = start_server().await;

    let out = trogon_atlas(&endpoint, &["operation", "get", "test-op-never-claimed-1"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        out.stdout.contains("test-op-never-claimed-1: unknown"),
        "{out:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_apply_dry_run_does_not_claim_the_operation_id() {
    let endpoint = start_server().await;
    let dir = tempfile_dir("dry-run-no-claim");
    let path = write_manifest(&dir, "opcli", "dryrun.happened");
    let op_id = "test-op-dry-run-no-claim-1";

    let dry = trogon_atlas(
        &endpoint,
        &[
            "apply",
            "-f",
            path.to_str().unwrap(),
            "--dry-run",
            "--operation-id",
            op_id,
        ],
    );
    assert!(dry.status.success(), "{dry:?}");

    let looked_up = trogon_atlas(&endpoint, &["operation", "get", op_id]);
    assert!(looked_up.status.success(), "{looked_up:?}");
    assert!(
        looked_up.stdout.contains(&format!("{op_id}: unknown")),
        "a dry run must never claim its operation_id: {looked_up:?}"
    );
}
