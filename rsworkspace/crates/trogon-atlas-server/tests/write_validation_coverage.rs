#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{event_model_service_server::EventModelService, validation_check_result::State};
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

fn domain(namespace: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id(namespace, slug)),
            title: format!("{namespace}/{slug}"),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn put_op(entity: pb::Entity) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
            entity: Some(entity),
            ..Default::default()
        })),
    }
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

fn assert_write_path_skips(coverage: &pb::ValidationCoverage) {
    assert_check(coverage, "model", State::Skipped, "write_path", 0);
    assert_check(coverage, "store_hygiene", State::Skipped, "write_path", 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_reports_only_the_suites_its_kind_reaches() {
    let service = service().await;
    for validate_only in [true, false] {
        let response = service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(event("wvc-put", "placed")),
                validate_only,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        let coverage = response.coverage.expect("PutEntity must report coverage");
        assert_check(&coverage, "request_shape", State::Completed, "", 1);
        assert_check(
            &coverage,
            "architecture",
            State::Skipped,
            "kind_not_applicable",
            0,
        );
        assert_check(
            &coverage,
            "scenarios",
            State::Skipped,
            "kind_not_applicable",
            0,
        );
        assert_check(
            &coverage,
            "strategic",
            State::Skipped,
            "kind_not_applicable",
            0,
        );
        assert_write_path_skips(&coverage);
        assert!(!coverage.snapshot_truncated);
    }

    let coverage = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(domain("wvc-put", "sales")),
            validate_only: true,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .coverage
        .expect("PutEntity must report coverage");
    assert_check(&coverage, "strategic", State::Completed, "", 1);
    assert_check(&coverage, "architecture", State::Completed, "", 1);
    assert_write_path_skips(&coverage);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_reports_coverage_across_its_ops() {
    let service = service().await;
    for validate_only in [true, false] {
        let response = service
            .batch_mutate(Request::new(pb::BatchMutateRequest {
                ops: vec![
                    put_op(event("wvc-batch", "placed")),
                    put_op(domain("wvc-batch", "sales")),
                ],
                validate_only,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        let coverage = response.coverage.expect("BatchMutate must report coverage");
        assert_check(&coverage, "request_shape", State::Completed, "", 2);
        assert_check(&coverage, "strategic", State::Completed, "", 1);
        assert_check(
            &coverage,
            "scenarios",
            State::Skipped,
            "kind_not_applicable",
            0,
        );
        assert_check(&coverage, "architecture", State::Completed, "", 1);
        assert_write_path_skips(&coverage);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_branch_reports_that_it_skips_model_and_scenarios() {
    let service = service().await;
    let branch = "wvc-merge";
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: branch.into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut write = Request::new(pb::PutEntityRequest {
        entity: Some(domain("wvc-merge", "sales")),
        ..Default::default()
    });
    write
        .metadata_mut()
        .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    let branch_coverage = service
        .put_entity(write)
        .await
        .unwrap()
        .into_inner()
        .coverage
        .expect("PutEntity must report coverage");
    assert_eq!(
        branch_coverage.effective_scope.unwrap().branch,
        branch,
        "branch writes validate the branch view"
    );

    for dry_run in [true, false] {
        let response = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: branch.into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        let coverage = response.coverage.expect("MergeBranch must report coverage");
        assert_check(&coverage, "store_hygiene", State::Completed, "", 1);
        assert_check(&coverage, "strategic", State::Completed, "", 1);
        assert_check(&coverage, "architecture", State::Completed, "", 1);
        assert_check(&coverage, "model", State::Skipped, "merge_path", 0);
        assert_check(&coverage, "scenarios", State::Skipped, "merge_path", 0);
        assert_eq!(coverage.requested_scope.unwrap().branch, branch);
        assert_eq!(
            coverage.effective_scope.unwrap().branch,
            "",
            "merge validates the projected baseline"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_failure_after_validation_keeps_coverage() {
    let service = service().await;
    let ui_slice = pb::Entity {
        kind: Some(pb::entity::Kind::UiSlice(pb::UiSlice {
            id: Some(id("wvc-batch-fail", "browse")),
            title: "wvc-batch-fail/browse".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let stale = pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
            entity: Some(event("wvc-batch-fail", "missing")),
            if_match: "1".into(),
            ..Default::default()
        })),
    };
    let response = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![put_op(ui_slice), stale],
            validate_only: true,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        response.status,
        pb::batch_mutate_response::Status::Failed as i32,
        "{response:?}"
    );
    let coverage = response
        .coverage
        .expect("a batch that failed after validation must report coverage");
    assert_check(&coverage, "request_shape", State::Completed, "", 2);
    assert_check(
        &coverage,
        "scenarios",
        State::Skipped,
        "kind_not_applicable",
        0,
    );
    assert_write_path_skips(&coverage);
}
