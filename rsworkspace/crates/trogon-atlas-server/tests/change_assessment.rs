#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    change_assessment::{Reason, Verdict},
    event_model_service_server::EventModelService,
};
use tonic::Request;
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

fn event(namespace: &str, slug: &str, title: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(namespace, slug)),
            title: title.into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn model(namespace: &str, slug: &str, members: &[&str]) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id(namespace, slug)),
            title: format!("{namespace}/{slug}"),
            members: members
                .iter()
                .map(|member| pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id(namespace, member)),
                })
                .collect(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn on<T>(branch: Option<&str>, value: T) -> Request<T> {
    let mut request = Request::new(value);
    if let Some(branch) = branch {
        request
            .metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    request
}

async fn put(service: &EventModelServiceImpl, branch: Option<&str>, entity: pb::Entity) {
    service
        .put_entity(on(
            branch,
            pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            },
        ))
        .await
        .unwrap();
}

async fn branch(service: &EventModelServiceImpl, name: &str) {
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: name.into(),
            ..Default::default()
        }))
        .await
        .unwrap();
}

async fn merge(
    service: &EventModelServiceImpl,
    name: &str,
    dry_run: bool,
) -> pb::MergeBranchResponse {
    service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: name.into(),
            dry_run,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
}

fn judged(response: &pb::MergeBranchResponse) -> (Verdict, Vec<Reason>) {
    let assessment = response
        .change_assessment
        .as_ref()
        .expect("a merge dry run carries an assessment");
    (assessment.verdict(), assessment.reasons().collect())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clean_landing_lands_and_is_bound_to_what_it_judged() {
    let service = service().await;
    put(&service, None, event("ca-land", "placed", "Placed")).await;
    put(&service, None, model("ca-land", "checkout", &["placed"])).await;
    branch(&service, "ca-land").await;
    put(
        &service,
        Some("ca-land"),
        event("ca-land", "placed", "Order placed"),
    )
    .await;

    let response = merge(&service, "ca-land", true).await;

    assert_eq!(judged(&response), (Verdict::Land, vec![]), "{response:?}");
    let binding = response.change_assessment.unwrap().binding.unwrap();
    let delta = response.consequence_delta.unwrap();
    assert_eq!(binding.branch, "ca-land");
    assert_eq!(binding.baseline_snapshot_id, delta.before_snapshot_id);
    assert_eq!(binding.landed_snapshot_id, delta.after_snapshot_id);
    assert_eq!(binding.scope, "*");
    assert!(binding.catalogue_revision.starts_with("sha256:"));
    let pinned: Vec<_> = binding
        .entries
        .iter()
        .map(|pin| {
            (
                pin.r#ref
                    .as_ref()
                    .unwrap()
                    .id
                    .as_ref()
                    .unwrap()
                    .slug
                    .as_str(),
                pin.baseline_etag.is_empty(),
            )
        })
        .collect();
    assert_eq!(pinned, vec![("placed", false)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_landing_that_introduces_a_finding_needs_review() {
    let service = service().await;
    put(&service, None, event("ca-review", "placed", "Placed")).await;
    put(&service, None, model("ca-review", "checkout", &["placed"])).await;
    branch(&service, "ca-review").await;
    put(
        &service,
        Some("ca-review"),
        model("ca-review", "checkout", &["placed", "missing"]),
    )
    .await;

    let response = merge(&service, "ca-review", true).await;

    assert_eq!(
        judged(&response),
        (Verdict::NeedsReview, vec![Reason::FindingsIntroduced]),
        "{response:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conflicting_branch_is_rejected() {
    let service = service().await;
    put(&service, None, event("ca-conflict", "placed", "Placed")).await;
    branch(&service, "ca-conflict").await;
    put(
        &service,
        Some("ca-conflict"),
        event("ca-conflict", "placed", "Ours"),
    )
    .await;
    put(&service, None, event("ca-conflict", "placed", "Theirs")).await;

    let response = merge(&service, "ca-conflict", true).await;

    assert_eq!(
        response.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    assert_eq!(
        judged(&response),
        (Verdict::Reject, vec![Reason::Conflicts])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_verdict_is_advisory_and_a_real_merge_carries_none() {
    let service = service().await;
    put(&service, None, event("ca-real", "placed", "Placed")).await;
    put(&service, None, model("ca-real", "checkout", &["placed"])).await;
    branch(&service, "ca-real").await;
    put(
        &service,
        Some("ca-real"),
        model("ca-real", "checkout", &["placed", "missing"]),
    )
    .await;
    assert_eq!(
        judged(&merge(&service, "ca-real", true).await).0,
        Verdict::NeedsReview
    );

    let landed = merge(&service, "ca-real", false).await;

    assert_eq!(
        landed.status,
        pb::merge_branch_response::Status::Applied as i32,
        "a needs_review verdict does not stop the merge: {landed:?}"
    );
    assert!(landed.change_assessment.is_none());
}
