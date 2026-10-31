#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_server::EventModelService,
    invariant_outcome::{Reason, Status as Verdict},
};
use prost::Message as _;
use tonic::{Code, Request, Status};
use tonic_types::StatusExt as _;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn service() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default()).unwrap()
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

fn member(namespace: &str, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: Some(id(namespace, slug)),
    }
}

fn forbidding(name: &str, codes: &[&str]) -> pb::InvariantAnnotation {
    pb::InvariantAnnotation {
        name: name.into(),
        forbidden_codes: codes.iter().map(|c| (*c).into()).collect(),
        ..Default::default()
    }
}

fn advising(mut annotation: pb::InvariantAnnotation) -> pb::InvariantAnnotation {
    annotation.set_on_unevaluable(pb::invariant_annotation::Unevaluable::Advise);
    annotation
}

fn model(
    namespace: &str,
    slug: &str,
    members: &[pb::EntityRef],
    invariants: &[pb::InvariantAnnotation],
) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id(namespace, slug)),
            title: format!("{namespace}/{slug}"),
            members: members.to_vec(),
            metadata: invariants
                .iter()
                .map(|a| prost_types::Any {
                    type_url:
                        "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.InvariantAnnotation"
                            .into(),
                    value: a.encode_to_vec(),
                })
                .collect(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn put_request(entity: pb::Entity, validate_only: bool) -> Request<pb::PutEntityRequest> {
    Request::new(pb::PutEntityRequest {
        entity: Some(entity),
        validate_only,
        ..Default::default()
    })
}

async fn put(
    service: &EventModelServiceImpl,
    entity: pb::Entity,
) -> Result<pb::PutEntityResponse, Status> {
    service
        .put_entity(put_request(entity, false))
        .await
        .map(tonic::Response::into_inner)
}

async fn preview_put(
    service: &EventModelServiceImpl,
    entity: pb::Entity,
) -> Result<pb::PutEntityResponse, Status> {
    service
        .put_entity(put_request(entity, true))
        .await
        .map(tonic::Response::into_inner)
}

async fn stored_members(service: &EventModelServiceImpl, namespace: &str, slug: &str) -> usize {
    let entity = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::EventModel as i32,
            id: Some(id(namespace, slug)),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    match entity.kind {
        Some(pb::entity::Kind::EventModel(model)) => model.members.len(),
        _ => panic!("not an event model"),
    }
}

fn reason(status: &Status) -> (Code, String) {
    let info = status
        .get_details_error_info()
        .expect("a typed refusal carries ErrorInfo");
    (status.code(), info.reason)
}

fn metadata(status: &Status, key: &str) -> String {
    status
        .get_details_error_info()
        .unwrap()
        .metadata
        .get(key)
        .cloned()
        .unwrap_or_default()
}

fn statuses(check: &pb::InvariantCheck) -> Vec<(String, Verdict)> {
    check
        .outcomes
        .iter()
        .map(|o| (o.name.clone(), o.status()))
        .collect()
}

const NO_DANGLING: &[&str] = &["MEMBER_NOT_FOUND"];

#[tokio::test(flavor = "multi_thread")]
async fn a_write_that_keeps_the_invariant_reports_it_held() {
    let service = service().await;
    put(&service, event("ig-hold", "placed")).await.unwrap();

    let response = put(
        &service,
        model(
            "ig-hold",
            "checkout",
            &[member("ig-hold", "placed")],
            &[forbidding("no-dangling", NO_DANGLING)],
        ),
    )
    .await
    .unwrap();

    let check = response.invariant_check.expect("the scope declares one");
    assert_eq!(
        statuses(&check),
        vec![("no-dangling".into(), Verdict::Holds)]
    );
    assert!(check.complete, "{check:?}");
    assert_ne!(check.snapshot_id, "");
    assert!(
        check.inputs.iter().any(|i| i
            .r#ref
            .as_ref()
            .and_then(|r| r.id.as_ref())
            .is_some_and(|id| id.slug == "checkout")),
        "{check:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scope_without_invariants_reports_no_check() {
    let service = service().await;
    let response = put(&service, event("ig-none", "placed")).await.unwrap();
    assert!(response.invariant_check.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_violating_write_is_refused_and_leaves_the_store_unchanged() {
    let service = service().await;
    put(&service, event("ig-break", "placed")).await.unwrap();
    let invariant = forbidding("no-dangling", NO_DANGLING);
    put(
        &service,
        model(
            "ig-break",
            "checkout",
            &[member("ig-break", "placed")],
            std::slice::from_ref(&invariant),
        ),
    )
    .await
    .unwrap();

    let breaking = model(
        "ig-break",
        "checkout",
        &[member("ig-break", "placed"), member("ig-break", "missing")],
        &[invariant],
    );
    let refused = put(&service, breaking.clone()).await.unwrap_err();
    let previewed = preview_put(&service, breaking).await.unwrap_err();

    assert_eq!(
        reason(&refused),
        (Code::FailedPrecondition, "INVARIANT_VIOLATED".into())
    );
    assert_eq!(reason(&previewed), reason(&refused));
    assert_eq!(metadata(&refused, "invariant"), "no-dangling");
    assert_eq!(metadata(&refused, "carrier"), "ig-break/checkout@1");
    assert_eq!(
        metadata(&refused, "snapshot_id"),
        metadata(&previewed, "snapshot_id")
    );
    let violations = refused.get_details_precondition_failure().unwrap();
    assert!(
        violations
            .violations
            .iter()
            .any(|v| v.description.starts_with("MEMBER_NOT_FOUND")),
        "{violations:?}"
    );
    assert_eq!(stored_members(&service, "ig-break", "checkout").await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_an_entity_another_model_depends_on_is_judged_by_that_model() {
    let service = service().await;
    put(&service, event("ig-cross", "placed")).await.unwrap();
    put(
        &service,
        model(
            "ig-cross",
            "checkout",
            &[member("ig-cross", "placed")],
            &[forbidding("no-dangling", NO_DANGLING)],
        ),
    )
    .await
    .unwrap();

    let refused = service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("ig-cross", "placed")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap_err();

    assert_eq!(
        reason(&refused),
        (Code::FailedPrecondition, "INVARIANT_VIOLATED".into())
    );
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("ig-cross", "placed")),
        }))
        .await
        .expect("the refused delete removed nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finding_that_predates_the_invariant_does_not_block() {
    let service = service().await;
    let nameless = pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: None,
    };
    put(
        &service,
        model("ig-old", "checkout", std::slice::from_ref(&nameless), &[]),
    )
    .await
    .unwrap();

    let response = put(
        &service,
        model(
            "ig-old",
            "checkout",
            &[nameless],
            &[forbidding("ids-everywhere", &["MISSING_ID"])],
        ),
    )
    .await
    .unwrap();

    let check = response.invariant_check.unwrap();
    assert_eq!(
        statuses(&check),
        vec![("ids-everywhere".into(), Verdict::AlreadyViolated)]
    );
    assert_ne!(check.outcomes[0].preexisting.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_inputs_refuse_by_default_and_advise_when_declared() {
    let service = service().await;
    let dangling = [member("ig-missing", "absent")];

    let refused = put(
        &service,
        model(
            "ig-missing",
            "strict",
            &dangling,
            &[forbidding("ids-everywhere", &["MISSING_ID"])],
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(
        reason(&refused),
        (Code::FailedPrecondition, "INVARIANT_UNEVALUABLE".into())
    );
    assert_eq!(metadata(&refused, "reason"), "MISSING_INPUTS");

    let response = put(
        &service,
        model(
            "ig-missing",
            "lenient",
            &dangling,
            &[advising(forbidding("ids-everywhere", &["MISSING_ID"]))],
        ),
    )
    .await
    .unwrap();
    let check = response.invariant_check.unwrap();
    let outcome = &check.outcomes[0];
    assert_eq!(outcome.status(), Verdict::Unevaluable);
    assert_eq!(outcome.reason(), Reason::MissingInputs);
    assert!(outcome.advisory);
    assert_eq!(outcome.missing, dangling.to_vec());
    assert!(!check.complete);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_declaration_is_rejected_at_write() {
    let service = service().await;
    for invariant in [forbidding("", NO_DANGLING), forbidding("empty", &[])] {
        let refused = put(&service, model("ig-bad", "checkout", &[], &[invariant]))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument, "{refused:?}");
    }
    let duplicate = forbidding("twice", NO_DANGLING);
    let refused = put(
        &service,
        model("ig-bad", "checkout", &[], &[duplicate.clone(), duplicate]),
    )
    .await
    .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument, "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn declarations_past_the_budget_are_unevaluable() {
    let nats = trogon_atlas_testsupport::shared().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default())
            .unwrap()
            .with_invariant_budget(1);

    let refused = put(
        &service,
        model(
            "ig-budget",
            "checkout",
            &[],
            &[
                forbidding("first", NO_DANGLING),
                forbidding("second", NO_DANGLING),
            ],
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(metadata(&refused, "invariant"), "second");
    assert_eq!(metadata(&refused, "reason"), "BUDGET_EXHAUSTED");
}

fn force_delete(namespace: &str, slug: &str) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id(namespace, slug)),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        })),
    }
}

async fn guarded_checkout(service: &EventModelServiceImpl, namespace: &str) {
    put(service, event(namespace, "placed")).await.unwrap();
    put(
        service,
        model(
            namespace,
            "checkout",
            &[member(namespace, "placed")],
            &[forbidding("no-dangling", NO_DANGLING)],
        ),
    )
    .await
    .unwrap();
}

async fn event_exists(service: &EventModelServiceImpl, namespace: &str, slug: &str) -> bool {
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id(namespace, slug)),
        }))
        .await
        .is_ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_is_refused_whole_and_its_preview_agrees() {
    let service = service().await;
    guarded_checkout(&service, "ig-batch").await;
    put(&service, event("ig-batch", "spare")).await.unwrap();

    let batch = |validate_only| {
        Request::new(pb::BatchMutateRequest {
            ops: vec![
                force_delete("ig-batch", "spare"),
                force_delete("ig-batch", "placed"),
            ],
            validate_only,
            ..Default::default()
        })
    };
    let previewed = service.batch_mutate(batch(true)).await.unwrap_err();
    let refused = service.batch_mutate(batch(false)).await.unwrap_err();

    assert_eq!(
        reason(&refused),
        (Code::FailedPrecondition, "INVARIANT_VIOLATED".into())
    );
    assert_eq!(reason(&previewed), reason(&refused));
    assert!(
        event_exists(&service, "ig-batch", "spare").await,
        "no op of a refused batch applies"
    );
    assert!(event_exists(&service, "ig-batch", "placed").await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_write_is_judged_by_the_invariants_it_inherits() {
    const BRANCH: &str = "ig-landing";
    let service = service().await;
    guarded_checkout(&service, "ig-merge").await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: BRANCH.into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut delete = Request::new(pb::DeleteEntityRequest {
        kind: pb::EntityKind::Event as i32,
        id: Some(id("ig-merge", "placed")),
        mode: pb::delete_entity_request::Mode::Force as i32,
        ..Default::default()
    });
    delete
        .metadata_mut()
        .insert("x-trogon-atlas-branch", BRANCH.parse().unwrap());
    let on_branch = service.delete_entity(delete).await;
    assert_eq!(
        reason(&on_branch.unwrap_err()),
        (Code::FailedPrecondition, "INVARIANT_VIOLATED".into()),
        "the branch carries the baseline's invariants"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_merge_preview_reports_the_check_of_the_landed_world() {
    const BRANCH: &str = "ig-merge-ok";
    let service = service().await;
    guarded_checkout(&service, "ig-merge-ok").await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: BRANCH.into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut add = put_request(event("ig-merge-ok", "shipped"), false);
    add.metadata_mut()
        .insert("x-trogon-atlas-branch", BRANCH.parse().unwrap());
    service.put_entity(add).await.unwrap();

    let merge = |dry_run| {
        Request::new(pb::MergeBranchRequest {
            name: BRANCH.into(),
            dry_run,
            ..Default::default()
        })
    };
    let previewed = service
        .merge_branch(merge(true))
        .await
        .unwrap()
        .into_inner();
    let landed = service
        .merge_branch(merge(false))
        .await
        .unwrap()
        .into_inner();

    let check = previewed
        .invariant_check
        .expect("the landing scope declares one");
    assert_eq!(
        statuses(&check),
        vec![("no-dangling".into(), Verdict::Holds)]
    );
    assert_eq!(landed.invariant_check, Some(check));
    assert!(event_exists(&service, "ig-merge-ok", "shipped").await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delete_by_query_is_refused_and_its_dry_run_agrees() {
    let service = service().await;
    guarded_checkout(&service, "ig-dbq").await;

    let query = |mode: pb::delete_entity_request::Mode| {
        Request::new(pb::DeleteByQueryRequest {
            namespace: "ig-dbq".into(),
            kind: pb::EntityKind::Event as i32,
            mode: mode as i32,
            max_deletes: 10,
            ..Default::default()
        })
    };
    let previewed = service
        .delete_by_query(query(pb::delete_entity_request::Mode::DryRun))
        .await
        .unwrap_err();
    let refused = service
        .delete_by_query(query(pb::delete_entity_request::Mode::Force))
        .await
        .unwrap_err();

    assert_eq!(
        reason(&refused),
        (Code::FailedPrecondition, "INVARIANT_VIOLATED".into())
    );
    assert_eq!(reason(&previewed), reason(&refused));
    assert!(event_exists(&service, "ig-dbq", "placed").await);
}
