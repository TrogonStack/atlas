#![allow(clippy::unwrap_used, clippy::expect_used)]

// Operation receipts: a client-supplied operation_id makes a mutation
// safely retriable. Same id + same request digest replays the stored
// outcome instead of re-executing; same id + a different digest is refused.
// In-process handler calls, same pattern as tests/handler_tests.rs, with a
// manual pre-claim to simulate the in-flight-elsewhere race that
// lock_mutations() makes impossible to trigger via two real concurrent
// calls on one service instance.

use std::sync::Arc;

use tonic::{Code, Request};
use tonic_types::StatusExt;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::{operation_receipts::OperationAttempt, service::EventModelServiceImpl};
use trogon_atlas_store::Store;

fn id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn event(ns: &str, slug: &str, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug)),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn put_req(entity: pb::Entity, operation_id: &str) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: operation_id.into(),
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }
}

async fn svc_with_store() -> (EventModelServiceImpl, Arc<dyn Store>) {
    let nats = trogon_atlas_testsupport::shared().await;
    let store: Arc<dyn Store> = Arc::new(nats.store().await);
    let svc = EventModelServiceImpl::try_new(Arc::clone(&store)).unwrap();
    (svc, store)
}

// ---------------------------------------------------------------------------
// Replay: same operation_id, same digest
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_replays_same_digest_without_reexecuting() {
    let (svc, _store) = svc_with_store().await;
    let ns = "oprcpt-replay";

    let first = svc
        .put_entity(Request::new(put_req(
            event(ns, "thing.happened", "Thing"),
            "op-replay-1",
        )))
        .await
        .unwrap()
        .into_inner();
    let receipt = first
        .operation_receipt
        .expect("claimed call carries a receipt");
    assert!(
        !receipt.replayed,
        "first call must not be reported as a replay"
    );
    assert_ne!(receipt.changeset_id, "");

    let second = svc
        .put_entity(Request::new(put_req(
            event(ns, "thing.happened", "Thing"),
            "op-replay-1",
        )))
        .await
        .unwrap()
        .into_inner();
    let replay_receipt = second
        .operation_receipt
        .expect("replayed call still carries a receipt");
    assert!(
        replay_receipt.replayed,
        "second call must be reported as a replay"
    );
    assert_eq!(
        replay_receipt.changeset_id, receipt.changeset_id,
        "replay must report the original changeset, not a new one"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_entity_replays_same_digest_without_reexecuting() {
    let (svc, _store) = svc_with_store().await;
    let ns = "oprcpt-delete-replay";

    svc.put_entity(Request::new(put_req(
        event(ns, "thing.happened", "Thing"),
        "",
    )))
    .await
    .unwrap();

    let del_req = |operation_id: &str| pb::DeleteEntityRequest {
        operation_id: operation_id.into(),
        kind: pb::EntityKind::Event as i32,
        id: Some(id(ns, "thing.happened")),
        mode: pb::delete_entity_request::Mode::Force as i32,
        if_match: String::new(),
    };

    let first = svc
        .delete_entity(Request::new(del_req("op-delete-1")))
        .await
        .unwrap()
        .into_inner();
    let receipt = first
        .operation_receipt
        .expect("claimed call carries a receipt");
    assert!(!receipt.replayed);

    let second = svc
        .delete_entity(Request::new(del_req("op-delete-1")))
        .await
        .unwrap()
        .into_inner();
    let replay_receipt = second
        .operation_receipt
        .expect("replayed call still carries a receipt");
    assert!(replay_receipt.replayed);
    assert_eq!(replay_receipt.changeset_id, receipt.changeset_id);
}

// ---------------------------------------------------------------------------
// Reuse with a different digest: ALREADY_EXISTS
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_reused_operation_id_with_different_digest_fails_already_exists() {
    let (svc, _store) = svc_with_store().await;
    let ns = "oprcpt-reuse";

    svc.put_entity(Request::new(put_req(
        event(ns, "thing.happened", "Thing"),
        "op-reuse-1",
    )))
    .await
    .unwrap();

    let err = svc
        .put_entity(Request::new(put_req(
            event(ns, "thing.happened", "A different title"),
            "op-reuse-1",
        )))
        .await
        .expect_err("same operation_id with a different request digest must be refused");
    assert_eq!(err.code(), Code::AlreadyExists);
    let info = err
        .get_details_error_info()
        .expect("OPERATION_ID_REUSED must carry ErrorInfo");
    assert_eq!(
        info.reason,
        trogon_atlas_server::conv::reason::OPERATION_ID_REUSED
    );
}

// ---------------------------------------------------------------------------
// operation_id does not compose with validate_only / dry-run
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_rejects_operation_id_with_validate_only() {
    let (svc, _store) = svc_with_store().await;
    let mut req = put_req(
        event("oprcpt-validate-only", "thing.happened", "Thing"),
        "op-vo-1",
    );
    req.validate_only = true;

    let err = svc
        .put_entity(Request::new(req))
        .await
        .expect_err("operation_id combined with validate_only must be refused");
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_entity_rejects_operation_id_with_dry_run_mode() {
    let (svc, _store) = svc_with_store().await;
    let ns = "oprcpt-delete-dry-run";
    svc.put_entity(Request::new(put_req(
        event(ns, "thing.happened", "Thing"),
        "",
    )))
    .await
    .unwrap();

    let err = svc
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            operation_id: "op-dr-1".into(),
            kind: pb::EntityKind::Event as i32,
            id: Some(id(ns, "thing.happened")),
            mode: pb::delete_entity_request::Mode::DryRun as i32,
            if_match: String::new(),
        }))
        .await
        .expect_err("operation_id combined with MODE_DRY_RUN must be refused");
    assert_eq!(err.code(), Code::InvalidArgument);
}

