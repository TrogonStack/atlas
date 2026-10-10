#![allow(clippy::unwrap_used, clippy::expect_used)]

// End-to-end apply against a real in-process gRPC server backed by an
// ephemeral JetStream store (trogon-atlas-testsupport, same pattern as
// trogon-atlas-mcp/tests). Requires Docker at test time, like every other
// integration test in this workspace.

use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_client::{
    apply, branch, client, manifest, operation,
    precondition::{BranchEntryState, Revision, StalePlan, StaleResolution},
};
use trogon_atlas_core::transcode::TranscodePool;
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

const SHOP_MANIFESTS: &str = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Persona
metadata: { namespace: trogon-atlas-shop, name: buyer }
spec: { title: Buyer }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: trogon-atlas-shop, name: order.placed }
spec: { title: Order placed }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Command
metadata: { namespace: trogon-atlas-shop, name: place-order }
spec: { title: Place order }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: CommandSlice
metadata: { namespace: trogon-atlas-shop, name: place-order-slice }
spec:
  title: Place order
  command: place-order
  emittedEvents: [order.placed]
  scenarios:
    - id: happy
      title: Order accepted
      given: []
      when: place-order
      emit:
        events: [order.placed]
";

async fn apply_str(client: &mut client::Client, yaml: &str, dry_run: bool) -> apply::ApplyOutcome {
    let manifests = manifest::parse_str(yaml, "test.yaml").unwrap();
    let plan = apply::plan(client, manifests).await.unwrap();
    apply::apply(client, plan, dry_run, None).await.unwrap()
}

#[tokio::test]
async fn apply_create_then_unchanged_then_configure() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    // First apply: everything is created.
    let out = apply_str(&mut client, SHOP_MANIFESTS, false).await;
    assert_eq!(out.lines.len(), 4);
    for line in &out.lines {
        assert!(line.ends_with(" created"), "{line}");
    }

    // The slice round-trips: shorthand refs became fully-pinned ids.
    let stored = client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::CommandSlice as i32,
            id: Some(pb::Id {
                namespace: "trogon-atlas-shop".into(),
                slug: "place-order-slice".into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    let Some(pb::entity::Kind::CommandSlice(slice)) = stored.entity.unwrap().kind else {
        panic!("expected commandSlice");
    };
    let cmd = slice.command.unwrap().command.unwrap().id.unwrap();
    assert_eq!(cmd.namespace, "trogon-atlas-shop");
    assert_eq!(cmd.slug, "place-order");
    assert_eq!(cmd.version, 1);
    assert_eq!(slice.scenarios[0].id, "happy");

    // Second apply of the identical manifests: pure no-op.
    let out = apply_str(&mut client, SHOP_MANIFESTS, false).await;
    for line in &out.lines {
        assert!(line.ends_with(" unchanged"), "{line}");
    }

    // Edit one title: exactly that entity is configured.
    let edited = SHOP_MANIFESTS.replace("title: Order placed", "title: Order was placed");
    let out = apply_str(&mut client, &edited, false).await;
    let configured: Vec<&String> = out
        .lines
        .iter()
        .filter(|l| l.ends_with(" configured"))
        .collect();
    assert_eq!(configured.len(), 1, "{:?}", out.lines);
    assert!(configured[0].contains("order.placed"), "{configured:?}");
}

#[tokio::test]
async fn dry_run_validates_without_persisting() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    let yaml = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: trogon-atlas-dry, name: thing.happened }
spec: { title: Thing happened }
";
    let out = apply_str(&mut client, yaml, true).await;
    assert!(
        out.lines[0].ends_with("created (server dry run)"),
        "{:?}",
        out.lines
    );

    let missing = client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "trogon-atlas-dry".into(),
                slug: "thing.happened".into(),
                version: 1,
            }),
        })
        .await;
    assert!(missing.is_err(), "dry run must not persist");
}

#[tokio::test]
async fn diff_shows_changes_and_duplicate_manifests_are_deduped_or_rejected() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    let yaml = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: trogon-atlas-diff, name: a }
spec: { title: A }
";
    let manifests = manifest::parse_str(yaml, "t").unwrap();
    let plan = apply::plan(&mut client, manifests).await.unwrap();
    let (diff, changed) = apply::render_diff(&plan, &TranscodePool::builtin().unwrap()).unwrap();
    assert!(changed);
    assert!(diff.contains('+'), "{diff}");

    // Identical duplicate documents (e.g. the same entity bundled into two
    // self-contained slice files by a slices-layout export) must be
    // accepted as one no-op duplicate, not rejected as a conflict.
    let identical_dup = format!("{yaml}---\n{yaml}");
    let manifests = manifest::parse_str(&identical_dup, "t").unwrap();
    let plan = apply::plan(&mut client, manifests).await.unwrap();
    assert_eq!(
        plan.len(),
        1,
        "identical duplicates must collapse to one planned change"
    );

    // Two documents writing the same identity with different content
    // remain a genuine authoring conflict.
    let conflicting = format!(
        "{yaml}---\napiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Event\nmetadata: {{ namespace: trogon-atlas-diff, name: a }}\nspec: {{ title: Conflicting }}\n"
    );
    let manifests = manifest::parse_str(&conflicting, "t").unwrap();
    let err = apply::plan(&mut client, manifests).await.unwrap_err();
    assert!(format!("{err:#}").contains("duplicate manifest"), "{err:#}");
}

