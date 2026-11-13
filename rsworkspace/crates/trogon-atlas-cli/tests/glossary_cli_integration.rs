#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, process::Command, sync::Arc};

use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::data_plane,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const TOKEN: &str = "glossary-cli-reviewer-fixture";

async fn serve() -> String {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "reviewer": {"role": "writer", "tokens": [TOKEN]}
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
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane(
                EventModelServiceServer::new(service),
                BearerAuth::new(registry),
            ))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{address}")
}

fn command(endpoint: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"));
    command
        .arg("--endpoint")
        .arg(endpoint)
        .env("TROGON_ATLAS_AUTH_TOKEN", TOKEN);
    command
}

fn glossary(endpoint: &str, action: &str, request: Value) -> Value {
    let result = command(endpoint)
        .args(["glossary", action, "--request", &request.to_string()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{action}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

fn apply(endpoint: &str, path: &Path, manifest: Value) {
    std::fs::write(path, manifest.to_string()).unwrap();
    let result = command(endpoint)
        .args(["apply", "-f", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn reference(slug: &str) -> Value {
    json!({"id": {"namespace": "glossary-cli", "slug": slug, "version": "1"}})
}

fn selection() -> Value {
    json!({"namespaces": ["glossary-cli"], "audience": "internal"})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_glossary_source_review_lookup_and_analysis_roundtrip() {
    let endpoint = serve().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("glossary.yaml");
    let content = "A parcel is a shipment handled by fulfillment.";
    apply(
        &endpoint,
        &path,
        json!({
            "apiVersion": trogon_atlas_client::manifest::API_VERSION, "kind": "Event",
            "metadata": {"namespace": "glossary-cli", "name": "context"},
            "spec": {"title": "Controlled vocabulary context"}
        }),
    );
    apply(
        &endpoint,
        &path,
        json!({
            "apiVersion": trogon_atlas_client::manifest::API_VERSION, "kind": "Document",
            "metadata": {"namespace": "glossary-cli", "name": "source"},
            "spec": {
                "title": {"value": "Controlled source"},
                "subjects": [{"subject": {"entity": {
                    "kind": "ENTITY_KIND_EVENT", "id": reference("context")["id"]
                }}}],
                "retained": {"mediaType": {"value": "text/plain"}, "content": content,
                    "digest": {"sha256": trogon_atlas_core::specification::content_digest(content).sha256}}
            }
        }),
    );
    let sources = glossary(
        &endpoint,
        "sources",
        json!({"documents": [reference("source")]}),
    );
    let source = &sources["sources"][0];
    assert_eq!(source["text"], content);
    apply(
        &endpoint,
        &path,
        json!({
            "apiVersion": trogon_atlas_client::manifest::API_VERSION, "kind": "Term",
            "metadata": {"namespace": "glossary-cli", "name": "parcel"},
            "spec": {"title": "Parcel", "doc": "A shipment handled by fulfillment.",
                "glossary": {
                    "evidence": [{"document": source["document"], "ownerUid": source["ownerUid"],
                        "contentDigest": source["contentDigest"], "locator": "paragraph:1", "quote": content}],
                    "generationRevision": "controlled-extraction-v1", "audience": "internal",
                    "nextReviewOn": {"value": "9999-12-31"},
                    "policy": {"usage": "GLOSSARY_USAGE_ALLOWED"}
                }
            }
        }),
    );
    let draft = glossary(&endpoint, "get", json!({"term": reference("parcel")}));
    assert_eq!(draft["entry"]["state"], "GLOSSARY_ENTRY_STATE_DRAFT");
    let accepted = glossary(
        &endpoint,
        "review",
        json!({
            "term": reference("parcel"), "ifMatch": draft["etag"],
            "disposition": "GLOSSARY_REVIEW_DISPOSITION_ACCEPTED", "reason": "Checked against the retained source."
        }),
    );
    assert_eq!(
        accepted["entity"]["term"]["glossary"]["review"]["actor"],
        "reviewer"
    );
    let listed = glossary(&endpoint, "list", json!({"selection": selection()}));
    assert_eq!(listed["entries"].as_array().unwrap().len(), 1);
    assert_eq!(listed["entries"][0]["published"], true);
    let checked = glossary(
        &endpoint,
        "check",
        json!({
            "selection": selection(), "text": "Ship this parcel.", "textRevision": "copy-v1",
            "on": {"value": "2026-11-12"}
        }),
    );
    assert_eq!(checked["textRevision"], "copy-v1");
    assert!(checked["findings"].as_array().is_none_or(Vec::is_empty));
    let compared = glossary(
        &endpoint,
        "compare",
        json!({"left": selection(), "right": selection()}),
    );
    assert!(!compared["snapshotRevision"].as_str().unwrap().is_empty());
    let refreshed = glossary(
        &endpoint,
        "refresh",
        json!({
            "selection": selection(), "documents": [reference("source")],
            "generationRevision": "controlled-extraction-v1"
        }),
    );
    assert_eq!(refreshed["observations"].as_array().unwrap().len(), 1);
    assert_eq!(
        refreshed["observations"][0]["entry"]["state"],
        "GLOSSARY_ENTRY_STATE_REVIEWED"
    );
}

#[test]
fn cli_glossary_rejects_malformed_requests_as_input_errors() {
    let result = command("http://127.0.0.1:1")
        .args(["glossary", "list", "--request", "{"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid request JSON"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controlled_corpus_roundtrip_retains_contextual_text_and_exact_code() {
    let endpoint = serve().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source-corpus.yaml");
    std::fs::write(
        &path,
        include_str!("../../../../examples/manifests/generated-glossary/sources.yaml"),
    )
    .unwrap();
    for _ in 0..2 {
        let applied = command(&endpoint)
            .args(["apply", "-f", path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            applied.status.success(),
            "{}",
            String::from_utf8_lossy(&applied.stderr)
        );
    }
    let inspected = glossary(
        &endpoint,
        "sources",
        json!({"documents": [
            {"id": {"namespace": "glossary-fulfillment", "slug": "source", "version": "1"}},
            {"id": {"namespace": "glossary-fulfillment", "slug": "code-source", "version": "1"}},
            {"id": {"namespace": "glossary-customer", "slug": "source", "version": "1"}}
        ]}),
    );
    let sources = inspected["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 3);
    let code = sources
        .iter()
        .find(|source| source["document"]["id"]["slug"] == "code-source")
        .unwrap();
    assert_eq!(
        code["text"],
        include_str!("../../../../examples/glossary/shipment.rs")
    );
    assert!(sources.iter().any(|source| source["text"]
        .as_str()
        .unwrap()
        .contains("reserved carrier handoff slot")));
    assert!(sources.iter().any(|source| source["text"]
        .as_str()
        .unwrap()
        .contains("package a customer can track")));
}
