#![allow(clippy::unwrap_used, clippy::expect_used)]

// `apply --prune` end to end, against a real in-process gRPC server backed by
// an ephemeral JetStream store (same harness as apply_integration.rs; needs
// Docker at test time).

use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_client::{
    apply::{self, Action, ApplyRejected},
    branch, client, manifest,
    precondition::StalePlan,
};
use trogon_atlas_core::transcode::TranscodePool;
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

const HEADER: &str = "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1";

/// A manifest for a `kind` whose spec is just its title.
fn titled(kind: &str, ns: &str, slug: &str) -> String {
    format!("{HEADER}\nkind: {kind}\nmetadata: {{ namespace: {ns}, name: {slug} }}\nspec: {{ title: {slug} }}\n")
}

fn event(ns: &str, slug: &str) -> String {
    titled("Event", ns, slug)
}

fn command(ns: &str, slug: &str) -> String {
    titled("Command", ns, slug)
}

/// A command slice in `ns` whose command and emitted event are given as
/// manifest refs (`slug` or `other-ns/slug`).
fn slice(ns: &str, slug: &str, command: &str, event: &str) -> String {
    format!(
        "{HEADER}\nkind: CommandSlice\nmetadata: {{ namespace: {ns}, name: {slug} }}\nspec:\n  title: {slug}\n  command: {command}\n  emittedEvents: [{event}]\n"
    )
}

fn docs(parts: &[String]) -> String {
    parts.join("---\n")
}

/// The model the tests start from, in namespace `ns`: an ordering slice with
/// its command and event, plus a standalone event nothing references.
fn model(ns: &str) -> Vec<String> {
    vec![
        command(ns, "place-order"),
        event(ns, "order.placed"),
        slice(ns, "place-order-slice", "place-order", "order.placed"),
        event(ns, "order.archived"),
    ]
}

async fn plan(client: &mut client::Client, yaml: &str, prune: bool) -> Vec<apply::PlannedChange> {
    let manifests = manifest::parse_str(yaml, "test.yaml").unwrap();
    apply::plan_with_prune(client, manifests, prune)
        .await
        .unwrap()
}

async fn apply_str(
    client: &mut client::Client,
    yaml: &str,
    prune: bool,
    dry_run: bool,
) -> anyhow::Result<apply::ApplyOutcome> {
    let plan = plan(client, yaml, prune).await;
    apply::apply(client, plan, dry_run, None).await
}

async fn exists(client: &mut client::Client, kind: pb::EntityKind, ns: &str, slug: &str) -> bool {
    match client
        .get_entity(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version: 1,
            }),
        })
        .await
    {
        Ok(_) => true,
        Err(status) if status.code() == tonic::Code::NotFound => false,
        Err(status) => panic!("get_entity failed: {status}"),
    }
}

fn pruned(outcome: &apply::ApplyOutcome) -> Vec<&str> {
    outcome
        .lines
        .iter()
        .filter_map(|l| l.strip_suffix(" pruned"))
        .collect()
}

#[tokio::test]
async fn prune_deletes_only_undeclared_entities_in_covered_namespaces() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    let mut initial = model("shop");
    initial.push(event("billing", "invoice.sent"));
    apply_str(&mut client, &docs(&initial), false, false)
        .await
        .unwrap();

    // The revised set drops order.archived and never mentions `billing`.
    let revised = docs(&model("shop")[..3]);

    // Plain apply and plain diff never delete.
    let out = apply_str(&mut client, &revised, false, false)
        .await
        .unwrap();
    assert!(pruned(&out).is_empty(), "{:?}", out.lines);
    let types = TranscodePool::builtin().unwrap();
    let (_, changed) =
        apply::render_diff(&plan(&mut client, &revised, false).await, &types).unwrap();
    assert!(!changed);

    // diff --prune shows the delete as a removed entity.
    let (diff, changed) =
        apply::render_diff(&plan(&mut client, &revised, true).await, &types).unwrap();
    assert!(changed);
    assert!(diff.contains("diff event:shop/order.archived@1"), "{diff}");
    assert!(diff.contains("-  \"event\": {"), "{diff}");
    assert!(!diff.contains("+  \"event\""), "{diff}");
    assert!(!diff.contains("billing"), "{diff}");

    let out = apply_str(&mut client, &revised, true, false).await.unwrap();
    assert_eq!(pruned(&out), vec!["event:shop/order.archived@1"]);
    assert!(!exists(&mut client, pb::EntityKind::Event, "shop", "order.archived").await);
    assert!(exists(&mut client, pb::EntityKind::Event, "shop", "order.placed").await);
    assert!(
        exists(
            &mut client,
            pb::EntityKind::Event,
            "billing",
            "invoice.sent"
        )
        .await,
        "a namespace the manifest set does not name is out of scope"
    );

    // Converged: pruning again changes nothing.
    let out = apply_str(&mut client, &revised, true, false).await.unwrap();
    assert!(
        out.lines.iter().all(|l| l.ends_with(" unchanged")),
        "{:?}",
        out.lines
    );
}