#[tokio::test]
async fn export_round_trips_applied_manifests() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    // Seed the ephemeral store with the shop model (uses its own namespace
    // to stay independent from the other tests sharing the container).
    let seeded = SHOP_MANIFESTS.replace("trogon-atlas-shop", "trogon-atlas-export");
    apply_str(&mut client, &seeded, false).await;

    // Export it back and re-parse: the exported YAML must be a complete,
    // drift-free representation (every entity classifies as Unchanged).
    let outcome = trogon_atlas_client::export::export_namespace(&mut client, "trogon-atlas-export")
        .await
        .unwrap();
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    let mut manifests = Vec::new();
    for (name, content) in &outcome.files {
        manifests.extend(manifest::parse_str(content, name).unwrap());
    }
    assert_eq!(manifests.len(), 4, "persona, event, command, commandSlice");

    let plan = apply::plan(&mut client, manifests).await.unwrap();
    for change in &plan {
        assert_eq!(
            change.action,
            apply::Action::Unchanged,
            "exported manifest drifted for {}",
            change.manifest.source
        );
    }
}

const SCENARIO_METADATA_MANIFESTS: &str = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Persona
metadata: { namespace: trogon-atlas-scenario-meta, name: buyer }
spec: { title: Buyer }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: trogon-atlas-scenario-meta, name: order.placed }
spec: { title: Order placed }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Command
metadata: { namespace: trogon-atlas-scenario-meta, name: place-order }
spec: { title: Place order }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: CommandSlice
metadata: { namespace: trogon-atlas-scenario-meta, name: place-order-slice }
spec:
  title: Place order
  command: place-order
  emittedEvents: [order.placed]
  scenarios:
    - id: happy
      title: Order accepted
      given: []
      when: place-order
      emit:
        events: [order.placed]
      metadata:
        - '@type': type.googleapis.com/trogonatlas.annotation.v1alpha1.NoteAnnotation
          text: Confirmed with product.
";

#[tokio::test]
async fn scenario_metadata_round_trips_through_apply_and_export() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    apply_str(&mut client, SCENARIO_METADATA_MANIFESTS, false).await;

    let stored = client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::CommandSlice as i32,
            id: Some(pb::Id {
                namespace: "trogon-atlas-scenario-meta".into(),
                slug: "place-order-slice".into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    let Some(pb::entity::Kind::CommandSlice(slice)) = stored.entity.unwrap().kind else {
        panic!("expected commandSlice");
    };
    assert_eq!(slice.scenarios[0].metadata.len(), 1);
    let note = <pb::NoteAnnotation as prost::Message>::decode(
        slice.scenarios[0].metadata[0].value.as_slice(),
    )
    .unwrap();
    assert_eq!(note.text, "Confirmed with product.");

    // Export round-trips: the annotation survives the YAML pipeline unchanged,
    // so re-applying the exported manifests reports no drift.
    let outcome =
        trogon_atlas_client::export::export_namespace(&mut client, "trogon-atlas-scenario-meta")
            .await
            .unwrap();
    let mut manifests = Vec::new();
    for (name, content) in &outcome.files {
        manifests.extend(manifest::parse_str(content, name).unwrap());
    }
    let plan = apply::plan(&mut client, manifests).await.unwrap();
    for change in &plan {
        assert_eq!(
            change.action,
            apply::Action::Unchanged,
            "exported manifest drifted for {}",
            change.manifest.source
        );
    }
}

// Phase 1: Isolation -- a client connected with `ConnectOptions::branch` set
// scopes every RPC (apply, diff, get_entity) to that branch via the
// `x-trogon-atlas-branch` metadata header injected by `BearerAuth`. Baseline
// stays untouched by branch-scoped applies, matching the store/server-level
// contract exercised in trogon-atlas-store/tests/branches.rs and
// trogon-atlas-server/tests/branching.rs.
#[tokio::test]
async fn apply_scoped_to_branch_is_invisible_on_baseline() {
    let endpoint = start_server().await;
    let mut baseline_client = client::connect(&endpoint, None, 30).await.unwrap();

    branch::create_branch(
        &mut baseline_client,
        "trogon-atlas-branch",
        "client branch test",
    )
    .await
    .unwrap();

    let mut branch_client = client::connect_with_options(
        &endpoint,
        None,
        30,
        client::ConnectOptions {
            branch: Some("trogon-atlas-branch".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let yaml = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: trogon-atlas-branch-ns, name: on.branch }
spec: { title: On branch }
";
    let out = apply_str(&mut branch_client, yaml, false).await;
    assert!(out.lines[0].ends_with(" created"), "{:?}", out.lines);

    // Invisible through the baseline (non-branch) client.
    let missing = baseline_client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "trogon-atlas-branch-ns".into(),
                slug: "on.branch".into(),
                version: 1,
            }),
        })
        .await;
    assert!(
        missing.is_err(),
        "branch-scoped apply must not touch baseline"
    );

    // Visible through the branch-scoped client.
    let found = branch_client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "trogon-atlas-branch-ns".into(),
                slug: "on.branch".into(),
                version: 1,
            }),
        })
        .await
        .unwrap();
    assert!(found.into_inner().entity.is_some());

    // diff against the branch-scoped client also sees the branch's own state
    // (a second identical apply/diff must show no drift).
    let manifests = manifest::parse_str(yaml, "t").unwrap();
    let plan = apply::plan(&mut branch_client, manifests).await.unwrap();
    let (diff, changed) = apply::render_diff(&plan, &TranscodePool::builtin().unwrap()).unwrap();
    assert!(!changed, "{diff}");
}

