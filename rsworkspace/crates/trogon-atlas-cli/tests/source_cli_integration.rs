#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelServiceServer};
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::data_plane,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

async fn server() -> (String, String) {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "admin":{"role":"admin","tokens":["source-cli-admin"]},
            "writer":{"role":"writer","tokens":["source-cli-writer"],"parent":"source-cli-owner"},
            "reader":{"role":"reader","tokens":["source-cli-reader"],"parent":"source-cli-owner"}
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let secured = data_plane(
        EventModelServiceServer::new(service)
            .max_decoding_message_size(36 * 1024 * 1024)
            .max_encoding_message_size(36 * 1024 * 1024),
        BearerAuth::new(registry),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(secured)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let endpoint = format!("http://{address}");
    let namespace = trogon_atlas_client::client::connect(&endpoint, Some("source-cli-admin"), 30)
        .await
        .unwrap()
        .register_namespace(pb::RegisterNamespaceRequest {
            name: "source-cli".into(),
            parent: "source-cli-owner".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    (endpoint, namespace)
}
fn command(endpoint: &str, token: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            endpoint,
            "--auth-token",
            token,
            "--format",
            "json",
        ])
        .args(args)
        .output()
        .unwrap()
}
fn cli(endpoint: &str, token: &str, name: &str, request: &Value) -> Value {
    let output = command(endpoint, token, &[name, "--request", &request.to_string()]);
    assert!(
        output.status.success(),
        "{name}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["data"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_source_cli_retains_explicit_bytes_and_reports_overlay_only_run() {
    let (endpoint, namespace) = server().await;
    let registration = cli(
        &endpoint,
        "source-cli-admin",
        "create-source-registration",
        &json!({"registration":{"id":"cli-source","administrationNamespace":namespace,"destinationNamespaces":[namespace],"profileRevision":1,"ownershipPolicyRevision":"policy","deletionPolicy":"SOURCE_DELETION_POLICY_RETAIN_NATIVE"}}),
    );
    let revision = registration["registration"]["revision"].as_str().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let inventory_path = directory.path().join("inventory.json");
    let payload_path = directory.path().join("private-host-name.yaml");
    let inventory = json!({"profile":"supplied-native-v1","profile_revision":1,"registration":"cli-source","source_version":"00.RC", "partition":{"categories":["model"],"logical_roots":["native"],"selected_keys":[]},"coverage":"COMPLETE","resources":[{"key":{"category":"model","key":"receipt"},"file":"native/model.yaml","pointer":{"kind":"opaque","raw":"never-fetch"}}],"relations":[]});
    std::fs::write(&inventory_path, inventory.to_string()).unwrap();
    std::fs::write(&payload_path, b"# exact bytes\nreceipt: source\n").unwrap();
    let file_arg = format!(
        "native/model.yaml=application/yaml={}",
        payload_path.display()
    );
    let upload = command(
        &endpoint,
        "source-cli-writer",
        &[
            "upload-source-snapshot",
            "--registration-id",
            "cli-source",
            "--registration-revision",
            revision,
            "--inventory",
            inventory_path.to_str().unwrap(),
            "--file",
            &file_arg,
        ],
    );
    assert!(
        upload.status.success(),
        "{}",
        String::from_utf8_lossy(&upload.stderr)
    );
    assert!(!String::from_utf8_lossy(&upload.stdout).contains("private-host-name"));
    let uploaded: Value = serde_json::from_slice(&upload.stdout).unwrap();
    let snapshot = &uploaded["data"]["snapshot"];
    assert!(snapshot.get("acceptedAtUnixMs").is_none());
    let id = snapshot["id"].as_str().unwrap();
    let recovered = cli(
        &endpoint,
        "source-cli-reader",
        "get-source-snapshot",
        &json!({"registrationId":"cli-source","snapshotId":id,"includeBytes":true}),
    );
    let recovered: pb::GetSourceSnapshotResponse =
        trogon_atlas_client::source_integration::parse_request(
            "GetSourceSnapshotResponse",
            &recovered,
        )
        .unwrap();
    assert_eq!(
        recovered.snapshot.unwrap().files[0].content,
        b"# exact bytes\nreceipt: source\n"
    );
    let preview = cli(
        &endpoint,
        "source-cli-writer",
        "preview-source-run",
        &json!({"registrationId":"cli-source","registrationRevision":revision,"snapshotId":id,"runId":"cli-run","overlayOnly":true}),
    );
    assert_eq!(preview["run"]["overlayOnly"], true);
    assert_eq!(preview["run"]["status"], "SOURCE_RUN_STATUS_PREVIEWED");
    let guard = json!({"registrationId":"cli-source","runId":"cli-run","ifMatch":preview["run"]["revision"],"previewDigest":preview["run"]["previewDigest"]});
    let denied = command(
        &endpoint,
        "source-cli-reader",
        &["apply-source-run", "--request", &guard.to_string()],
    );
    assert!(!denied.status.success());
    let applied = cli(&endpoint, "source-cli-writer", "apply-source-run", &guard);
    assert_eq!(applied["run"]["overlayOnly"], true);
    for name in [
        "get-source-run",
        "list-source-run-results",
        "list-source-run-operations",
    ] {
        let result = cli(
            &endpoint,
            "source-cli-reader",
            name,
            &json!({"registrationId":"cli-source","runId":"cli-run"}),
        );
        assert!(result.is_object());
    }
}

#[test]
fn malformed_source_request_fails_locally_and_does_not_disclose_file_path() {
    let output = command(
        "http://127.0.0.1:1",
        "unused",
        &[
            "get-source-snapshot",
            "--request",
            r#"{"includeBytes":"false"}"#,
        ],
    );
    assert!(!output.status.success());
    let output = command(
        "http://127.0.0.1:1",
        "unused",
        &[
            "upload-source-snapshot",
            "--registration-id",
            "r",
            "--registration-revision",
            "rev",
            "--inventory",
            "/private/unavailable-source-file",
        ],
    );
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("/private/unavailable-source-file"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("/private/unavailable-source-file"));
}

#[test]
fn malformed_source_scalar_types_are_rejected_before_connection() {
    for (name, input) in [
        (
            "list-source-runs",
            &json!({"registrationId":"r","pageSize":"100"}),
        ),
        (
            "create-source-registration",
            &json!({"registration":{"deletionPolicy":991}}),
        ),
        (
            "get-source-snapshot",
            &json!({"registrationId":"r","includeBytes":null}),
        ),
    ] {
        let output = command(
            "http://127.0.0.1:1",
            "unused",
            &[name, "--request", &input.to_string()],
        );
        assert!(!output.status.success());
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            result["failure"]["category"], "validation",
            "{name}: {result}"
        );
        assert_eq!(result["failure"]["message"], "invalid source request");
    }
}

fn assert_native_owner_local_error(owners: &[&str], message: &str) -> std::process::Output {
    let mut args = vec![
        "preview-source-run",
        "--request",
        r#"{"registrationId":"r","registrationRevision":"rev","snapshotId":"snapshot","runId":"run"}"#,
    ];
    for owner in owners {
        args.extend(["--native-owner", owner]);
    }
    let output = command("http://127.0.0.1:1", "unused", &args);
    assert_eq!(
        output.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(result["failure"]["category"], "validation");
    assert_eq!(result["failure"]["next_action"], "fix_input");
    assert_eq!(result["failure"]["message"], message);
    output
}

#[test]
fn malformed_source_native_owner_shape_is_rejected_before_connection() {
    assert_native_owner_local_error(
        &["event=priced"],
        "native-owner requires category=source-key=manifest-path",
    );
}

#[test]
fn missing_source_native_owner_file_is_rejected_before_connection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PRIVATE_MISSING_NATIVE_OWNER.yaml");
    let owner = format!("event=priced={}", path.display());
    let output = assert_native_owner_local_error(&[&owner], "supplied file unavailable");
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains("PRIVATE_MISSING_NATIVE_OWNER"));
        assert!(!text.contains(path.to_str().unwrap()));
    }
}

