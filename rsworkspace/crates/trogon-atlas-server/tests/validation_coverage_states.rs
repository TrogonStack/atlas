#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_server::EventModelService, validate_project_request::Scope,
    validation_check_result::State, validation_issue::Severity,
};
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn service() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap()
}

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn event(namespace: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(namespace, slug)),
            title: format!("{namespace}/{slug}"),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn member(kind: pb::EntityKind, id: pb::Id) -> pb::EntityRef {
    pb::EntityRef {
        kind: kind as i32,
        id: Some(id),
    }
}

fn event_model(namespace: &str, slug: &str, members: Vec<pb::EntityRef>) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id(namespace, slug)),
            title: format!("{namespace}/{slug}"),
            members,
            ..Default::default()
        })),
        ..Default::default()
    }
}

async fn put(service: &EventModelServiceImpl, entity: pb::Entity) {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity),
            ..Default::default()
        }))
        .await
        .unwrap();
}

async fn validate_model_by_id(
    service: &EventModelServiceImpl,
    model: pb::Id,
) -> pb::ValidateEventModelResponse {
    service
        .validate_event_model(Request::new(pb::ValidateEventModelRequest {
            event_model: None,
            event_model_id: Some(model),
        }))
        .await
        .unwrap()
        .into_inner()
}

fn check<'a>(
    coverage: &'a pb::ValidationCoverage,
    check_id: &str,
) -> &'a pb::ValidationCheckResult {
    coverage
        .checks
        .iter()
        .find(|c| c.check_id == check_id)
        .unwrap_or_else(|| panic!("missing {check_id} in {coverage:?}"))
}

fn assert_check(
    coverage: &pb::ValidationCoverage,
    check_id: &str,
    state: State,
    reason: &str,
    subjects_examined: i32,
) {
    let result = check(coverage, check_id);
    assert_eq!(
        (
            result.state,
            result.reason.as_str(),
            result.subjects_examined
        ),
        (state as i32, reason, subjects_examined),
        "{check_id}: {coverage:?}"
    );
}