#[tokio::test]
async fn branch_lifecycle_via_client_helpers() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    let created = branch::create_branch(&mut client, "trogon-atlas-lifecycle", "doc")
        .await
        .unwrap();
    assert_eq!(created.name, "trogon-atlas-lifecycle");

    let listed = branch::list_branches(&mut client).await.unwrap();
    assert!(listed.iter().any(|b| b.name == "trogon-atlas-lifecycle"));

    branch::delete_branch(&mut client, "trogon-atlas-lifecycle")
        .await
        .unwrap();

    let listed_after = branch::list_branches(&mut client).await.unwrap();
    assert!(!listed_after
        .iter()
        .any(|b| b.name == "trogon-atlas-lifecycle"));
}

// Phase 2: Review and merge, driven entirely through the
// `trogon_atlas_client::branch` wrappers (diff_branch/merge_branch/
// update_branch/resolve_branch_entry/render_diff), not raw gRPC. Mirrors
// the server-level conflict-crafting technique in
// trogon-atlas-server/tests/branching.rs
// (`merge_branch_conflict_edit_edit_blocks_everything`): a branch edits an
// entity while baseline independently moves the same entity, producing an
// edit/edit conflict that blocks the merge until resolved.

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

async fn branch_client(endpoint: &str, name: &str) -> client::Client {
    client::connect_with_options(
        endpoint,
        None,
        30,
        client::ConnectOptions {
            branch: Some(name.to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn branch_conflict_lifecycle_diff_resolve_update_merge() {
    let endpoint = start_server().await;
    let mut baseline = client::connect(&endpoint, None, 30).await.unwrap();

    baseline
        .put_entity(put_req(event_entity("p2c", "a", 1, "Base"), false))
        .await
        .unwrap();

    branch::create_branch(&mut baseline, "client-p2-conflict", "doc")
        .await
        .unwrap();

    let mut on_branch = branch_client(&endpoint, "client-p2-conflict").await;
    on_branch
        .put_entity(put_req(event_entity("p2c", "a", 1, "Branch edit"), false))
        .await
        .unwrap();

    // Baseline independently moves the same entity: edit/edit conflict.
    baseline
        .put_entity(put_req(
            event_entity("p2c", "a", 1, "Baseline moved"),
            false,
        ))
        .await
        .unwrap();

    // diff_branch shows the conflict, with field paths pointing at the
    // differing field.
    let entries = branch::diff_branch(&mut baseline, "client-p2-conflict")
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].status,
        pb::branch_diff_entry::Status::ConflictEditEdit as i32
    );
    assert_ne!(
        entries[0].conflict_field_paths,
        [] as [std::string::String; 0]
    );
    assert!(
        entries[0]
            .conflict_field_paths
            .iter()
            .any(|p| p.contains("title")),
        "{:?}",
        entries[0].conflict_field_paths
    );

    // render_diff produces readable output naming the conflict and the
    // conflicting fields, without erroring.
    let rendered = branch::render_diff(&entries).unwrap();
    assert!(rendered.contains("conflict (edit/edit)"), "{rendered}");
    assert!(rendered.contains("conflicting fields:"), "{rendered}");
    let inspected = BranchEntryState::from_wire(entries[0].state.as_ref().unwrap()).unwrap();
    assert!(
        rendered.contains(&format!("state: {inspected}")),
        "{rendered}"
    );

    // update_branch cannot rebase a conflicting entry: it comes back in
    // resp.conflicts, unresolved.
    let updated = branch::update_branch(&mut baseline, "client-p2-conflict")
        .await
        .unwrap();
    assert_eq!(updated.rebased_count, 0);
    assert_eq!(updated.conflicts.len(), 1);

    // A merge attempt while still conflicting reports Conflicts and lands
    // nothing.
    let blocked = branch::merge_branch(
        &mut baseline,
        "client-p2-conflict",
        branch::MergeOptions {
            keep_branch: true,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        blocked.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    let still_baseline = baseline
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "p2c".into(),
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

    // Resolve keep_ours: the branch's own edit wins, rebased onto the new
    // baseline so the conflict clears.
    branch::resolve_branch_entry(
        &mut baseline,
        "client-p2-conflict",
        pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "p2c".into(),
                slug: "a".into(),
                version: 1,
            }),
        },
        branch::Resolution::KeepOurs,
        &inspected,
        None,
    )
    .await
    .unwrap();

    // Merge now lands cleanly.
    let applied = branch::merge_branch(
        &mut baseline,
        "client-p2-conflict",
        branch::MergeOptions::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        applied.status,
        pb::merge_branch_response::Status::Applied as i32
    );
    assert_eq!(applied.applied_count, 1);

    let landed = baseline
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "p2c".into(),
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

    // keep_branch defaulted to false: the branch is gone.
    let listed = branch::list_branches(&mut baseline).await.unwrap();
    assert!(!listed.iter().any(|b| b.name == "client-p2-conflict"));
}

#[tokio::test]
async fn branch_dry_run_merge_does_not_persist() {
    let endpoint = start_server().await;
    let mut baseline = client::connect(&endpoint, None, 30).await.unwrap();

    branch::create_branch(&mut baseline, "client-p2-dry", "doc")
        .await
        .unwrap();
    let mut on_branch = branch_client(&endpoint, "client-p2-dry").await;
    on_branch
        .put_entity(put_req(event_entity("p2d", "a", 1, "On branch"), false))
        .await
        .unwrap();

    let resp = branch::merge_branch(
        &mut baseline,
        "client-p2-dry",
        branch::MergeOptions {
            dry_run: true,
            keep_branch: true,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Validated as i32
    );

    let missing = baseline
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "p2d".into(),
                slug: "a".into(),
                version: 1,
            }),
        })
        .await;
    assert!(missing.is_err(), "dry run must not persist");

    // Branch is untouched by a dry run: still listed with its delta.
    let listed = branch::list_branches(&mut baseline).await.unwrap();
    assert!(listed.iter().any(|b| b.name == "client-p2-dry"));
}

