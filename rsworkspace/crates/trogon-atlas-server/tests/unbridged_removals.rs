#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn service() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default()).unwrap()
}

fn id(namespace: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version,
    }
}

fn event_ref(id: pb::Id) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: Some(id),
    }
}

fn event(id: pb::Id, supersedes: Option<pb::Id>) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            title: id.slug.clone(),
            id: Some(id),
            supersedes,
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn read_model(id: pb::Id, source: pb::Id) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
            title: id.slug.clone(),
            id: Some(id),
            source_events: vec![pb::EventRef { id: Some(source) }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn event_model(id: pb::Id, member: pb::Id) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            title: id.slug.clone(),
            id: Some(id),
            members: vec![event_ref(member)],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn on_branch<T>(value: T, branch: &str) -> Request<T> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    request
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

async fn retarget(
    service: &EventModelServiceImpl,
    from: pb::Id,
    to: pb::Id,
    filter_kinds: &[pb::EntityKind],
    dry_run: bool,
) -> pb::RetargetReferencesResponse {
    service
        .retarget_references(Request::new(pb::RetargetReferencesRequest {
            from: Some(event_ref(from)),
            to: Some(event_ref(to)),
            filter_kinds: filter_kinds.iter().map(|k| *k as i32).collect(),
            dry_run,
        }))
        .await
        .unwrap()
        .into_inner()
}

async fn merge(
    service: &EventModelServiceImpl,
    branch: &str,
    dry_run: bool,
) -> pb::MergeBranchResponse {
    service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: branch.into(),
            dry_run,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
}

async fn create_branch(service: &EventModelServiceImpl, branch: &str) {
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: branch.into(),
            ..Default::default()
        }))
        .await
        .unwrap();
}

/// Each removal as the retired entity and the owners still pinning it.
fn pins(removals: &[pb::UnbridgedRemoval]) -> Vec<(pb::EntityRef, Vec<pb::EntityRef>)> {
    removals
        .iter()
        .map(|r| {
            (
                r.entity.clone().unwrap(),
                r.remaining_referrers
                    .iter()
                    .map(|reference| reference.from.clone().unwrap())
                    .collect(),
            )
        })
        .collect()
}

fn model_ref(id: pb::Id) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::EventModel as i32,
        id: Some(id),
    }
}

