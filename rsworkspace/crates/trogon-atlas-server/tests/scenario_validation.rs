#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tonic::{Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::service::EventModelServiceImpl;

const NS: &str = "scenario-validation";

fn id(slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: NS.into(),
        slug: slug.into(),
        version,
    }
}

fn event_edge(slug: &str) -> pb::EventEdge {
    pb::EventEdge {
        event: Some(pb::EventRef {
            id: Some(id(slug, 1)),
        }),
        doc: String::new(),
        metadata: Vec::new(),
    }
}

fn command_edge(slug: &str) -> pb::CommandEdge {
    pb::CommandEdge {
        command: Some(pb::CommandRef {
            id: Some(id(slug, 1)),
        }),
        doc: String::new(),
        metadata: Vec::new(),
    }
}

fn ui_edge(slug: &str) -> pb::UiEdge {
    pb::UiEdge {
        ui: Some(pb::UiRef {
            id: Some(id(slug, 1)),
        }),
        doc: String::new(),
        metadata: Vec::new(),
    }
}

fn persona_edge(slug: &str) -> pb::PersonaEdge {
    pb::PersonaEdge {
        persona: Some(pb::PersonaRef {
            id: Some(id(slug, 1)),
        }),
        doc: String::new(),
        metadata: Vec::new(),
    }
}

fn event_example(slug: &str) -> pb::EventExample {
    pb::EventExample {
        event: Some(pb::EventRef {
            id: Some(id(slug, 1)),
        }),
        payload: None,
    }
}

fn command_example(slug: &str) -> pb::CommandExample {
    pb::CommandExample {
        command: Some(pb::CommandRef {
            id: Some(id(slug, 1)),
        }),
        payload: None,
    }
}

fn make_event(slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(slug, 1)),
            title: slug.into(),
            doc: String::new(),
            swimlane: None,
            metadata: Vec::new(),
            supersedes: None,
            schema: None,
        })),
    }
}

fn make_command(slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Command(pb::Command {
            id: Some(id(slug, 1)),
            title: slug.into(),
            doc: String::new(),
            swimlane: None,
            metadata: Vec::new(),
            supersedes: None,
            schema: None,
        })),
    }
}

fn make_persona(slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Persona(pb::Persona {
            id: Some(id(slug, 1)),
            title: slug.into(),
            role: "tester".into(),
            doc: String::new(),
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

fn make_ui(slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Ui(pb::Ui {
            id: Some(id(slug, 1)),
            title: slug.into(),
            doc: String::new(),
            design_ref: String::new(),
            metadata: Vec::new(),
            supersedes: None,
            transitions: Vec::new(),
            slot: None,
        })),
    }
}

async fn seed_basics() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    let basics = vec![
        make_event("order.placed"),
        make_event("order.paid"),
        make_command("place-order"),
        make_command("cancel-order"),
        make_persona("buyer"),
        make_ui("checkout"),
    ];
    for e in basics {
        svc.put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(e),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap();
    }
    svc
}

fn command_slice_with_scenarios(scenarios: Vec<pb::CommandScenario>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id("place-order-slice", 1)),
            title: "place-order-slice".into(),
            doc: String::new(),
            persona: Some(persona_edge("buyer")),
            ui: Some(ui_edge("checkout")),
            command: Some(command_edge("place-order")),
            emitted_events: vec![event_edge("order.placed")],
            scenarios,
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

async fn put_slice(svc: &EventModelServiceImpl, slice: pb::Entity) -> Result<(), tonic::Status> {
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(slice),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .map(|_| ())
}

