#![allow(clippy::unwrap_used, clippy::expect_used)]

// Exit-code plumbing for `trogon-atlas export`, `trogon-atlas branch update`, and
// `trogon-atlas branch resolve`, spawning the actual
// compiled binary (env!("CARGO_BIN_EXE_trogon-atlas")) against an in-process gRPC
// server -- same harness as branch_cli_integration.rs. All tests here MUST
// use #[tokio::test(flavor = "multi_thread", worker_threads = 2)]: a
// blocking Command::output on the current_thread runtime deadlocks the
// in-process server.

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
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

fn inspected_state(endpoint: &str, branch: &str) -> String {
    let diff = trogon_atlas(endpoint, &["branch", "diff", branch]);
    diff.stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("state: "))
        .unwrap_or_else(|| panic!("branch diff must print the entry state: {diff:?}"))
        .to_string()
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

fn tempfile_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("trogon-atlas-export-repair-cli-test-")
        .tempdir()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_export_writes_files_and_exits_0_when_clean() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    client
        .put_entity(put_req(event_entity("cli-export", "a", 1, "A"), true))
        .await
        .unwrap();

    let base_dir = tempfile_dir();
    let out_dir = base_dir.path().join("out");
    let exported = trogon_atlas(
        &endpoint,
        &[
            "export",
            "--namespace",
            "cli-export",
            "-o",
            out_dir.to_str().unwrap(),
        ],
    );
    assert!(exported.status.success(), "{exported:?}");
    assert!(
        exported.stdout.contains("event.yaml"),
        "{:?}",
        exported.stdout
    );
    assert!(
        exported.stdout.contains("(1 entities)"),
        "{:?}",
        exported.stdout
    );

    let written = std::fs::read_to_string(out_dir.join("event.yaml")).unwrap();
    assert!(written.contains("name: a"), "{written}");
    assert!(written.contains("apiVersion:"), "{written}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_export_reports_skipped_entities_and_exits_1() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    client
        .put_entity(put_req(
            event_entity("cli-export-skip", "good", 1, "Good"),
            true,
        ))
        .await
        .unwrap();
    let poisoned = pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: "cli-export-skip".into(),
                slug: "poisoned".into(),
                version: 1,
            }),
            title: "Poisoned".into(),
            metadata: vec![prost_types::Any {
                type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.NeverExisted".into(),
                value: vec![1, 2, 3],
            }],
            ..Default::default()
        })),
    };
    client.put_entity(put_req(poisoned, true)).await.unwrap();

    let base_dir = tempfile_dir();
    let out_dir = base_dir.path().join("out");
    let exported = trogon_atlas(
        &endpoint,
        &[
            "export",
            "--namespace",
            "cli-export-skip",
            "-o",
            out_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(exported.status.code(), Some(1), "{exported:?}");
    assert!(
        exported.stderr.contains("NOT exported"),
        "{:?}",
        exported.stderr
    );
    assert!(
        exported.stderr.contains("poisoned"),
        "{:?}",
        exported.stderr
    );

    // The good entity was still written.
    let written = std::fs::read_to_string(out_dir.join("event.yaml")).unwrap();
    assert!(written.contains("name: good"), "{written}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_export_missing_namespace_errors_nonzero() {
    let endpoint = start_server().await;
    let base_dir = tempfile_dir();
    let out_dir = base_dir.path().join("out");
    let exported = trogon_atlas(
        &endpoint,
        &[
            "export",
            "--namespace",
            "cli-export-does-not-exist",
            "-o",
            out_dir.to_str().unwrap(),
        ],
    );
    assert!(!exported.status.success(), "{exported:?}");
    assert!(
        exported.stderr.contains("no entities"),
        "{:?}",
        exported.stderr
    );
}

const CLI_SLICES_MODEL: &str = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Persona
metadata: { namespace: NS, name: buyer }
spec: { title: Buyer }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Ui
metadata: { namespace: NS, name: checkout }
spec: { title: Checkout }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Command
metadata: { namespace: NS, name: place-order }
spec: { title: Place order }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: CommandSlice
metadata: { namespace: NS, name: place-order-slice }
spec:
  title: Place order
  persona: buyer
  ui: checkout
  command: place-order
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Storyboard
metadata: { namespace: NS, name: buy-flow }
spec:
  title: Buy flow
  entry:
    human:
      persona: buyer
      ui: checkout
  slices:
    - id: place-order-slice
";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_export_layout_slices_writes_nested_self_contained_files() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    let ns = "cli-export-slices";
    let seeded = CLI_SLICES_MODEL.replace("NS", ns);
    let manifests = trogon_atlas_client::manifest::parse_str(&seeded, "t").unwrap();
    let plan = trogon_atlas_client::apply::plan(&mut client, manifests)
        .await
        .unwrap();
    trogon_atlas_client::apply::apply(&mut client, plan, false, None)
        .await
        .unwrap();

    let base_dir = tempfile_dir();
    let out_dir = base_dir.path().join("out");
    let exported = trogon_atlas(
        &endpoint,
        &[
            "export",
            "--namespace",
            ns,
            "-o",
            out_dir.to_str().unwrap(),
            "--layout",
            "slices",
        ],
    );
    assert!(exported.status.success(), "{exported:?}");

    let slice_path = out_dir.join(ns).join("place-order-slice.yaml");
    let slice_file = std::fs::read_to_string(&slice_path)
        .unwrap_or_else(|e| panic!("{}: {e}", slice_path.display()));
    for needle in [
        "kind: CommandSlice",
        "name: place-order-slice",
        "kind: Persona",
        "name: buyer",
        "kind: Ui",
        "name: checkout",
        "kind: Command",
        "name: place-order",
    ] {
        assert!(
            slice_file.contains(needle),
            "{needle:?} missing from {slice_file}"
        );
    }

    let unsliced_file = std::fs::read_to_string(out_dir.join("unsliced.yaml")).unwrap();
    assert!(
        unsliced_file.contains("kind: Storyboard"),
        "{unsliced_file}"
    );
    assert!(unsliced_file.contains("name: buy-flow"), "{unsliced_file}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_export_refuses_nonempty_out_dir_without_force() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    client
        .put_entity(put_req(event_entity("cli-export-force", "a", 1, "A"), true))
        .await
        .unwrap();

    let base_dir = tempfile_dir();
    let out_dir = base_dir.path().join("out");
    let first = trogon_atlas(
        &endpoint,
        &[
            "export",
            "--namespace",
            "cli-export-force",
            "--layout",
            "slices",
            "-o",
            out_dir.to_str().unwrap(),
        ],
    );
    assert!(first.status.success(), "{first:?}");

    let second = trogon_atlas(
        &endpoint,
        &[
            "export",
            "--namespace",
            "cli-export-force",
            "--layout",
            "slices",
            "-o",
            out_dir.to_str().unwrap(),
        ],
    );
    assert!(!second.status.success(), "{second:?}");
    assert!(second.stderr.contains("--force"), "{:?}", second.stderr);

    let forced = trogon_atlas(
        &endpoint,
        &[
            "export",
            "--namespace",
            "cli-export-force",
            "--layout",
            "slices",
            "-o",
            out_dir.to_str().unwrap(),
            "--force",
        ],
    );
    assert!(forced.status.success(), "{forced:?}");

    let single = trogon_atlas(
        &endpoint,
        &[
            "export",
            "--namespace",
            "cli-export-force",
            "-o",
            out_dir.to_str().unwrap(),
        ],
    );
    assert!(single.status.success(), "{single:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_branch_update_rebases_and_reports_remaining_conflicts() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    client
        .put_entity(put_req(event_entity("cli-update", "a", 1, "Base"), false))
        .await
        .unwrap();

    let created = trogon_atlas(&endpoint, &["branch", "create", "cli-update-branch"]);
    assert!(created.status.success(), "{:?}", created.stderr);

    // A branch-scoped apply that only touches a different entity than the
    // one baseline moves: update_branch can rebase it cleanly (no
    // conflicts), so `resp.rebased_count` shows up and exit code is 0.
    let dir = tempfile_dir();
    let dir = dir.path().to_path_buf();
    let manifest_path = dir.join("update.yaml");
    std::fs::write(
        &manifest_path,
        r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: cli-update, name: b }
spec: { title: 'On branch only' }
",
    )
    .unwrap();
    let applied = trogon_atlas_branched(
        &endpoint,
        "cli-update-branch",
        &["apply", "-f", manifest_path.to_str().unwrap()],
    );
    assert!(applied.status.success(), "{:?}", applied.stderr);

    // Baseline moves an unrelated entity, so the branch has nothing to
    // rebase against for its own delta: this exercises the "clean update"
    // arm of `branch update`.
    client
        .put_entity(put_req(
            event_entity("cli-update", "a", 1, "Baseline moved"),
            false,
        ))
        .await
        .unwrap();

    let updated = trogon_atlas(&endpoint, &["branch", "update", "cli-update-branch"]);
    assert_eq!(updated.status.code(), Some(0), "{updated:?}");
    assert!(
        updated.stdout.contains("entries rebased"),
        "{:?}",
        updated.stdout
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_branch_update_reports_conflicts_and_exits_1() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    client
        .put_entity(put_req(event_entity("cli-update-c", "a", 1, "Base"), false))
        .await
        .unwrap();

    let created = trogon_atlas(&endpoint, &["branch", "create", "cli-update-conflict"]);
    assert!(created.status.success(), "{:?}", created.stderr);

    let dir = tempfile_dir();
    let dir = dir.path().to_path_buf();
    let manifest_path = dir.join("update-conflict.yaml");
    std::fs::write(
        &manifest_path,
        r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: cli-update-c, name: a }
spec: { title: Branch edit }
",
    )
    .unwrap();
    let applied = trogon_atlas_branched(
        &endpoint,
        "cli-update-conflict",
        &["apply", "-f", manifest_path.to_str().unwrap()],
    );
    assert!(applied.status.success(), "{:?}", applied.stderr);

    // Baseline independently moves the same entity: update_branch cannot
    // rebase this entry, so it comes back as an unresolved conflict.
    client
        .put_entity(put_req(
            event_entity("cli-update-c", "a", 1, "Baseline moved"),
            false,
        ))
        .await
        .unwrap();

    let updated = trogon_atlas(&endpoint, &["branch", "update", "cli-update-conflict"]);
    assert_eq!(updated.status.code(), Some(1), "{updated:?}");
    assert!(
        updated.stderr.contains("conflicts remain"),
        "{:?}",
        updated.stderr
    );
    assert!(
        updated.stdout.contains("conflict (edit/edit)"),
        "{:?}",
        updated.stdout
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_branch_resolve_take_theirs_then_merge_lands_baseline_value() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    client
        .put_entity(put_req(
            event_entity("cli-resolve-tt", "a", 1, "Base"),
            false,
        ))
        .await
        .unwrap();

    let created = trogon_atlas(&endpoint, &["branch", "create", "cli-resolve-tt-branch"]);
    assert!(created.status.success(), "{:?}", created.stderr);

    let dir = tempfile_dir();
    let dir = dir.path().to_path_buf();
    let manifest_path = dir.join("resolve-tt.yaml");
    std::fs::write(
        &manifest_path,
        r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: cli-resolve-tt, name: a }
spec: { title: Branch edit }
",
    )
    .unwrap();
    let applied = trogon_atlas_branched(
        &endpoint,
        "cli-resolve-tt-branch",
        &["apply", "-f", manifest_path.to_str().unwrap()],
    );
    assert!(applied.status.success(), "{:?}", applied.stderr);

    client
        .put_entity(put_req(
            event_entity("cli-resolve-tt", "a", 1, "Baseline moved"),
            false,
        ))
        .await
        .unwrap();

    let state = inspected_state(&endpoint, "cli-resolve-tt-branch");
    let resolved = trogon_atlas(
        &endpoint,
        &[
            "branch",
            "resolve",
            "cli-resolve-tt-branch",
            "--kind",
            "event",
            "--namespace",
            "cli-resolve-tt",
            "--slug",
            "a",
            "--version",
            "1",
            "--take-theirs",
            "--expect-state",
            state.as_str(),
        ],
    );
    assert!(resolved.status.success(), "{resolved:?}");
    assert!(
        resolved.stdout.contains("resolved"),
        "{:?}",
        resolved.stdout
    );

    let merged = trogon_atlas(&endpoint, &["branch", "merge", "cli-resolve-tt-branch"]);
    assert_eq!(merged.status.code(), Some(0), "{merged:?}");

    let landed = client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "cli-resolve-tt".into(),
                slug: "a".into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    match landed.entity.unwrap().kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Baseline moved"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_branch_resolve_keep_ours_then_merge_lands_branch_value() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    client
        .put_entity(put_req(
            event_entity("cli-resolve-ko", "a", 1, "Base"),
            false,
        ))
        .await
        .unwrap();

    let created = trogon_atlas(&endpoint, &["branch", "create", "cli-resolve-ko-branch"]);
    assert!(created.status.success(), "{:?}", created.stderr);

    let dir = tempfile_dir();
    let dir = dir.path().to_path_buf();
    let manifest_path = dir.join("resolve-ko.yaml");
    std::fs::write(
        &manifest_path,
        r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: cli-resolve-ko, name: a }
spec: { title: Branch edit }
",
    )
    .unwrap();
    let applied = trogon_atlas_branched(
        &endpoint,
        "cli-resolve-ko-branch",
        &["apply", "-f", manifest_path.to_str().unwrap()],
    );
    assert!(applied.status.success(), "{:?}", applied.stderr);

    client
        .put_entity(put_req(
            event_entity("cli-resolve-ko", "a", 1, "Baseline moved"),
            false,
        ))
        .await
        .unwrap();

    let state = inspected_state(&endpoint, "cli-resolve-ko-branch");
    let resolved = trogon_atlas(
        &endpoint,
        &[
            "branch",
            "resolve",
            "cli-resolve-ko-branch",
            "--kind",
            "event",
            "--namespace",
            "cli-resolve-ko",
            "--slug",
            "a",
            "--version",
            "1",
            "--keep-ours",
            "--expect-state",
            state.as_str(),
        ],
    );
    assert!(resolved.status.success(), "{resolved:?}");

    let merged = trogon_atlas(&endpoint, &["branch", "merge", "cli-resolve-ko-branch"]);
    assert_eq!(merged.status.code(), Some(0), "{merged:?}");

    let landed = client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "cli-resolve-ko".into(),
                slug: "a".into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    match landed.entity.unwrap().kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Branch edit"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_branch_resolve_requires_exactly_one_flag() {
    let endpoint = start_server().await;
    let created = trogon_atlas(&endpoint, &["branch", "create", "cli-resolve-badflags"]);
    assert!(created.status.success(), "{:?}", created.stderr);

    // No resolution flag at all: trogon-atlas's own validation must reject this
    // before making any RPC.
    let resolved = trogon_atlas(
        &endpoint,
        &[
            "branch",
            "resolve",
            "cli-resolve-badflags",
            "--kind",
            "event",
            "--namespace",
            "cli-resolve-badflags-ns",
            "--slug",
            "a",
            "--expect-state",
            "base=1,ours=2,theirs=3",
        ],
    );
    assert!(!resolved.status.success(), "{resolved:?}");
    assert!(
        resolved
            .stderr
            .contains("specify exactly one of --take-theirs, --keep-ours or --take-merged"),
        "{:?}",
        resolved.stderr
    );

    // Two of them: clap's own conflict rules must reject the pair, so a
    // third resolution never widened the way past the exclusivity check.
    for pair in [
        ["--take-theirs", "--keep-ours"],
        ["--take-theirs", "--take-merged"],
        ["--keep-ours", "--take-merged"],
    ] {
        let resolved = trogon_atlas(
            &endpoint,
            &[
                "branch",
                "resolve",
                "cli-resolve-badflags",
                "--kind",
                "event",
                "--namespace",
                "cli-resolve-badflags-ns",
                "--slug",
                "a",
                "--expect-state",
                "base=1,ours=2,theirs=3",
                pair[0],
                pair[1],
            ],
        );
        assert!(
            !resolved.status.success(),
            "{pair:?} must be rejected: {resolved:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trogon_atlas_branch_resolve_exits_stale_when_the_entry_moved_after_diff() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    client
        .put_entity(put_req(
            event_entity("cli-resolve-stale", "a", 1, "Base"),
            false,
        ))
        .await
        .unwrap();
    let created = trogon_atlas(&endpoint, &["branch", "create", "cli-resolve-stale-branch"]);
    assert!(created.status.success(), "{:?}", created.stderr);

    let dir = tempfile_dir();
    let dir = dir.path().to_path_buf();
    let manifest_path = dir.join("resolve-stale.yaml");
    std::fs::write(
        &manifest_path,
        r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: cli-resolve-stale, name: a }
spec: { title: Branch edit }
",
    )
    .unwrap();
    let applied = trogon_atlas_branched(
        &endpoint,
        "cli-resolve-stale-branch",
        &["apply", "-f", manifest_path.to_str().unwrap()],
    );
    assert!(applied.status.success(), "{:?}", applied.stderr);
    client
        .put_entity(put_req(
            event_entity("cli-resolve-stale", "a", 1, "Baseline moved"),
            false,
        ))
        .await
        .unwrap();

    let state = inspected_state(&endpoint, "cli-resolve-stale-branch");
    client
        .put_entity(put_req(
            event_entity("cli-resolve-stale", "a", 1, "Baseline moved again"),
            false,
        ))
        .await
        .unwrap();

    let resolved = trogon_atlas(
        &endpoint,
        &[
            "branch",
            "resolve",
            "cli-resolve-stale-branch",
            "--kind",
            "event",
            "--namespace",
            "cli-resolve-stale",
            "--slug",
            "a",
            "--keep-ours",
            "--expect-state",
            state.as_str(),
        ],
    );
    assert_eq!(resolved.status.code(), Some(3), "{resolved:?}");
    assert!(
        resolved.stderr.contains("stale, replan"),
        "{:?}",
        resolved.stderr
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn architecture_resources_apply_get_and_export_through_cli() {
    let endpoint = start_server().await;
    let directory = tempfile_dir();
    let anchors = directory.path().join("anchors.yaml");
    std::fs::write(
        &anchors,
        r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ReadModel
metadata: {namespace: cli-architecture, name: order-feed, version: 2}
spec: {title: Order feed}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Processor
metadata: {namespace: cli-architecture, name: order-intake}
spec: {title: Order intake}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: AutomationSlice
metadata: {namespace: cli-architecture, name: observe-orders, version: 3}
spec:
  title: Observe orders
  sourceReadModels: [order-feed@2]
  processor: order-intake
",
    )
    .unwrap();
    let applied = trogon_atlas(&endpoint, &["apply", "-f", anchors.to_str().unwrap()]);
    assert!(applied.status.success(), "{applied:?}");
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    let owner = client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::AutomationSlice as i32,
            id: Some(pb::Id {
                namespace: "cli-architecture".into(),
                slug: "observe-orders".into(),
                version: 3,
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let Some(pb::entity::Kind::AutomationSlice(slice)) = &owner.kind else {
        panic!("expected automation")
    };
    let observation = serde_json::json!({
        "owner": {"kind": "ENTITY_KIND_AUTOMATION_SLICE", "id": {"namespace": "cli-architecture", "slug": "observe-orders", "version": "3"}},
        "ownerUid": owner.system.as_ref().unwrap().uid,
        "id": {"value": slice.source_read_models[0].connection_id.as_ref().unwrap().value}
    });
    let resources = directory.path().join("resources.yaml");
    let channel = serde_json::json!({
        "apiVersion": "eventmodel.atlas.trogonstack.com/v1alpha1", "kind": "Channel",
        "metadata": {"namespace": "cli-architecture", "name": "delivery"},
        "spec": {"title": "Order delivery", "bindings": [{"observation": observation}]}
    });
    let store = serde_json::json!({
        "apiVersion": "eventmodel.atlas.trogonstack.com/v1alpha1", "kind": "DataStore",
        "metadata": {"namespace": "cli-architecture", "name": "cache"},
        "spec": {"title": "Order cache", "storeClass": {"known": "KNOWN_CACHE"}, "authoritative": false,
            "attachments": [{"readModel": "order-feed@2"}]}
    });
    std::fs::write(&resources, format!("{channel}\n---\n{store}\n")).unwrap();
    let applied = trogon_atlas(&endpoint, &["apply", "-f", resources.to_str().unwrap()]);
    assert!(applied.status.success(), "{applied:?}");
    for (kind, slug) in [("channel", "delivery"), ("data_store", "cache")] {
        let key = format!("{kind}/cli-architecture/{slug}@1");
        let got = trogon_atlas(&endpoint, &["get", &key]);
        assert!(got.status.success(), "{got:?}");
        let parsed = trogon_atlas_client::manifest::parse_str(&got.stdout, "get.yaml").unwrap();
        match parsed[0].entity.kind.as_ref().unwrap() {
            pb::entity::Kind::Channel(resource) => {
                let binding = &resource.bindings[0];
                assert!(binding.connection_id.is_some());
                let selector = binding.observation.as_ref().unwrap();
                assert_eq!(selector.owner_uid, owner.system.as_ref().unwrap().uid);
                assert_eq!(selector.id, slice.source_read_models[0].connection_id);
                assert_eq!(selector.owner.as_ref().unwrap().id, slice.id);
            }
            pb::entity::Kind::DataStore(resource) => {
                assert_eq!(resource.authoritative, Some(false));
                assert!(resource.attachments[0].connection_id.is_some());
                assert_eq!(
                    resource.attachments[0]
                        .read_model
                        .as_ref()
                        .unwrap()
                        .id
                        .as_ref()
                        .unwrap()
                        .version,
                    2
                );
            }
            _ => panic!("expected architecture resource"),
        }
    }
    let output = directory.path().join("exported");
    let exported = trogon_atlas(
        &endpoint,
        &[
            "export",
            "--namespace",
            "cli-architecture",
            "-o",
            output.to_str().unwrap(),
        ],
    );
    assert!(exported.status.success(), "{exported:?}");
    assert!(output.join("channel.yaml").exists());
    assert!(output.join("data-store.yaml").exists());
    let reapplied = trogon_atlas(&endpoint, &["apply", "-f", output.to_str().unwrap()]);
    assert!(reapplied.status.success(), "{reapplied:?}");
    assert!(!reapplied.stdout.contains("created"), "{reapplied:?}");
    assert!(!reapplied.stdout.contains("updated"), "{reapplied:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn specification_cli_roundtrip_preserves_stored_members_associations_and_artifact_bytes() {
    let endpoint = start_server().await;
    let ns = "cli-specification";
    let directory = tempfile_dir();
    let anchors = directory.path().join("anchors.yaml");
    std::fs::write(
        &anchors,
        format!(
            r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ReadModel
metadata: {{namespace: {ns}, name: receipt, version: 2}}
spec: {{title: Receipt}}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Component
metadata: {{namespace: {ns}, name: gateway}}
spec: {{title: Gateway}}
"
        ),
    )
    .unwrap();
    let applied = trogon_atlas(&endpoint, &["apply", "-f", anchors.to_str().unwrap()]);
    assert!(applied.status.success(), "{applied:?}");
    let artifact_id = "11111111-1111-4111-8111-111111111111";
    let schema_id = "22222222-2222-4222-8222-222222222222";
    let message_id = "33333333-3333-4333-8333-333333333333";
    let operation_id = "44444444-4444-4444-8444-444444444444";
    let content = "{\"type\":\"object\"}\r\n";
    let digest = "6f8f8b83ea6711930c12c3c32001521787644432b59f31251b32aefe9f0d494d";
    let spec = serde_json::json!({
        "title": "Order contract",
        "artifacts": [{"memberId": {"value": artifact_id}, "name": {"value": "Order source"},
            "path": {"value": "contracts/order.json"}, "format": {"known": "KNOWN_JSON_SCHEMA"},
            "formatVersion": {"value": "2020-12"}, "mediaType": {"value": "application/schema+json"},
            "content": content, "digest": {"sha256": digest}}],
        "schemas": [{"memberId": {"value": schema_id}, "name": {"value": "Order shape"},
            "definition": {"artifact": {"artifactId": {"value": artifact_id}, "expectedDigest": {"sha256": digest}},
                "selector": {"syntax": "json-pointer", "value": "/$defs/Order"}}}],
        "messages": [{"memberId": {"value": message_id}, "name": {"value": "Order message"},
            "schemas": [{"role": "ROLE_PAYLOAD", "localSchema": {"value": schema_id}}],
            "mapping": {"readModel": {"id": {"namespace": ns, "slug": "receipt", "version": "2"}}}}],
        "operations": [{"memberId": {"value": operation_id}, "name": {"value": "Submit"},
            "binding": {"http": {"method": "POST", "path": "/orders", "responseStatusCodes": [201]}},
            "messages": [
                {"role": "ROLE_REQUEST", "messageId": {"value": message_id}},
                {"role": "ROLE_RESPONSE", "messageId": {"value": message_id}}]}],
        "exposures": [{"provider": {"component": {"id": {"namespace": ns, "slug": "gateway", "version": "1"}}},
            "operationId": {"value": operation_id}}],
        "usages": [{"caller": {"component": {"id": {"namespace": ns, "slug": "gateway", "version": "1"}}},
            "operationId": {"value": operation_id}}]
    });

    let manifest = serde_json::json!({"apiVersion": "eventmodel.atlas.trogonstack.com/v1alpha1",
        "kind": "Specification", "metadata": {"namespace": ns, "name": "orders"}, "spec": spec});
    let source = directory.path().join("contract.yaml");
    std::fs::write(&source, manifest.to_string()).unwrap();
    let applied = trogon_atlas(&endpoint, &["apply", "-f", source.to_str().unwrap()]);
    assert!(applied.status.success(), "{applied:?}");
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    let request = pb::GetEntityRequest {
        kind: pb::EntityKind::Specification as i32,
        id: Some(pb::Id {
            namespace: ns.into(),
            slug: "orders".into(),
            version: 1,
        }),
    };
    let stored = client
        .get_entity(request.clone())
        .await
        .unwrap()
        .into_inner();
    let entity = stored.entity.as_ref().unwrap();
    let Some(pb::entity::Kind::Specification(contract)) = entity.kind.as_ref() else {
        panic!("expected Specification")
    };
    assert_eq!(contract.artifacts[0].content.as_bytes(), content.as_bytes());
    assert_eq!(
        contract.artifacts[0].digest.as_ref().unwrap().sha256,
        digest
    );
    assert_eq!(
        contract.schemas[0].member_id.as_ref().unwrap().value,
        schema_id
    );
    let occurrence_ids = [
        contract.schemas[0]
            .definition
            .as_ref()
            .unwrap()
            .connection_id
            .as_ref()
            .unwrap(),
        contract.messages[0].schemas[0]
            .connection_id
            .as_ref()
            .unwrap(),
        contract.messages[0]
            .mapping
            .as_ref()
            .unwrap()
            .connection_id
            .as_ref()
            .unwrap(),
        contract.operations[0].messages[0]
            .connection_id
            .as_ref()
            .unwrap(),
        contract.operations[0].messages[1]
            .connection_id
            .as_ref()
            .unwrap(),
        contract.exposures[0].connection_id.as_ref().unwrap(),
        contract.usages[0].connection_id.as_ref().unwrap(),
    ];
    assert_eq!(
        occurrence_ids
            .iter()
            .map(|id| &id.value)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        occurrence_ids.len()
    );
    for spelling in [
        "specification",
        "Specification",
        "ENTITY_KIND_SPECIFICATION",
    ] {
        let got = trogon_atlas(&endpoint, &["get", &format!("{spelling}/{ns}/orders@1")]);
        assert!(got.status.success(), "{got:?}");
        let loaded = trogon_atlas_client::manifest::parse_str(&got.stdout, "get.yaml").unwrap();
        assert_eq!(loaded[0].entity.kind, entity.kind);
    }
    let output = directory.path().join("exported");
    let exported = trogon_atlas(
        &endpoint,
        &["export", "--namespace", ns, "-o", output.to_str().unwrap()],
    );
    assert!(exported.status.success(), "{exported:?}");
    let written = std::fs::read_to_string(output.join("specification.yaml")).unwrap();
    let loaded = trogon_atlas_client::manifest::parse_str(&written, "export.yaml").unwrap();
    assert_eq!(loaded[0].entity.kind, entity.kind);
    let reapplied = trogon_atlas(&endpoint, &["apply", "-f", output.to_str().unwrap()]);
    assert!(reapplied.status.success(), "{reapplied:?}");
    let after = client.get_entity(request).await.unwrap().into_inner();
    assert_eq!(stored.entity, after.entity);
    assert_eq!(stored.etag, after.etag);
}