// Another agent writes between this agent's plan and its apply. The apply
// must refuse the plan, leave the other agent's write in place, and say which
// entity moved so the agent can plan again from fresh state.

fn stale_event_yaml(ns: &str, title: &str) -> String {
    format!(
        "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\n\
         kind: Event\n\
         metadata: {{ namespace: {ns}, name: order.placed }}\n\
         spec: {{ title: {title} }}\n"
    )
}

fn stale_event_id(ns: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: "order.placed".into(),
        version: 1,
    }
}

async fn stored_title(client: &mut client::Client, ns: &str) -> Option<String> {
    match client
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(stale_event_id(ns)),
        })
        .await
    {
        Ok(resp) => match resp.into_inner().entity.and_then(|e| e.kind) {
            Some(pb::entity::Kind::Event(e)) => Some(e.title),
            other => panic!("unexpected entity: {other:?}"),
        },
        Err(status) if status.code() == tonic::Code::NotFound => None,
        Err(status) => panic!("get_entity failed: {status}"),
    }
}

async fn plan_str(client: &mut client::Client, yaml: &str) -> Vec<apply::PlannedChange> {
    let manifests = manifest::parse_str(yaml, "test.yaml").unwrap();
    apply::plan(client, manifests).await.unwrap()
}

fn assert_stale_error(err: &anyhow::Error) -> &StalePlan {
    let message = format!("{err:#}");
    assert!(message.contains("stale"), "{message}");
    assert!(message.contains("order.placed"), "{message}");
    let stale = err
        .downcast_ref::<StalePlan>()
        .unwrap_or_else(|| panic!("a stale plan must be a typed StalePlan: {message}"));
    assert_eq!(stale.entity.slug, "order.placed");
    assert_ne!(stale.expected, stale.actual, "{stale}");
    stale
}

