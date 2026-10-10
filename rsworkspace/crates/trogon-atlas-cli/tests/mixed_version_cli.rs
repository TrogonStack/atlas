#![allow(clippy::unwrap_used, clippy::expect_used)]

// trogon-atlas must ask GetServerInfo before mutating and refuse, without sending
// the mutation, when the server it reaches cannot honor what it was asked to
// do. The server here is a discovery-only fake posing as another version.

use std::process::Output;

use trogon_atlas_proto as pb;
use trogon_atlas_testsupport::discovery::{pre_revision_server_info, DiscoveryOnlyServer};

const MANIFEST: &str = "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: trogon-atlas-compat, name: thing.happened }
spec: { title: Thing happened }
";

fn current_server_info() -> pb::GetServerInfoResponse {
    pb::GetServerInfoResponse {
        schema_version: pb::SCHEMA_VERSION.into(),
        server_version: "test".into(),
        features: Some(pb::get_server_info_response::Features {
            mutations: true,
            validate_only: true,
            branch_scoped_requests: true,
            state_preconditions: true,
            ..Default::default()
        }),
        limits: None,
        contract_revision: pb::CONTRACT_REVISION,
        min_client_contract_revision: pb::MIN_CLIENT_CONTRACT_REVISION,
        writer_status: None,
    }
}

async fn trogon_atlas(endpoint: &str, args: &[&str]) -> Output {
    let dir = std::env::temp_dir().join(format!(
        "trogon-atlas-compat-{}",
        trogon_atlas_testsupport::unique_suffix_for_test()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = dir.join("event.yaml");
    std::fs::write(&manifest, MANIFEST).unwrap();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_trogon-atlas"));
    command
        .arg("--endpoint")
        .arg(endpoint)
        .arg("--rpc-timeout-secs")
        .arg("5")
        .args(args)
        .arg("-f")
        .arg(&manifest);
    let output = tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    output
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_refuses_server_that_predates_contract_revisions() {
    let server = DiscoveryOnlyServer::start(pre_revision_server_info())
        .await
        .unwrap();
    let out = trogon_atlas(&server.endpoint, &["apply"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "apply must fail: {stderr}");
    assert!(
        stderr.contains("upgrade the server"),
        "refusal must say which side to upgrade: {stderr}"
    );
    assert!(
        server.calls_other_than_discovery().is_empty(),
        "nothing past discovery may reach the server: {:?}",
        server.calls()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_refuses_server_that_requires_a_newer_client() {
    let server = DiscoveryOnlyServer::start(pb::GetServerInfoResponse {
        contract_revision: pb::CONTRACT_REVISION + 1,
        min_client_contract_revision: pb::CONTRACT_REVISION + 1,
        ..current_server_info()
    })
    .await
    .unwrap();
    let out = trogon_atlas(&server.endpoint, &["apply"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "apply must fail: {stderr}");
    assert!(
        stderr.contains("upgrade this client"),
        "refusal must say which side to upgrade: {stderr}"
    );
    assert_eq!(server.calls_other_than_discovery(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_refuses_server_without_validate_only() {
    let mut info = current_server_info();
    info.features.as_mut().unwrap().validate_only = false;
    let server = DiscoveryOnlyServer::start(info).await.unwrap();
    let out = trogon_atlas(&server.endpoint, &["apply", "--dry-run"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "dry run must fail: {stderr}");
    assert!(
        stderr.contains("validate_only"),
        "refusal must name the missing capability: {stderr}"
    );
    assert_eq!(server.calls_other_than_discovery(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_apply_refuses_server_that_ignores_branch_header() {
    let mut info = current_server_info();
    info.features.as_mut().unwrap().branch_scoped_requests = false;
    let server = DiscoveryOnlyServer::start(info).await.unwrap();
    let out = trogon_atlas(&server.endpoint, &["--branch", "feature-x", "apply"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "branch apply must fail: {stderr}");
    assert!(
        stderr.contains("branch_scoped_requests"),
        "refusal must name the missing capability: {stderr}"
    );
    assert_eq!(server.calls_other_than_discovery(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn supported_server_passes_the_gate() {
    let server = DiscoveryOnlyServer::start(current_server_info())
        .await
        .unwrap();
    let out = trogon_atlas(&server.endpoint, &["apply"]).await;
    assert!(
        !server.calls_other_than_discovery().is_empty(),
        "a supported server must see apply proceed past discovery: {:?} {}",
        server.calls(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn stored_event_without_revision(title: &str) -> pb::BatchGetEntitiesResponse {
    pb::BatchGetEntitiesResponse {
        entities: vec![pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: "trogon-atlas-compat".into(),
                    slug: "thing.happened".into(),
                    version: 1,
                }),
                title: title.into(),
                ..Default::default()
            })),
        }],
        found: vec![true],
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_still_reads_a_server_that_predates_state_preconditions() {
    let server = DiscoveryOnlyServer::start_serving_entities(
        pre_revision_server_info(),
        stored_event_without_revision("Earlier title"),
    )
    .await
    .unwrap();
    let out = trogon_atlas(&server.endpoint, &["diff"]).await;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "diff must report drift, not fail: {stdout}{stderr}"
    );
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with('-') && l.contains("Earlier title")),
        "{stdout}"
    );
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with('+') && l.contains("Thing happened")),
        "{stdout}"
    );
    assert_eq!(
        server.calls_other_than_discovery(),
        vec!["/trogonatlas.api.eventmodel.v1alpha1.EventModelService/BatchGetEntities".to_string()]
    );
}
