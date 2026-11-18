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
use serde_json::json;
fn fixture() -> serde_json::Value {
    let target = json!({"kind":"ENTITY_KIND_COMPONENT","id":{"namespace":"catalog","slug":"target","version":"18446744073709551615"}});
    let raw = json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"Component","metadata":{"namespace":"catalog","name":"agent","version":"9007199254740993"},"spec":{"title":"Agent","metadata":[
        {"@type":"type.googleapis.com/trogonatlas.eventmodel.v1alpha1.AgentAnnotation","model":{"version":{"value":"opaque preview"}},"tools":[{"name":{"value":"lookup"},"type":{"value":"manual"},"description":{"value":"Inert description"},"icon":{"value":"unknown:token"},"url":{"value":"https://example.test/tool"}},{"name":{"value":"lookup"},"type":{"value":"other"}}]},
        {"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation","status":"draft","draft":{"message":{"value":"Review required"}}},
        {"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.BadgesAnnotation","badges":[{"content":{"value":"Preview"},"backgroundColor":{"value":"#aAbBcC"},"textColor":{"value":"#FFFFFF"}}]},
        {"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation","groups":[{"groupId":{"value":"10000000-0000-4000-8000-000000000001"},"displayLimit":{"value":1},"sidebar":false,"items":[{"connectionId":{"value":"10000000-0000-4000-8000-000000000002"},"target":target},{"connectionId":{"value":"10000000-0000-4000-8000-000000000003"},"target":target}]}]}
    ]}});
    raw
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_metadata_cli_apply_get_and_same_store_reapply() {
    let endpoint = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("metadata.yaml");
    let target = serde_json::json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"Component","metadata":{"namespace":"catalog","name":"target","version":"18446744073709551615"},"spec":{"title":"Target"}});
    std::fs::write(&path, target.to_string()).unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    std::fs::write(&path, fixture().to_string()).unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    let address = "component/catalog/agent@9007199254740993";
    let before = cli(&endpoint, &["get", address]);
    let mut native = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    let request = trogon_atlas_proto::GetEntityRequest {
        kind: trogon_atlas_proto::EntityKind::Component as i32,
        id: Some(trogon_atlas_proto::Id {
            namespace: "catalog".into(),
            slug: "agent".into(),
            version: 9_007_199_254_740_993,
        }),
    };
    let authoritative_before = native
        .get_entity(request.clone())
        .await
        .unwrap()
        .into_inner();
    assert_ne!(authoritative_before.etag, "");
    assert_ne!(
        authoritative_before
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid,
        ""
    );
    cli(&endpoint, &["diff", "-f", path.to_str().unwrap()]);
    let description: serde_json::Value =
        serde_json::from_str(&cli(&endpoint, &["--format", "json", "describe", address])).unwrap();
    assert!(description.to_string().contains("opaque preview"));
    let export_dir = dir.path().join("export");
    cli(
        &endpoint,
        &[
            "export",
            "--namespace",
            "catalog",
            "-o",
            export_dir.to_str().unwrap(),
        ],
    );
    cli(&endpoint, &["diff", "-f", export_dir.to_str().unwrap()]);
    cli(&endpoint, &["apply", "-f", export_dir.to_str().unwrap()]);
    std::fs::write(&path, &before).unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    assert_eq!(cli(&endpoint, &["get", address]), before);
    assert_eq!(
        native.get_entity(request).await.unwrap().into_inner(),
        authoritative_before
    );
    let mut reordered = fixture();
    reordered["spec"]["metadata"][0]["tools"]
        .as_array_mut()
        .unwrap()
        .reverse();
    std::fs::write(&path, reordered.to_string()).unwrap();
    let diff = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            &endpoint,
            "diff",
            "-f",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(diff.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&diff.stdout).contains("lookup"));
    assert!(before.contains("10000000-0000-4000-8000-000000000002"));
    assert!(before.contains("10000000-0000-4000-8000-000000000003"));
    assert!(before.contains("18446744073709551615"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_metadata_invalid_admission_is_atomic() {
    let endpoint = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.yaml");
    let mut valid = fixture();
    valid["spec"]["metadata"].as_array_mut().unwrap().pop();
    std::fs::write(&path, valid.to_string()).unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    let address = "component/catalog/agent@9007199254740993";
    let before = cli(&endpoint, &["get", address]);
    let mut native = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    let request = trogon_atlas_proto::GetEntityRequest {
        kind: trogon_atlas_proto::EntityKind::Component as i32,
        id: Some(trogon_atlas_proto::Id {
            namespace: "catalog".into(),
            slug: "agent".into(),
            version: 9_007_199_254_740_993,
        }),
    };
    let authoritative = native
        .get_entity(request.clone())
        .await
        .unwrap()
        .into_inner();
    let mut cases = Vec::new();
    let mut empty_model = valid.clone();
    empty_model["spec"]["metadata"][0]["model"] = json!({});
    cases.push(empty_model);
    let mut duplicate = valid.clone();
    duplicate["spec"]["metadata"]
        .as_array_mut()
        .unwrap()
        .push(valid["spec"]["metadata"][0].clone());
    cases.push(duplicate);
    let mut tools = valid.clone();
    tools["spec"]["metadata"][0]["tools"][1] = tools["spec"]["metadata"][0]["tools"][0].clone();
    cases.push(tools);
    let mut lifecycle = valid.clone();
    lifecycle["spec"]["metadata"][1]["status"] = json!("accepted");
    cases.push(lifecycle);
    let mut badge = valid.clone();
    badge["spec"]["metadata"][2]["badges"][0]["url"] = json!({"value":"http://example.test"});
    cases.push(badge);
    for annotation in valid["spec"]["metadata"].as_array().unwrap() {
        let mut duplicate = valid.clone();
        duplicate["spec"]["metadata"]
            .as_array_mut()
            .unwrap()
            .push(annotation.clone());
        cases.push(duplicate);
    }
    for date in [
        "2023-02-29",
        "2026-04-31",
        "0000-01-01",
        "10000-01-01",
        "2024-02-29T00:00:00Z",
        "",
    ] {
        let mut invalid = valid.clone();
        invalid["spec"]["metadata"][1] = json!({"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation","status":"deprecated","deprecatedOn":{"value":date}});
        cases.push(invalid);
    }
    for field in ["provider", "name", "version"] {
        let mut invalid = valid.clone();
        invalid["spec"]["metadata"][0]["model"] = json!({field:{"value":" "}});
        cases.push(invalid);
    }
    for (index, invalid) in cases.into_iter().enumerate() {
        std::fs::write(&path, invalid.to_string()).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
            .args([
                "--endpoint",
                &endpoint,
                "apply",
                "-f",
                path.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!result.status.success(), "case {index} accepted");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("invalid"),
            "case {index}: {result:?}"
        );
        assert_eq!(cli(&endpoint, &["get", address]), before);
        assert_eq!(
            native
                .get_entity(request.clone())
                .await
                .unwrap()
                .into_inner(),
            authoritative
        );
    }
    for kind in ["Event", "Processor", "ExternalSystem"] {
        let invalid = json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":kind,"metadata":{"namespace":"catalog","name":"invalid-owner"},"spec":{"title":"Invalid","metadata":[valid["spec"]["metadata"][0].clone()]}});
        std::fs::write(&path, invalid.to_string()).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
            .args([
                "--endpoint",
                &endpoint,
                "apply",
                "-f",
                path.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!result.status.success(), "scope {kind} accepted");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("metadata[0]"),
            "{result:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_metadata_tenant_descriptors_follow_branch_context() {
    let endpoint = start_server().await;
    cli(&endpoint, &["branch", "create", "metadata-types"]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tenant.yaml");
    let library = json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"TypeLibrary","metadata":{"namespace":"catalog","name":"catalog.notes"},"spec":{"title":"Notes","files":[{"path":"catalog/notes.proto","content":"syntax = \"proto3\"; package catalog.notes; message Note { string text = 1; }"}]}});
    let mut owner = fixture();
    owner["spec"]["metadata"].as_array_mut().unwrap().pop();
    owner["spec"]["metadata"].as_array_mut().unwrap().push(
        json!({"@type":"type.googleapis.com/catalog.notes.Note","text":"Tenant context retained"}),
    );
    std::fs::write(&path, format!("{library}\n---\n{owner}")).unwrap();
    let documents = trogon_atlas_client::manifest::read_str(
        &std::fs::read_to_string(&path).unwrap(),
        "tenant.yaml",
    )
    .unwrap();
    let mut native = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    let resolved = trogon_atlas_client::manifest::load_with_tenant_types(&mut native, documents)
        .await
        .unwrap();
    let rendered = trogon_atlas_client::export::render_board(
        &resolved
            .manifests
            .iter()
            .map(|item| item.entity.clone())
            .collect::<Vec<_>>(),
        &[],
        &resolved.types,
    )
    .unwrap();
    assert!(rendered.manifests.contains("Tenant context retained"));
    assert!(
        trogon_atlas_client::manifest::parse_str(&owner.to_string(), "missing-types.yaml").is_err()
    );
    cli(
        &endpoint,
        &[
            "--branch",
            "metadata-types",
            "apply",
            "-f",
            path.to_str().unwrap(),
        ],
    );
    let address = "component/catalog/agent@9007199254740993";
    let before = cli(&endpoint, &["--branch", "metadata-types", "get", address]);
    assert!(before.contains("Tenant context retained"));
    assert!(before.contains("opaque preview"));
    let description = cli(
        &endpoint,
        &["--branch", "metadata-types", "describe", address],
    );
    assert!(description.contains("Tenant context retained"));
    std::fs::write(&path, &before).unwrap();
    cli(
        &endpoint,
        &[
            "--branch",
            "metadata-types",
            "diff",
            "-f",
            path.to_str().unwrap(),
        ],
    );
    cli(
        &endpoint,
        &[
            "--branch",
            "metadata-types",
            "apply",
            "-f",
            path.to_str().unwrap(),
        ],
    );
    assert_eq!(
        cli(&endpoint, &["--branch", "metadata-types", "get", address]),
        before
    );
    let missing = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            &endpoint,
            "apply",
            "-f",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("catalog.notes.Note"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_metadata_branch_isolation_and_noop() {
    let endpoint = start_server().await;
    cli(&endpoint, &["branch", "create", "metadata-native"]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("branch.yaml");
    let mut owner = fixture();
    owner["spec"]["metadata"].as_array_mut().unwrap().pop();
    std::fs::write(&path, owner.to_string()).unwrap();
    cli(
        &endpoint,
        &[
            "--branch",
            "metadata-native",
            "apply",
            "-f",
            path.to_str().unwrap(),
        ],
    );
    let address = "component/catalog/agent@9007199254740993";
    let before = cli(&endpoint, &["--branch", "metadata-native", "get", address]);
    cli(
        &endpoint,
        &[
            "--branch",
            "metadata-native",
            "diff",
            "-f",
            path.to_str().unwrap(),
        ],
    );
    std::fs::write(&path, &before).unwrap();
    cli(
        &endpoint,
        &[
            "--branch",
            "metadata-native",
            "apply",
            "-f",
            path.to_str().unwrap(),
        ],
    );
    assert_eq!(
        cli(&endpoint, &["--branch", "metadata-native", "get", address]),
        before
    );
    let baseline = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args(["--endpoint", &endpoint, "get", address])
        .output()
        .unwrap();
    assert!(!baseline.status.success());
    assert!(String::from_utf8_lossy(&baseline.stderr).contains("not found"));
}