#[tokio::test]
async fn prune_removes_entities_that_only_reference_each_other_in_one_batch() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    apply_str(&mut client, &docs(&model("shop")), false, false)
        .await
        .unwrap();

    // Drop the slice together with the command and event it references.
    let out = apply_str(&mut client, &docs(&model("shop")[3..]), true, false)
        .await
        .unwrap();
    assert_eq!(
        pruned(&out),
        vec![
            "command:shop/place-order@1",
            "command_slice:shop/place-order-slice@1",
            "event:shop/order.placed@1",
        ]
    );
    for (kind, slug) in [
        (pb::EntityKind::Command, "place-order"),
        (pb::EntityKind::CommandSlice, "place-order-slice"),
        (pb::EntityKind::Event, "order.placed"),
    ] {
        assert!(!exists(&mut client, kind, "shop", slug).await, "{slug}");
    }
}

#[tokio::test]
async fn prune_fails_closed_on_a_reference_from_outside_the_set() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    let mut initial = model("shop");
    initial.push(command("billing", "bill"));
    initial.push(slice(
        "billing",
        "bill-slice",
        "bill",
        "shop/order.archived",
    ));
    apply_str(&mut client, &docs(&initial), false, false)
        .await
        .unwrap();

    let revised = docs(&model("shop")[..3]);
    let err = apply_str(&mut client, &revised, true, false)
        .await
        .err()
        .expect("a prune that strands a referrer must be refused");
    let rejected = err
        .downcast_ref::<ApplyRejected>()
        .unwrap_or_else(|| panic!("expected ApplyRejected: {err:#}"));
    assert_eq!(rejected.failed, "event:shop/order.archived@1");
    assert_eq!(rejected.issues[0].code, "ENTITY_REFERENCED");
    assert!(exists(&mut client, pb::EntityKind::Event, "shop", "order.archived").await);

    // The dry run reaches the same verdict.
    let err = apply_str(&mut client, &revised, true, true)
        .await
        .err()
        .unwrap();
    assert!(err.downcast_ref::<ApplyRejected>().is_some(), "{err:#}");
}

#[tokio::test]
async fn prune_fails_closed_when_a_declared_entity_still_references_a_pruned_one() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    apply_str(&mut client, &docs(&model("shop")), false, false)
        .await
        .unwrap();

    // The slice is still declared, and still emits order.placed, but the
    // event itself is gone from the set.
    let m = model("shop");
    let revised = docs(&[m[0].clone(), m[2].clone(), m[3].clone()]);
    let err = apply_str(&mut client, &revised, true, false)
        .await
        .err()
        .expect("deleting an event a declared slice still emits must be refused");
    let rejected = err.downcast_ref::<ApplyRejected>().unwrap();
    assert_eq!(rejected.failed, "event:shop/order.placed@1");
    assert_eq!(rejected.issues[0].code, "ENTITY_REFERENCED");
    assert!(exists(&mut client, pb::EntityKind::Event, "shop", "order.placed").await);
}

#[tokio::test]
async fn prune_dry_run_reports_without_deleting() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    apply_str(&mut client, &docs(&model("shop")), false, false)
        .await
        .unwrap();

    let out = apply_str(&mut client, &docs(&model("shop")[..3]), true, true)
        .await
        .unwrap();
    assert!(
        out.lines
            .contains(&"event:shop/order.archived@1 pruned (server dry run)".to_string()),
        "{:?}",
        out.lines
    );
    assert!(exists(&mut client, pb::EntityKind::Event, "shop", "order.archived").await);
}

