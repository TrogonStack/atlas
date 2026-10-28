#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}
fn cli(endpoint: &str, args: &[&str]) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(endpoint)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

const MANIFEST: &str = r"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Domain
metadata: {namespace: vocabulary, name: sales}
spec: {title: Sales}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Term
metadata: {namespace: vocabulary, name: order}
spec:
  title: Order
  owningDomain: {id: {namespace: vocabulary, slug: sales, version: '1'}}
";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn term_domain_cli_roundtrip_and_paged_query() {
    let endpoint = start_server().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("terms.yaml");
    std::fs::write(&path, MANIFEST).unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    let first = cli(&endpoint, &["get", "term/vocabulary/order@1"]);
    assert!(first.contains("owningDomain"));
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    assert_eq!(first, cli(&endpoint, &["get", "term/vocabulary/order@1"]));
    let query =
        r#"{"domain":{"id":{"namespace":"vocabulary","slug":"sales","version":"1"}},"pageSize":1}"#;
    let listed = cli(
        &endpoint,
        &["list-term-domain-declarations", "--request", query],
    );
    assert!(listed.contains("declarations"));
    assert!(listed.contains("owningDomain") || listed.contains("domain"));
}

fn complete_model_manifest() -> String {
    let version = u64::MAX.to_string();
    let native = |kind: &str, name: &str, version: &str, spec: serde_json::Value| serde_json::json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":kind,"metadata":{"namespace":"term-export","name":name,"version":version},"spec":spec});
    let reference = |kind: &str, slug: &str| serde_json::json!({"kind":kind,"id":{"namespace":"term-export","slug":slug,"version":version}});
    [native("Domain", "domain", &version, serde_json::json!({"title":"Domain"})),
     native("Term", "declared", &version, serde_json::json!({"title":"Word","doc":"Exact definition","owningDomain":{"id":{"namespace":"term-export","slug":"domain","version":version}}})),
     native("Term", "undeclared", &version, serde_json::json!({"title":"Word","doc":"Exact definition"})),
     native("EventModel", "model", "1", serde_json::json!({"title":"Vocabulary model","members":[reference("ENTITY_KIND_TERM","declared"),reference("ENTITY_KIND_TERM","undeclared")]}))]
        .iter().map(ToString::to_string).collect::<Vec<_>>().join("\n---\n")
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_model_export_diff_and_reapply_preserve_full_native_identity_at_u64_max() {
    use trogon_atlas_proto as pb;
    let endpoint = start_server().await;
    let mut authoritative =
        pb::event_model_service_client::EventModelServiceClient::connect(endpoint.clone())
            .await
            .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.yaml");
    std::fs::write(&path, complete_model_manifest()).unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    let keys = [
        (pb::EntityKind::Domain, "domain", u64::MAX),
        (pb::EntityKind::Term, "declared", u64::MAX),
        (pb::EntityKind::Term, "undeclared", u64::MAX),
        (pb::EntityKind::EventModel, "model", 1),
    ];
    let mut before = Vec::new();
    for (kind, slug, version) in keys {
        before.push(
            authoritative
                .get_entity(pb::GetEntityRequest {
                    kind: kind as i32,
                    id: Some(pb::Id {
                        namespace: "term-export".into(),
                        slug: slug.into(),
                        version,
                    }),
                })
                .await
                .unwrap()
                .into_inner(),
        );
    }
    let model = authoritative
        .export_event_model_manifests(pb::ExportEventModelManifestsRequest {
            id: Some(pb::Id {
                namespace: "term-export".into(),
                slug: "model".into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!model.truncated);
    assert_eq!(model.skipped.len(), 0);
    let parsed = trogon_atlas_client::manifest::parse_str(&model.manifests, "model.yaml").unwrap();
    assert_eq!(parsed.len(), 4);
    for entry in &parsed {
        assert!(
            before
                .iter()
                .any(|native| trogon_atlas_core::semantically_equal(
                    native.entity.as_ref().unwrap(),
                    &entry.entity
                ))
        );
    }
    let export = directory.path().join("export");
    cli(
        &endpoint,
        &[
            "export",
            "--namespace",
            "term-export",
            "-o",
            export.to_str().unwrap(),
        ],
    );
    cli(&endpoint, &["diff", "-f", export.to_str().unwrap()]);
    cli(&endpoint, &["apply", "-f", export.to_str().unwrap()]);
    let model_path = directory.path().join("model.yaml");
    std::fs::write(&model_path, &model.manifests).unwrap();
    cli(&endpoint, &["diff", "-f", model_path.to_str().unwrap()]);
    cli(&endpoint, &["apply", "-f", model_path.to_str().unwrap()]);
    let declared = parsed
        .iter()
        .find_map(|entry| match &entry.entity.kind {
            Some(pb::entity::Kind::Term(term)) if term.id.as_ref().unwrap().slug == "declared" => {
                Some(term)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(declared.id.as_ref().unwrap().version, u64::MAX);
    assert_eq!(
        declared
            .owning_domain
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        u64::MAX
    );
    assert!(parsed.iter().any(|entry| matches!(&entry.entity.kind,Some(pb::entity::Kind::Term(term)) if term.id.as_ref().unwrap().slug == "undeclared" && term.owning_domain.is_none())));
    let changed = serde_json::json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"Term","metadata":{"namespace":"term-export","name":"declared","version":u64::MAX.to_string()},"spec":{"title":"Word","doc":"Exact definition"}});
    let changed_path = directory.path().join("changed.yaml");
    std::fs::write(&changed_path, changed.to_string()).unwrap();
    let drift = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            &endpoint,
            "diff",
            "-f",
            changed_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        drift.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&drift.stderr)
    );
    assert!(
        String::from_utf8(drift.stdout)
            .unwrap()
            .contains("owningDomain")
    );
    for ((kind, slug, version), expected) in keys.into_iter().zip(before) {
        let actual = authoritative
            .get_entity(pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(pb::Id {
                    namespace: "term-export".into(),
                    slug: slug.into(),
                    version,
                }),
            })
            .await
            .unwrap()
            .into_inner();
        assert_ne!(actual.etag, "");
        assert_ne!(
            actual.entity.as_ref().unwrap().system.as_ref().unwrap().uid,
            ""
        );
        assert_eq!(actual, expected);
    }
}
