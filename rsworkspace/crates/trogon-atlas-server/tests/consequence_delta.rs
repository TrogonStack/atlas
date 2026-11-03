#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{event_model_service_server::EventModelService, finding_change::Kind};
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

const BRANCH: &str = "consequence";

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

fn event_model(namespace: &str, slug: &str, members: &[(pb::EntityKind, pb::Id)]) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id(namespace, slug)),
            title: format!("{namespace}/{slug}"),
            members: members
                .iter()
                .map(|(kind, id)| pb::EntityRef {
                    kind: *kind as i32,
                    id: Some(id.clone()),
                })
                .collect(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn member_event(namespace: &str, slug: &str) -> (pb::EntityKind, pb::Id) {
    (pb::EntityKind::Event, id(namespace, slug))
}

fn put_request(entity: pb::Entity, validate_only: bool) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(entity),
        validate_only,
        ..Default::default()
    }
}

async fn put(service: &EventModelServiceImpl, entity: pb::Entity) {
    service
        .put_entity(Request::new(put_request(entity, false)))
        .await
        .unwrap();
}

async fn preview_put(service: &EventModelServiceImpl, entity: pb::Entity) -> pb::ConsequenceDelta {
    service
        .put_entity(Request::new(put_request(entity, true)))
        .await
        .unwrap()
        .into_inner()
        .consequence_delta
        .expect("a dry run reports its consequence delta")
}

fn on_branch<T>(value: T) -> Request<T> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert("x-trogon-atlas-branch", BRANCH.parse().unwrap());
    request
}

fn slug(entity: Option<&pb::EntityRef>) -> &str {
    entity
        .and_then(|e| e.id.as_ref())
        .map_or("", |id| id.slug.as_str())
}

/// Each change as (kind, rule code, subject slug, origin slug).
fn changes(delta: &pb::ConsequenceDelta) -> Vec<(Kind, &str, &str, &str)> {
    delta
        .changes
        .iter()
        .map(|change| {
            let issue = change.after.as_ref().or(change.before.as_ref()).unwrap();
            (
                Kind::try_from(change.kind).unwrap(),
                issue.code.as_str(),
                slug(issue.subject.as_ref()),
                slug(change.origin.as_ref()),
            )
        })
        .collect()
}

fn kinds_of<'a>(delta: &'a pb::ConsequenceDelta, code: &str) -> Vec<(Kind, &'a str, &'a str)> {
    changes(delta)
        .into_iter()
        .filter(|(_, c, _, _)| *c == code)
        .map(|(kind, _, subject, origin)| (kind, subject, origin))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unchanged_world_reports_no_change() {
    let service = service().await;
    put(&service, event("cd-noop", "placed")).await;
    let model = event_model("cd-noop", "checkout", &[member_event("cd-noop", "placed")]);
    put(&service, model.clone()).await;

    let delta = preview_put(&service, model).await;

    assert!(delta.changes.is_empty(), "{delta:?}");
    assert!(delta.complete, "{delta:?}");
    assert_eq!(delta.before_snapshot_id, delta.after_snapshot_id);
    assert_ne!(delta.before_snapshot_id, "");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_error_is_introduced() {
    let service = service().await;
    put(&service, event("cd-new", "placed")).await;
    put(
        &service,
        event_model("cd-new", "checkout", &[member_event("cd-new", "placed")]),
    )
    .await;

    let delta = preview_put(
        &service,
        event_model(
            "cd-new",
            "checkout",
            &[
                member_event("cd-new", "placed"),
                member_event("cd-new", "missing"),
            ],
        ),
    )
    .await;

    assert_eq!(
        kinds_of(&delta, "MEMBER_NOT_FOUND"),
        vec![(Kind::Introduced, "missing", "checkout")],
        "{delta:?}"
    );
    let change = delta
        .changes
        .iter()
        .find(|c| {
            c.after
                .as_ref()
                .is_some_and(|i| i.code == "MEMBER_NOT_FOUND")
        })
        .unwrap();
    assert_eq!(change.check_id, "model");
    assert!(delta.complete);
    assert_ne!(delta.before_snapshot_id, delta.after_snapshot_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fixed_error_is_resolved_and_unrelated_findings_stay_unchanged() {
    let service = service().await;
    put(
        &service,
        event_model("cd-fix", "checkout", &[member_event("cd-fix", "missing")]),
    )
    .await;
    put(
        &service,
        event_model("cd-fix", "refunds", &[member_event("cd-fix", "elsewhere")]),
    )
    .await;

    let delta = preview_put(&service, event("cd-fix", "missing")).await;

    assert_eq!(
        kinds_of(&delta, "MEMBER_NOT_FOUND"),
        vec![(Kind::Resolved, "missing", "checkout")],
        "{delta:?}"
    );
    assert!(
        changes(&delta)
            .iter()
            .all(|(_, _, subject, origin)| { *subject != "elsewhere" && *origin != "refunds" }),
        "the untouched model's findings stay unchanged: {delta:?}"
    );
    assert!(delta.unchanged_count >= 1, "{delta:?}");
    assert!(delta.complete);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_delete_reports_the_finding_it_causes_on_another_model() {
    let service = service().await;
    put(&service, event("cd-cross", "placed")).await;
    put(
        &service,
        event_model(
            "cd-cross",
            "checkout",
            &[member_event("cd-cross", "placed")],
        ),
    )
    .await;

    let response = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("cd-cross", "placed")),
                    mode: pb::delete_entity_request::Mode::Force as i32,
                    ..Default::default()
                })),
            }],
            validate_only: true,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        response.status,
        pb::batch_mutate_response::Status::Validated as i32,
        "{response:?}"
    );
    let delta = response.consequence_delta.expect("dry run reports a delta");
    assert_eq!(
        kinds_of(&delta, "MEMBER_NOT_FOUND"),
        vec![(Kind::Introduced, "placed", "checkout")],
        "{delta:?}"
    );
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("cd-cross", "placed")),
        }))
        .await
        .expect("a dry run never deletes");
}

