#![allow(clippy::vec_init_then_push, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::service::EventModelServiceImpl;

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

fn eref(kind: pb::EntityKind, ns: &str, slug: &str, version: u64) -> pb::EntityRef {
    pb::EntityRef {
        kind: kind as i32,
        id: Some(id(ns, slug, version)),
    }
}

fn field(name: &str, kind: pb::field_type::Kind) -> pb::FieldSpec {
    pb::FieldSpec {
        name: name.into(),
        r#type: Some(pb::FieldType { kind: Some(kind) }),
        repeated: false,
        optional: false,
        doc: String::new(),
        metadata: Vec::new(),
    }
}

/// Same as `field`, but tags the field with a `DerivedFieldAnnotation` so
/// validation's `EVENT_FIELD_UNSOURCED` rule treats it as system-generated.
/// Used for timestamps that the event materializes at emit time rather
/// than carrying from the command's input payload.
fn derived_field(name: &str, kind: pb::field_type::Kind, doc: &str) -> pb::FieldSpec {
    use prost::Message;
    let annotation = pb::DerivedFieldAnnotation {
        doc: doc.into(),
        kind: 0,
    };
    let any = prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.DerivedFieldAnnotation"
            .into(),
        value: annotation.encode_to_vec(),
    };
    pb::FieldSpec {
        name: name.into(),
        r#type: Some(pb::FieldType { kind: Some(kind) }),
        repeated: false,
        optional: false,
        doc: String::new(),
        metadata: vec![any],
    }
}

fn mixed_surface_annotation(doc: &str) -> prost_types::Any {
    use prost::Message;
    let annotation = pb::MixedSurfaceAnnotation { doc: doc.into() };
    prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.MixedSurfaceAnnotation"
            .into(),
        value: annotation.encode_to_vec(),
    }
}

fn uuid_kind() -> pb::field_type::Kind {
    pb::field_type::Kind::Uuid(pb::field_type::UuidType { version: 0 })
}
fn string_kind() -> pb::field_type::Kind {
    pb::field_type::Kind::String(pb::field_type::StringType {})
}
fn int_kind() -> pb::field_type::Kind {
    pb::field_type::Kind::Int(pb::field_type::IntType {})
}
fn timestamp_kind() -> pb::field_type::Kind {
    pb::field_type::Kind::Timestamp(pb::field_type::TimestampType {})
}

const NS: &str = "ecommerce";

fn put(entity: pb::Entity) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(entity),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        })),
    }
}

fn build_swimlane(slug: &str, title: &str, order: u32) -> pb::Entity {
    build_swimlane_with_state(slug, title, order, Vec::new())
}

fn build_swimlane_with_state(
    slug: &str,
    title: &str,
    order: u32,
    state: Vec<pb::FieldSpec>,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
            id: Some(id(NS, slug, 1)),
            title: title.into(),
            doc: String::new(),
            order,
            color: String::new(),
            metadata: Vec::new(),
            supersedes: None,
            stream_id: String::new(),
            transitions: Vec::new(),
            state,
        })),
    }
}

fn build_persona(slug: &str, title: &str, role: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Persona(pb::Persona {
            id: Some(id(NS, slug, 1)),
            title: title.into(),
            role: role.into(),
            doc: String::new(),
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

fn build_event(slug: &str, swimlane_slug: &str, fields: Vec<pb::FieldSpec>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            swimlane: Some(pb::SwimlaneRef {
                id: Some(id(NS, swimlane_slug, 1)),
            }),
            metadata: Vec::new(),
            supersedes: None,
            schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
        })),
    }
}

fn build_command(slug: &str, swimlane_slug: &str, fields: Vec<pb::FieldSpec>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Command(pb::Command {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            swimlane: Some(pb::SwimlaneRef {
                id: Some(id(NS, swimlane_slug, 1)),
            }),
            metadata: Vec::new(),
            supersedes: None,
            schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
        })),
    }
}

fn build_read_model(
    slug: &str,
    fields: Vec<pb::FieldSpec>,
    source_events: Vec<&str>,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            source_events: source_events
                .into_iter()
                .map(|s| pb::EventRef {
                    id: Some(id(NS, s, 1)),
                })
                .collect(),
            external_source: None,
            metadata: Vec::new(),
            supersedes: None,
            schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
            key: Vec::new(),
        })),
    }
}

