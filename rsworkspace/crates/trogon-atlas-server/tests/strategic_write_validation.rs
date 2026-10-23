#![allow(clippy::unwrap_used, clippy::expect_used, clippy::large_futures)]

use std::{path::PathBuf, sync::Arc};

use pb::subdomain::Classification;
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

const BRANCH: &str = "x-trogon-atlas-branch";

fn id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn domain(ns: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id(ns, slug)),
            title: slug.into(),
            ..Default::default()
        })),
    }
}

fn subdomain(ns: &str, slug: &str, classification: Classification) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Subdomain(pb::Subdomain {
            id: Some(id(ns, slug)),
            title: slug.into(),
            domain: Some(pb::DomainRef {
                id: Some(id(ns, ns)),
            }),
            classification: classification as i32,
            ..Default::default()
        })),
    }
}

fn context(ns: &str, realizes: &[(&str, &str)]) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::BoundedContext(pb::BoundedContext {
            id: Some(id(ns, ns)),
            title: ns.into(),
            realizes: realizes
                .iter()
                .map(|(sns, slug)| pb::SubdomainRef {
                    id: Some(id(sns, slug)),
                })
                .collect(),
            ..Default::default()
        })),
    }
}

fn put_request(entity: pb::Entity, validate_only: bool) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only,
        force: false,
    }
}

fn batch_put(entity: pb::Entity) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(put_request(entity, false))),
    }
}

async fn svc() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap()
}

async fn put(svc: &EventModelServiceImpl, entity: pb::Entity) -> Vec<pb::ValidationIssue> {
    svc.put_entity(Request::new(put_request(entity, false)))
        .await
        .unwrap()
        .into_inner()
        .validation
}

fn count(issues: &[pb::ValidationIssue], code: &str) -> usize {
    issues.iter().filter(|i| i.code == code).count()
}

fn about(issue: &pb::ValidationIssue) -> (pb::EntityKind, String, String) {
    let subject = issue
        .subject
        .as_ref()
        .expect("strategic findings name a subject");
    let id = subject.id.as_ref().unwrap();
    (
        pb::EntityKind::try_from(subject.kind).unwrap(),
        id.namespace.clone(),
        id.slug.clone(),
    )
}

async fn batch(
    svc: &EventModelServiceImpl,
    entities: Vec<pb::Entity>,
    validate_only: bool,
) -> pb::BatchMutateResponse {
    svc.batch_mutate(Request::new(pb::BatchMutateRequest {
        operation_id: String::new(),
        ops: entities.into_iter().map(batch_put).collect(),
        validate_only,
    }))
    .await
    .unwrap()
    .into_inner()
}

fn op_validation(resp: &pb::BatchMutateResponse, index: usize) -> &[pb::ValidationIssue] {
    match resp.results[index].result.as_ref() {
        Some(pb::batch_mutate_op_result::Result::Put(put)) => &put.validation,
        other => panic!("op {index} is not a put result: {other:?}"),
    }
}