#[tokio::test]
async fn apply_refuses_plan_when_absent_entity_was_created_concurrently() {
    let endpoint = start_server().await;
    let mut agent = client::connect(&endpoint, None, 30).await.unwrap();
    let mut other = client::connect(&endpoint, None, 30).await.unwrap();
    let ns = "stale-create";

    let plan = plan_str(&mut agent, &stale_event_yaml(ns, "Mine")).await;
    assert_eq!(plan[0].action, apply::Action::Create);

    other
        .put_entity(put_req(
            event_entity(ns, "order.placed", 1, "Other agent"),
            false,
        ))
        .await
        .unwrap();

    let err = apply::apply(&mut agent, plan, false, None)
        .await
        .err()
        .expect("a plan made while the entity was absent must not overwrite a concurrent create");
    let stale = assert_stale_error(&err);
    assert_eq!(stale.expected, Revision::Absent);
    assert!(matches!(stale.actual, Revision::At(_)), "{stale}");
    assert_eq!(
        stored_title(&mut agent, ns).await.as_deref(),
        Some("Other agent")
    );
}

#[tokio::test]
async fn apply_refuses_plan_when_entity_was_updated_concurrently() {
    let endpoint = start_server().await;
    let mut agent = client::connect(&endpoint, None, 30).await.unwrap();
    let mut other = client::connect(&endpoint, None, 30).await.unwrap();
    let ns = "stale-update";

    apply_str(&mut agent, &stale_event_yaml(ns, "Base"), false).await;
    let plan = plan_str(&mut agent, &stale_event_yaml(ns, "Mine")).await;
    assert_eq!(plan[0].action, apply::Action::Configure);

    other
        .put_entity(put_req(
            event_entity(ns, "order.placed", 1, "Other agent"),
            false,
        ))
        .await
        .unwrap();

    let err = apply::apply(&mut agent, plan, false, None)
        .await
        .err()
        .expect("a plan made against an older revision must not overwrite a newer one");
    let stale = assert_stale_error(&err);
    assert!(matches!(stale.expected, Revision::At(_)), "{stale}");
    assert!(matches!(stale.actual, Revision::At(_)), "{stale}");
    assert_eq!(
        stored_title(&mut agent, ns).await.as_deref(),
        Some("Other agent")
    );
}

#[tokio::test]
async fn apply_refuses_plan_when_entity_was_deleted_concurrently() {
    let endpoint = start_server().await;
    let mut agent = client::connect(&endpoint, None, 30).await.unwrap();
    let mut other = client::connect(&endpoint, None, 30).await.unwrap();
    let ns = "stale-delete";

    apply_str(&mut agent, &stale_event_yaml(ns, "Base"), false).await;
    let plan = plan_str(&mut agent, &stale_event_yaml(ns, "Mine")).await;
    assert_eq!(plan[0].action, apply::Action::Configure);

    other
        .delete_entity(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(stale_event_id(ns)),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            if_match: String::new(),
            operation_id: String::new(),
        })
        .await
        .unwrap();

    let err = apply::apply(&mut agent, plan, false, None)
        .await
        .err()
        .expect("a plan to update an entity must not resurrect it after a concurrent delete");
    let stale = assert_stale_error(&err);
    assert_eq!(stale.actual, Revision::Absent);
    assert_eq!(stored_title(&mut agent, ns).await, None);
}

#[tokio::test]
async fn dry_run_apply_also_refuses_a_stale_plan() {
    let endpoint = start_server().await;
    let mut agent = client::connect(&endpoint, None, 30).await.unwrap();
    let mut other = client::connect(&endpoint, None, 30).await.unwrap();
    let ns = "stale-dry-run";

    let plan = plan_str(&mut agent, &stale_event_yaml(ns, "Mine")).await;
    other
        .put_entity(put_req(
            event_entity(ns, "order.placed", 1, "Other agent"),
            false,
        ))
        .await
        .unwrap();

    let err = apply::apply(&mut agent, plan, true, None)
        .await
        .err()
        .expect("a dry run must report the same staleness the real apply would hit");
    assert_stale_error(&err);
}