#[tokio::test(flavor = "multi_thread")]
async fn command_scenario_with_unknown_given_event_is_rejected() {
    let svc = seed_basics().await;
    let scenario = pb::CommandScenario {
        id: "s1".into(),
        title: "happy path".into(),
        doc: String::new(),
        given: vec![event_example("does-not-exist")],
        when: Some(command_example("place-order")),
        then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
            events: vec![event_example("order.placed")],
        })),
        metadata: Vec::new(),
    };
    let err = put_slice(&svc, command_slice_with_scenarios(vec![scenario]))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(
        err.message().contains("SCENARIO_DANGLING_REF"),
        "msg={}",
        err.message()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_scenario_command_must_match_slice_command() {
    let svc = seed_basics().await;
    let scenario = pb::CommandScenario {
        id: "s1".into(),
        title: "mismatch".into(),
        doc: String::new(),
        given: Vec::new(),
        when: Some(command_example("cancel-order")),
        then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
            events: vec![event_example("order.placed")],
        })),
        metadata: Vec::new(),
    };
    let err = put_slice(&svc, command_slice_with_scenarios(vec![scenario]))
        .await
        .unwrap_err();
    assert!(
        err.message().contains("SCENARIO_COMMAND_MISMATCH"),
        "msg={}",
        err.message()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_scenario_duplicate_titles_are_rejected() {
    let svc = seed_basics().await;
    let mk = || pb::CommandScenario {
        id: "s".into(),
        title: "happy".into(),
        doc: String::new(),
        given: Vec::new(),
        when: Some(command_example("place-order")),
        then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
            events: vec![event_example("order.placed")],
        })),
        metadata: Vec::new(),
    };
    let err = put_slice(&svc, command_slice_with_scenarios(vec![mk(), mk()]))
        .await
        .unwrap_err();
    assert!(
        err.message().contains("SCENARIO_DUPLICATE_TITLE"),
        "msg={}",
        err.message()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_scenario_reject_without_reason_is_rejected() {
    let svc = seed_basics().await;
    let scenario = pb::CommandScenario {
        id: "s1".into(),
        title: "unhappy".into(),
        doc: String::new(),
        given: Vec::new(),
        when: Some(command_example("place-order")),
        then: Some(pb::command_scenario::Then::Reject(pb::Rejection {
            reason_code: String::new(),
            doc: "boom".into(),
        })),
        metadata: Vec::new(),
    };
    let err = put_slice(&svc, command_slice_with_scenarios(vec![scenario]))
        .await
        .unwrap_err();
    assert!(
        err.message().contains("SCENARIO_REJECT_NO_REASON"),
        "msg={}",
        err.message()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_scenario_missing_then_is_rejected() {
    let svc = seed_basics().await;
    let scenario = pb::CommandScenario {
        id: "s1".into(),
        title: "no then".into(),
        doc: String::new(),
        given: Vec::new(),
        when: Some(command_example("place-order")),
        then: None,
        metadata: Vec::new(),
    };
    let err = put_slice(&svc, command_slice_with_scenarios(vec![scenario]))
        .await
        .unwrap_err();
    assert!(
        err.message().contains("SCENARIO_MISSING_THEN"),
        "msg={}",
        err.message()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_scenario_clean_is_accepted() {
    let svc = seed_basics().await;
    let scenario = pb::CommandScenario {
        id: "s1".into(),
        title: "ok".into(),
        doc: String::new(),
        given: vec![event_example("order.paid")],
        when: Some(command_example("place-order")),
        then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
            events: vec![event_example("order.placed")],
        })),
        metadata: Vec::new(),
    };
    put_slice(&svc, command_slice_with_scenarios(vec![scenario]))
        .await
        .unwrap();
}

fn make_event_on_lane(slug: &str, lane: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(slug, 1)),
            title: slug.into(),
            swimlane: Some(pb::SwimlaneRef {
                id: Some(id(lane, 1)),
            }),
            ..Default::default()
        })),
    }
}

fn make_lane_with_transitions(slug: &str, after: &str, next: &[&str]) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
            id: Some(id(slug, 1)),
            title: slug.into(),
            transitions: vec![pb::swimlane::Transition {
                after: Some(pb::EventRef {
                    id: Some(id(after, 1)),
                }),
                next: next
                    .iter()
                    .map(|n| pb::swimlane::transition::Next {
                        event: Some(pb::EventRef { id: Some(id(n, 1)) }),
                        doc: String::new(),
                        metadata: Vec::new(),
                    })
                    .collect(),
            }],
            ..Default::default()
        })),
    }
}

