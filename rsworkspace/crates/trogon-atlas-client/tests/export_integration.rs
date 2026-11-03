#![allow(clippy::unwrap_used, clippy::expect_used)]

// export_namespace: live store -> manifest YAML. Covers the grouping/sort/
// multi-doc-render loop (only exercised end-to-end via
// apply_integration.rs::export_round_trips_applied_manifests before this
// file existed) plus the two branches that loop leaves untouched: the
// skipped-entity report for un-renderable payloads, empty-namespace error,
// multi-kind file grouping, and metadata.version omission when version==1.

use std::sync::Arc;

use serde::Deserialize as _;
use tokio::net::TcpListener;
use trogon_atlas_client::{apply, client, export, manifest};
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

const SLICES_MODEL: &str = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Persona
metadata: { namespace: export-slices, name: buyer }
spec: { title: Buyer }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Swimlane
metadata: { namespace: export-slices, name: order }
spec: { title: Order }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: export-slices, name: order.placed }
spec: { title: Order placed, swimlane: order }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Command
metadata: { namespace: export-slices, name: place-order }
spec: { title: Place order }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Ui
metadata: { namespace: export-slices, name: checkout }
spec: { title: Checkout }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: CommandSlice
metadata: { namespace: export-slices, name: place-order-slice }
spec:
  title: Place order
  persona: buyer
  ui: checkout
  command: place-order
  emittedEvents: [order.placed]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ReadModel
metadata: { namespace: export-slices, name: order-status }
spec:
  title: Order status
  sourceEvents: [order.placed]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ReadModelSlice
metadata: { namespace: export-slices, name: order-status-slice }
spec:
  title: Order status renders
  readModel: order-status
  sourceEvents: [order.placed]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Storyboard
metadata: { namespace: export-slices, name: buy-flow }
spec:
  title: Buy flow
  entry:
    human:
      persona: buyer
      ui: checkout
  slices:
    - id: place-order-slice
    - id: order-status-slice
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: EventModel
metadata: { namespace: export-slices, name: shop }
spec:
  title: Shop
  members:
    - { kind: storyboard, id: buy-flow }
";