#[tokio::test]
async fn apply_on_a_branch_refuses_plan_when_entity_changed_on_that_branch() {
    let endpoint = start_server().await;
    let mut baseline = client::connect(&endpoint, None, 30).await.unwrap();
    let ns = "stale-branch";
    branch::create_branch(&mut baseline, "stale-branch", "doc")
        .await
        .unwrap();
    let mut agent = branch_client(&endpoint, "stale-branch").await;
    let mut other = branch_client(&endpoint, "stale-branch").await;

    apply_str(&mut agent, &stale_event_yaml(ns, "Base"), false).await;
    let plan = plan_str(&mut agent, &stale_event_yaml(ns, "Mine")).await;
    other
        .put_entity(put_req(
            event_entity(ns, "order.placed", 1, "Other agent"),
            false,
        ))
        .await
        .unwrap();

    let err = apply::apply(&mut agent, plan, false, None)
        .await
        .err()
        .expect("branch-scoped plans carry the same guarantee as baseline ones");
    assert_stale_error(&err);
    assert_eq!(
        stored_title(&mut agent, ns).await.as_deref(),
        Some("Other agent")
    );
}

async fn conflicted_branch(endpoint: &str, name: &str) -> client::Client {
    let mut baseline = client::connect(endpoint, None, 30).await.unwrap();
    baseline
        .put_entity(put_req(event_entity(name, "a", 1, "Base"), false))
        .await
        .unwrap();
    branch::create_branch(&mut baseline, name, "doc")
        .await
        .unwrap();
    let mut on_branch = branch_client(endpoint, name).await;
    on_branch
        .put_entity(put_req(event_entity(name, "a", 1, "Branch edit"), false))
        .await
        .unwrap();
    baseline
        .put_entity(put_req(event_entity(name, "a", 1, "Baseline moved"), false))
        .await
        .unwrap();
    baseline
}

fn conflict_ref(name: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: Some(pb::Id {
            namespace: name.into(),
            slug: "a".into(),
            version: 1,
        }),
    }
}

async fn inspected_state(baseline: &mut client::Client, name: &str) -> pb::BranchEntryState {
    let inspected = branch::diff_branch(baseline, name).await.unwrap();
    assert_eq!(
        inspected[0].status,
        pb::branch_diff_entry::Status::ConflictEditEdit as i32
    );
    inspected[0]
        .state
        .clone()
        .expect("a diff entry must report the revisions it was classified from")
}

async fn assert_resolution_refused(
    baseline: &mut client::Client,
    name: &str,
    resolution: pb::resolve_branch_entry_request::Resolution,
    expected_state: pb::BranchEntryState,
) {
    let status = baseline
        .resolve_branch_entry(pb::ResolveBranchEntryRequest {
            name: name.into(),
            r#ref: Some(conflict_ref(name)),
            resolution: resolution as i32,
            expected_state: Some(expected_state),
            operation_id: String::new(),
        })
        .await
        .expect_err("a decision about older conflict inputs must not apply to newer ones");
    assert_eq!(status.code(), tonic::Code::Aborted, "{status}");
    assert!(status.message().contains(name), "{status}");
}

#[tokio::test]
async fn resolve_refuses_a_decision_when_baseline_moved_after_inspection() {
    let endpoint = start_server().await;
    let name = "stale-resolve-theirs";
    let mut baseline = conflicted_branch(&endpoint, name).await;
    let state = inspected_state(&mut baseline, name).await;

    baseline
        .put_entity(put_req(
            event_entity(name, "a", 1, "Baseline moved again"),
            false,
        ))
        .await
        .unwrap();

    assert_resolution_refused(
        &mut baseline,
        name,
        pb::resolve_branch_entry_request::Resolution::KeepOurs,
        state,
    )
    .await;
    let after = branch::diff_branch(&mut baseline, name).await.unwrap();
    assert_eq!(
        after[0].status,
        pb::branch_diff_entry::Status::ConflictEditEdit as i32,
        "the refused resolution must leave the conflict in place"
    );
}

#[tokio::test]
async fn resolve_refuses_a_decision_when_the_branch_side_moved_after_inspection() {
    let endpoint = start_server().await;
    let name = "stale-resolve-ours";
    let mut baseline = conflicted_branch(&endpoint, name).await;
    let state = inspected_state(&mut baseline, name).await;

    let mut on_branch = branch_client(&endpoint, name).await;
    on_branch
        .put_entity(put_req(
            event_entity(name, "a", 1, "Branch edited again"),
            false,
        ))
        .await
        .unwrap();

    assert_resolution_refused(
        &mut baseline,
        name,
        pb::resolve_branch_entry_request::Resolution::TakeTheirs,
        state,
    )
    .await;
    let after = branch::diff_branch(&mut baseline, name).await.unwrap();
    match after[0].ours.as_ref().and_then(|e| e.kind.as_ref()) {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Branch edited again"),
        other => panic!("the refused take_theirs must keep the branch edit: {other:?}"),
    }
}

