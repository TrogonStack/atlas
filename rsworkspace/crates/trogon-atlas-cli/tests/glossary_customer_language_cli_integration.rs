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

const TOKEN: &str = "customer-language-synthetic-reviewer";
const INTERNAL: &str = "glossary-fulfillment";
const CUSTOMER: &str = "glossary-customer";
const UI_COPY: &str = "📦 Your booking is confirmed.";
const POLICY_DIGEST: &str = "07dbf598735aecb830b99454dc274b142688e3efc9c01fbd05a9f9c8933101e2";

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
        .args(["--endpoint", endpoint, "--format", "json"])
        .env("TROGON_ATLAS_AUTH_TOKEN", TOKEN);
    command
}

fn successful(endpoint: &str, arguments: &[&str]) -> Value {
    let output = command(endpoint).args(arguments).output().unwrap();
    assert!(
        output.status.success(),
        "{arguments:?}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let outcome: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(outcome["schema"], "trogon-atlas.result.v1");
    assert_eq!(outcome["ok"], true);
    outcome["data"].clone()
}

fn glossary(endpoint: &str, action: &str, request: &Value) -> Value {
    successful(
        endpoint,
        &["glossary", action, "--request", &request.to_string()],
    )
}

fn reference(namespace: &str, slug: &str) -> Value {
    json!({"id": {"namespace": namespace, "slug": slug, "version": "1"}})
}

fn selection(namespace: &str, audience: &str) -> Value {
    json!({"namespaces": [namespace], "audience": audience})
}

fn rows<'a>(response: &'a Value, key: &str) -> &'a [Value] {
    response
        .get(key)
        .map_or(&[], |value| value.as_array().unwrap().as_slice())
}

fn check(endpoint: &str, scope: &Value, text: &str, revision: &str) -> Value {
    let result = glossary(
        endpoint,
        "check",
        &json!({"selection": scope, "text": text, "textRevision": revision,
            "on": {"value": "2026-11-13"}}),
    );
    assert_ne!(result["incomplete"], true);
    assert_eq!(result["textRevision"], revision);
    assert_ne!(result["snapshotRevision"], "");
    result
}

fn source<'a>(sources: &'a [Value], namespace: &str, slug: &str) -> &'a Value {
    let source = sources
        .iter()
        .find(|source| source["document"] == reference(namespace, slug))
        .unwrap();
    assert_eq!(source["state"], "GLOSSARY_EVIDENCE_STATE_CURRENT");
    assert_ne!(source["ownerUid"].as_str().unwrap(), "");
    assert_eq!(
        source["contentDigest"]["sha256"],
        trogon_atlas_core::specification::content_digest(source["text"].as_str().unwrap()).sha256
    );
    source
}

fn glossary_entry(source: &Value, audience: &str, quote: &str, policy: &Value) -> Value {
    assert!(source["text"].as_str().unwrap().contains(quote));
    json!({
        "audience": audience, "generationRevision": "customer-language-workflow-v1",
        "nextReviewOn": {"value": "9999-12-31"}, "policy": policy,
        "evidence": [{"document": source["document"], "ownerUid": source["ownerUid"],
            "contentDigest": source["contentDigest"], "locator": "paragraph:1", "quote": quote}]
    })
}

fn author(endpoint: &str, path: &Path, namespace: &str, slug: &str, spec: &Value) -> Value {
    let manifest = json!({
        "apiVersion": trogon_atlas_client::manifest::API_VERSION, "kind": "Term",
        "metadata": {"namespace": namespace, "name": slug}, "spec": spec
    });
    std::fs::write(path, manifest.to_string()).unwrap();
    successful(endpoint, &["apply", "-f", path.to_str().unwrap()]);
    let draft = glossary(
        endpoint,
        "get",
        &json!({"term": reference(namespace, slug)}),
    );
    assert_eq!(draft["entry"]["state"], "GLOSSARY_ENTRY_STATE_DRAFT");
    assert_ne!(draft["entry"]["published"], true);
    assert!(draft["entity"]["term"]["glossary"]["review"].is_null());
    draft
}

