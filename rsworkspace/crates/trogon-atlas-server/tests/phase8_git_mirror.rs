#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::{
    git_mirror::{walk_repo, GitMirror, MirrorAuthor, MirrorBatch, MirrorChange},
    service::EventModelServiceImpl,
};

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

fn make_event(ns: &str, slug: &str, version: u64) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, version)),
            title: format!("{ns}/{slug}@{version}"),
            doc: String::new(),
            swimlane: None,
            metadata: Vec::new(),
            supersedes: None,
            schema: None,
        })),
    }
}

fn tempdir(suffix: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    p.push(format!("trogon-atlas-{suffix}-{pid}-{nanos}"));
    std::fs::create_dir_all(&p).unwrap();
    p
}

async fn make_store() -> Arc<dyn trogon_atlas_store::Store> {
    let nats = trogon_atlas_testsupport::shared().await;
    Arc::new(nats.store().await)
}

/// Count commits with git isolated from user/system config so the test
/// cannot hang on gpg prompts or hooks when many test binaries run at once.
fn commit_count(dir: &std::path::Path) -> u32 {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("rev-list")
        .arg("--count")
        .arg("HEAD")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("spawning git rev-list");
    assert!(
        out.status.success(),
        "git rev-list failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("git rev-list output is utf-8");
    stdout
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("unparseable rev-list output: {stdout:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_writes_commit() {
    let dir = tempdir("mirror-put");
    let mirror = GitMirror::open(
        dir.clone(),
        MirrorAuthor {
            name: "trogon-atlas-server".into(),
            email: "trogon-atlas@local".into(),
        },
    )
    .unwrap();
    let svc = EventModelServiceImpl::try_new(make_store().await)
        .unwrap()
        .with_git_mirror(mirror);

    let _ = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("orders", "order-placed", 1)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap();

    let binpb = dir.join("events/orders/order-placed/1.binpb");
    let txt = dir.join("events/orders/order-placed/1.txt");
    assert!(
        binpb.exists(),
        "binpb file should exist at {}",
        binpb.display()
    );
    // The .txt debug dump is gated behind TROGON_ATLAS_GIT_MIRROR_WRITE_TXT
    // (default off), so it must not be written here.
    assert!(!txt.exists(), "txt sidecar should not exist by default");

    assert_eq!(commit_count(&dir), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_one_commit() {
    let dir = tempdir("mirror-batch");
    let mirror = GitMirror::open(
        dir.clone(),
        MirrorAuthor {
            name: "trogon-atlas-server".into(),
            email: "trogon-atlas@local".into(),
        },
    )
    .unwrap();
    let svc = EventModelServiceImpl::try_new(make_store().await)
        .unwrap()
        .with_git_mirror(mirror);

    let ops = vec![
        pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(make_event("orders", "order-placed", 1)),
                create_only: true,
                if_match: String::new(),
                validate_only: false,
                force: false,
            })),
        },
        pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(make_event("orders", "order-shipped", 1)),
                create_only: true,
                if_match: String::new(),
                validate_only: false,
                force: false,
            })),
        },
    ];
    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops,
            validate_only: false,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32
    );

    assert_eq!(commit_count(&dir), 1, "one batch -> one commit");

    let files = walk_repo(&dir).unwrap();
    assert_eq!(files.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_entity_removes_file_and_commits() {
    let dir = tempdir("mirror-del");
    let mirror = GitMirror::open(
        dir.clone(),
        MirrorAuthor {
            name: "trogon-atlas-server".into(),
            email: "trogon-atlas@local".into(),
        },
    )
    .unwrap();
    let svc = EventModelServiceImpl::try_new(make_store().await)
        .unwrap()
        .with_git_mirror(mirror);

    let _ = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("orders", "doomed", 1)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap();

    let _ = svc
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            operation_id: String::new(),
            kind: pb::EntityKind::Event as i32,
            id: Some(id("orders", "doomed", 1)),
            if_match: String::new(),
            mode: pb::delete_entity_request::Mode::Force as i32,
        }))
        .await
        .unwrap();

    let binpb = dir.join("events/orders/doomed/1.binpb");
    assert!(!binpb.exists(), "binpb file should be removed");

    assert_eq!(commit_count(&dir), 2, "put + delete => 2 commits");
}