#[tokio::test]
async fn resolve_refuses_a_decision_when_baseline_deleted_after_inspection() {
    let endpoint = start_server().await;
    let name = "stale-resolve-deleted";
    let mut baseline = conflicted_branch(&endpoint, name).await;
    let state = inspected_state(&mut baseline, name).await;

    baseline
        .delete_entity(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: conflict_ref(name).id,
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            if_match: String::new(),
            operation_id: String::new(),
        })
        .await
        .unwrap();

    assert_resolution_refused(
        &mut baseline,
        name,
        pb::resolve_branch_entry_request::Resolution::TakeTheirs,
        state,
    )
    .await;
    let after = branch::diff_branch(&mut baseline, name).await.unwrap();
    assert_eq!(
        after[0].status,
        pb::branch_diff_entry::Status::ConflictDeleteEdit as i32,
        "the refused take_theirs must not drop the branch edit"
    );
}

#[tokio::test]
async fn resolve_with_the_inspected_state_still_succeeds() {
    let endpoint = start_server().await;
    let name = "fresh-resolve";
    let mut baseline = conflicted_branch(&endpoint, name).await;
    let state = inspected_state(&mut baseline, name).await;

    baseline
        .resolve_branch_entry(pb::ResolveBranchEntryRequest {
            name: name.into(),
            r#ref: Some(conflict_ref(name)),
            resolution: pb::resolve_branch_entry_request::Resolution::KeepOurs as i32,
            expected_state: Some(state),
            operation_id: String::new(),
        })
        .await
        .unwrap();
    let after = branch::diff_branch(&mut baseline, name).await.unwrap();
    assert_eq!(
        after[0].status,
        pb::branch_diff_entry::Status::Changed as i32
    );
}

#[tokio::test]
async fn client_resolve_reports_which_side_moved_as_a_typed_stale_resolution() {
    let endpoint = start_server().await;
    let name = "stale-resolve-typed";
    let mut baseline = conflicted_branch(&endpoint, name).await;
    let inspected =
        BranchEntryState::from_wire(&inspected_state(&mut baseline, name).await).unwrap();

    baseline
        .put_entity(put_req(
            event_entity(name, "a", 1, "Baseline moved again"),
            false,
        ))
        .await
        .unwrap();

    let err = branch::resolve_branch_entry(
        &mut baseline,
        name,
        conflict_ref(name),
        branch::Resolution::KeepOurs,
        &inspected,
        None,
    )
    .await
    .expect_err("a decision about older conflict inputs must not apply to newer ones");
    let message = format!("{err:#}");
    let stale = err
        .downcast_ref::<StaleResolution>()
        .unwrap_or_else(|| panic!("a stale resolution must be typed: {message}"));
    assert_eq!(stale.branch, name);
    assert_eq!(stale.expected, inspected);
    let actual = stale.actual.as_ref().expect("the entry still exists");
    assert_eq!(actual.ours, inspected.ours, "only baseline moved");
    assert_ne!(actual.theirs, inspected.theirs, "{stale}");
    assert!(message.contains("stale"), "{message}");

    branch::resolve_branch_entry(
        &mut baseline,
        name,
        conflict_ref(name),
        branch::Resolution::KeepOurs,
        actual,
        None,
    )
    .await
    .expect("deciding again from the refreshed state must succeed");
}

// Operation receipts: `apply` threads a caller-supplied `operation_id` onto
// the batch, never onto a nested op (the server rejects that), and a second
// call with the same id and the same manifest replays the first outcome
// instead of re-executing it. `operation::get_operation` is the durable
// side door back to that receipt for a caller that lost the response.

#[tokio::test]
async fn apply_with_an_operation_id_replays_instead_of_reapplying() {
    let endpoint = start_server().await;
    let mut agent = client::connect(&endpoint, None, 30).await.unwrap();
    let ns = "operation-receipts";
    let op_id = "test-op-apply-replay-1";

    // Same plan resent twice with the same operation_id: this is what a
    // client that dropped the first response and retried looks like, not a
    // second `apply::plan` call (which would see the entity already there
    // and classify the retry as Unchanged, never sending a second batch).
    let manifest = manifest::parse_str(&stale_event_yaml(ns, "First"), "test.yaml")
        .unwrap()
        .remove(0);
    let build_create_plan = || {
        vec![apply::PlannedChange {
            manifest: manifest.clone(),
            action: apply::Action::Create,
            current: None,
            observed: Some(Revision::Absent),
        }]
    };

    let first = apply::apply(&mut agent, build_create_plan(), false, Some(op_id))
        .await
        .unwrap();
    let receipt = first
        .operation_receipt
        .expect("a batch carrying an operation_id must get a receipt back");
    assert_eq!(receipt.operation_id, op_id);
    assert!(!receipt.replayed, "the first call must not be a replay");
    assert_ne!(receipt.changeset_id, "");

    let second = apply::apply(&mut agent, build_create_plan(), false, Some(op_id))
        .await
        .unwrap();
    let replay_receipt = second
        .operation_receipt
        .expect("replaying an operation_id must still return a receipt");
    assert!(replay_receipt.replayed, "the second call must be a replay");
    assert_eq!(replay_receipt.changeset_id, receipt.changeset_id);

    let looked_up = operation::get_operation(&mut agent, op_id).await.unwrap();
    assert_eq!(
        operation::status_label(looked_up.status),
        "applied",
        "{looked_up:?}"
    );
    assert_eq!(looked_up.changeset_id, receipt.changeset_id);
}

