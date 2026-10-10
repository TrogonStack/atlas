#![allow(clippy::unwrap_used, clippy::expect_used)]

// Exit-code plumbing for `trogon-atlas branch diff/merge/update/resolve`, spawning
// the actual compiled binary (env!("CARGO_BIN_EXE_trogon-atlas")) against an
// in-process gRPC server, the same way a real user or CI job invokes it.
// Complements trogon-atlas-client/tests/apply_integration.rs, which drives
// the same RPCs through the library wrappers directly rather than through
// the CLI's argument parsing and exit-code mapping.

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
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

fn event_entity(ns: &str, slug: &str, version: u64, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version,
            }),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn put_req(entity: pb::Entity, create_only: bool) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(entity),
        create_only,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    }
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

fn trogon_atlas_branched(endpoint: &str, branch: &str, args: &[&str]) -> TrogonAtlasOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(endpoint)
        .arg("--branch")
        .arg(branch)
        .args(args)
        .output()
        .expect("trogon-atlas must spawn");
    TrogonAtlasOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_branch_merge_with_conflict_exits_1_and_prints_conflict() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();

    client
        .put_entity(put_req(event_entity("cli-p2c", "a", 1, "Base"), false))
        .await
        .unwrap();

    let created = trogon_atlas(&endpoint, &["branch", "create", "cli-conflict"]);
    assert!(created.status.success(), "{:?}", created.stderr);

    let dir = tempfile_dir();
    let manifest_path = dir.join("branch-edit.yaml");
    std::fs::write(
        &manifest_path,
        r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: cli-p2c, name: a }
spec: { title: Branch edit }
",
    )
    .unwrap();
    let applied = trogon_atlas_branched(
        &endpoint,
        "cli-conflict",
        &["apply", "-f", manifest_path.to_str().unwrap()],
    );
    assert!(applied.status.success(), "{:?}", applied.stderr);

    // Baseline independently moves the same entity: edit/edit conflict.
    client
        .put_entity(put_req(
            event_entity("cli-p2c", "a", 1, "Baseline moved"),
            false,
        ))
        .await
        .unwrap();

    let merged = trogon_atlas(
        &endpoint,
        &["branch", "merge", "cli-conflict", "--keep-branch"],
    );
    assert_eq!(merged.status.code(), Some(1), "{merged:?}");
    assert!(
        merged.stderr.contains("cli-conflict has conflicts"),
        "{:?}",
        merged.stderr
    );
    assert!(
        merged.stdout.contains("conflict (edit/edit)"),
        "{:?}",
        merged.stdout
    );

    // Nothing landed: baseline still shows the independently-moved value.
    let still_baseline = client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "cli-p2c".into(),
                slug: "a".into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    match still_baseline.entity.unwrap().kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Baseline moved"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_branch_merge_dry_run_exits_0() {
    let endpoint = start_server().await;

    let created = trogon_atlas(&endpoint, &["branch", "create", "cli-dry"]);
    assert!(created.status.success(), "{:?}", created.stderr);

    let dir = tempfile_dir();
    let manifest_path = dir.join("dry-run.yaml");
    std::fs::write(
        &manifest_path,
        r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: cli-dry-ns, name: a }
spec: { title: On branch }
",
    )
    .unwrap();
    let applied = trogon_atlas_branched(
        &endpoint,
        "cli-dry",
        &["apply", "-f", manifest_path.to_str().unwrap()],
    );
    assert!(applied.status.success(), "{:?}", applied.stderr);

    let merged = trogon_atlas(&endpoint, &["branch", "merge", "cli-dry", "--dry-run"]);
    assert_eq!(merged.status.code(), Some(0), "{:?}", merged.stderr);
    assert!(
        merged.stdout.contains("would merge cleanly"),
        "{:?}",
        merged.stdout
    );

    // Dry run must not persist: baseline still lacks the entity.
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    let missing = client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "cli-dry-ns".into(),
                slug: "a".into(),
                version: 1,
            }),
        })
        .await;
    assert!(missing.is_err(), "dry run must not persist");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_branch_merge_clean_exits_0() {
    let endpoint = start_server().await;

    let created = trogon_atlas(&endpoint, &["branch", "create", "cli-clean"]);
    assert!(created.status.success(), "{:?}", created.stderr);

    let dir = tempfile_dir();
    let manifest_path = dir.join("clean.yaml");
    std::fs::write(
        &manifest_path,
        r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: cli-clean-ns, name: a }
spec: { title: On branch }
",
    )
    .unwrap();
    let applied = trogon_atlas_branched(
        &endpoint,
        "cli-clean",
        &["apply", "-f", manifest_path.to_str().unwrap()],
    );
    assert!(applied.status.success(), "{:?}", applied.stderr);

    let merged = trogon_atlas(&endpoint, &["branch", "merge", "cli-clean"]);
    assert_eq!(merged.status.code(), Some(0), "{:?}", merged.stderr);
    assert!(merged.stdout.contains("merged"), "{:?}", merged.stdout);

    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    let landed = client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "cli-clean-ns".into(),
                slug: "a".into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(landed.entity.is_some());

    // keep_branch defaulted to false: the branch is gone after merge.
    let listed = trogon_atlas(&endpoint, &["branch", "list"]);
    assert!(!listed.stdout.contains("cli-clean"), "{:?}", listed.stdout);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_branch_diff_exits_1_when_conflict_present() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    client
        .put_entity(put_req(event_entity("cli-diff", "a", 1, "Base"), false))
        .await
        .unwrap();

    let created = trogon_atlas(&endpoint, &["branch", "create", "cli-diff-branch"]);
    assert!(created.status.success(), "{:?}", created.stderr);

    let dir = tempfile_dir();
    let manifest_path = dir.join("diff-edit.yaml");
    std::fs::write(
        &manifest_path,
        r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: cli-diff, name: a }
spec: { title: Branch edit }
",
    )
    .unwrap();
    let applied = trogon_atlas_branched(
        &endpoint,
        "cli-diff-branch",
        &["apply", "-f", manifest_path.to_str().unwrap()],
    );
    assert!(applied.status.success(), "{:?}", applied.stderr);

    client
        .put_entity(put_req(
            event_entity("cli-diff", "a", 1, "Baseline moved"),
            false,
        ))
        .await
        .unwrap();

    let diffed = trogon_atlas(&endpoint, &["branch", "diff", "cli-diff-branch"]);
    assert_eq!(diffed.status.code(), Some(1), "{:?}", diffed.stdout);
    assert!(
        diffed.stdout.contains("conflict (edit/edit)"),
        "{:?}",
        diffed.stdout
    );
}

fn tempfile_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("trogon-atlas-branch-cli-test-{}", uuid_like()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{nanos}-{:?}", std::thread::current().id())
}