/// Verify that two concurrent `apply` calls produce exactly two sequential
/// commits with no interleaved index errors.
///
/// Without the `apply_lock` each call would race on `git add` and `git
/// commit`, which can produce `.git/index.lock` conflicts or mix changes
/// from both batches into a single commit.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_apply_produces_sequential_commits() {
    let dir = tempdir("mirror-concurrent");
    let mirror = GitMirror::open(
        dir.clone(),
        MirrorAuthor {
            name: "trogon-atlas-server".into(),
            email: "trogon-atlas@local".into(),
        },
    )
    .unwrap();

    let batch_a = MirrorBatch {
        changes: vec![MirrorChange::Put(make_event("ns", "event-a", 1))],
        author: MirrorAuthor {
            name: "Author A".into(),
            email: "a@test".into(),
        },
        message: "add event-a".into(),
    };
    let batch_b = MirrorBatch {
        changes: vec![MirrorChange::Put(make_event("ns", "event-b", 1))],
        author: MirrorAuthor {
            name: "Author B".into(),
            email: "b@test".into(),
        },
        message: "add event-b".into(),
    };

    let m1 = Arc::clone(&mirror);
    let m2 = Arc::clone(&mirror);
    let (r1, r2) = tokio::join!(
        tokio::spawn(async move { m1.apply(batch_a).await }),
        tokio::spawn(async move { m2.apply(batch_b).await }),
    );
    r1.expect("task a panicked").expect("apply a failed");
    r2.expect("task b panicked").expect("apply b failed");

    let count = commit_count(&dir);
    assert_eq!(
        count, 2,
        "two concurrent applies must produce exactly 2 commits, got {count}"
    );

    let files = walk_repo(&dir).unwrap();
    assert_eq!(
        files.len(),
        2,
        "both entities must be present in the mirror"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn walk_repo_imports_entities() {
    let dir = tempdir("mirror-import");
    let mirror = GitMirror::open(
        dir.clone(),
        MirrorAuthor {
            name: "trogon-atlas-server".into(),
            email: "trogon-atlas@local".into(),
        },
    )
    .unwrap();
    let svc = EventModelServiceImpl::try_new(make_store().await)
        .unwrap()
        .with_git_mirror(mirror);

    for slug in ["a", "b", "c"] {
        let _ = svc
            .put_entity(Request::new(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(make_event("ns", slug, 1)),
                create_only: true,
                if_match: String::new(),
                validate_only: false,
                force: false,
            }))
            .await
            .unwrap();
    }
    let walked = walk_repo(&dir).unwrap();
    assert_eq!(walked.len(), 3);
}

/// A `batch_mutate` with a delete op mixed in must both remove the file
/// from the git mirror (`apply_batch_side_effects`'s `Deleted` arm /
/// `search_apply_delete`) and land in a single commit alongside the put.
#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_mixed_put_and_delete_one_commit() {
    let dir = tempdir("mirror-batch-mixed");
    let mirror = GitMirror::open(
        dir.clone(),
        MirrorAuthor {
            name: "trogon-atlas-server".into(),
            email: "trogon-atlas@local".into(),
        },
    )
    .unwrap();
    let svc = EventModelServiceImpl::try_new(make_store().await)
        .unwrap()
        .with_git_mirror(mirror);

    let _ = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("orders", "mixed-doomed", 1)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap();

    let ops = vec![
        pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(make_event("orders", "mixed-kept", 1)),
                create_only: true,
                if_match: String::new(),
                validate_only: false,
                force: false,
            })),
        },
        pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                operation_id: String::new(),
                kind: pb::EntityKind::Event as i32,
                id: Some(id("orders", "mixed-doomed", 1)),
                if_match: String::new(),
                mode: pb::delete_entity_request::Mode::Force as i32,
            })),
        },
    ];
    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops,
            validate_only: false,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32
    );

    assert!(
        !dir.join("events/orders/mixed-doomed/1.binpb").exists(),
        "deleted entity's file must be removed from the mirror"
    );
    assert!(
        dir.join("events/orders/mixed-kept/1.binpb").exists(),
        "put entity's file must exist in the mirror"
    );

    let files = walk_repo(&dir).unwrap();
    assert_eq!(files.len(), 1, "only the surviving entity remains");
}

/// `author_from_metadata` must cap self-asserted author headers
/// (`sanitize_author_field`'s 256-char `MAX_LEN`) before they reach the git
/// commit author, so an oversized header can't bloat the commit trailer.
///
/// Note: `sanitize_author_field` also strips control characters, but that
/// branch is unreachable through the public gRPC surface: `tonic`'s
/// `MetadataValue<Ascii>` construction rejects `\n`/`\r`/other C0 controls
/// at the `http::HeaderValue` layer before `sanitize_author_field` ever
/// runs, so only the length cap is exercisable end-to-end here.
#[tokio::test(flavor = "multi_thread")]
async fn put_entity_caps_oversized_author_headers() {
    let dir = tempdir("mirror-sanitize");
    let mirror = GitMirror::open(
        dir.clone(),
        MirrorAuthor {
            name: "trogon-atlas-server".into(),
            email: "trogon-atlas@local".into(),
        },
    )
    .unwrap();
    let svc = EventModelServiceImpl::try_new(make_store().await)
        .unwrap()
        .with_git_mirror(mirror);

    let oversized_name: String = "a".repeat(300);

    let mut req = Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("orders", "sanitize-me", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    });
    req.metadata_mut().insert(
        "x-trogon-atlas-author-name",
        tonic::metadata::MetadataValue::try_from(oversized_name.as_str()).unwrap(),
    );
    req.metadata_mut().insert(
        "x-trogon-atlas-author-email",
        tonic::metadata::MetadataValue::try_from("sanitize-me@example.com").unwrap(),
    );

    svc.put_entity(req).await.expect("put_entity must succeed");

    let log = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .arg("log")
        .arg("-1")
        .arg("--format=%an")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git log");
    assert!(log.status.success());
    let out = String::from_utf8(log.stdout).expect("utf-8");
    let name = out.trim();
    assert_eq!(
        name.len(),
        256,
        "author name must be capped at sanitize_author_field's MAX_LEN, got len {}",
        name.len()
    );
}
