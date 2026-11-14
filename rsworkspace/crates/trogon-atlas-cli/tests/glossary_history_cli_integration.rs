#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, process::Command, sync::Arc};

use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_client::client::Client;
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelServiceServer};
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::data_plane,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const TOKEN: &str = "glossary-history-public-fixture";
const NAMESPACE: &str = "glossary-history";
const SOURCE: &str = "A parcel is a shipment handled by fulfillment.\r\n";

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

fn successful(endpoint: &str, arguments: &[&str]) -> String {
    let output = command(endpoint).args(arguments).output().unwrap();
    assert!(
        output.status.success(),
        "{arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn glossary(endpoint: &str, action: &str, request: &Value) -> Value {
    serde_json::from_str(&successful(
        endpoint,
        &["glossary", action, "--request", &request.to_string()],
    ))
    .unwrap()
}

fn reference(slug: &str) -> Value {
    json!({"id": {"namespace": NAMESPACE, "slug": slug, "version": "1"}})
}

fn manifest(kind: &str, slug: &str, spec: &Value) -> Value {
    json!({
        "apiVersion": trogon_atlas_client::manifest::API_VERSION,
        "kind": kind, "metadata": {"namespace": NAMESPACE, "name": slug}, "spec": spec
    })
}

fn apply(endpoint: &str, path: &Path, value: &Value) {
    std::fs::write(path, value.to_string()).unwrap();
    successful(endpoint, &["apply", "-f", path.to_str().unwrap()]);
}

fn seed_source(endpoint: &str, path: &Path) -> Value {
    apply(
        endpoint,
        path,
        &manifest("Event", "context", &json!({"title": "Fulfillment"})),
    );
    apply(
        endpoint,
        path,
        &manifest(
            "Document",
            "source",
            &json!({
                "title": {"value": "Retained fulfillment source"},
                "subjects": [{"subject": {"entity": {
                    "kind": "ENTITY_KIND_EVENT", "id": reference("context")["id"]
                }}}],
                "retained": {"mediaType": {"value": "text/plain"}, "content": SOURCE,
                    "digest": {"sha256": trogon_atlas_core::specification::content_digest(SOURCE).sha256}}
            }),
        ),
    );
    let sources = glossary(
        endpoint,
        "sources",
        &json!({"documents": [reference("source")]}),
    );
    assert_eq!(sources["sources"][0]["text"], SOURCE);
    sources["sources"][0].clone()
}

fn candidate(source: &Value) -> Value {
    manifest(
        "Term",
        "parcel",
        &json!({"title": "Parcel", "doc": "A shipment.", "glossary": {
            "evidence": [{"document": source["document"], "ownerUid": source["ownerUid"],
                "contentDigest": source["contentDigest"], "locator": "paragraph:1", "quote": SOURCE}],
            "generationRevision": "public-history-fixture-v1", "audience": "internal",
            "nextReviewOn": {"value": "9999-12-31"},
            "policy": {"usage": "GLOSSARY_USAGE_ALLOWED"}
        }}),
    )
}

async fn stored(client: &mut Client, kind: pb::EntityKind, slug: &str) -> pb::GetEntityResponse {
    client
        .get_entity(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(pb::Id {
                namespace: NAMESPACE.into(),
                slug: slug.into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner()
}

async fn history(client: &mut Client) -> Vec<pb::EntityRevision> {
    let mut revisions = Vec::new();
    let mut page_token = String::new();
    loop {
        let page = client
            .get_entity_history(pb::GetEntityHistoryRequest {
                r#ref: Some(pb::EntityRef {
                    kind: pb::EntityKind::Term as i32,
                    id: Some(pb::Id {
                        namespace: NAMESPACE.into(),
                        slug: "parcel".into(),
                        version: 1,
                    }),
                }),
                page_token,
                page_size: 2,
            })
            .await
            .unwrap()
            .into_inner();
        assert!(page.revisions.len() <= 2);
        revisions.extend(page.revisions);
        if page.next_page_token.is_empty() {
            return revisions;
        }
        page_token = page.next_page_token;
    }
}

fn term(entity: &pb::Entity) -> &pb::Term {
    let Some(pb::entity::Kind::Term(value)) = entity.kind.as_ref() else {
        panic!("expected Term")
    };
    value
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_corrections_history_export_and_reapply_preserve_exact_review_and_source() {
    let endpoint = serve().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, Some(TOKEN), 30)
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("candidate.yaml");
    let source = seed_source(&endpoint, &path);
    let retained = stored(&mut client, pb::EntityKind::Document, "source").await;
    apply(&endpoint, &path, &candidate(&source));
    let draft = stored(&mut client, pb::EntityKind::Term, "parcel").await;
    glossary(
        &endpoint,
        "review",
        &json!({"term": reference("parcel"), "ifMatch": draft.etag,
            "disposition": "GLOSSARY_REVIEW_DISPOSITION_NEEDS_CHANGES",
            "reason": "Synthetic User check: describe fulfillment handling."}),
    );
    let needs_changes = stored(&mut client, pb::EntityKind::Term, "parcel").await;
    let get = successful(&endpoint, &["get", &format!("term/{NAMESPACE}/parcel@1")]);
    let mut correction = trogon_atlas_client::manifest::parse_str(&get, "get.yaml").unwrap();
    let mut corrected_spec =
        trogon_atlas_core::transcode::entity_json_value(&correction.remove(0).entity).unwrap()
            ["term"]
            .clone();
    corrected_spec.as_object_mut().unwrap().remove("id");
    corrected_spec["doc"] = json!("A shipment handled by fulfillment.");
    apply(
        &endpoint,
        &path,
        &manifest("Term", "parcel", &corrected_spec),
    );
    let corrected = stored(&mut client, pb::EntityKind::Term, "parcel").await;
    let corrected_detail = glossary(&endpoint, "get", &json!({"term": reference("parcel")}));
    assert_eq!(
        corrected_detail["entry"]
            .get("published")
            .unwrap_or(&Value::Bool(false)),
        &Value::Bool(false)
    );
    assert_eq!(
        term(corrected.entity.as_ref().unwrap())
            .glossary
            .as_ref()
            .unwrap()
            .review,
        term(needs_changes.entity.as_ref().unwrap())
            .glossary
            .as_ref()
            .unwrap()
            .review
    );
    glossary(
        &endpoint,
        "review",
        &json!({"term": reference("parcel"), "ifMatch": corrected.etag,
            "disposition": "GLOSSARY_REVIEW_DISPOSITION_ACCEPTED",
            "reason": "Synthetic User check: corrected definition matches retained source."}),
    );
    let accepted = stored(&mut client, pb::EntityKind::Term, "parcel").await;
    let accepted_entity = accepted.entity.as_ref().unwrap();
    let accepted_glossary = term(accepted_entity).glossary.as_ref().unwrap();
    let review = accepted_glossary.review.as_ref().unwrap();
    assert_eq!(review.actor, "reviewer");
    assert_eq!(
        review.disposition,
        pb::GlossaryReviewDisposition::Accepted as i32
    );
    assert_eq!(
        review.owner_uid,
        accepted_entity.system.as_ref().unwrap().uid
    );
    assert_ne!(review.reviewed_at, "");
    assert_ne!(
        review.content_digest,
        term(needs_changes.entity.as_ref().unwrap())
            .glossary
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap()
            .content_digest
    );
    assert_eq!(
        accepted_glossary.evidence,
        term(draft.entity.as_ref().unwrap())
            .glossary
            .as_ref()
            .unwrap()
            .evidence
    );
    let states = [&draft, &needs_changes, &corrected, &accepted];
    for pair in states.windows(2) {
        assert!(pair[0].etag.parse::<u64>().unwrap() < pair[1].etag.parse::<u64>().unwrap());
        assert_eq!(
            pair[0].entity.as_ref().unwrap().system,
            pair[1].entity.as_ref().unwrap().system
        );
    }
    let revisions = history(&mut client).await;
    assert_eq!(revisions.len(), states.len());
    for (index, state) in states.iter().rev().enumerate() {
        assert_eq!(revisions[index].after, state.entity);
        assert_eq!(revisions[index].author, "reviewer");
        if let Some(older) = revisions.get(index + 1) {
            assert_eq!(revisions[index].before, older.after);
            assert!(revisions[index].changeset_id > older.changeset_id);
        } else {
            assert!(revisions[index].before.is_none());
        }
    }
    let output = directory.path().join("exported");
    successful(
        &endpoint,
        &[
            "export",
            "--namespace",
            NAMESPACE,
            "-o",
            output.to_str().unwrap(),
        ],
    );
    for (filename, entity) in [
        ("term.yaml", accepted.entity.as_ref().unwrap()),
        ("document.yaml", retained.entity.as_ref().unwrap()),
    ] {
        let written = std::fs::read_to_string(output.join(filename)).unwrap();
        let parsed = trogon_atlas_client::manifest::parse_str(&written, filename).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].entity.kind, entity.kind);
        assert!(parsed[0].entity.system.is_none());
    }
    let reapplied = successful(&endpoint, &["apply", "-f", output.to_str().unwrap()]);
    assert_eq!(reapplied.matches("unchanged").count(), 3);
    let after = stored(&mut client, pb::EntityKind::Term, "parcel").await;
    assert_eq!(after, accepted);
    assert_eq!(
        stored(&mut client, pb::EntityKind::Document, "source").await,
        retained
    );
    assert_eq!(history(&mut client).await, revisions);
    let published = glossary(&endpoint, "get", &json!({"term": reference("parcel")}));
    assert_eq!(published["entry"]["published"], true);
    let final_sources = glossary(
        &endpoint,
        "sources",
        &json!({"documents": [reference("source")]}),
    );
    assert_eq!(final_sources["sources"][0], source);

    let fresh_endpoint = serve().await;
    let mut fresh_client = trogon_atlas_client::client::connect(&fresh_endpoint, Some(TOKEN), 30)
        .await
        .unwrap();
    let refused = command(&fresh_endpoint)
        .args(["apply", "-f", output.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("glossary reviews are server-owned"));
    let missing = fresh_client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Term as i32,
            id: Some(pb::Id {
                namespace: NAMESPACE.into(),
                slug: "parcel".into(),
                version: 1,
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code(), tonic::Code::NotFound);
    assert_eq!(
        history(&mut fresh_client).await,
        Vec::<pb::EntityRevision>::new()
    );
    let fresh_source = seed_source(&fresh_endpoint, &path);
    assert_ne!(fresh_source["ownerUid"], source["ownerUid"]);
    let fresh_candidate = candidate(&fresh_source);
    apply(&fresh_endpoint, &path, &fresh_candidate);
    let fresh = stored(&mut fresh_client, pb::EntityKind::Term, "parcel").await;
    assert_ne!(
        fresh.entity.as_ref().unwrap().system,
        accepted.entity.as_ref().unwrap().system
    );
    assert!(term(fresh.entity.as_ref().unwrap())
        .glossary
        .as_ref()
        .unwrap()
        .review
        .is_none());
    let fresh_detail = glossary(
        &fresh_endpoint,
        "get",
        &json!({"term": reference("parcel")}),
    );
    assert_eq!(fresh_detail["entry"]["state"], "GLOSSARY_ENTRY_STATE_DRAFT");
    assert_eq!(
        fresh_detail["entry"]
            .get("published")
            .unwrap_or(&Value::Bool(false)),
        &Value::Bool(false)
    );
}