#[tokio::test]
async fn export_slices_layout_is_self_contained_and_round_trips() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    let src_ns = "export-slices-src";
    let seeded = SLICES_MODEL.replace("export-slices", src_ns);
    let manifests = manifest::parse_str(&seeded, "t").unwrap();
    let plan = trogon_atlas_client::apply::plan(&mut client, manifests)
        .await
        .unwrap();
    trogon_atlas_client::apply::apply(&mut client, plan, false, None)
        .await
        .unwrap();

    let baseline = export::export_namespace(&mut client, src_ns).await.unwrap();
    assert!(baseline.skipped.is_empty(), "{:?}", baseline.skipped);

    let sliced = export::export_namespace_slices(&mut client, src_ns)
        .await
        .unwrap();
    assert!(sliced.skipped.is_empty(), "{:?}", sliced.skipped);

    let command_slice_name = format!("{src_ns}/place-order-slice.yaml");
    let read_model_slice_name = format!("{src_ns}/order-status-slice.yaml");
    let expected_files: std::collections::BTreeSet<String> = [
        command_slice_name.clone(),
        read_model_slice_name.clone(),
        "unsliced.yaml".to_string(),
    ]
    .into_iter()
    .collect();
    let actual_files: std::collections::BTreeSet<String> = sliced.files.keys().cloned().collect();
    assert_eq!(actual_files, expected_files, "{actual_files:?}");

    let command_slice_file = &sliced.files[&command_slice_name];
    for needle in [
        "kind: CommandSlice",
        "name: place-order-slice",
        "kind: Persona",
        "name: buyer",
        "kind: Ui",
        "name: checkout",
        "kind: Command",
        "name: place-order",
        "kind: Event",
        "name: order.placed",
        "kind: Swimlane",
        "name: order",
    ] {
        assert!(
            command_slice_file.contains(needle),
            "{needle:?} missing from {command_slice_file}"
        );
    }
    assert!(!command_slice_file.contains("kind: ReadModel"));

    let read_model_slice_file = &sliced.files[&read_model_slice_name];
    for needle in [
        "kind: ReadModelSlice",
        "name: order-status-slice",
        "kind: ReadModel",
        "name: order-status",
        "kind: Event",
        "name: order.placed",
        "kind: Swimlane",
        "name: order",
    ] {
        assert!(
            read_model_slice_file.contains(needle),
            "{needle:?} missing from {read_model_slice_file}"
        );
    }
    assert!(!read_model_slice_file.contains("kind: CommandSlice"));
    assert!(!read_model_slice_file.contains("kind: Persona"));
    assert!(!read_model_slice_file.contains("kind: Ui"));

    let unsliced_file = &sliced.files["unsliced.yaml"];
    for needle in [
        "kind: Storyboard",
        "name: buy-flow",
        "kind: EventModel",
        "name: shop",
    ] {
        assert!(
            unsliced_file.contains(needle),
            "{needle:?} missing from {unsliced_file}"
        );
    }
    for absent in [
        "kind: Persona\n",
        "kind: Event\n",
        "kind: Command\n",
        "kind: ReadModel\n",
        "kind: CommandSlice\n",
        "kind: ReadModelSlice\n",
        "kind: Ui\n",
        "kind: Swimlane\n",
    ] {
        assert!(
            !unsliced_file.contains(absent),
            "{absent:?} unexpectedly present in unsliced.yaml:\n{unsliced_file}"
        );
    }

    // The Event doc bundled into both slice files must be byte-identical,
    // not merely semantically equal (deterministic, reused rendering).
    fn doc_containing<'a>(content: &'a str, needle: &str) -> &'a str {
        let at = content.find(needle).unwrap();
        let start = content[..at].rfind("apiVersion:").unwrap();
        match content[at..].find("\n---\n") {
            Some(rel_end) => &content[start..=(at + rel_end)],
            None => &content[start..],
        }
    }
    assert_eq!(
        doc_containing(command_slice_file, "name: order.placed"),
        doc_containing(read_model_slice_file, "name: order.placed"),
        "duplicated Event document must render byte-identical across slice files"
    );

    // Applying the whole slices-layout directory (with its duplicates) back
    // into the namespace it was exported from must be a pure no-op: the
    // duplicates collapse and every entity classifies Unchanged.
    let mut all_docs = Vec::new();
    for (name, content) in &sliced.files {
        all_docs.extend(manifest::parse_str(content, name).unwrap());
    }
    assert_eq!(
        all_docs.len(),
        12,
        "6 (command slice file) + 4 (read model slice file) + 2 (unsliced)"
    );
    let plan = trogon_atlas_client::apply::plan(&mut client, all_docs)
        .await
        .unwrap();
    assert_eq!(
        plan.len(),
        10,
        "duplicate Event/Swimlane manifests collapse to the 10 distinct entities"
    );
    for change in &plan {
        assert_eq!(
            change.action,
            trogon_atlas_client::apply::Action::Unchanged,
            "re-applying the sliced export must be a no-op for {}",
            change.manifest.source
        );
    }

    // Re-apply the same directory into a fresh namespace and assert the
    // model round-trips: a single-layout export of the fresh namespace must
    // equal the baseline single-layout export, modulo the namespace.
    let dst_ns = "export-slices-dst";
    let mut fresh_docs = Vec::new();
    for (name, content) in &sliced.files {
        let retargeted = content.replace(src_ns, dst_ns);
        fresh_docs.extend(manifest::parse_str(&retargeted, name).unwrap());
    }
    let plan = trogon_atlas_client::apply::plan(&mut client, fresh_docs)
        .await
        .unwrap();
    trogon_atlas_client::apply::apply(&mut client, plan, false, None)
        .await
        .unwrap();

    let roundtripped = export::export_namespace(&mut client, dst_ns).await.unwrap();
    assert!(
        roundtripped.skipped.is_empty(),
        "{:?}",
        roundtripped.skipped
    );
    assert_eq!(
        roundtripped.files.keys().collect::<Vec<_>>(),
        baseline.files.keys().collect::<Vec<_>>(),
        "same set of kind files"
    );
    for (name, content) in &baseline.files {
        let expected = content.replace(src_ns, dst_ns);
        assert_eq!(
            roundtripped.files[name], expected,
            "exported {name} must round-trip through the slices layout"
        );
    }
}

#[tokio::test]
async fn export_empty_namespace_errors() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    let Err(err) = export::export_namespace(&mut client, "export-empty-ns").await else {
        panic!("expected empty namespace to error")
    };
    assert!(format!("{err:#}").contains("no entities"), "{err:#}");
}