#[tokio::test(flavor = "multi_thread")]
async fn findings_of_a_deleted_model_leave_with_it() {
    let service = service().await;
    put(
        &service,
        event_model("cd-gone", "checkout", &[member_event("cd-gone", "missing")]),
    )
    .await;

    let delta = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                    kind: pb::EntityKind::EventModel as i32,
                    id: Some(id("cd-gone", "checkout")),
                    mode: pb::delete_entity_request::Mode::Force as i32,
                    ..Default::default()
                })),
            }],
            validate_only: true,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .consequence_delta
        .expect("dry run reports a delta");

    assert_eq!(
        kinds_of(&delta, "MEMBER_NOT_FOUND"),
        vec![(Kind::SubjectRemoved, "missing", "checkout")],
        "{delta:?}"
    );
    assert!(
        changes(&delta)
            .iter()
            .all(|(kind, _, _, _)| *kind != Kind::Resolved),
        "deleting a model fixes nothing: {delta:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn models_that_reference_each_other_terminate() {
    let service = service().await;
    let a = (pb::EntityKind::EventModel, id("cd-cycle", "a"));
    let b = (pb::EntityKind::EventModel, id("cd-cycle", "b"));
    put(
        &service,
        event_model("cd-cycle", "a", std::slice::from_ref(&b)),
    )
    .await;
    put(
        &service,
        event_model("cd-cycle", "b", std::slice::from_ref(&a)),
    )
    .await;

    let delta = preview_put(
        &service,
        event_model("cd-cycle", "a", &[b, member_event("cd-cycle", "missing")]),
    )
    .await;

    assert!(
        changes(&delta)
            .iter()
            .any(|(kind, _, subject, origin)| *kind == Kind::Introduced
                && (*subject == "missing" || *subject == "a" || *origin == "a")),
        "{delta:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_truncated_scan_never_claims_a_resolution() {
    let service = service().await.with_snapshot_cap(1);
    put(
        &service,
        event_model(
            "cd-trunc",
            "checkout",
            &[member_event("cd-trunc", "missing")],
        ),
    )
    .await;
    put(&service, event("cd-trunc", "other")).await;

    let delta = preview_put(&service, event("cd-trunc", "missing")).await;

    assert!(!delta.complete, "{delta:?}");
    let after = delta.after_coverage.as_ref().unwrap();
    assert!(after.snapshot_truncated, "{after:?}");
    assert!(
        delta
            .changes
            .iter()
            .all(|change| change.kind == Kind::Unknown as i32),
        "a partial scan must not classify: {delta:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_merge_preview_reports_what_landing_resolves() {
    let service = service().await;
    put(
        &service,
        event_model(
            "cd-merge",
            "checkout",
            &[member_event("cd-merge", "missing")],
        ),
    )
    .await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: BRANCH.into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    service
        .put_entity(on_branch(put_request(event("cd-merge", "missing"), false)))
        .await
        .unwrap();

    let response = service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: BRANCH.into(),
            dry_run: true,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    let delta = response
        .consequence_delta
        .expect("merge preview reports a delta");
    assert_eq!(
        kinds_of(&delta, "MEMBER_NOT_FOUND"),
        vec![(Kind::Resolved, "missing", "checkout")],
        "{delta:?}"
    );
    assert!(delta.complete);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_real_write_carries_no_delta() {
    let service = service().await;
    let response = service
        .put_entity(Request::new(put_request(
            event("cd-write", "placed"),
            false,
        )))
        .await
        .unwrap()
        .into_inner();
    assert!(response.consequence_delta.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_world_without_content_identity_still_gets_an_unbound_delta() {
    let service = service().await;
    let mut opaque = event("cd-opaque", "priced");
    if let Some(pb::entity::Kind::Event(event)) = opaque.kind.as_mut() {
        event.metadata.push(prost_types::Any {
            type_url: "type.googleapis.com/acme.unknown.v1.Thing".into(),
            value: vec![8, 1],
        });
    }
    put(&service, opaque).await;

    let delta = preview_put(&service, event("cd-opaque", "other")).await;

    assert!(!delta.complete, "{delta:?}");
    assert_eq!(delta.before_snapshot_id, "");
    assert_eq!(delta.after_snapshot_id, "");
}