fn model_scope(model: &pb::Id) -> pb::ValidationScope {
    pb::ValidationScope {
        branch: String::new(),
        project_id: None,
        domain_id: None,
        namespace: model.namespace.clone(),
        event_model_id: Some(model.clone()),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn completed_clean_validation_reports_no_findings() {
    let service = service().await;
    let model = id("vcs-clean", "checkout");
    put(&service, event_model("vcs-clean", "checkout", Vec::new())).await;

    let response = validate_model_by_id(&service, model).await;

    assert!(response.issues.is_empty(), "{:?}", response.issues);
    let coverage = response.coverage.expect("coverage required");
    assert_check(&coverage, "model", State::Completed, "", 1);
    assert_check(&coverage, "strategic", State::Completed, "", 1);
    assert!(!coverage.snapshot_truncated);
}

#[tokio::test(flavor = "multi_thread")]
async fn completed_check_still_reports_its_error_findings() {
    let service = service().await;
    let model = id("vcs-failing", "checkout");
    put(
        &service,
        event_model(
            "vcs-failing",
            "checkout",
            vec![member(pb::EntityKind::Event, id("vcs-failing", "missing"))],
        ),
    )
    .await;

    let response = validate_model_by_id(&service, model).await;

    assert!(
        response
            .issues
            .iter()
            .any(|i| i.code == "MEMBER_NOT_FOUND" && i.severity == Severity::Error as i32),
        "{:?}",
        response.issues
    );
    let coverage = response.coverage.expect("coverage required");
    assert_check(&coverage, "model", State::Completed, "", 1);
    assert!(!coverage.snapshot_truncated);
}

#[tokio::test(flavor = "multi_thread")]
async fn skipped_check_names_why_it_did_not_run() {
    let service = service().await;
    let response = service
        .validate_event_model(Request::new(pb::ValidateEventModelRequest {
            event_model: Some(pb::EventModel {
                id: None,
                title: "inline".into(),
                ..Default::default()
            }),
            event_model_id: None,
        }))
        .await
        .unwrap()
        .into_inner();

    let coverage = response.coverage.expect("coverage required");
    assert_check(&coverage, "model", State::Completed, "", 1);
    assert_check(
        &coverage,
        "strategic",
        State::Skipped,
        "event_model_id_absent",
        0,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn truncated_snapshot_reports_partial_checks() {
    let service = service().await.with_snapshot_cap(1);
    let model = id("vcs-partial", "checkout");
    put(&service, event("vcs-partial", "placed")).await;
    put(&service, event_model("vcs-partial", "checkout", Vec::new())).await;

    let coverage = validate_model_by_id(&service, model)
        .await
        .coverage
        .expect("coverage required");
    assert!(coverage.snapshot_truncated, "{coverage:?}");
    assert_check(&coverage, "model", State::Partial, "snapshot_truncated", 1);
    assert_check(
        &coverage,
        "strategic",
        State::Partial,
        "snapshot_truncated",
        1,
    );

    let coverage = service
        .validate_project(Request::new(pb::ValidateProjectRequest {
            scope: Some(Scope::Namespace("vcs-partial".into())),
        }))
        .await
        .unwrap()
        .into_inner()
        .coverage
        .expect("coverage required");
    assert!(coverage.snapshot_truncated, "{coverage:?}");
    for check_id in ["store_hygiene", "architecture", "model", "strategic"] {
        let result = check(&coverage, check_id);
        assert_eq!(
            (result.state, result.reason.as_str()),
            (State::Partial as i32, "snapshot_truncated"),
            "{check_id}: {coverage:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unresolvable_event_model_is_a_status_error_not_coverage() {
    let service = service().await;
    let err = service
        .validate_event_model(Request::new(pb::ValidateEventModelRequest {
            event_model: None,
            event_model_id: Some(id("vcs-error", "absent")),
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::NotFound, "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn single_model_validation_does_not_claim_its_namespace() {
    let service = service().await;
    let model = id("vcs-scope", "checkout");
    put(&service, event("vcs-scope", "placed")).await;
    put(&service, event("vcs-scope", "unrelated")).await;
    put(&service, event("vcs-scope-other", "elsewhere")).await;
    put(
        &service,
        event_model(
            "vcs-scope",
            "refunds",
            vec![member(pb::EntityKind::Event, id("vcs-scope", "missing"))],
        ),
    )
    .await;
    put(
        &service,
        event_model(
            "vcs-scope",
            "checkout",
            vec![member(pb::EntityKind::Event, id("vcs-scope", "placed"))],
        ),
    )
    .await;

    let response = validate_model_by_id(&service, model.clone()).await;

    assert!(
        response.issues.iter().all(|i| i.code != "MEMBER_NOT_FOUND"),
        "another model's findings leaked in: {:?}",
        response.issues
    );
    let coverage = response.coverage.expect("coverage required");
    assert_check(&coverage, "model", State::Completed, "", 1);
    assert_check(&coverage, "strategic", State::Completed, "", 1);
    assert!(
        coverage
            .checks
            .iter()
            .all(|c| c.check_id != "store_hygiene"),
        "single-model validation must not report store-wide suites: {coverage:?}"
    );
    assert_eq!(coverage.effective_scope, Some(model_scope(&model)));
    assert_eq!(coverage.requested_scope, Some(model_scope(&model)));
}

fn event_on_lane(namespace: &str, slug: &str, lane: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(namespace, slug)),
            title: slug.into(),
            swimlane: Some(pb::SwimlaneRef {
                id: Some(id(namespace, lane)),
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn lane(namespace: &str, slug: &str, after: &str, next: &[&str]) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
            id: Some(id(namespace, slug)),
            title: slug.into(),
            transitions: vec![pb::swimlane::Transition {
                after: Some(pb::EventRef {
                    id: Some(id(namespace, after)),
                }),
                next: next
                    .iter()
                    .map(|n| pb::swimlane::transition::Next {
                        event: Some(pb::EventRef {
                            id: Some(id(namespace, n)),
                        }),
                        ..Default::default()
                    })
                    .collect(),
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn command_slice(
    namespace: &str,
    slug: &str,
    command: &str,
    given: &str,
    emit: &str,
) -> pb::Entity {
    let command_ref = || pb::CommandRef {
        id: Some(id(namespace, command)),
    };
    let example = |slug: &str| pb::EventExample {
        event: Some(pb::EventRef {
            id: Some(id(namespace, slug)),
        }),
        payload: None,
    };
    pb::Entity {
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(namespace, slug)),
            title: slug.into(),
            command: Some(pb::CommandEdge {
                command: Some(command_ref()),
                ..Default::default()
            }),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id(namespace, emit)),
                }),
                ..Default::default()
            }],
            scenarios: vec![pb::CommandScenario {
                id: "s1".into(),
                title: format!("{given} then {emit}"),
                given: vec![example(given)],
                when: Some(pb::CommandExample {
                    command: Some(command_ref()),
                    payload: None,
                }),
                then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
                    events: vec![example(emit)],
                })),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn transition_violations(issues: &[pb::ValidationIssue], slice: &pb::Id) -> usize {
    issues
        .iter()
        .filter(|i| {
            i.code == "SCENARIO_TRANSITION_VIOLATION"
                && i.subject.as_ref().and_then(|s| s.id.as_ref()) == Some(slice)
        })
        .count()
}

/// A lane edit can make a stored scenario illegal without touching the
/// slice, so validation must re-run the scenario check over the model's
/// slices rather than leave it to the next write of the slice.
#[tokio::test(flavor = "multi_thread")]
async fn validation_rechecks_scenarios_after_a_lane_edit() {
    let service = service().await;
    let ns = "vcs-scenarios";
    for entity in [
        event_on_lane(ns, "review.submitted", "review"),
        event_on_lane(ns, "review.approved", "review"),
        event_on_lane(ns, "review.rejected", "review"),
        lane(
            ns,
            "review",
            "review.submitted",
            &["review.approved", "review.rejected"],
        ),
        pb::Entity {
            kind: Some(pb::entity::Kind::Command(pb::Command {
                id: Some(id(ns, "approve-review")),
                title: "approve-review".into(),
                ..Default::default()
            })),
            ..Default::default()
        },
        command_slice(
            ns,
            "approve-review",
            "approve-review",
            "review.submitted",
            "review.approved",
        ),
    ] {
        put(&service, entity).await;
    }
    let slice = id(ns, "approve-review");
    let model = id(ns, "reviews");
    put(
        &service,
        event_model(
            ns,
            "reviews",
            vec![member(pb::EntityKind::CommandSlice, slice.clone())],
        ),
    )
    .await;
    let before = validate_model_by_id(&service, model.clone()).await;
    assert_eq!(
        transition_violations(&before.issues, &slice),
        0,
        "{before:?}"
    );

    // The lane no longer allows approval, so the stored scenario is illegal.
    // Edit the stored lane so the kept successor keeps its connection id.
    let mut edited = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Swimlane as i32,
            id: Some(id(ns, "review")),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .expect("stored lane");
    let Some(pb::entity::Kind::Swimlane(stored_lane)) = edited.kind.as_mut() else {
        panic!("not a swimlane: {edited:?}");
    };
    stored_lane.transitions[0].next.retain(|n| {
        n.event.as_ref().and_then(|e| e.id.as_ref()) != Some(&id(ns, "review.approved"))
    });
    put(&service, edited).await;

    let response = validate_model_by_id(&service, model.clone()).await;
    assert_eq!(
        transition_violations(&response.issues, &slice),
        1,
        "{:?}",
        response.issues
    );
    let coverage = response.coverage.expect("coverage required");
    assert_check(&coverage, "scenarios", State::Completed, "", 1);

    let project = service
        .validate_project(Request::new(pb::ValidateProjectRequest {
            scope: Some(Scope::Namespace(ns.into())),
        }))
        .await
        .unwrap()
        .into_inner();
    let report = project
        .reports
        .iter()
        .find(|r| r.event_model_id.as_ref() == Some(&model))
        .expect("model report");
    assert_eq!(
        transition_violations(&report.issues, &slice),
        1,
        "{:?}",
        report.issues
    );
    let coverage = project.coverage.expect("coverage required");
    assert_check(&coverage, "scenarios", State::Completed, "", 1);
}
