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