/// PutEntity runs synchronous scenario validation on every write. Warnings
/// (e.g. SCENARIO_TRANSITION_VIOLATION) must survive into
/// `PutEntityResponse.validation`: the proto says an empty list means no
/// issues were detected, not that the server discarded them after persist.
#[tokio::test(flavor = "multi_thread")]
async fn put_entity_preserves_scenario_warnings_after_write() {
    let svc = seed_basics().await;
    for e in [
        make_event_on_lane("reserved", "order-lane"),
        make_event_on_lane("claimed", "order-lane"),
        make_event_on_lane("expired", "order-lane"),
        make_lane_with_transitions("order-lane", "reserved", &["claimed", "expired"]),
    ] {
        svc.put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(e),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap();
    }

    // reserved -> reserved is not a legal successor of reserved's table.
    let scenario = pb::CommandScenario {
        id: "s1".into(),
        title: "illegal transition".into(),
        doc: String::new(),
        given: vec![event_example("reserved")],
        when: Some(command_example("place-order")),
        then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
            events: vec![event_example("reserved")],
        })),
        metadata: Vec::new(),
    };
    let resp = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(command_slice_with_scenarios(vec![scenario])),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .expect("warning-only scenario must still persist")
        .into_inner();

    assert!(
        resp.validation.iter().any(|i| {
            i.code == "SCENARIO_TRANSITION_VIOLATION"
                && i.severity == pb::validation_issue::Severity::Warning as i32
        }),
        "PutEntity must return scenario warnings after a successful write; got {:?}",
        resp.validation
    );
}

fn has_transition_warning(issues: &[pb::ValidationIssue]) -> bool {
    issues.iter().any(|i| {
        i.code == "SCENARIO_TRANSITION_VIOLATION"
            && i.severity == pb::validation_issue::Severity::Warning as i32
    })
}

fn put_op(entity: pb::Entity) -> pb::BatchMutateOp {
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

fn delete_op(kind: pb::EntityKind, slug: &str) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
            operation_id: String::new(),
            kind: kind as i32,
            id: Some(id(slug, 1)),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            if_match: String::new(),
        })),
    }
}

fn command_slice_named(slug: &str, scenarios: Vec<pb::CommandScenario>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(slug, 1)),
            title: slug.into(),
            doc: String::new(),
            persona: Some(persona_edge("buyer")),
            ui: Some(ui_edge("checkout")),
            command: Some(command_edge("place-order")),
            emitted_events: vec![event_edge("order.placed")],
            scenarios,
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

fn illegal_transition_scenario() -> pb::CommandScenario {
    pb::CommandScenario {
        id: "s1".into(),
        title: "illegal transition".into(),
        doc: String::new(),
        given: vec![event_example("reserved")],
        when: Some(command_example("place-order")),
        then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
            events: vec![event_example("reserved")],
        })),
        metadata: Vec::new(),
    }
}

async fn seed_order_lane(svc: &EventModelServiceImpl) {
    for e in [
        make_event_on_lane("reserved", "order-lane"),
        make_event_on_lane("claimed", "order-lane"),
        make_event_on_lane("expired", "order-lane"),
        make_lane_with_transitions("order-lane", "reserved", &["claimed", "expired"]),
    ] {
        let _ = svc
            .put_entity(Request::new(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(e),
                create_only: true,
                if_match: String::new(),
                validate_only: false,
                force: false,
            }))
            .await;
    }
}

fn put_validation(resp: &pb::BatchMutateResponse, op_index: usize) -> &[pb::ValidationIssue] {
    match resp.results.get(op_index).and_then(|r| r.result.as_ref()) {
        Some(pb::batch_mutate_op_result::Result::Put(p)) => p.validation.as_slice(),
        other => panic!("expected Put result at op {op_index}, got {other:?}"),
    }
}

/// BatchMutate Applied path must surface per-op scenario warnings on
/// `PutEntityResponse.validation`, same contract as single PutEntity.
#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_preserves_scenario_warnings_after_write() {
    let svc = seed_basics().await;
    seed_order_lane(&svc).await;

    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![put_op(command_slice_named(
                "batch-warn-slice",
                vec![illegal_transition_scenario()],
            ))],
            validate_only: false,
        }))
        .await
        .expect("warning-only batch must apply")
        .into_inner();

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
    assert!(
        has_transition_warning(put_validation(&resp, 0)),
        "BatchMutate Applied must return scenario warnings on the put result; got {:?}",
        put_validation(&resp, 0)
    );
}