fn build_ui(slug: &str) -> pb::Entity {
    build_ui_with_metadata(slug, Vec::new())
}

fn build_ui_with_metadata(slug: &str, metadata: Vec<prost_types::Any>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Ui(pb::Ui {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            design_ref: String::new(),
            metadata,
            supersedes: None,
            transitions: Vec::new(),
            slot: None,
        })),
    }
}

fn build_processor(slug: &str, trigger: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Processor(pb::Processor {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            doc_trigger: trigger.into(),
            metadata: Vec::new(),
            supersedes: None,
            calls: Vec::new(),
        })),
    }
}

fn event_edge(slug: &str) -> pb::EventEdge {
    pb::EventEdge {
        event: Some(pb::EventRef {
            id: Some(id(NS, slug, 1)),
        }),
        doc: String::new(),
        metadata: Vec::new(),
    }
}

fn command_edge(slug: &str) -> pb::CommandEdge {
    pb::CommandEdge {
        command: Some(pb::CommandRef {
            id: Some(id(NS, slug, 1)),
        }),
        doc: String::new(),
        metadata: Vec::new(),
    }
}

fn ui_edge(slug: &str) -> pb::UiEdge {
    pb::UiEdge {
        ui: Some(pb::UiRef {
            id: Some(id(NS, slug, 1)),
        }),
        doc: String::new(),
        metadata: Vec::new(),
    }
}

fn persona_edge(slug: &str) -> pb::PersonaEdge {
    pb::PersonaEdge {
        persona: Some(pb::PersonaRef {
            id: Some(id(NS, slug, 1)),
        }),
        doc: String::new(),
        metadata: Vec::new(),
    }
}

fn read_model_edge(slug: &str) -> pb::ReadModelEdge {
    pb::ReadModelEdge {
        read_model: Some(pb::ReadModelRef {
            id: Some(id(NS, slug, 1)),
        }),
        doc: String::new(),
        metadata: Vec::new(),
    }
}

fn processor_edge(slug: &str) -> pb::ProcessorEdge {
    pb::ProcessorEdge {
        processor: Some(pb::ProcessorRef {
            id: Some(id(NS, slug, 1)),
        }),
        doc: String::new(),
        metadata: Vec::new(),
    }
}

fn build_command_slice(
    slug: &str,
    persona: &str,
    ui: &str,
    command: &str,
    emitted: Vec<&str>,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            persona: Some(persona_edge(persona)),
            ui: Some(ui_edge(ui)),
            command: Some(command_edge(command)),
            emitted_events: emitted.into_iter().map(event_edge).collect(),
            scenarios: Vec::new(),
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

fn build_command_handler_slice(slug: &str, command: &str, emitted: Vec<&str>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            command: Some(command_edge(command)),
            emitted_events: emitted.into_iter().map(event_edge).collect(),
            scenarios: Vec::new(),
            metadata: Vec::new(),
            supersedes: None,
            persona: None,
            ui: None,
        })),
    }
}

fn build_read_model_slice(slug: &str, source_events: Vec<&str>, read_model: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            source_events: source_events.into_iter().map(event_edge).collect(),
            read_model: Some(read_model_edge(read_model)),
            scenarios: Vec::new(),
            metadata: Vec::new(),
            supersedes: None,
            projection_role: pb::ProjectionRole::Unspecified as i32,
        })),
    }
}