async fn seed_partial_migration(service: &EventModelServiceImpl, ns: &str, successor_link: bool) {
    put(service, event(id(ns, "placed", 1), None)).await;
    put(
        service,
        event(
            id(ns, "placed", 2),
            successor_link.then(|| id(ns, "placed", 1)),
        ),
    )
    .await;
    put(
        service,
        read_model(id(ns, "orders", 1), id(ns, "placed", 1)),
    )
    .await;
    put(
        service,
        event_model(id(ns, "board", 1), id(ns, "placed", 1)),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn retarget_dry_run_reports_a_version_left_pinned_without_successor() {
    let service = service().await;
    let ns = "ur-partial";
    seed_partial_migration(&service, ns, false).await;

    let response = retarget(
        &service,
        id(ns, "placed", 1),
        id(ns, "placed", 2),
        &[pb::EntityKind::ReadModel],
        true,
    )
    .await;

    assert_eq!(
        pins(&response.unbridged_removals),
        vec![(
            event_ref(id(ns, "placed", 1)),
            vec![model_ref(id(ns, "board", 1))]
        )]
    );
    assert!(!response.unbridged_removals_incomplete);
    assert_eq!(response.rewritten, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn retarget_dry_run_with_successor_link_reports_nothing() {
    let service = service().await;
    let ns = "ur-linked";
    seed_partial_migration(&service, ns, true).await;

    let response = retarget(
        &service,
        id(ns, "placed", 1),
        id(ns, "placed", 2),
        &[pb::EntityKind::ReadModel],
        true,
    )
    .await;

    assert_eq!(response.unbridged_removals, Vec::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn retarget_dry_run_that_moves_every_referrer_reports_nothing() {
    let service = service().await;
    let ns = "ur-complete";
    seed_partial_migration(&service, ns, false).await;

    let response = retarget(
        &service,
        id(ns, "placed", 1),
        id(ns, "placed", 2),
        &[],
        true,
    )
    .await;

    assert_eq!(response.unbridged_removals, Vec::new());
    assert_eq!(response.rewritten, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn retarget_ignores_older_versions_of_a_referrer() {
    let service = service().await;
    let ns = "ur-history";
    put(&service, event(id(ns, "placed", 1), None)).await;
    put(&service, event(id(ns, "placed", 2), None)).await;
    put(
        &service,
        read_model(id(ns, "orders", 1), id(ns, "placed", 1)),
    )
    .await;
    put(
        &service,
        read_model(id(ns, "orders", 2), id(ns, "placed", 2)),
    )
    .await;

    let response = retarget(
        &service,
        id(ns, "placed", 1),
        id(ns, "placed", 2),
        &[pb::EntityKind::EventModel],
        true,
    )
    .await;

    assert_eq!(response.unbridged_removals, Vec::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn retarget_apply_leaves_the_report_empty() {
    let service = service().await;
    let ns = "ur-apply";
    seed_partial_migration(&service, ns, false).await;

    let response = retarget(
        &service,
        id(ns, "placed", 1),
        id(ns, "placed", 2),
        &[pb::EntityKind::ReadModel],
        false,
    )
    .await;

    assert_eq!(response.rewritten, 1);
    assert_eq!(response.unbridged_removals, Vec::new());
    assert!(!response.unbridged_removals_incomplete);
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_dry_run_reports_a_replaced_version_without_successor() {
    let service = service().await;
    let ns = "ur-merge-bump";
    put(&service, event(id(ns, "placed", 1), None)).await;
    put(
        &service,
        event_model(id(ns, "board", 1), id(ns, "placed", 1)),
    )
    .await;
    create_branch(&service, "ur-merge-bump").await;
    service
        .put_entity(on_branch(
            pb::PutEntityRequest {
                entity: Some(event(id(ns, "placed", 2), None)),
                ..Default::default()
            },
            "ur-merge-bump",
        ))
        .await
        .unwrap();

    let dry = merge(&service, "ur-merge-bump", true).await;
    assert_eq!(
        dry.status,
        pb::merge_branch_response::Status::Validated as i32
    );
    assert_eq!(
        pins(&dry.unbridged_removals),
        vec![(
            event_ref(id(ns, "placed", 1)),
            vec![model_ref(id(ns, "board", 1))]
        )]
    );
    assert!(!dry.unbridged_removals_incomplete);

    let applied = merge(&service, "ur-merge-bump", false).await;
    assert_eq!(
        applied.status,
        pb::merge_branch_response::Status::Applied as i32
    );
    assert_eq!(applied.unbridged_removals, Vec::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_dry_run_reports_a_branch_removal_still_referenced() {
    let service = service().await;
    let ns = "ur-merge-delete";
    put(&service, event(id(ns, "placed", 1), None)).await;
    put(
        &service,
        event_model(id(ns, "board", 1), id(ns, "placed", 1)),
    )
    .await;
    create_branch(&service, "ur-merge-delete").await;
    service
        .delete_entity(on_branch(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id(ns, "placed", 1)),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            "ur-merge-delete",
        ))
        .await
        .unwrap();

    let dry = merge(&service, "ur-merge-delete", true).await;

    assert_eq!(
        pins(&dry.unbridged_removals),
        vec![(
            event_ref(id(ns, "placed", 1)),
            vec![model_ref(id(ns, "board", 1))]
        )]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_dry_run_with_successor_link_reports_nothing() {
    let service = service().await;
    let ns = "ur-merge-linked";
    put(&service, event(id(ns, "placed", 1), None)).await;
    put(
        &service,
        event_model(id(ns, "board", 1), id(ns, "placed", 1)),
    )
    .await;
    create_branch(&service, "ur-merge-linked").await;
    service
        .put_entity(on_branch(
            pb::PutEntityRequest {
                entity: Some(event(id(ns, "placed", 2), Some(id(ns, "placed", 1)))),
                ..Default::default()
            },
            "ur-merge-linked",
        ))
        .await
        .unwrap();

    let dry = merge(&service, "ur-merge-linked", true).await;

    assert_eq!(
        dry.status,
        pb::merge_branch_response::Status::Validated as i32
    );
    assert_eq!(dry.unbridged_removals, Vec::new());
}