/// Dry-run BatchMutate (`validate_only`) must also return scenario warnings
/// on each Put result, the same as PutEntity validate_only already does.
#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_validate_only_preserves_scenario_warnings() {
    let svc = seed_basics().await;
    seed_order_lane(&svc).await;

    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![put_op(command_slice_named(
                "batch-dry-slice",
                vec![illegal_transition_scenario()],
            ))],
            validate_only: true,
        }))
        .await
        .expect("warning-only dry-run must validate")
        .into_inner();

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Validated as i32
    );
    assert!(
        has_transition_warning(put_validation(&resp, 0)),
        "BatchMutate validate_only must return scenario warnings; got {:?}",
        put_validation(&resp, 0)
    );
}

/// Mixed batch (delete + put): warnings attach to the put op, not zeroed.
#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_mixed_ops_preserve_put_scenario_warnings() {
    let svc = seed_basics().await;
    seed_order_lane(&svc).await;

    // Disposable event solely so the delete half of the mixed batch is real.
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("batch-mixed-temp")),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![
                delete_op(pb::EntityKind::Event, "batch-mixed-temp"),
                put_op(command_slice_named(
                    "batch-mixed-slice",
                    vec![illegal_transition_scenario()],
                )),
            ],
            validate_only: false,
        }))
        .await
        .expect("mixed warning batch must apply")
        .into_inner();

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
    assert_eq!(resp.results.len(), 2);
    assert!(
        has_transition_warning(put_validation(&resp, 1)),
        "mixed-ops BatchMutate must keep warnings on the put op; got {:?}",
        put_validation(&resp, 1)
    );
}

/// Scenario errors on a non-zero op must set `failed_op_index` to that op,
/// not silently hardcode 0.
#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_scenario_error_reports_correct_failed_op_index() {
    let svc = seed_basics().await;

    let dangling = pb::CommandScenario {
        id: "s1".into(),
        title: "dangling".into(),
        doc: String::new(),
        given: vec![event_example("does-not-exist")],
        when: Some(command_example("place-order")),
        then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
            events: vec![event_example("order.placed")],
        })),
        metadata: Vec::new(),
    };

    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![
                put_op(make_event("batch-idx-ok")),
                put_op(command_slice_named("batch-idx-slice", vec![dangling])),
            ],
            validate_only: false,
        }))
        .await
        .expect("scenario failure is a Failed response, not gRPC error")
        .into_inner();

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Failed as i32
    );
    assert!(
        resp.failure
            .iter()
            .any(|i| i.code == "SCENARIO_DANGLING_REF"),
        "failure should carry SCENARIO_DANGLING_REF; got {:?}",
        resp.failure
    );
    assert_eq!(
        resp.failed_op_index, 1,
        "failed_op_index must point at the slice op with the scenario error, not 0"
    );
}

/// Branch-scoped BatchMutate shares build_op_results; warnings must survive.
#[tokio::test(flavor = "multi_thread")]
async fn branch_batch_mutate_preserves_scenario_warnings() {
    let svc = seed_basics().await;
    seed_order_lane(&svc).await;

    svc.create_branch(Request::new(pb::CreateBranchRequest {
        name: "batch-warn-branch".into(),
        doc: String::new(),
    }))
    .await
    .unwrap();

    let mut req = Request::new(pb::BatchMutateRequest {
        operation_id: String::new(),
        ops: vec![put_op(command_slice_named(
            "batch-branch-slice",
            vec![illegal_transition_scenario()],
        ))],
        validate_only: false,
    });
    req.metadata_mut().insert(
        "x-trogon-atlas-branch",
        "batch-warn-branch".parse().unwrap(),
    );

    let resp = svc
        .batch_mutate(req)
        .await
        .expect("branch warning batch must apply")
        .into_inner();

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
    assert!(
        has_transition_warning(put_validation(&resp, 0)),
        "branch BatchMutate must return scenario warnings; got {:?}",
        put_validation(&resp, 0)
    );
}