fn build_ui_slice(
    slug: &str,
    persona: &str,
    source_read_models: Vec<&str>,
    ui: &str,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::UiSlice(pb::UiSlice {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            persona: Some(persona_edge(persona)),
            source_read_models: source_read_models
                .into_iter()
                .map(read_model_edge)
                .collect(),
            ui: Some(ui_edge(ui)),
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

fn build_automation_slice(
    slug: &str,
    source_read_models: Vec<&str>,
    processor: &str,
    emitted_command: &str,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            source_read_models: source_read_models
                .into_iter()
                .map(read_model_edge)
                .collect(),
            processor: Some(processor_edge(processor)),
            emitted_command: Some(command_edge(emitted_command)),
            scenarios: Vec::new(),
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

fn build_storyboard(slug: &str, slices: Vec<&str>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            entry: Some(pb::StoryboardEntry {
                read_model: Some(read_model_edge("orders.list")),
                observer: Some(pb::storyboard_entry::Observer::Human(
                    pb::storyboard_entry::HumanObserver {
                        persona: Some(persona_edge("billy-the-customer")),
                        ui: Some(ui_edge("cart")),
                    },
                )),
            }),
            outcome: Some(pb::StoryboardOutcome {
                events: vec![event_edge("order.fulfilled")],
                ui: Some(ui_edge("admin-fulfillment")),
            }),
            slices: slices
                .into_iter()
                .map(|s| pb::SliceRef {
                    id: Some(id(NS, s, 1)),
                })
                .collect(),
            traces: vec![
                pb::StoryboardTrace {
                    id: "happy-path".into(),
                    title: "happy path".into(),
                    doc: "order placed -> payment captured -> fulfilled".into(),
                    data: None,
                    initial_observed: None,
                    steps: Vec::new(),
                },
                pb::StoryboardTrace {
                    id: "payment-declined".into(),
                    title: "payment declined".into(),
                    doc: "order placed -> capture fails; order never fulfilled".into(),
                    data: None,
                    initial_observed: None,
                    steps: Vec::new(),
                },
            ],
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

// A monitoring narrative: a persona WATCHES a screen without acting on it.
// This is the only way today to attribute a display-only screen to a
// persona: ReadModelSlice carries `ui` but no persona field, and the
// command behind this screen is automation-issued, so a CommandSlice with
// persona + ui would trip COMMAND_MULTIPLE_ISSUERS. Carries no slices: the
// write-side slices belong to the `order-to-fulfillment` narrative, and
// claiming them twice is STORYBOARD_SHARED_SLICE.
fn build_monitoring_storyboard(
    slug: &str,
    persona: &str,
    ui: &str,
    read_model: &str,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: "Manager watches the fulfillment queue".into(),
            entry: Some(pb::StoryboardEntry {
                read_model: Some(read_model_edge(read_model)),
                observer: Some(pb::storyboard_entry::Observer::Human(
                    pb::storyboard_entry::HumanObserver {
                        persona: Some(persona_edge(persona)),
                        ui: Some(ui_edge(ui)),
                    },
                )),
            }),
            outcome: Some(pb::StoryboardOutcome {
                events: vec![event_edge("order.fulfilled")],
                ui: Some(ui_edge(ui)),
            }),
            slices: Vec::new(),
            traces: Vec::new(),
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

fn build_event_model(slug: &str, members: Vec<(pb::EntityKind, &str)>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id(NS, slug, 1)),
            title: slug.into(),
            doc: "End-to-end e-commerce model".into(),
            members: members
                .into_iter()
                .map(|(k, s)| eref(k, NS, s, 1))
                .collect(),
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

fn ecommerce_ops() -> Vec<pb::BatchMutateOp> {
    let mut ops = Vec::new();

    ops.push(put(build_swimlane_with_state(
        "order",
        "Order",
        1,
        vec![field("status", string_kind())],
    )));
    ops.push(put(build_swimlane("payment", "Payment", 2)));

    ops.push(put(build_persona("bob-the-manager", "Bob", "manager")));
    ops.push(put(build_persona(
        "billy-the-customer",
        "Billy",
        "customer",
    )));

    ops.push(put(build_event(
        "order.placed",
        "order",
        vec![
            field("order_id", uuid_kind()),
            field("customer_id", uuid_kind()),
            field("total_cents", int_kind()),
            derived_field(
                "placed_at",
                timestamp_kind(),
                "Materialized by the system at event emission time.",
            ),
        ],
    )));
    ops.push(put(build_event(
        "payment.captured",
        "payment",
        vec![
            field("order_id", uuid_kind()),
            field("amount_cents", int_kind()),
            derived_field(
                "captured_at",
                timestamp_kind(),
                "Materialized by the system at event emission time.",
            ),
        ],
    )));
    ops.push(put(build_event(
        "order.fulfilled",
        "order",
        vec![
            field("order_id", uuid_kind()),
            derived_field(
                "fulfilled_at",
                timestamp_kind(),
                "Materialized by the system at event emission time.",
            ),
        ],
    )));

    ops.push(put(build_command(
        "place-order",
        "order",
        vec![
            field("order_id", uuid_kind()),
            field("customer_id", uuid_kind()),
            field("total_cents", int_kind()),
        ],
    )));
    ops.push(put(build_command(
        "capture-payment",
        "payment",
        vec![
            field("order_id", uuid_kind()),
            field("amount_cents", int_kind()),
        ],
    )));
    ops.push(put(build_command(
        "fulfill-order",
        "order",
        vec![field("order_id", uuid_kind())],
    )));

    ops.push(put(build_read_model(
        "orders.list",
        vec![
            field("order_id", uuid_kind()),
            field("customer_id", uuid_kind()),
            field("total_cents", int_kind()),
            field("status", string_kind()),
            field("placed_at", timestamp_kind()),
            field("amount_cents", int_kind()),
            field("captured_at", timestamp_kind()),
            field("fulfilled_at", timestamp_kind()),
        ],
        vec!["order.placed", "payment.captured", "order.fulfilled"],
    )));
    ops.push(put(build_read_model(
        "orders.awaiting-payment",
        vec![
            field("order_id", uuid_kind()),
            field("amount_cents", int_kind()),
            field("customer_id", uuid_kind()),
            field("total_cents", int_kind()),
            field("status", string_kind()),
            field("placed_at", timestamp_kind()),
            field("captured_at", timestamp_kind()),
        ],
        vec!["order.placed", "payment.captured"],
    )));
    ops.push(put(build_read_model(
        "orders.awaiting-fulfillment",
        vec![
            field("order_id", uuid_kind()),
            field("amount_cents", int_kind()),
            field("captured_at", timestamp_kind()),
            field("status", string_kind()),
            field("fulfilled_at", timestamp_kind()),
        ],
        vec!["payment.captured", "order.fulfilled"],
    )));

    ops.push(put(build_ui("cart")));
    ops.push(put(build_ui_with_metadata(
        "admin-fulfillment",
        vec![mixed_surface_annotation(
            "Admin fulfillment displays order state and hosts operational actions for the same work queue.",
        )],
    )));

    ops.push(put(build_processor(
        "payment-capture-processor",
        "fires when an order is placed and no payment has been captured yet",
    )));
    ops.push(put(build_processor(
        "fulfillment-notifier",
        "fires when payment is captured and order has not been fulfilled",
    )));

    ops.push(put(build_command_slice(
        "place-order-slice",
        "billy-the-customer",
        "cart",
        "place-order",
        vec!["order.placed"],
    )));
    ops.push(put(build_command_handler_slice(
        "capture-payment-slice",
        "capture-payment",
        vec!["payment.captured"],
    )));
    ops.push(put(build_command_handler_slice(
        "fulfill-order-slice",
        "fulfill-order",
        vec!["order.fulfilled"],
    )));

    ops.push(put(build_read_model_slice(
        "orders-list-slice",
        vec!["order.placed"],
        "orders.list",
    )));
    ops.push(put(build_read_model_slice(
        "awaiting-payment-slice",
        vec!["order.placed"],
        "orders.awaiting-payment",
    )));
    ops.push(put(build_read_model_slice(
        "orders-list-paid-slice",
        vec!["payment.captured"],
        "orders.list",
    )));
    ops.push(put(build_read_model_slice(
        "awaiting-payment-cleared-slice",
        vec!["payment.captured"],
        "orders.awaiting-payment",
    )));
    ops.push(put(build_read_model_slice(
        "awaiting-fulfillment-slice",
        vec!["payment.captured"],
        "orders.awaiting-fulfillment",
    )));
    ops.push(put(build_read_model_slice(
        "orders-list-fulfilled-slice",
        vec!["order.fulfilled"],
        "orders.list",
    )));
    ops.push(put(build_read_model_slice(
        "awaiting-fulfillment-cleared-slice",
        vec!["order.fulfilled"],
        "orders.awaiting-fulfillment",
    )));

    ops.push(put(build_ui_slice(
        "admin-fulfillment-ui",
        "bob-the-manager",
        vec![
            "orders.list",
            "orders.awaiting-payment",
            "orders.awaiting-fulfillment",
        ],
        "admin-fulfillment",
    )));

    ops.push(put(build_automation_slice(
        "auto-capture-payment",
        vec!["orders.awaiting-payment"],
        "payment-capture-processor",
        "capture-payment",
    )));
    ops.push(put(build_automation_slice(
        "auto-fulfill-order",
        vec!["orders.awaiting-fulfillment"],
        "fulfillment-notifier",
        "fulfill-order",
    )));

    ops.push(put(build_monitoring_storyboard(
        "fulfillment-monitoring",
        "bob-the-manager",
        "admin-fulfillment",
        "orders.awaiting-fulfillment",
    )));

    ops.push(put(build_storyboard(
        "order-to-fulfillment",
        vec![
            "place-order-slice",
            "orders-list-slice",
            "awaiting-payment-slice",
            "auto-capture-payment",
            "capture-payment-slice",
            "orders-list-paid-slice",
            "awaiting-payment-cleared-slice",
            "awaiting-fulfillment-slice",
            "auto-fulfill-order",
            "fulfill-order-slice",
            "orders-list-fulfilled-slice",
            "awaiting-fulfillment-cleared-slice",
            "admin-fulfillment-ui",
        ],
    )));

    ops.push(put(build_event_model(
        "ecommerce",
        vec![
            (pb::EntityKind::Swimlane, "order"),
            (pb::EntityKind::Swimlane, "payment"),
            (pb::EntityKind::Persona, "bob-the-manager"),
            (pb::EntityKind::Persona, "billy-the-customer"),
            (pb::EntityKind::Event, "order.placed"),
            (pb::EntityKind::Event, "payment.captured"),
            (pb::EntityKind::Event, "order.fulfilled"),
            (pb::EntityKind::Command, "place-order"),
            (pb::EntityKind::Command, "capture-payment"),
            (pb::EntityKind::Command, "fulfill-order"),
            (pb::EntityKind::ReadModel, "orders.list"),
            (pb::EntityKind::ReadModel, "orders.awaiting-payment"),
            (pb::EntityKind::ReadModel, "orders.awaiting-fulfillment"),
            (pb::EntityKind::Ui, "cart"),
            (pb::EntityKind::Ui, "admin-fulfillment"),
            (pb::EntityKind::Processor, "payment-capture-processor"),
            (pb::EntityKind::Processor, "fulfillment-notifier"),
            (pb::EntityKind::CommandSlice, "place-order-slice"),
            (pb::EntityKind::CommandSlice, "capture-payment-slice"),
            (pb::EntityKind::CommandSlice, "fulfill-order-slice"),
            (pb::EntityKind::ReadModelSlice, "orders-list-slice"),
            (pb::EntityKind::ReadModelSlice, "awaiting-payment-slice"),
            (pb::EntityKind::ReadModelSlice, "orders-list-paid-slice"),
            (
                pb::EntityKind::ReadModelSlice,
                "awaiting-payment-cleared-slice",
            ),
            (pb::EntityKind::ReadModelSlice, "awaiting-fulfillment-slice"),
            (
                pb::EntityKind::ReadModelSlice,
                "orders-list-fulfilled-slice",
            ),
            (
                pb::EntityKind::ReadModelSlice,
                "awaiting-fulfillment-cleared-slice",
            ),
            (pb::EntityKind::UiSlice, "admin-fulfillment-ui"),
            (pb::EntityKind::AutomationSlice, "auto-capture-payment"),
            (pb::EntityKind::AutomationSlice, "auto-fulfill-order"),
            (pb::EntityKind::Storyboard, "order-to-fulfillment"),
            (pb::EntityKind::Storyboard, "fulfillment-monitoring"),
        ],
    )));

    ops
}

async fn seed_service() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: ecommerce_ops(),
            validate_only: false,
        }))
        .await
        .expect("batch_mutate failed")
        .into_inner();
    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "fixture seeding must succeed; failure: {:?}",
        resp.failure
    );
    svc
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_seeds_full_model() {
    let svc = seed_service().await;
    let list = svc
        .list_entities(Request::new(pb::ListEntitiesRequest {
            kinds: Vec::new(),
            namespaces: vec![NS.into()],
            latest_versions_only: false,
            page_size: 500,
            page_token: String::new(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(list.entities.len(), 33);
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_tracker_round_trip() {
    // Project management overlays the model: a Tracker points at slices
    // from the outside; the slices themselves carry no status.
    let svc = seed_service().await;
    let entity = pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Tracker(pb::Tracker {
            id: Some(id(NS, "mvp", 1)),
            title: "MVP buildout".into(),
            doc: String::new(),
            items: vec![pb::tracker::Item {
                subject: Some(eref(
                    pb::EntityKind::CommandSlice,
                    NS,
                    "place-order-slice",
                    1,
                )),
                status: pb::TrackStatus::InProgress as i32,
                doc: String::new(),
                metadata: Vec::new(),
            }],
            metadata: Vec::new(),
            supersedes: None,
        })),
    };
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();
    let resp = svc
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Tracker as i32,
            id: Some(id(NS, "mvp", 1)),
        }))
        .await
        .unwrap()
        .into_inner();
    match resp.entity.expect("entity").kind.expect("kind") {
        pb::entity::Kind::Tracker(t) => {
            assert_eq!(t.items.len(), 1);
            assert_eq!(t.items[0].status, pb::TrackStatus::InProgress as i32);
        }
        _ => panic!("expected Tracker"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_projection_event_model() {
    let svc = seed_service().await;
    let resp = svc
        .get_event_model_projection(Request::new(pb::GetEventModelProjectionRequest {
            id: Some(id(NS, "ecommerce", 1)),
            include_cross_model: false,
        }))
        .await
        .unwrap()
        .into_inner();
    let projection = resp.projection.expect("projection present");
    assert!(projection.entities.len() >= 23);
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_impact_from_order_placed() {
    let svc = seed_service().await;
    let resp = svc
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(eref(pb::EntityKind::Event, NS, "order.placed", 1)),
            max_depth: 0,
            filter_kinds: Vec::new(),
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(resp.nodes.len() > 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_search_finds_capture_payment() {
    let svc = seed_service().await;
    let resp = svc
        .search_entities(Request::new(pb::SearchEntitiesRequest {
            query: "capture".into(),
            kinds: Vec::new(),
            namespaces: vec![NS.into()],
            limit: 50,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(resp.results, [] as [trogon_atlas_proto::SearchResult; 0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_infer_data_flow_command_slice() {
    let svc = seed_service().await;
    let scope = pb::AnalysisScope {
        scope: Some(pb::analysis_scope::Scope::Slice(eref(
            pb::EntityKind::CommandSlice,
            NS,
            "place-order-slice",
            1,
        ))),
    };
    let resp = svc
        .infer_data_flow(Request::new(pb::InferDataFlowRequest {
            scope: Some(scope),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(
        !resp.mappings.is_empty(),
        "command slice should yield mappings"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_completeness_check_event_model() {
    let svc = seed_service().await;
    let scope = pb::AnalysisScope {
        scope: Some(pb::analysis_scope::Scope::EventModel(eref(
            pb::EntityKind::EventModel,
            NS,
            "ecommerce",
            1,
        ))),
    };
    let resp = svc
        .check_information_completeness(Request::new(pb::CheckInformationCompletenessRequest {
            scope: Some(scope),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(resp.overall_score >= 0.0 && resp.overall_score <= 1.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_validate_event_model_clean() {
    let svc = seed_service().await;
    let resp = svc
        .validate_event_model(Request::new(pb::ValidateEventModelRequest {
            event_model: None,
            event_model_id: Some(id(NS, "ecommerce", 1)),
        }))
        .await
        .unwrap()
        .into_inner();
    let errors = resp
        .issues
        .iter()
        .filter(|i| i.severity == pb::validation_issue::Severity::Error as i32)
        .count();
    assert_eq!(
        errors, 0,
        "fixture model should validate clean: {:?}",
        resp.issues
    );
}
