#![allow(clippy::unwrap_used, clippy::expect_used)]

// export_namespace: live store -> manifest YAML. Covers the grouping/sort/
// multi-doc-render loop (only exercised end-to-end via
// apply_integration.rs::export_round_trips_applied_manifests before this
// file existed) plus the two branches that loop leaves untouched: the
// skipped-entity report for un-renderable payloads, empty-namespace error,
// multi-kind file grouping, and metadata.version omission when version==1.

use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_client::{client, export, manifest};
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
            "bounded-context",
            K::BoundedContext(pb::BoundedContext {
                id: Some(id(ns, "bounded-context")),
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