#[test]
fn all_source_native_owner_files_are_read_before_connection() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first=owner.yaml");
    std::fs::write(&first, b"native conversion must stay connected\n").unwrap();
    let missing = directory.path().join("missing-owner.yaml");
    let first = format!("event=first={}", first.display());
    let missing = format!("event=missing={}", missing.display());
    assert_native_owner_local_error(&[&first, &missing], "supplied file unavailable");
}

#[path = "../../trogon-atlas-client/tests/support/source_tenant.rs"]
mod source_tenant;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connected_source_cli_tenant_any_request_and_reports() {
    let (endpoint, namespace) = server().await;
    let request = Box::pin(source_tenant::prepare(
        &endpoint,
        "source-cli-admin",
        namespace,
    ))
    .await;
    for schema in [
        json!({"@type":"type.googleapis.com/acme.money.v1.Money","unknown":"PRIVATE_SENTINEL"}),
        json!({"@type":"type.googleapis.com/acme.money.v1.Money","cents":true}),
    ] {
        let mut invalid = request.clone();
        invalid["nativePayloads"][0]["entity"]["event"]["schema"] = schema;
        let output = command(
            &endpoint,
            "source-cli-admin",
            &["preview-source-run", "--request", &invalid.to_string()],
        );
        assert!(!output.status.success());
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("invalid complete native owner"), "{text}");
        assert!(!text.contains("PRIVATE_SENTINEL"));
    }
    let raw_request = request.clone();
    let preview = cli(
        &endpoint,
        "source-cli-admin",
        "preview-source-run",
        &request,
    );
    assert_eq!(preview["run"]["status"], "SOURCE_RUN_STATUS_PREVIEWED");
    let query = json!({"registrationId":"surface-tenant-source","runId":"surface-tenant-run","pageSize":100});
    source_tenant::assert_operations(&cli(
        &endpoint,
        "source-cli-admin",
        "list-source-run-operations",
        &query,
    ));
    let applied = cli(
        &endpoint,
        "source-cli-admin",
        "apply-source-run",
        &json!({"registrationId":"surface-tenant-source","runId":"surface-tenant-run","ifMatch":preview["run"]["revision"],"previewDigest":preview["run"]["previewDigest"]}),
    );
    assert_eq!(applied["run"]["status"], "SOURCE_RUN_STATUS_APPLIED");
    source_tenant::assert_mapping(&cli(
        &endpoint,
        "source-cli-admin",
        "get-source-mapping",
        &json!({"registrationId":"surface-tenant-source","mappingId":"surface-priced-schema"}),
    ));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PRIVATE_HOST_PATH.json");
    let event = &raw_request["nativePayloads"][0]["entity"]["event"];
    std::fs::write(&path, json!({"apiVersion":"eventmodel.atlas.trogonstack.com/v1alpha1","kind":"Event","metadata":{"namespace":event["id"]["namespace"],"name":"priced"},"spec":{"title":event["title"],"doc":event["doc"],"schema":event["schema"]}}).to_string()).unwrap();
    let mut native_request = raw_request;
    native_request["nativePayloads"] = json!([]);
    native_request["runId"] = json!("surface-tenant-file-run");
    let owner = format!("event=priced={}", path.display());
    let output = command(
        &endpoint,
        "source-cli-admin",
        &[
            "preview-source-run",
            "--request",
            &native_request.to_string(),
            "--native-owner",
            &owner,
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE_HOST_PATH"));
    let results = cli(
        &endpoint,
        "source-cli-admin",
        "list-source-run-results",
        &json!({"registrationId":"surface-tenant-source","runId":"surface-tenant-file-run","pageSize":100}),
    );
    if results["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|result| result["disposition"] == "SOURCE_RESULT_DISPOSITION_UPDATE")
    {
        source_tenant::assert_operations(&cli(
            &endpoint,
            "source-cli-admin",
            "list-source-run-operations",
            &json!({"registrationId":"surface-tenant-source","runId":"surface-tenant-file-run","pageSize":100}),
        ));
    }
    assert!(
        results["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|result| result["disposition"] == "SOURCE_RESULT_DISPOSITION_UNCHANGED"),
        "{results}"
    );
}