#[tokio::test]
async fn export_groups_by_kind_sorts_and_omits_default_version() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    let yaml = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Persona
metadata: { namespace: export-multi, name: buyer }
spec: { title: Buyer }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: export-multi, name: z.event }
spec: { title: Z event }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: export-multi, name: a.event }
spec: { title: A event }
";
    let manifests = manifest::parse_str(yaml, "t").unwrap();
    let plan = trogon_atlas_client::apply::plan(&mut client, manifests)
        .await
        .unwrap();
    trogon_atlas_client::apply::apply(&mut client, plan, false, None)
        .await
        .unwrap();

    // A second version of a.event, so the export must sort by (slug,
    // version) and include both versions of the same slug.
    client
        .put_entity(put_req(
            pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(pb::Id {
                        namespace: "export-multi".into(),
                        slug: "a.event".into(),
                        version: 2,
                    }),
                    title: "A event v2".into(),
                    ..Default::default()
                })),
            },
            true,
        ))
        .await
        .unwrap();

    let outcome = export::export_namespace(&mut client, "export-multi")
        .await
        .unwrap();
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);

    // One file per kind, PascalCase-derived, lowercased-hyphenated file name.
    assert!(
        outcome.files.contains_key("persona.yaml"),
        "{:?}",
        outcome.files.keys()
    );
    assert!(
        outcome.files.contains_key("event.yaml"),
        "{:?}",
        outcome.files.keys()
    );

    let event_yaml = &outcome.files["event.yaml"];
    // Multi-doc YAML: three documents (a.event@1, a.event@2, z.event@1),
    // separated by `---` between docs (not before the first).
    assert_eq!(event_yaml.matches("---\n").count(), 2, "{event_yaml}");
    assert_eq!(event_yaml.matches("apiVersion:").count(), 3, "{event_yaml}");

    // Sorted by (slug, version): a.event@1, a.event@2, then z.event@1.
    let a1_pos = event_yaml.find("name: a.event").unwrap();
    let a2_pos = event_yaml.rfind("name: a.event").unwrap();
    let z_pos = event_yaml.find("name: z.event").unwrap();
    assert!(a1_pos < a2_pos && a2_pos < z_pos, "{event_yaml}");

    // version: 1 is omitted from metadata (only version 2 shows a version
    // field).
    assert_eq!(event_yaml.matches("version:").count(), 1, "{event_yaml}");

    // The exported YAML re-parses and applies as a pure no-op: proves the
    // manifest is a complete, drift-free representation.
    let mut reparsed = Vec::new();
    for (name, content) in &outcome.files {
        reparsed.extend(manifest::parse_str(content, name).unwrap());
    }
    assert_eq!(reparsed.len(), 4, "persona + 3 event versions");
    let plan = trogon_atlas_client::apply::plan(&mut client, reparsed)
        .await
        .unwrap();
    for change in &plan {
        assert_eq!(
            change.action,
            trogon_atlas_client::apply::Action::Unchanged,
            "exported manifest drifted for {}",
            change.manifest.source
        );
    }
}

#[tokio::test]
async fn export_reports_skipped_entities_with_unrenderable_payload() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    // A well-formed event next to a poisoned one in the same namespace: the
    // export must still succeed for the good entity and separately report
    // the bad one, never silently dropping it.
    client
        .put_entity(put_req(
            pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(pb::Id {
                        namespace: "export-skip".into(),
                        slug: "good.event".into(),
                        version: 1,
                    }),
                    title: "Good event".into(),
                    ..Default::default()
                })),
            },
            true,
        ))
        .await
        .unwrap();

    // An Any payload with a type_url the schema pool no longer resolves
    // (a "retired-package" annotation) forces entity_to_manifest's JSON
    // rendering to fail while still round-tripping through raw put_entity
    // (which does not validate metadata payloads against the descriptor
    // pool).
    let poisoned = pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: "export-skip".into(),
                slug: "poisoned.event".into(),
                version: 1,
            }),
            title: "Poisoned event".into(),
            metadata: vec![prost_types::Any {
                type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.NeverExisted".into(),
                value: vec![1, 2, 3],
            }],
            ..Default::default()
        })),
    };
    client.put_entity(put_req(poisoned, true)).await.unwrap();

    let outcome = export::export_namespace(&mut client, "export-skip")
        .await
        .unwrap();

    assert_eq!(outcome.skipped.len(), 1, "{:?}", outcome.skipped);
    assert!(
        outcome.skipped[0].contains("poisoned.event"),
        "{:?}",
        outcome.skipped
    );

    let event_yaml = &outcome.files["event.yaml"];
    assert_eq!(event_yaml.matches("apiVersion:").count(), 1, "{event_yaml}");
    assert!(event_yaml.contains("good.event"), "{event_yaml}");
    assert!(!event_yaml.contains("poisoned.event"), "{event_yaml}");
}

fn poisoned_metadata() -> Vec<prost_types::Any> {
    vec![prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.NeverExisted".into(),
        value: vec![1, 2, 3],
    }]
}

fn id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

