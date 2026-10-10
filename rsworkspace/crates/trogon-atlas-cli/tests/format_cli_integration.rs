#![allow(clippy::unwrap_used, clippy::expect_used)]

// `--format json` (and its TROGON_ATLAS_FORMAT env equivalent) must make stdout
// carry exactly one CommandOutcome document, schema trogon-atlas.result.v1, with
// every other line of output (progress, diagnostics) moved to stderr.
// Spawns the compiled trogon-atlas binary, the same way branch_cli_integration.rs
// and fmt_integration.rs do.

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

fn trogon_atlas(args: &[&str]) -> TrogonAtlasOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args(args)
        .output()
        .expect("trogon-atlas must spawn");
    TrogonAtlasOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn trogon_atlas_env(env: &[(&str, &str)], args: &[&str]) -> TrogonAtlasOutput {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"));
    cmd.args(args);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let output = cmd.output().expect("trogon-atlas must spawn");
    TrogonAtlasOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Stdout must be exactly one line: a caller must never have to skip blank
/// lines or concatenate multiple JSON values to get the outcome.
fn one_outcome_line(stdout: &str) -> Value {
    let mut lines = stdout.lines();
    let first = lines.next().unwrap_or_else(|| panic!("empty stdout"));
    assert!(
        lines.next().is_none(),
        "expected exactly one stdout line, got: {stdout:?}"
    );
    serde_json::from_str(first).unwrap_or_else(|e| panic!("{first}: not valid JSON: {e}"))
}

#[test]
fn schema_format_json_prints_exactly_one_outcome_document() {
    let out = trogon_atlas(&["--format", "json", "schema"]);
    assert!(out.status.success(), "{out:?}");
    let value = one_outcome_line(&out.stdout);
    assert_eq!(value["schema"], "trogon-atlas.result.v1");
    assert_eq!(value["ok"], true);
    assert!(value.get("data").is_some_and(Value::is_object));
    assert!(value.get("failure").is_none());
}

#[test]
fn trogon_atlas_format_env_var_selects_json_the_same_as_the_flag() {
    let out = trogon_atlas_env(&[("TROGON_ATLAS_FORMAT", "json")], &["schema"]);
    assert!(out.status.success(), "{out:?}");
    let value = one_outcome_line(&out.stdout);
    assert_eq!(value["schema"], "trogon-atlas.result.v1");
}

#[test]
fn text_format_is_still_the_default_and_is_not_a_json_document() {
    let out = trogon_atlas(&["schema"]);
    assert!(out.status.success(), "{out:?}");
    // The default `text` mode pretty-prints the schema itself to stdout, not
    // a CommandOutcome envelope around it.
    assert!(
        serde_json::from_str::<Value>(&out.stdout).is_ok_and(|v| v.get("schema").is_none()),
        "{}",
        out.stdout
    );
}

const NON_CANONICAL_EVENT: &str = "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Event\nmetadata: {namespace: shop, name: order.placed}\nspec: {title: Order placed}\n";

#[test]
fn fmt_format_json_carries_the_canonical_text_in_data_not_on_raw_stdout() {
    let dir = std::env::temp_dir().join(format!("trogon-atlas-format-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("event.yaml");
    std::fs::write(&path, NON_CANONICAL_EVENT).unwrap();

    let out = trogon_atlas(&["--format", "json", "fmt", path.to_str().unwrap()]);
    assert!(out.status.success(), "{out:?}");
    let value = one_outcome_line(&out.stdout);
    let canonical = value["data"]["canonical"]
        .as_str()
        .unwrap_or_else(|| panic!("{value}"));
    assert!(canonical.starts_with("apiVersion:"), "{canonical}");
    assert!(canonical.contains("namespace: shop"), "{canonical}");

    // Nothing was written: stdout mode never writes, format or no format.
    let unchanged = std::fs::read_to_string(&path).unwrap();
    assert_eq!(unchanged, NON_CANONICAL_EVENT);
}

#[test]
fn export_format_json_reports_a_local_validation_failure_as_a_failed_outcome() {
    let dir = std::env::temp_dir().join(format!(
        "trogon-atlas-format-export-test-{}",
        std::process::id()
    ));
    let out = trogon_atlas(&[
        "--format",
        "json",
        "export",
        "--namespace",
        "",
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(!out.status.success(), "{out:?}");
    assert_eq!(
        out.stderr, "",
        "json format must route the failure through stdout, not stderr"
    );
    let value = one_outcome_line(&out.stdout);
    assert_eq!(value["schema"], "trogon-atlas.result.v1");
    assert_eq!(value["ok"], false);
    assert!(value.get("data").is_none());
    assert!(
        value["failure"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("namespace must not be empty"),
        "{value}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn branch_list_format_json_reports_a_structured_empty_list() {
    let endpoint = start_server().await;
    let out = trogon_atlas(&[
        "--endpoint",
        &endpoint,
        "--format",
        "json",
        "branch",
        "list",
    ]);
    assert!(out.status.success(), "{out:?}");
    let value = one_outcome_line(&out.stdout);
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["branches"], serde_json::json!([]));
}