#[tokio::test]
async fn get_operation_reports_unknown_for_an_id_nothing_ever_claimed() {
    let endpoint = start_server().await;
    let mut agent = client::connect(&endpoint, None, 30).await.unwrap();

    let looked_up = operation::get_operation(&mut agent, "never-claimed-op-id")
        .await
        .unwrap();
    assert_eq!(operation::status_label(looked_up.status), "unknown");
}

#[tokio::test]
async fn merge_branch_with_an_operation_id_returns_a_receipt() {
    let endpoint = start_server().await;
    let mut baseline = client::connect(&endpoint, None, 30).await.unwrap();
    let op_id = "test-op-merge-1";

    branch::create_branch(&mut baseline, "client-op-merge", "doc")
        .await
        .unwrap();
    let mut on_branch = branch_client(&endpoint, "client-op-merge").await;
    on_branch
        .put_entity(put_req(
            event_entity("op-merge-ns", "a", 1, "On branch"),
            false,
        ))
        .await
        .unwrap();

    let resp = branch::merge_branch(
        &mut baseline,
        "client-op-merge",
        branch::MergeOptions::default(),
        Some(op_id),
    )
    .await
    .unwrap();
    let receipt = resp
        .operation_receipt
        .expect("a merge carrying an operation_id must get a receipt back");
    assert_eq!(receipt.operation_id, op_id);
    assert!(!receipt.replayed);
}

const TENANT_TYPED_MANIFESTS: &str = r#"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: tenant-typed, name: order.placed }
spec:
  title: Order placed
  schema:
    "@type": type.googleapis.com/shop.orders.v1.OrderPlaced
    orderId: o-1
    totalCents: "1250"
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: TypeLibrary
metadata: { namespace: tenant-typed, name: shop.orders.v1 }
spec:
  title: Order types
  files:
    - path: shop/orders/v1/orders.proto
      content: |
        syntax = "proto3";
        package shop.orders.v1;
        message OrderPlaced {
          string order_id = 1;
          int64 total_cents = 2;
        }
"#;

#[tokio::test]
async fn a_manifest_set_can_introduce_a_type_and_its_first_event_together() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    let err = manifest::parse_str(TENANT_TYPED_MANIFESTS, "t").unwrap_err();
    assert!(format!("{err:#}").contains("OrderPlaced"), "{err:#}");

    let documents = manifest::read_str(TENANT_TYPED_MANIFESTS, "t").unwrap();
    let resolved = manifest::load_with_tenant_types(&mut client, documents)
        .await
        .unwrap();
    let kinds: Vec<_> = resolved.manifests.iter().map(|m| m.kind).collect();
    assert_eq!(
        kinds,
        vec![pb::EntityKind::TypeLibrary, pb::EntityKind::Event],
        "type libraries go first"
    );
    let plan = apply::plan(&mut client, resolved.manifests).await.unwrap();
    let out = apply::apply(&mut client, plan, false, None).await.unwrap();
    for line in &out.lines {
        assert!(line.ends_with(" created"), "{line}");
    }

    let outcome = trogon_atlas_client::export::export_namespace(&mut client, "tenant-typed")
        .await
        .unwrap();
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    let exported = outcome
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("---\n");
    assert!(
        exported.contains("type.googleapis.com/shop.orders.v1.OrderPlaced")
            && exported.contains("o-1"),
        "{exported}"
    );

    let mut documents = Vec::new();
    for (name, content) in &outcome.files {
        documents.extend(manifest::read_str(content, name).unwrap());
    }
    let resolved = manifest::load_with_tenant_types(&mut client, documents)
        .await
        .unwrap();
    let plan = apply::plan(&mut client, resolved.manifests).await.unwrap();
    for change in &plan {
        assert_eq!(
            change.action,
            apply::Action::Unchanged,
            "exported manifest drifted for {}",
            change.manifest.source
        );
    }
    let (diff, changed) = apply::render_diff(&plan, &resolved.types).unwrap();
    assert!(!changed, "{diff}");
}