/// One entity per non-Event `EntityKind`, each carrying poisoned metadata so
/// `entity_to_manifest` fails and `entity_label` is exercised for that kind
/// (the label match arm is otherwise unreachable, since every other export
/// test only ever puts well-formed Event/Persona entities). Also proves
/// `entity_kind_of`'s corresponding match arm runs (it is called before the
/// label fallback, on the `Ok` path elsewhere, but here via the label itself
/// since `entity_label` calls `entity_kind_of` on every branch).
#[tokio::test]
async fn export_reports_skipped_label_for_every_entity_kind() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    let ns = "export-all-kinds-poisoned";
    use pb::entity::Kind as K;
    let kinds: Vec<(&str, K)> = vec![
        (
            "command",
            K::Command(pb::Command {
                id: Some(id(ns, "command")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "read-model",
            K::ReadModel(pb::ReadModel {
                id: Some(id(ns, "read-model")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "processor",
            K::Processor(pb::Processor {
                id: Some(id(ns, "processor")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "ui",
            K::Ui(pb::Ui {
                id: Some(id(ns, "ui")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "persona",
            K::Persona(pb::Persona {
                id: Some(id(ns, "persona")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "swimlane",
            K::Swimlane(pb::Swimlane {
                id: Some(id(ns, "swimlane")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "command-slice",
            K::CommandSlice(pb::CommandSlice {
                id: Some(id(ns, "command-slice")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "read-model-slice",
            K::ReadModelSlice(pb::ReadModelSlice {
                id: Some(id(ns, "read-model-slice")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "automation-slice",
            K::AutomationSlice(pb::AutomationSlice {
                id: Some(id(ns, "automation-slice")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "storyboard",
            K::Storyboard(pb::Storyboard {
                id: Some(id(ns, "storyboard")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "event-model",
            K::EventModel(pb::EventModel {
                id: Some(id(ns, "event-model")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "component",
            K::Component(pb::Component {
                id: Some(id(ns, "component")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "external-system",
            K::ExternalSystem(pb::ExternalSystem {
                id: Some(id(ns, "external-system")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "tracker",
            K::Tracker(pb::Tracker {
                id: Some(id(ns, "tracker")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            // The server rejects a context whose slug differs from its
            // namespace, so this entry is matched by its kind prefix.
            "bounded_context:",
            K::BoundedContext(pb::BoundedContext {
                id: Some(id(ns, ns)),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "domain",
            K::Domain(pb::Domain {
                id: Some(id(ns, "domain")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "subdomain",
            K::Subdomain(pb::Subdomain {
                id: Some(id(ns, "subdomain")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "schema",
            K::Schema(pb::Schema {
                id: Some(id(ns, "schema")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "project",
            K::Project(pb::Project {
                id: Some(id(ns, "project")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "screen",
            K::Screen(pb::Screen {
                id: Some(id(ns, "screen")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "term",
            K::Term(pb::Term {
                id: Some(id(ns, "term")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
        (
            "ambiguity",
            K::Ambiguity(pb::Ambiguity {
                id: Some(id(ns, "ambiguity")),
                metadata: poisoned_metadata(),
                ..Default::default()
            }),
        ),
    ];

    for (label, kind) in &kinds {
        client
            .put_entity(put_req(
                pb::Entity {
                    system: None,
                    kind: Some(kind.clone()),
                },
                true,
            ))
            .await
            .unwrap_or_else(|err| panic!("put {label} failed: {err:?}"));
    }

    let outcome = export::export_namespace(&mut client, ns).await.unwrap();
    assert_eq!(outcome.skipped.len(), kinds.len(), "{:?}", outcome.skipped);
    for (label, _) in &kinds {
        assert!(
            outcome.skipped.iter().any(|s| s.contains(label)),
            "expected a skip entry labeled {label:?}, got {:?}",
            outcome.skipped
        );
    }
}

#[tokio::test]
async fn export_pages_through_more_than_one_page_of_entities() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    // list_entities pages at 200 per call inside export_namespace; put
    // strictly more than one page's worth of distinct entities so the
    // pagination loop actually reads a non-empty next_page_token and issues
    // a second ListEntities call (page_token continuation).
    const TOTAL: u64 = 205;
    for n in 1..=TOTAL {
        client
            .put_entity(put_req(
                pb::Entity {
                    system: None,
                    kind: Some(pb::entity::Kind::Event(pb::Event {
                        id: Some(pb::Id {
                            namespace: "export-paged".into(),
                            slug: format!("e{n}"),
                            version: 1,
                        }),
                        title: format!("E {n}"),
                        ..Default::default()
                    })),
                },
                true,
            ))
            .await
            .unwrap();
    }

    let outcome = export::export_namespace(&mut client, "export-paged")
        .await
        .unwrap();
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    let event_yaml = &outcome.files["event.yaml"];
    assert_eq!(
        event_yaml.matches("apiVersion:").count(),
        usize::try_from(TOTAL).unwrap(),
        "{event_yaml}"
    );
}

// ExportEventModelManifests: a board rendered as manifests someone else can
// apply. Pins what the board carries (its own closure, foreign entities it
// references once, nothing it does not reach) and that the output loads
// back with no drift.

const BOARD_PAYMENTS: &str = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Swimlane
metadata: { namespace: board-pay, name: payments }
spec: { title: Payments }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: board-pay, name: payment.captured }
spec: { title: Payment captured, swimlane: payments }
";

const BOARD_SHOP: &str = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Persona
metadata: { namespace: board-src, name: buyer }
spec: { title: Buyer }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Ui
metadata: { namespace: board-src, name: checkout }
spec: { title: Checkout }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Swimlane
metadata: { namespace: board-src, name: order }
spec: { title: Order }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: board-src, name: order.placed }
spec:
  title: Order placed
  swimlane: order
  metadata:
    - '@type': type.googleapis.com/trogonatlas.annotation.v1alpha1.TagAnnotation
      tags: ['context:shop']
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: board-src, name: not-on-the-board }
spec: { title: Not on the board }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Command
metadata: { namespace: board-src, name: place-order }
spec: { title: Place order }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: CommandSlice
metadata: { namespace: board-src, name: place-order-slice }
spec:
  title: Place order
  persona: buyer
  ui: checkout
  command: place-order
  emittedEvents: [order.placed]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ReadModel
metadata: { namespace: board-src, name: paid-orders }
spec:
  title: Paid orders
  sourceEvents: [board-pay/payment.captured]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ReadModelSlice
metadata: { namespace: board-src, name: paid-orders-slice }
spec:
  title: Paid orders
  readModel: paid-orders
  sourceEvents: [board-pay/payment.captured]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Storyboard
metadata: { namespace: board-src, name: buy-flow }
spec:
  title: Buy flow
  entry:
    human:
      persona: buyer
      ui: checkout
  slices:
    - id: place-order-slice
    - id: paid-orders-slice
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: EventModel
metadata: { namespace: board-src, name: shop }
spec:
  title: Shop
  members:
    - { kind: storyboard, id: buy-flow }
";

async fn seed(client: &mut client::Client, yaml: &str) {
    let docs = manifest::parse_str(yaml, "seed").unwrap();
    let plan = trogon_atlas_client::apply::plan(client, docs)
        .await
        .unwrap();
    trogon_atlas_client::apply::apply(client, plan, false, None)
        .await
        .unwrap();
}

async fn export_board(
    client: &mut client::Client,
    namespace: &str,
) -> Result<pb::ExportEventModelManifestsResponse, tonic::Status> {
    client
        .export_event_model_manifests(pb::ExportEventModelManifestsRequest {
            id: Some(pb::Id {
                namespace: namespace.into(),
                slug: "shop".into(),
                version: 1,
            }),
        })
        .await
        .map(tonic::Response::into_inner)
}

fn doc_names(yaml: &str) -> std::collections::BTreeSet<String> {
    manifest::parse_str(yaml, "export")
        .unwrap()
        .iter()
        .map(|doc| format!("{}/{}", doc.id.namespace, doc.id.slug))
        .collect()
}

#[tokio::test]
async fn board_manifests_carry_the_board_and_load_back_without_drift() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    seed(&mut client, BOARD_PAYMENTS).await;
    seed(&mut client, BOARD_SHOP).await;

    let out = export_board(&mut client, "board-src").await.unwrap();
    assert!(out.skipped.is_empty(), "{:?}", out.skipped);
    assert!(!out.truncated);

    let own: std::collections::BTreeSet<String> = [
        "buy-flow",
        "buyer",
        "checkout",
        "order",
        "order.placed",
        "paid-orders",
        "paid-orders-slice",
        "place-order",
        "place-order-slice",
        "shop",
    ]
    .iter()
    .map(|slug| format!("board-src/{slug}"))
    .collect();
    assert_eq!(
        doc_names(&out.manifests),
        own,
        "an entity the board does not reach stays out"
    );
    assert_eq!(
        doc_names(&out.external_manifests),
        std::collections::BTreeSet::from(["board-pay/payment.captured".to_string()]),
        "a foreign entity comes once, without its own swimlane",
    );
    assert!(
        out.manifests.contains("TagAnnotation") && out.manifests.contains("context:shop"),
        "annotations render as typed values:\n{}",
        out.manifests
    );

    let mut docs = manifest::parse_str(&out.manifests, "own").unwrap();
    docs.extend(manifest::parse_str(&out.external_manifests, "external").unwrap());
    for change in trogon_atlas_client::apply::plan(&mut client, docs)
        .await
        .unwrap()
    {
        assert_eq!(
            change.action,
            trogon_atlas_client::apply::Action::Unchanged,
            "{} drifted from the store it was exported from",
            change.manifest.source
        );
    }

    // Loading the board into another namespace reproduces it exactly.
    let copy = out.manifests.replace("board-src", "board-dst");
    seed(&mut client, &copy).await;
    let copied = export_board(&mut client, "board-dst").await.unwrap();
    assert_eq!(copied.manifests, copy);
    assert_eq!(copied.external_manifests, out.external_manifests);
}

/// Write `model` and `external` into a board file the way Studio's Export
/// HTML does, `<` escaped so the YAML cannot end the data block.
fn write_board_html(dir: &std::path::Path, model: &str, external: &str) -> std::path::PathBuf {
    let snapshot = serde_json::json!({ "manifests": { "model": model, "external": external } });
    let data = snapshot.to_string().replace('<', "\\u003c");
    let path = dir.join("shop.html");
    std::fs::write(
        &path,
        format!("<script id=\"atlas-snapshot\" type=\"application/json\">{data}</script>"),
    )
    .unwrap();
    path
}

#[tokio::test]
async fn board_html_applies_into_a_fresh_namespace_and_checks_external_entities() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    seed(&mut client, BOARD_PAYMENTS).await;
    seed(&mut client, BOARD_SHOP).await;
    let out = export_board(&mut client, "board-src").await.unwrap();
    let dir = tempfile::tempdir().unwrap();

    let model = out.manifests.replace("board-src", "board-html");
    let files = manifest::read_paths(&[write_board_html(
        dir.path(),
        &model,
        &out.external_manifests,
    )])
    .unwrap();
    apply::require_external(&mut client, files.external)
        .await
        .unwrap();
    let resolved = manifest::load_with_tenant_types(&mut client, files.documents.clone())
        .await
        .unwrap();
    let plan = apply::plan(&mut client, resolved.manifests).await.unwrap();
    assert!(plan.iter().all(|c| c.action == apply::Action::Create));
    apply::apply(&mut client, plan, false, None).await.unwrap();
    let resolved = manifest::load_with_tenant_types(&mut client, files.documents)
        .await
        .unwrap();
    for change in apply::plan(&mut client, resolved.manifests).await.unwrap() {
        assert_eq!(
            change.action,
            apply::Action::Unchanged,
            "{}",
            change.manifest.source
        );
    }

    // A target that lacks a foreign entity the board uses is refused before any write.
    let gone = |yaml: &str| yaml.replace("board-pay", "board-gone");
    let path = write_board_html(dir.path(), &gone(&model), &gone(&out.external_manifests));
    let files = manifest::read_paths(&[path]).unwrap();
    let err = apply::require_external(&mut client, files.external)
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("event:board-gone/payment.captured"),
        "{err:#}"
    );

    // The bundled copy is never written, so a spec this client cannot transcode
    // must neither hide a missing entity nor block a present one.
    let untyped = |ns: &str| {
        format!(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: {{ namespace: {ns}, name: payment.captured }}
spec:
  title: Payment captured
  metadata:
    - '@type': type.googleapis.com/acme.payments.v1.Retired
      note: gone
"
        )
    };
    let path = write_board_html(dir.path(), &model, &untyped("board-gone"));
    let files = manifest::read_paths(&[path]).unwrap();
    let err = apply::require_external(&mut client, files.external)
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("missing external entities"),
        "{err:#}"
    );
    let path = write_board_html(dir.path(), &model, &untyped("board-pay"));
    let files = manifest::read_paths(&[path]).unwrap();
    apply::require_external(&mut client, files.external)
        .await
        .unwrap();
}

#[tokio::test]
async fn board_manifests_for_a_missing_event_model_are_not_found() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    let status = export_board(&mut client, "board-none").await.unwrap_err();
    assert_eq!(status.code(), tonic::Code::NotFound);
}

async fn assert_board_namespace_types(foreign_proto_path: &str) {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    for (namespace, proto_path, field, json_field, value) in [
        (
            "board-types-local",
            "shared/payload.proto",
            "local_value",
            "localValue",
            "local payload",
        ),
        (
            "board-types-foreign",
            foreign_proto_path,
            "foreign_value",
            "foreignValue",
            "foreign payload",
        ),
    ] {
        let yaml = format!(
            r#"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: TypeLibrary
metadata: {{ namespace: {namespace}, name: shared.payload }}
spec:
  title: Payload types
  files:
    - path: {proto_path}
      content: |
        syntax = "proto3";
        package shared.payload;
        message Payload {{ string {field} = 1; }}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: {{ namespace: {namespace}, name: payload }}
spec:
  title: Typed payload
  schema:
    "@type": type.googleapis.com/shared.payload.Payload
    {json_field}: {value}
"#
        );
        let documents = manifest::read_str(&yaml, "seed").unwrap();
        let resolved = manifest::load_with_tenant_types(&mut client, documents)
            .await
            .unwrap();
        let plan = trogon_atlas_client::apply::plan(&mut client, resolved.manifests)
            .await
            .unwrap();
        trogon_atlas_client::apply::apply(&mut client, plan, false, None)
            .await
            .unwrap();
    }
    seed(
        &mut client,
        r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: EventModel
metadata: { namespace: board-types-local, name: shop }
spec:
  title: Payload board
  members:
    - { kind: event, id: payload }
    - { kind: event, id: board-types-foreign/payload }
",
    )
    .await;

    let out = export_board(&mut client, "board-types-local")
        .await
        .unwrap();
    assert!(out.skipped.is_empty(), "{:?}", out.skipped);
    assert!(!out.truncated);
    for (yaml, namespace, field, other_field, value) in [
        (
            &out.manifests,
            "board-types-local",
            "localValue",
            "foreignValue",
            "local payload",
        ),
        (
            &out.external_manifests,
            "board-types-foreign",
            "foreignValue",
            "localValue",
            "foreign payload",
        ),
    ] {
        let documents: Vec<serde_json::Value> = serde_norway::Deserializer::from_str(yaml)
            .map(|doc| serde_json::Value::deserialize(doc).unwrap())
            .collect();
        let event = documents.iter().find(|doc| doc["kind"] == "Event").unwrap();
        assert_eq!(event["metadata"]["namespace"], namespace);
        let schema = &event["spec"]["schema"];
        assert_eq!(
            schema["@type"],
            "type.googleapis.com/shared.payload.Payload"
        );
        assert_eq!(schema[field], value, "{yaml}");
        assert!(schema.get(other_field).is_none(), "{yaml}");
    }
}

#[tokio::test]
async fn board_manifests_keep_same_filename_types_namespace_scoped() {
    assert_board_namespace_types("shared/payload.proto").await;
}

#[tokio::test]
async fn board_manifests_keep_same_package_types_namespace_scoped() {
    assert_board_namespace_types("foreign/payload.proto").await;
}

#[tokio::test]
async fn board_manifests_carry_metadata_only_tenant_types() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    let yaml = r#"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: TypeLibrary
metadata: { namespace: board-annotations-src, name: board.annotations }
spec:
  title: Board annotations
  files:
    - path: board/annotations.proto
      content: |
        syntax = "proto3";
        package board.annotations;
        message Note { string text = 1; }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: EventModel
metadata: { namespace: board-annotations-src, name: shop }
spec:
  title: Annotated board
  metadata:
    - "@type": type.googleapis.com/board.annotations.Note
      text: Board rationale
"#;
    let documents = manifest::read_str(yaml, "seed").unwrap();
    let resolved = manifest::load_with_tenant_types(&mut client, documents)
        .await
        .unwrap();
    let plan = trogon_atlas_client::apply::plan(&mut client, resolved.manifests)
        .await
        .unwrap();
    trogon_atlas_client::apply::apply(&mut client, plan, false, None)
        .await
        .unwrap();

    let out = export_board(&mut client, "board-annotations-src")
        .await
        .unwrap();
    assert!(out.skipped.is_empty(), "{:?}", out.skipped);
    assert!(!out.truncated);
    assert_eq!(out.external_manifests, "");
    let copy = out
        .manifests
        .replace("board-annotations-src", "board-annotations-copy");
    let documents = manifest::read_str(&copy, "export").unwrap();
    let resolved = manifest::load_with_tenant_types(&mut client, documents)
        .await
        .expect("export must carry the library needed to decode tenant metadata");
    assert!(resolved
        .manifests
        .iter()
        .any(|doc| doc.kind == pb::EntityKind::TypeLibrary));
    let plan = trogon_atlas_client::apply::plan(&mut client, resolved.manifests)
        .await
        .unwrap();
    trogon_atlas_client::apply::apply(&mut client, plan, false, None)
        .await
        .unwrap();
    let copied = export_board(&mut client, "board-annotations-copy")
        .await
        .unwrap();
    assert_eq!(copied.manifests, copy);
}

#[tokio::test]
async fn board_manifests_carry_nested_tenant_metadata_types() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    let yaml = r#"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: TypeLibrary
metadata: { namespace: board-nested-src, name: board.outer }
spec:
  title: Outer annotation
  files:
    - path: board/outer.proto
      content: |
        syntax = "proto3";
        package board.outer;
        import "google/protobuf/any.proto";
        message Envelope { google.protobuf.Any detail = 1; }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: TypeLibrary
metadata: { namespace: board-nested-src, name: board.inner }
spec:
  title: Inner annotation
  files:
    - path: board/inner.proto
      content: |
        syntax = "proto3";
        package board.inner;
        message Note { string text = 1; }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: EventModel
metadata: { namespace: board-nested-src, name: shop }
spec:
  title: Nested annotation board
  metadata:
    - "@type": type.googleapis.com/board.outer.Envelope
      detail:
        "@type": type.googleapis.com/board.inner.Note
        text: Nested rationale
"#;
    let documents = manifest::read_str(yaml, "seed").unwrap();
    let resolved = manifest::load_with_tenant_types(&mut client, documents)
        .await
        .unwrap();
    for document in resolved.manifests {
        let plan = trogon_atlas_client::apply::plan(&mut client, vec![document])
            .await
            .unwrap();
        trogon_atlas_client::apply::apply(&mut client, plan, false, None)
            .await
            .unwrap();
    }

    let out = export_board(&mut client, "board-nested-src").await.unwrap();
    assert!(out.skipped.is_empty(), "{:?}", out.skipped);
    assert!(!out.truncated);
    assert_eq!(out.external_manifests, "");
    let copy = out
        .manifests
        .replace("board-nested-src", "board-nested-copy");
    let documents = manifest::read_str(&copy, "export").unwrap();
    let resolved = manifest::load_with_tenant_types(&mut client, documents)
        .await
        .expect("export must carry libraries used inside tenant Any payloads");
    let libraries: std::collections::BTreeSet<_> = resolved
        .manifests
        .iter()
        .filter(|doc| doc.kind == pb::EntityKind::TypeLibrary)
        .map(|doc| doc.id.slug.as_str())
        .collect();
    assert_eq!(
        libraries,
        std::collections::BTreeSet::from(["board.inner", "board.outer"])
    );
    for document in resolved.manifests {
        let plan = trogon_atlas_client::apply::plan(&mut client, vec![document])
            .await
            .unwrap();
        trogon_atlas_client::apply::apply(&mut client, plan, false, None)
            .await
            .unwrap();
    }
    let copied = export_board(&mut client, "board-nested-copy")
        .await
        .unwrap();
    assert_eq!(copied.manifests, copy);
}

#[tokio::test]
async fn namespace_export_refuses_a_capped_listing_instead_of_returning_a_complete_artifact() {
    async fn serve(service: EventModelServiceImpl) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(EventModelServiceServer::new(service))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        format!("http://{addr}")
    }
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let full =
        serve(EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap())
            .await;
    let mut complete = client::connect(&full, None, 30).await.unwrap();
    for slug in ["first", "omitted"] {
        complete
            .put_entity(put_req(
                pb::Entity {
                    kind: Some(pb::entity::Kind::SystemGroup(pb::SystemGroup {
                        id: Some(pb::Id {
                            namespace: "export-groups".into(),
                            slug: slug.into(),
                            version: 1,
                        }),
                        title: slug.into(),
                        scope: pb::SystemGroupScope::Internal as i32,
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                true,
            ))
            .await
            .unwrap();
    }
    let complete_artifact = export::export_namespace(&mut complete, "export-groups")
        .await
        .unwrap();
    assert_eq!(complete_artifact.skipped, [] as [String; 0]);
    let yaml = &complete_artifact.files["system-group.yaml"];
    assert!(yaml.contains("name: first"));
    assert!(yaml.contains("name: omitted"));
    let capped = serve(
        EventModelServiceImpl::try_new(store, ServerMetrics::default())
            .unwrap()
            .with_snapshot_cap(1),
    )
    .await;
    let mut partial = client::connect(&capped, None, 30).await.unwrap();
    let snapshot = partial
        .get_snapshot_id(pb::GetSnapshotIdRequest {
            namespaces: vec!["export-groups".into()],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert!(snapshot.truncated);
    assert_eq!(snapshot.entity_count, 1);
    assert_eq!(
        partial
            .get_entity(pb::GetEntityRequest {
                kind: pb::EntityKind::SystemGroup as i32,
                id: Some(pb::Id {
                    namespace: "export-groups".into(),
                    slug: "omitted".into(),
                    version: 1
                }),
            })
            .await
            .unwrap()
            .into_inner()
            .entity
            .unwrap()
            .kind
            .unwrap(),
        pb::entity::Kind::SystemGroup(pb::SystemGroup {
            id: Some(pb::Id {
                namespace: "export-groups".into(),
                slug: "omitted".into(),
                version: 1
            }),
            title: "omitted".into(),
            scope: pb::SystemGroupScope::Internal as i32,
            ..Default::default()
        })
    );
    for layout in [export::ExportLayout::Single, export::ExportLayout::Slices] {
        let result = match layout {
            export::ExportLayout::Single => {
                export::export_namespace(&mut partial, "export-groups").await
            }
            export::ExportLayout::Slices => {
                export::export_namespace_slices(&mut partial, "export-groups").await
            }
        };
        let Err(error) = result else {
            panic!("a truncated entity listing must not become a complete namespace export")
        };
        assert_eq!(
            error.downcast_ref::<tonic::Status>().unwrap().code(),
            tonic::Code::ResourceExhausted
        );
    }
}
