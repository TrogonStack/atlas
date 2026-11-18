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
            "admin":{"role":"admin","tokens":["document-binary-cli-admin"]},
            "writer":{"role":"writer","tokens":["document-binary-cli-writer"],"parent":"document-binary-cli-owner"},
            "reader":{"role":"reader","tokens":["document-binary-cli-reader"],"parent":"document-binary-cli-owner"}
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
    let namespace =
        trogon_atlas_client::client::connect(&endpoint, Some("document-binary-cli-admin"), 30)
            .await
            .unwrap()
            .register_namespace(pb::RegisterNamespaceRequest {
                name: "document-binary-cli".into(),
                parent: "document-binary-cli-owner".into(),
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

const MANIFEST: &str = r"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: {namespace: NAMESPACE, name: receipt}
spec: {title: Receipt}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Document
metadata: {namespace: NAMESPACE, name: text}
spec:
  subjects: [{subject: {entity: {kind: event, id: receipt}}}]
  title: {value: Text anchor target}
  retained:
    mediaType: {value: text/plain}
    content: inert
    digest: {sha256: 3321675705932e37c55c0ba4b0870788177e3f5670b9d7ea7a3206e287dd5ead}
  anchors: [{fragment: {value: example}, label: {value: Example}}]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Document
metadata: {namespace: NAMESPACE, name: general}
spec:
  subjects: [{subject: {entity: {kind: event, id: receipt}}}]
  title: {value: Explicit general}
  purpose: DOCUMENT_PURPOSE_GENERAL
  link: {value: https://example.invalid/inert}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Document
metadata: {namespace: NAMESPACE, name: binary, version: 9007199254740993}
spec:
  subjects: [{subject: {entity: {kind: event, id: receipt}}}]
  title: {value: Inert attachment}
  filename: {value: sample.bin}
  metadata:
    - '@type': type.googleapis.com/trogonatlas.eventmodel.v1alpha1.DocumentCollectionAnnotation
      category: {value: Downloads}
  relatedContent:
    - document: {document: text}
      role: RELATED_CONTENT_ROLE_EXAMPLE
    - document: {document: text}
      role: RELATED_CONTENT_ROLE_DIAGRAM
  retainedContent:
    mediaType: {value: application/octet-stream}
    content: AGV4YWN0/wo=
    digest: {sha256: ea545ad36a9c4c5bfc95d99937328130e5834502247a772fd585d943c97e6831}
";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_document_binary_cli_roundtrip() {
    let (endpoint, namespace) = server().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("binary.yaml");
    let manifest = MANIFEST.replace("NAMESPACE", &namespace);
    std::fs::write(&path, &manifest).unwrap();
    let run = |args: &[&str]| {
        let out = command(&endpoint, "document-binary-cli-writer", args);
        assert!(
            out.status.success(),
            "{args:?}: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    run(&["apply", "-f", path.to_str().unwrap(), "--dry-run"]);
    run(&["apply", "-f", path.to_str().unwrap()]);
    let key = format!("document/{namespace}/binary@9007199254740993");
    let before = run(&["get", &key]);
    let exported = directory.path().join("export");
    run(&[
        "export",
        "--namespace",
        &namespace,
        "-o",
        exported.to_str().unwrap(),
    ]);
    let mut paths = std::fs::read_dir(&exported)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    for export in paths {
        run(&["diff", "-f", export.to_str().unwrap()]);
        run(&["apply", "-f", export.to_str().unwrap()]);
    }
    assert_eq!(before, run(&["get", &key]));
    let envelope: Value = serde_json::from_str(&before).unwrap();
    let parsed = trogon_atlas_client::manifest::parse_str(
        &envelope["data"]["manifest"].to_string(),
        "get.yaml",
    )
    .unwrap();
    let Some(pb::entity::Kind::Document(document)) = &parsed[0].entity.kind else {
        panic!("missing Document")
    };
    let Some(pb::document::Body::RetainedContent(content)) = &document.body else {
        panic!("missing binary body")
    };
    assert_eq!(content.content, b"\0exact\xff\n");
    assert!(document.purpose.is_none());
    assert_eq!(document.related_content.len(), 2);
    assert!(before.contains("9007199254740993"));
    assert!(before.contains("AGV4YWN0/wo="));
    assert!(before.contains("application/octet-stream"));
    assert!(before.contains("sample.bin"));
    let description = run(&["describe", &key]);
    assert!(
        description.contains("application/octet-stream"),
        "{description}"
    );
    assert!(description.contains("sample.bin"), "{description}");
    assert!(
        description.contains("ea545ad36a9c4c5bfc95d99937328130e5834502247a772fd585d943c97e6831"),
        "{description}"
    );
    run(&["apply", "-f", path.to_str().unwrap()]);
    assert_eq!(before, run(&["get", &key]));
    let detail = cli(
        &endpoint,
        "document-binary-cli-reader",
        "get-document-detail",
        &json!({"document":{"document":{"id":{"namespace":namespace,"slug":"binary","version":"9007199254740993"}}}}),
    );
    assert!(detail.to_string().contains("AGV4YWN0/wo="), "{detail}");
    let invalid_path = directory.path().join("source-selection.yaml");
    std::fs::write(
        &invalid_path,
        manifest.replace(
            "document: text",
            "source: {registrationId: native-document-source, category: model, key: retained-example}",
        ),
    ).unwrap();
    let refused = command(
        &endpoint,
        "document-binary-cli-writer",
        &["apply", "-f", invalid_path.to_str().unwrap()],
    );
    assert!(
        !refused.status.success(),
        "native Document accepted a Source selection"
    );
    assert_eq!(before, run(&["get", &key]));
}