// ---------------------------------------------------------------------------
// operation_id does not compose with a nested batch op
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_rejects_operation_id_on_nested_put() {
    let (svc, _store) = svc_with_store().await;
    let req = pb::BatchMutateRequest {
        operation_id: String::new(),
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(put_req(
                event("oprcpt-batch-nested", "thing.happened", "Thing"),
                "op-nested-1",
            ))),
        }],
        validate_only: false,
    };

    let err = svc
        .batch_mutate(Request::new(req))
        .await
        .expect_err("operation_id on a nested put must be refused");
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("ops[0]"));
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_rejects_operation_id_on_nested_delete() {
    let (svc, _store) = svc_with_store().await;
    let ns = "oprcpt-batch-nested-delete";
    svc.put_entity(Request::new(put_req(
        event(ns, "thing.happened", "Thing"),
        "",
    )))
    .await
    .unwrap();

    let req = pb::BatchMutateRequest {
        operation_id: String::new(),
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                operation_id: "op-nested-2".into(),
                kind: pb::EntityKind::Event as i32,
                id: Some(id(ns, "thing.happened")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                if_match: String::new(),
            })),
        }],
        validate_only: false,
    };

    let err = svc
        .batch_mutate(Request::new(req))
        .await
        .expect_err("operation_id on a nested delete must be refused");
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("ops[0]"));
}

// ---------------------------------------------------------------------------
// Concurrent claim: another call already owns this operation_id
// ---------------------------------------------------------------------------

/// `lock_mutations()` serializes every write the service handles, so two
/// real concurrent `put_entity` calls on one service instance can never
/// race each other into `claim_operation`. Instead, this claims the key
/// directly against the store first (the same thing a concurrent call
/// would have done), leaving it Pending the way an in-flight call would,
/// then drives the handler against that same id and digest.
#[tokio::test(flavor = "multi_thread")]
async fn put_entity_claim_in_progress_returns_unavailable() {
    let (svc, store) = svc_with_store().await;
    let ns = "oprcpt-in-progress";
    let entity = event(ns, "thing.happened", "Thing");

    let digest_req = pb::PutEntityRequest {
        entity: Some(entity.clone()),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    };
    let attempt = OperationAttempt::new(
        "unauthenticated",
        "op-in-progress-1",
        "PutEntity",
        None,
        "trogonatlas.api.eventmodel.v1alpha1.PutEntityRequest",
        &digest_req,
    )
    .unwrap();
    store
        .claim_operation(&attempt.claim_key, &attempt.digest, "PutEntity", None)
        .await
        .unwrap();

    let err = svc
        .put_entity(Request::new(put_req(entity, "op-in-progress-1")))
        .await
        .expect_err("a claim left Pending by another call must block this one");
    assert_eq!(err.code(), Code::Unavailable);
    let info = err
        .get_details_error_info()
        .expect("OPERATION_IN_PROGRESS must carry ErrorInfo");
    assert_eq!(
        info.reason,
        trogon_atlas_server::conv::reason::OPERATION_IN_PROGRESS
    );
}

// ---------------------------------------------------------------------------
// GetOperation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_operation_reports_unspecified_for_an_unknown_id() {
    let (svc, _store) = svc_with_store().await;
    let resp = svc
        .get_operation(Request::new(pb::GetOperationRequest {
            operation_id: "op-never-used".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.status, pb::OperationStatus::Unspecified as i32);
    assert_eq!(resp.changeset_id, "");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_operation_reports_applied_after_a_claimed_put() {
    let (svc, _store) = svc_with_store().await;
    let ns = "oprcpt-get-operation";

    let put = svc
        .put_entity(Request::new(put_req(
            event(ns, "thing.happened", "Thing"),
            "op-get-1",
        )))
        .await
        .unwrap()
        .into_inner();
    let changeset_id = put.operation_receipt.unwrap().changeset_id;

    let resp = svc
        .get_operation(Request::new(pb::GetOperationRequest {
            operation_id: "op-get-1".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.status, pb::OperationStatus::Applied as i32);
    assert_eq!(resp.changeset_id, changeset_id);
    assert_eq!(resp.rejection_code, "");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_operation_requires_a_valid_operation_id() {
    let (svc, _store) = svc_with_store().await;
    let err = svc
        .get_operation(Request::new(pb::GetOperationRequest {
            operation_id: "has a space".into(),
        }))
        .await
        .expect_err("a malformed operation_id must be refused");
    assert_eq!(err.code(), Code::InvalidArgument);
}