#[tokio::test]
async fn prune_refuses_a_plan_another_writer_overtook() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    apply_str(&mut client, &docs(&model("shop")), false, false)
        .await
        .unwrap();

    let planned = plan(&mut client, &docs(&model("shop")[..3]), true).await;
    let prune = planned.iter().find(|c| c.action == Action::Prune).unwrap();
    assert_eq!(prune.manifest.id.slug, "order.archived");

    // Someone edits the entity after the plan was made.
    let edited =
        event("shop", "order.archived").replace("title: order.archived", "title: Still used");
    apply_str(&mut client, &edited, false, false).await.unwrap();

    let err = apply::apply(&mut client, planned, false, None)
        .await
        .err()
        .expect("a prune planned against an older revision must be refused");
    let stale = err
        .downcast_ref::<StalePlan>()
        .unwrap_or_else(|| panic!("expected StalePlan: {err:#}"));
    assert_eq!(stale.entity.slug, "order.archived");
    assert!(exists(&mut client, pb::EntityKind::Event, "shop", "order.archived").await);
}

#[tokio::test]
async fn prune_on_a_branch_leaves_baseline_untouched() {
    let endpoint = start_server().await;
    let mut baseline = client::connect(&endpoint, None, 30).await.unwrap();
    apply_str(&mut baseline, &docs(&model("shop")), false, false)
        .await
        .unwrap();
    branch::create_branch(&mut baseline, "prune-branch", "prune test")
        .await
        .unwrap();
    let mut on_branch = client::connect_with_options(
        &endpoint,
        None,
        30,
        client::ConnectOptions {
            branch: Some("prune-branch".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let out = apply_str(&mut on_branch, &docs(&model("shop")[..3]), true, false)
        .await
        .unwrap();
    assert_eq!(pruned(&out), vec!["event:shop/order.archived@1"]);
    assert!(
        !exists(
            &mut on_branch,
            pb::EntityKind::Event,
            "shop",
            "order.archived"
        )
        .await
    );
    assert!(
        exists(
            &mut baseline,
            pb::EntityKind::Event,
            "shop",
            "order.archived"
        )
        .await
    );
}

#[tokio::test]
async fn prune_refuses_a_namespace_the_server_cannot_list_completely() {
    // A real server only truncates past its entity cap; pose as one that did.
    let server = trogon_atlas_testsupport::discovery::DiscoveryOnlyServer::start_serving_listing(
        trogon_atlas_testsupport::discovery::pre_revision_server_info(),
        pb::BatchGetEntitiesResponse {
            entities: vec![pb::Entity::default()],
            found: vec![false],
            ..Default::default()
        },
        pb::ListEntitiesResponse {
            snapshot_truncated: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut client = client::connect(&server.endpoint, None, 30).await.unwrap();

    let manifests = manifest::parse_str(&event("shop", "order.placed"), "test.yaml").unwrap();
    let err = apply::plan_with_prune(&mut client, manifests, true)
        .await
        .expect_err("a partial listing must not be planned as if it were complete");
    let message = format!("{err:#}");
    assert!(
        message.contains("namespace \"shop\" cannot be read completely"),
        "{message}"
    );
    assert!(message.contains("entity cap"), "{message}");
}

#[tokio::test]
async fn prune_reads_back_more_entities_than_one_batch_get_allows() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    // More leftovers than one BatchGetEntities call may carry (500).
    let ops = (0..520)
        .map(|i| pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    system: None,
                    kind: Some(pb::entity::Kind::Event(pb::Event {
                        id: Some(pb::Id {
                            namespace: "bulk".into(),
                            slug: format!("leftover-{i:03}"),
                            version: 1,
                        }),
                        title: format!("leftover {i}"),
                        ..Default::default()
                    })),
                }),
                ..Default::default()
            })),
        })
        .collect();
    client
        .batch_mutate(pb::BatchMutateRequest {
            ops,
            ..Default::default()
        })
        .await
        .unwrap();

    let out = apply_str(&mut client, &event("bulk", "kept"), true, false)
        .await
        .unwrap();
    assert_eq!(pruned(&out).len(), 520, "{:?}", out.lines);
    assert!(!exists(&mut client, pb::EntityKind::Event, "bulk", "leftover-519").await);
    assert!(exists(&mut client, pb::EntityKind::Event, "bulk", "kept").await);
}