fn accept(endpoint: &str, draft: &Value) -> Value {
    let accepted = glossary(
        endpoint,
        "review",
        &json!({
            "term": draft["entry"]["term"], "ifMatch": draft["etag"],
            "disposition": "GLOSSARY_REVIEW_DISPOSITION_ACCEPTED",
            "reason": "Synthetic User protocol decision over the exact controlled source and definition; no physical human expert review is claimed."
        }),
    );
    assert_eq!(accepted["entry"]["published"], true);
    assert_eq!(accepted["entry"]["state"], "GLOSSARY_ENTRY_STATE_REVIEWED");
    assert_eq!(
        accepted["entity"]["term"]["glossary"]["review"]["actor"],
        "reviewer"
    );
    assert_eq!(
        accepted["evidence"][0]["state"],
        "GLOSSARY_EVIDENCE_STATE_CURRENT"
    );
    accepted
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn customer_copy_uses_its_own_reviewed_policy_instead_of_internal_booking_language() {
    let endpoint = serve().await;
    let directory = tempfile::tempdir().unwrap();
    let corpus = directory.path().join("sources.yaml");
    std::fs::write(
        &corpus,
        include_str!("../../../../examples/manifests/generated-glossary/sources.yaml"),
    )
    .unwrap();
    successful(&endpoint, &["apply", "-f", corpus.to_str().unwrap()]);
    let initial = glossary(
        &endpoint,
        "list",
        &json!({"selection": {"namespaces": [INTERNAL, CUSTOMER], "includeUnreviewed": true}}),
    );
    assert_eq!(rows(&initial, "entries"), &[] as &[Value]);
    let inspected = glossary(
        &endpoint,
        "sources",
        &json!({"documents": [reference(INTERNAL, "source"), reference(CUSTOMER, "source"),
            reference(INTERNAL, "code-source"), reference(CUSTOMER, "customer-language-policy")]}),
    );
    assert_ne!(inspected["incomplete"], true);
    let sources = rows(&inspected, "sources");
    assert_eq!(sources.len(), 4);
    let internal_source = source(sources, INTERNAL, "source");
    let policy_source = source(sources, CUSTOMER, "customer-language-policy");
    assert_eq!(policy_source["contentDigest"]["sha256"], POLICY_DIGEST);
    assert_eq!(
        source(sources, CUSTOMER, "source")["contentDigest"]["sha256"],
        "1c80fe0552841139971259ac77ba6af6d48aff657765346f56671066da1e1b0f"
    );
    assert_eq!(
        source(sources, INTERNAL, "code-source")["text"],
        include_str!("../../../../examples/glossary/shipment.rs")
    );
    let path = directory.path().join("candidate.yaml");
    let internal_draft = author(
        &endpoint,
        &path,
        INTERNAL,
        "booking",
        &json!({"title": "Booking", "doc": "A reserved carrier handoff slot, not a customer purchase.",
            "glossary": glossary_entry(internal_source, "internal",
                "A booking is a reserved carrier handoff slot, not a customer purchase.",
                &json!({"usage": "GLOSSARY_USAGE_ALLOWED"}))}),
    );
    let internal_booking = accept(&endpoint, &internal_draft);
    assert_eq!(
        internal_booking["entry"]["term"],
        reference(INTERNAL, "booking")
    );
    assert_eq!(internal_booking["entry"]["audience"], "internal");
    assert_eq!(
        internal_booking["entity"]["term"]["glossary"]["policy"]["usage"],
        "GLOSSARY_USAGE_ALLOWED"
    );
    let internal_scope = selection(INTERNAL, "internal");
    let customer_scope = selection(CUSTOMER, "customer");
    let customer_before = glossary(&endpoint, "list", &json!({"selection": customer_scope}));
    assert_eq!(rows(&customer_before, "entries"), &[] as &[Value]);
    assert_eq!(
        rows(
            &check(
                &endpoint,
                &customer_scope,
                UI_COPY,
                "customer-copy-before-policy"
            ),
            "findings"
        ),
        &[] as &[Value]
    );
    let bare = command(&endpoint)
        .args([
            "glossary",
            "check",
            "--request",
            &json!({
                "selection": {"namespaces": [INTERNAL]}, "text": UI_COPY,
                "textRevision": "customer-copy-without-explicit-audience"
            })
            .to_string(),
        ])
        .output()
        .unwrap();
    assert_eq!(bare.status.code(), Some(5));
    let failure: Value = serde_json::from_slice(&bare.stdout).unwrap();
    assert_eq!(failure["ok"], false);
    assert!(failure["failure"]["message"]
        .as_str()
        .unwrap()
        .contains("published audience selection"));
    let pickup_draft = author(
        &endpoint,
        &path,
        CUSTOMER,
        "pickup-slot",
        &json!({"title": "Pickup slot",
            "doc": "The scheduled time window when a customer can hand a package to a carrier.",
            "glossary": glossary_entry(policy_source, "customer",
                "A pickup slot is the scheduled time window when a customer can hand a package to a carrier.",
                &json!({"usage": "GLOSSARY_USAGE_ALLOWED"}))}),
    );
    let pickup = accept(&endpoint, &pickup_draft);
    assert_eq!(pickup["entry"]["term"], reference(CUSTOMER, "pickup-slot"));
    assert_eq!(
        pickup["entity"]["term"]["glossary"]["policy"]["usage"],
        "GLOSSARY_USAGE_ALLOWED"
    );
    let customer_draft = author(
        &endpoint,
        &path,
        CUSTOMER,
        "booking",
        &json!({"title": "Booking", "doc": "An internal carrier handoff-slot label excluded from customer-facing copy.",
            "glossary": glossary_entry(policy_source, "customer",
                "Customer-facing copy must not use Booking and should use Pickup slot instead.",
                &json!({"usage": "GLOSSARY_USAGE_PROHIBITED", "preferredTerm": reference(CUSTOMER, "pickup-slot"),
                    "rationale": "Reserve Booking for internal carrier coordination; use Pickup slot in customer-facing copy."}))}),
    );
    let before_customer_review = glossary(&endpoint, "list", &json!({"selection": customer_scope}));
    assert_eq!(
        rows(&before_customer_review, "entries"),
        &[pickup["entry"].clone()]
    );
    assert_eq!(
        rows(
            &check(
                &endpoint,
                &customer_scope,
                UI_COPY,
                "customer-copy-before-review"
            ),
            "findings"
        ),
        &[] as &[Value]
    );
    let customer_booking = accept(&endpoint, &customer_draft);
    assert_eq!(
        customer_booking["entry"]["term"],
        reference(CUSTOMER, "booking")
    );
    let internal = check(&endpoint, &internal_scope, UI_COPY, "internal-copy-v1");
    assert_eq!(rows(&internal, "findings"), &[] as &[Value]);
    let checked = check(&endpoint, &customer_scope, UI_COPY, "customer-copy-v1");
    let findings = rows(&checked, "findings");
    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding["term"], reference(CUSTOMER, "booking"));
    assert_eq!(finding["usage"], "GLOSSARY_USAGE_PROHIBITED");
    assert_eq!(finding["preferredTerm"], reference(CUSTOMER, "pickup-slot"));
    assert_eq!(finding["matched"], "booking");
    assert_eq!(finding["range"], json!({"start": 10, "end": 17}));
    assert_eq!(&UI_COPY.as_bytes()[10..17], b"booking");
    assert_eq!(UI_COPY[..10].chars().count(), 7);
    assert_ne!(checked["snapshotRevision"], internal["snapshotRevision"]);
    for (namespace, audience) in [(INTERNAL, "customer"), (CUSTOMER, "internal")] {
        let scope = selection(namespace, audience);
        let mismatched = glossary(&endpoint, "list", &json!({"selection": scope}));
        assert_eq!(rows(&mismatched, "entries"), &[] as &[Value]);
        assert_eq!(
            rows(
                &check(&endpoint, &scope, UI_COPY, "mismatched-context"),
                "findings"
            ),
            &[] as &[Value]
        );
    }
    let mut corrected = UI_COPY.to_owned();
    corrected.replace_range(
        10..17,
        &pickup["entity"]["term"]["title"]
            .as_str()
            .unwrap()
            .to_lowercase(),
    );
    assert_eq!(corrected, "📦 Your pickup slot is confirmed.");
    assert_eq!(
        rows(
            &check(&endpoint, &customer_scope, &corrected, "customer-copy-v2"),
            "findings"
        ),
        &[] as &[Value]
    );
    assert_eq!(
        glossary(
            &endpoint,
            "get",
            &json!({"term": reference(INTERNAL, "booking")})
        ),
        internal_booking
    );
}