async fn exists(svc: &EventModelServiceImpl, kind: pb::EntityKind, id: pb::Id) -> bool {
    svc.get_entity(Request::new(pb::GetEntityRequest {
        kind: kind as i32,
        id: Some(id),
    }))
    .await
    .is_ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn putting_a_domain_beside_a_context_reports_it_and_still_writes() {
    let svc = svc().await;
    put(&svc, context("sw-dom", &[])).await;

    let issues = put(&svc, domain("sw-dom", "sw-dom")).await;

    assert_eq!(
        count(&issues, "DOMAIN_IN_CONTEXT_NAMESPACE"),
        1,
        "{issues:?}"
    );
    assert!(exists(&svc, pb::EntityKind::Domain, id("sw-dom", "sw-dom")).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_domain_error_finding_does_not_block_the_put() {
    let svc = svc().await;

    let issues = put(&svc, domain("sw-slug", "other")).await;

    let finding = issues
        .iter()
        .find(|i| i.code == "DOMAIN_SLUG_MISMATCH")
        .unwrap_or_else(|| panic!("{issues:?}"));
    assert_eq!(
        finding.severity,
        pb::validation_issue::Severity::Error as i32
    );
    assert!(exists(&svc, pb::EntityKind::Domain, id("sw-slug", "other")).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn putting_a_context_into_a_domain_namespace_reports_the_domain() {
    let svc = svc().await;
    put(&svc, domain("sw-bc", "sw-bc")).await;

    let issues = put(&svc, context("sw-bc", &[])).await;

    let finding = issues
        .iter()
        .find(|i| i.code == "DOMAIN_IN_CONTEXT_NAMESPACE")
        .unwrap_or_else(|| panic!("{issues:?}"));
    assert_eq!(
        about(finding),
        (pb::EntityKind::Domain, "sw-bc".into(), "sw-bc".into())
    );
    assert!(exists(&svc, pb::EntityKind::BoundedContext, id("sw-bc", "sw-bc")).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_context_put_before_its_subdomain_is_dangling_but_written() {
    let svc = svc().await;

    let issues = put(&svc, context("sw-order", &[("sw-order-ps", "ordering")])).await;

    assert_eq!(count(&issues, "REALIZES_DANGLING"), 1, "{issues:?}");
    assert!(
        exists(
            &svc,
            pb::EntityKind::BoundedContext,
            id("sw-order", "sw-order")
        )
        .await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reclassifying_a_subdomain_reports_the_context_it_makes_mixed() {
    let svc = svc().await;
    let ps = "sw-mixed-ps";
    put(&svc, domain(ps, ps)).await;
    put(&svc, subdomain(ps, "ordering", Classification::Core)).await;
    put(&svc, subdomain(ps, "billing", Classification::Core)).await;
    let clean = put(
        &svc,
        context("sw-mixed", &[(ps, "ordering"), (ps, "billing")]),
    )
    .await;
    assert_eq!(
        count(&clean, "REALIZES_MIXED_CLASSIFICATIONS"),
        0,
        "{clean:?}"
    );

    let issues = put(&svc, subdomain(ps, "billing", Classification::Supporting)).await;

    let finding = issues
        .iter()
        .find(|i| i.code == "REALIZES_MIXED_CLASSIFICATIONS")
        .unwrap_or_else(|| panic!("{issues:?}"));
    assert_eq!(
        about(finding),
        (
            pb::EntityKind::BoundedContext,
            "sw-mixed".into(),
            "sw-mixed".into()
        )
    );
}

fn mixed_batch(ps: &str, ctx: &str) -> Vec<pb::Entity> {
    vec![
        context(ctx, &[(ps, "ordering"), (ps, "billing")]),
        subdomain(ps, "ordering", Classification::Core),
        subdomain(ps, "billing", Classification::Supporting),
        domain(ps, ps),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_batch_reports_mixed_classifications_regardless_of_order() {
    let svc = svc().await;

    let resp = batch(&svc, mixed_batch("sw-dry-ps", "sw-dry"), true).await;

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Validated as i32,
        "{resp:?}"
    );
    let context_issues = op_validation(&resp, 0);
    assert_eq!(
        count(context_issues, "REALIZES_MIXED_CLASSIFICATIONS"),
        1,
        "{context_issues:?}"
    );
    let all: Vec<&pb::ValidationIssue> = (0..4).flat_map(|i| op_validation(&resp, i)).collect();
    assert!(
        all.iter().all(|i| i.code != "REALIZES_DANGLING"),
        "ordering inside the batch must not cause dangling: {all:?}"
    );
    assert!(!exists(&svc, pb::EntityKind::BoundedContext, id("sw-dry", "sw-dry")).await);
    assert!(!exists(&svc, pb::EntityKind::Domain, id("sw-dry-ps", "sw-dry-ps")).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_applied_batch_carries_the_same_findings_and_persists() {
    let svc = svc().await;

    let resp = batch(&svc, mixed_batch("sw-app-ps", "sw-app"), false).await;

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{resp:?}"
    );
    assert_eq!(
        count(op_validation(&resp, 0), "REALIZES_MIXED_CLASSIFICATIONS"),
        1
    );
    assert!(exists(&svc, pb::EntityKind::BoundedContext, id("sw-app", "sw-app")).await);
}

fn command_slice(ns: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(ns, slug)),
            ..Default::default()
        })),
    }
}

/// Two services over one store, the first with a warm baseline cache, as
/// two server replicas would be after the first one served a read.
async fn replicas(warm_ns: &str) -> (EventModelServiceImpl, EventModelServiceImpl) {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = Arc::new(nats.store().await);
    let replica = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let other = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    let warm = batch(&replica, vec![context(warm_ns, &[])], true).await;
    assert_eq!(
        warm.status,
        pb::batch_mutate_response::Status::Validated as i32,
        "{warm:?}"
    );
    (replica, other)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_with_slices_sees_strategic_writes_from_another_replica() {
    let (replica, other) = replicas("sw-rep").await;
    put(&other, domain("sw-rep-ps", "sw-rep-ps")).await;
    put(
        &other,
        subdomain("sw-rep-ps", "ordering", Classification::Core),
    )
    .await;

    let resp = batch(
        &replica,
        vec![
            context("sw-rep", &[("sw-rep-ps", "ordering")]),
            command_slice("sw-rep", "checkout"),
        ],
        true,
    )
    .await;

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Validated as i32,
        "{resp:?}"
    );
    let context_issues = op_validation(&resp, 0);
    assert_eq!(
        count(context_issues, "REALIZES_DANGLING"),
        0,
        "{context_issues:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_finding_about_a_stored_entity_rides_the_op_that_caused_it() {
    let svc = svc().await;
    put(&svc, domain("sw-cause", "sw-cause")).await;

    let resp = batch(&svc, vec![context("sw-cause", &[])], true).await;

    let issues = op_validation(&resp, 0);
    let finding = issues
        .iter()
        .find(|i| i.code == "DOMAIN_IN_CONTEXT_NAMESPACE")
        .unwrap_or_else(|| panic!("{issues:?}"));
    assert_eq!(
        about(finding),
        (pb::EntityKind::Domain, "sw-cause".into(), "sw-cause".into())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finding_is_reported_once_on_the_op_of_its_own_entity() {
    let svc = svc().await;

    let resp = batch(
        &svc,
        vec![context("sw-once", &[]), domain("sw-once", "sw-once")],
        true,
    )
    .await;

    assert_eq!(
        count(op_validation(&resp, 0), "DOMAIN_IN_CONTEXT_NAMESPACE"),
        0
    );
    assert_eq!(
        count(op_validation(&resp, 1), "DOMAIN_IN_CONTEXT_NAMESPACE"),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clean_problem_space_batch_has_no_strategic_warnings_or_errors() {
    let svc = svc().await;
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../examples/manifests/acme");
    let mut entities: Vec<pb::Entity> = trogon_atlas_client::manifest::load_paths(&[dir])
        .expect("acme manifests should load")
        .into_iter()
        .map(|m| m.entity)
        .collect();
    entities.insert(0, context("acme-ordering", &[("acme", "ordering")]));

    let resp = batch(&svc, entities, true).await;

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Validated as i32,
        "{resp:?}"
    );
    let loud: Vec<&pb::ValidationIssue> = (0..resp.results.len())
        .flat_map(|i| op_validation(&resp, i))
        .filter(|i| i.severity >= pb::validation_issue::Severity::Warning as i32)
        .collect();
    assert!(loud.is_empty(), "{loud:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn validate_project_reports_an_unrealized_subdomain_in_a_problem_space_namespace() {
    let svc = svc().await;
    let ps = "sw-vp-ps";
    put(&svc, domain(ps, ps)).await;
    put(&svc, subdomain(ps, "ordering", Classification::Core)).await;

    for scope in [
        pb::validate_project_request::Scope::Namespace(ps.into()),
        pb::validate_project_request::Scope::DomainId(id(ps, ps)),
    ] {
        let resp = svc
            .validate_project(Request::new(pb::ValidateProjectRequest {
                scope: Some(scope.clone()),
            }))
            .await
            .unwrap()
            .into_inner();
        let store_report = resp
            .reports
            .iter()
            .find(|r| r.event_model_id.is_none())
            .unwrap_or_else(|| panic!("{scope:?}: {resp:?}"));
        let finding = store_report
            .issues
            .iter()
            .find(|i| i.code == "SUBDOMAIN_UNREALIZED")
            .unwrap_or_else(|| panic!("{scope:?}: {:?}", store_report.issues));
        assert_eq!(
            about(finding),
            (pb::EntityKind::Subdomain, ps.into(), "ordering".into())
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn validate_project_by_project_reaches_the_realizing_context() {
    let svc = svc().await;
    let ps = "sw-pj-ps";
    let mut project_domain = domain(ps, ps);
    if let Some(pb::entity::Kind::Domain(d)) = project_domain.kind.as_mut() {
        d.project = Some(pb::ProjectRef {
            id: Some(id(ps, ps)),
        });
    }
    put(&svc, project_domain).await;
    put(&svc, subdomain(ps, "ordering", Classification::Core)).await;
    put(&svc, subdomain(ps, "billing", Classification::Supporting)).await;
    put(&svc, context("sw-pj", &[(ps, "ordering"), (ps, "billing")])).await;

    let resp = svc
        .validate_project(Request::new(pb::ValidateProjectRequest {
            scope: Some(pb::validate_project_request::Scope::ProjectId(id(ps, ps))),
        }))
        .await
        .unwrap()
        .into_inner();

    let store_report = resp
        .reports
        .iter()
        .find(|r| r.event_model_id.is_none())
        .unwrap_or_else(|| panic!("{resp:?}"));
    assert_eq!(
        count(&store_report.issues, "REALIZES_MIXED_CLASSIFICATIONS"),
        1,
        "{:?}",
        store_report.issues
    );
}

async fn merge_world(branch: &str) -> EventModelServiceImpl {
    let svc = svc().await;
    svc.create_branch(Request::new(pb::CreateBranchRequest {
        name: branch.into(),
        ..Default::default()
    }))
    .await
    .unwrap();
    svc
}

fn on_branch<T>(branch: &str, value: T) -> Request<T> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert(BRANCH, branch.parse().unwrap());
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn merging_a_branch_reports_the_strategic_finding_it_introduces() {
    let svc = merge_world("strategic").await;
    let ps = "sw-mg-ps";
    put(&svc, domain(ps, ps)).await;
    put(&svc, subdomain(ps, "ordering", Classification::Core)).await;
    put(&svc, subdomain(ps, "billing", Classification::Supporting)).await;
    svc.put_entity(on_branch(
        "strategic",
        put_request(
            context("sw-mg", &[(ps, "ordering"), (ps, "billing")]),
            false,
        ),
    ))
    .await
    .unwrap();

    for dry_run in [true, false] {
        let merged = svc
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "strategic".into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        let expected = if dry_run {
            pb::merge_branch_response::Status::Validated
        } else {
            pb::merge_branch_response::Status::Applied
        };
        assert_eq!(merged.status, expected as i32, "{merged:?}");
        assert_eq!(
            count(&merged.validation, "REALIZES_MIXED_CLASSIFICATIONS"),
            1,
            "dry_run={dry_run}: {:?}",
            merged.validation
        );
    }
    assert!(exists(&svc, pb::EntityKind::BoundedContext, id("sw-mg", "sw-mg")).await);
}

fn delete_request(
    kind: pb::EntityKind,
    id: pb::Id,
    mode: pb::delete_entity_request::Mode,
) -> pb::DeleteEntityRequest {
    pb::DeleteEntityRequest {
        kind: kind as i32,
        id: Some(id),
        mode: mode as i32,
        ..Default::default()
    }
}

async fn delete(
    svc: &EventModelServiceImpl,
    kind: pb::EntityKind,
    id: pb::Id,
    mode: pb::delete_entity_request::Mode,
) -> Vec<pb::ValidationIssue> {
    svc.delete_entity(Request::new(delete_request(kind, id, mode)))
        .await
        .unwrap()
        .into_inner()
        .validation
}

async fn realized_chain(svc: &EventModelServiceImpl, ps: &str, ctx: &str) {
    put(svc, domain(ps, ps)).await;
    put(svc, subdomain(ps, "ordering", Classification::Core)).await;
    put(svc, context(ctx, &[(ps, "ordering")])).await;
}

fn batch_delete(
    kind: pb::EntityKind,
    id: pb::Id,
    mode: pb::delete_entity_request::Mode,
) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Delete(delete_request(
            kind, id, mode,
        ))),
    }
}

async fn batch_ops(
    svc: &EventModelServiceImpl,
    ops: Vec<pb::BatchMutateOp>,
    validate_only: bool,
) -> pb::BatchMutateResponse {
    svc.batch_mutate(Request::new(pb::BatchMutateRequest {
        operation_id: String::new(),
        ops,
        validate_only,
    }))
    .await
    .unwrap()
    .into_inner()
}

fn delete_validation(resp: &pb::BatchMutateResponse, index: usize) -> &[pb::ValidationIssue] {
    match resp.results[index].result.as_ref() {
        Some(pb::batch_mutate_op_result::Result::Delete(deleted)) => &deleted.validation,
        other => panic!("op {index} is not a delete result: {other:?}"),
    }
}

use pb::delete_entity_request::Mode;

#[tokio::test(flavor = "multi_thread")]
async fn force_deleting_a_realized_subdomain_reports_the_dangling_context() {
    let svc = svc().await;
    realized_chain(&svc, "sw-df-ps", "sw-df").await;

    let issues = delete(
        &svc,
        pb::EntityKind::Subdomain,
        id("sw-df-ps", "ordering"),
        Mode::Force,
    )
    .await;

    let finding = issues
        .iter()
        .find(|i| i.code == "REALIZES_DANGLING")
        .unwrap_or_else(|| panic!("{issues:?}"));
    assert_eq!(
        about(finding),
        (
            pb::EntityKind::BoundedContext,
            "sw-df".into(),
            "sw-df".into()
        )
    );
    assert!(!exists(&svc, pb::EntityKind::Subdomain, id("sw-df-ps", "ordering")).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_the_only_realizing_context_reports_the_unrealized_subdomain() {
    let svc = svc().await;
    realized_chain(&svc, "sw-dc-ps", "sw-dc").await;

    let issues = delete(
        &svc,
        pb::EntityKind::BoundedContext,
        id("sw-dc", "sw-dc"),
        Mode::FailIfReferenced,
    )
    .await;

    let finding = issues
        .iter()
        .find(|i| i.code == "SUBDOMAIN_UNREALIZED")
        .unwrap_or_else(|| panic!("{issues:?}"));
    assert_eq!(
        about(finding),
        (
            pb::EntityKind::Subdomain,
            "sw-dc-ps".into(),
            "ordering".into()
        )
    );
    assert!(!exists(&svc, pb::EntityKind::BoundedContext, id("sw-dc", "sw-dc")).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn force_deleting_a_domain_reports_its_subdomains_dangling() {
    let svc = svc().await;
    realized_chain(&svc, "sw-dd-ps", "sw-dd").await;

    let issues = delete(
        &svc,
        pb::EntityKind::Domain,
        id("sw-dd-ps", "sw-dd-ps"),
        Mode::Force,
    )
    .await;

    let finding = issues
        .iter()
        .find(|i| i.code == "SUBDOMAIN_DANGLING_DOMAIN")
        .unwrap_or_else(|| panic!("{issues:?}"));
    assert_eq!(
        about(finding),
        (
            pb::EntityKind::Subdomain,
            "sw-dd-ps".into(),
            "ordering".into()
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_delete_previews_the_findings_and_deletes_nothing() {
    let svc = svc().await;
    realized_chain(&svc, "sw-dr-ps", "sw-dr").await;

    let issues = delete(
        &svc,
        pb::EntityKind::BoundedContext,
        id("sw-dr", "sw-dr"),
        Mode::DryRun,
    )
    .await;

    assert_eq!(count(&issues, "SUBDOMAIN_UNREALIZED"), 1, "{issues:?}");
    assert!(exists(&svc, pb::EntityKind::BoundedContext, id("sw-dr", "sw-dr")).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delete_that_leaves_nothing_behind_reports_nothing() {
    let svc = svc().await;
    put(&svc, domain("sw-dn", "sw-dn")).await;

    let issues = delete(
        &svc,
        pb::EntityKind::Domain,
        id("sw-dn", "sw-dn"),
        Mode::FailIfReferenced,
    )
    .await;

    assert!(issues.is_empty(), "{issues:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_batch_reports_delete_findings_on_the_delete_op() {
    let svc = svc().await;
    realized_chain(&svc, "sw-bd-ps", "sw-bd").await;

    let resp = batch_ops(
        &svc,
        vec![batch_delete(
            pb::EntityKind::BoundedContext,
            id("sw-bd", "sw-bd"),
            Mode::FailIfReferenced,
        )],
        true,
    )
    .await;

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Validated as i32,
        "{resp:?}"
    );
    let issues = delete_validation(&resp, 0);
    assert_eq!(count(issues, "SUBDOMAIN_UNREALIZED"), 1, "{issues:?}");
    assert!(exists(&svc, pb::EntityKind::BoundedContext, id("sw-bd", "sw-bd")).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_applied_batch_reports_delete_findings_and_persists() {
    let svc = svc().await;
    realized_chain(&svc, "sw-ba-ps", "sw-ba").await;

    let resp = batch_ops(
        &svc,
        vec![batch_delete(
            pb::EntityKind::Subdomain,
            id("sw-ba-ps", "ordering"),
            Mode::Force,
        )],
        false,
    )
    .await;

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{resp:?}"
    );
    let issues = delete_validation(&resp, 0);
    assert_eq!(count(issues, "REALIZES_DANGLING"), 1, "{issues:?}");
    assert!(!exists(&svc, pb::EntityKind::Subdomain, id("sw-ba-ps", "ordering")).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_reports_a_finding_once_when_a_put_and_a_delete_both_reach_it() {
    let svc = svc().await;
    realized_chain(&svc, "sw-bo-ps", "sw-bo").await;

    let resp = batch_ops(
        &svc,
        vec![
            batch_delete(
                pb::EntityKind::Subdomain,
                id("sw-bo-ps", "ordering"),
                Mode::Force,
            ),
            batch_put(context("sw-bo", &[("sw-bo-ps", "ordering")])),
        ],
        true,
    )
    .await;

    let on_delete = count(delete_validation(&resp, 0), "REALIZES_DANGLING");
    let on_put = count(op_validation(&resp, 1), "REALIZES_DANGLING");
    assert_eq!((on_delete, on_put), (0, 1), "{resp:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn merging_a_strategic_error_is_reported_without_blocking() {
    let branch = "strategic-error";
    let svc = merge_world(branch).await;
    svc.put_entity(on_branch(
        branch,
        put_request(domain("sw-me", "other"), false),
    ))
    .await
    .unwrap();

    for dry_run in [true, false] {
        let merged = svc
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: branch.into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        let expected = if dry_run {
            pb::merge_branch_response::Status::Validated
        } else {
            pb::merge_branch_response::Status::Applied
        };
        assert_eq!(merged.status, expected as i32, "{merged:?}");
        let finding = merged
            .validation
            .iter()
            .find(|i| i.code == "DOMAIN_SLUG_MISMATCH")
            .unwrap_or_else(|| panic!("dry_run={dry_run}: {:?}", merged.validation));
        assert_eq!(
            finding.severity,
            pb::validation_issue::Severity::Error as i32
        );
    }
    assert!(exists(&svc, pb::EntityKind::Domain, id("sw-me", "other")).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn merging_a_deletion_reports_the_finding_it_leaves_behind() {
    let branch = "strategic-delete";
    let svc = merge_world(branch).await;
    realized_chain(&svc, "sw-md-ps", "sw-md").await;
    svc.delete_entity(on_branch(
        branch,
        delete_request(
            pb::EntityKind::BoundedContext,
            id("sw-md", "sw-md"),
            Mode::FailIfReferenced,
        ),
    ))
    .await
    .unwrap();

    for dry_run in [true, false] {
        let merged = svc
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: branch.into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        let expected = if dry_run {
            pb::merge_branch_response::Status::Validated
        } else {
            pb::merge_branch_response::Status::Applied
        };
        assert_eq!(merged.status, expected as i32, "{merged:?}");
        assert_eq!(
            count(&merged.validation, "SUBDOMAIN_UNREALIZED"),
            1,
            "dry_run={dry_run}: {:?}",
            merged.validation
        );
    }
    assert!(!exists(&svc, pb::EntityKind::BoundedContext, id("sw-md", "sw-md")).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_put_sees_strategic_writes_from_another_replica() {
    let (replica, other) = replicas("sw-rep-put").await;
    put(&other, domain("sw-rep-put-ps", "sw-rep-put-ps")).await;
    put(
        &other,
        subdomain("sw-rep-put-ps", "ordering", Classification::Core),
    )
    .await;

    let issues = put(
        &replica,
        context("sw-rep-put", &[("sw-rep-put-ps", "ordering")]),
    )
    .await;

    assert_eq!(count(&issues, "REALIZES_DANGLING"), 0, "{issues:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delete_sees_strategic_writes_from_another_replica() {
    let (replica, other) = replicas("sw-rep-del-warm").await;
    realized_chain(&other, "sw-rep-del-ps", "sw-rep-del").await;

    let issues = delete(
        &replica,
        pb::EntityKind::Subdomain,
        id("sw-rep-del-ps", "ordering"),
        Mode::DryRun,
    )
    .await;

    assert_eq!(count(&issues, "REALIZES_DANGLING"), 1, "{issues:?}");
}
