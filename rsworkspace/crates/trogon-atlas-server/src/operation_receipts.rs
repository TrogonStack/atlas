//! Operation receipts: the gRPC-boundary wiring for mutations that carry a
//! client-supplied `operation_id`. See `OperationReceipt` / `GetOperation`
//! in `service.proto` for the contract, and
//! `trogon_atlas_store::store::Store::claim_operation` for the claim/settle
//! vocabulary this builds on.
//!
//! A handler's shape is always: validate the id and the flags that do not
//! compose with it, run every check that does not itself write, claim right
//! before the write it guards, then settle once the write (or its failure)
//! is known. Settling happens for every outcome of a claimed attempt --
//! `Claim::Proceed` is a promise to call exactly one of [`settle_applied`],
//! [`settle_not_applied`], or [`settle_for_status`] before returning.

use prost::Message;
use tonic::{Code, Status};
use trogon_atlas_core::{
    operation_id::{operation_claim_key, operation_digest, validate_operation_id},
    transcode::message_json_value,
};
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    store::{ClaimOutcome, OperationRecord, OperationStatus},
    Store,
};

use crate::conv::{operation_id_reused, operation_in_progress, store_err};

/// A validated `operation_id` together with the canonical digest of the
/// request it was submitted with, bundled so a handler cannot claim against
/// one request's digest and settle under a different key.
pub struct OperationAttempt {
    pub claim_key: String,
    pub digest: String,
}

impl OperationAttempt {
    /// Validate `operation_id`'s shape and compute the digest of
    /// `request_without_operation_id` (a clone with `operation_id` already
    /// cleared -- the digest must not depend on the very field that selects
    /// which digest to compare against).
    pub fn new<M: Message>(
        principal_name: &str,
        operation_id: &str,
        rpc: &str,
        branch: Option<&str>,
        full_name: &str,
        request_without_operation_id: &M,
    ) -> Result<Self, Status> {
        validate_operation_id(operation_id).map_err(|e| Status::invalid_argument(e.to_string()))?;
        let json = message_json_value(full_name, request_without_operation_id)
            .map_err(|e| Status::internal(format!("failed to compute operation digest: {e}")))?;
        let digest = operation_digest(rpc, branch, &json)
            .map_err(|e| Status::internal(format!("failed to compute operation digest: {e}")))?;
        Ok(Self {
            claim_key: operation_claim_key(principal_name, operation_id),
            digest,
        })
    }
}

/// What a handler does after [`claim`] returns: either proceed with the
/// write it is about to settle, or short-circuit with the previously
/// settled outcome.
pub enum Claim {
    Proceed,
    Replay(OperationRecord),
}

/// Claim `attempt.claim_key` for `rpc`/`branch`. Must be called after every
/// check that does not itself write, so a deterministically-failing retry
/// never consumes a claim slot that would otherwise block a differently
/// shaped retry under the same id, and before the write it guards, so two
/// concurrent retries of the same `operation_id` agree on exactly one
/// winner.
pub async fn claim(
    store: &dyn Store,
    attempt: &OperationAttempt,
    rpc: &str,
    branch: Option<&str>,
) -> Result<Claim, Status> {
    match store
        .claim_operation(&attempt.claim_key, &attempt.digest, rpc, branch)
        .await
    {
        Ok(ClaimOutcome::Claimed) => Ok(Claim::Proceed),
        Ok(ClaimOutcome::Replay(record)) => Ok(Claim::Replay(record)),
        Ok(ClaimOutcome::InProgress) => Err(operation_in_progress(
            "operation_id is already being applied by another call; retry shortly",
        )),
        Ok(ClaimOutcome::DigestMismatch) => Err(operation_id_reused(
            "operation_id was already used for a request with a different digest",
        )),
        Err(e) => Err(store_err(e)),
    }
}

/// Reject `operation_id` combined with a flag that persists nothing. The two
/// do not compose: idempotent replay is a property of a write that actually
/// happened, and a dry run / validate-only call never produces one.
pub fn reject_operation_id_with_dry_run(operation_id: &str, dry_run: bool) -> Result<(), Status> {
    if !operation_id.is_empty() && dry_run {
        return Err(Status::invalid_argument(
            "operation_id must not be set together with validate_only/dry_run",
        ));
    }
    Ok(())
}

/// Reject `operation_id` set on a nested batch op: a batch is one unit of
/// idempotency, selected by `BatchMutateRequest.operation_id`, not several.
pub fn reject_nested_operation_id(operation_id: &str, index: usize) -> Result<(), Status> {
    if !operation_id.is_empty() {
        return Err(Status::invalid_argument(format!(
            "ops[{index}]: operation_id must not be set on a nested batch op; set it on the \
             batch request instead"
        )));
    }
    Ok(())
}

/// Best-effort settle as applied. Settling is bookkeeping, never the source
/// of truth for whether the mutation happened, so a failure here is logged
/// and swallowed rather than surfaced to the caller.
pub async fn settle_applied(store: &dyn Store, claim_key: &str, changeset_id: &str) {
    if let Err(e) = store
        .settle_operation_applied(claim_key, changeset_id)
        .await
    {
        tracing::warn!(error = %e, claim_key, "failed to settle operation receipt as applied");
    }
}

/// Best-effort settle as not-applied: nothing landed, so a retry with the
/// same id and digest is free to claim again.
pub async fn settle_not_applied(store: &dyn Store, claim_key: &str) {
    if let Err(e) = store.settle_operation_not_applied(claim_key).await {
        tracing::warn!(error = %e, claim_key, "failed to settle operation receipt as not applied");
    }
}

/// Best-effort settle the failure `status` produced after a successful
/// claim. Transport/operational failures (`UNAVAILABLE`, `INTERNAL`,
/// `UNKNOWN`, `DEADLINE_EXCEEDED`) settle not-applied, since nothing is known
/// to have landed and a retry should be free to reclaim the id; every other
/// code is a deterministic business rejection and settles rejected, so a
/// replay returns the same failure instead of a generic one.
pub async fn settle_for_status(store: &dyn Store, claim_key: &str, status: &Status) {
    let code = status.code();
    if matches!(
        code,
        Code::Unavailable | Code::Internal | Code::Unknown | Code::DeadlineExceeded
    ) {
        settle_not_applied(store, claim_key).await;
        return;
    }
    if let Err(e) = store
        .settle_operation_rejected(claim_key, grpc_code_name(code), status.message())
        .await
    {
        tracing::warn!(error = %e, claim_key, "failed to settle operation receipt as rejected");
    }
}

/// Build the `OperationReceipt` for a fresh (non-replayed) settle.
pub fn fresh_receipt(operation_id: &str, changeset_id: &str) -> pb::OperationReceipt {
    pb::OperationReceipt {
        operation_id: operation_id.to_owned(),
        changeset_id: changeset_id.to_owned(),
        replayed: false,
    }
}

/// Build the `OperationReceipt` for a replayed `Applied` outcome.
pub fn replay_receipt(operation_id: &str, record: &OperationRecord) -> pb::OperationReceipt {
    pb::OperationReceipt {
        operation_id: operation_id.to_owned(),
        changeset_id: record.changeset_id.clone(),
        replayed: true,
    }
}

/// Reconstruct the `Status` a replayed `Rejected` claim originally returned.
pub fn replay_rejected_status(record: &OperationRecord) -> Status {
    Status::new(
        grpc_code_from_name(&record.rejection_code),
        record.rejection_message.clone(),
    )
}

/// `trogon_atlas_store::store::OperationStatus` (a claim's status) to
/// `pb::OperationStatus`. The server-level states `UNSPECIFIED` (no record
/// at all) and `UNKNOWN` (branch-scoped; no durable receipt) are not claim
/// states, so they are not representable here -- `GetOperation`'s handler
/// supplies them directly.
pub fn pb_operation_status(status: OperationStatus) -> pb::OperationStatus {
    match status {
        OperationStatus::Pending => pb::OperationStatus::Pending,
        OperationStatus::Applied => pb::OperationStatus::Applied,
        OperationStatus::Rejected => pb::OperationStatus::Rejected,
        OperationStatus::NotApplied => pb::OperationStatus::NotApplied,
    }
}

/// The gRPC status code's SCREAMING_SNAKE_CASE name, e.g. `ALREADY_EXISTS`.
/// `tonic::Code`'s `Display` prints a long human-readable description
/// instead, which is not what `GetOperationResponse.rejection_code`
/// documents, so this is hand-rolled rather than `code.to_string()`.
pub fn grpc_code_name(code: Code) -> &'static str {
    match code {
        Code::Ok => "OK",
        Code::Cancelled => "CANCELLED",
        Code::Unknown => "UNKNOWN",
        Code::InvalidArgument => "INVALID_ARGUMENT",
        Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
        Code::NotFound => "NOT_FOUND",
        Code::AlreadyExists => "ALREADY_EXISTS",
        Code::PermissionDenied => "PERMISSION_DENIED",
        Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        Code::FailedPrecondition => "FAILED_PRECONDITION",
        Code::Aborted => "ABORTED",
        Code::OutOfRange => "OUT_OF_RANGE",
        Code::Unimplemented => "UNIMPLEMENTED",
        Code::Internal => "INTERNAL",
        Code::Unavailable => "UNAVAILABLE",
        Code::DataLoss => "DATA_LOSS",
        Code::Unauthenticated => "UNAUTHENTICATED",
    }
}

/// Inverse of [`grpc_code_name`]. An unknown or empty name maps to
/// `Code::Unknown`, the honest answer for a record settled before this
/// mapping existed or holding something that is not a documented name.
pub fn grpc_code_from_name(name: &str) -> Code {
    match name {
        "OK" => Code::Ok,
        "CANCELLED" => Code::Cancelled,
        "INVALID_ARGUMENT" => Code::InvalidArgument,
        "DEADLINE_EXCEEDED" => Code::DeadlineExceeded,
        "NOT_FOUND" => Code::NotFound,
        "ALREADY_EXISTS" => Code::AlreadyExists,
        "PERMISSION_DENIED" => Code::PermissionDenied,
        "RESOURCE_EXHAUSTED" => Code::ResourceExhausted,
        "FAILED_PRECONDITION" => Code::FailedPrecondition,
        "ABORTED" => Code::Aborted,
        "OUT_OF_RANGE" => Code::OutOfRange,
        "UNIMPLEMENTED" => Code::Unimplemented,
        "INTERNAL" => Code::Internal,
        "UNAVAILABLE" => Code::Unavailable,
        "DATA_LOSS" => Code::DataLoss,
        "UNAUTHENTICATED" => Code::Unauthenticated,
        _ => Code::Unknown,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn grpc_code_name_round_trips_through_from_name() {
        let codes = [
            Code::Ok,
            Code::Cancelled,
            Code::Unknown,
            Code::InvalidArgument,
            Code::DeadlineExceeded,
            Code::NotFound,
            Code::AlreadyExists,
            Code::PermissionDenied,
            Code::ResourceExhausted,
            Code::FailedPrecondition,
            Code::Aborted,
            Code::OutOfRange,
            Code::Unimplemented,
            Code::Internal,
            Code::Unavailable,
            Code::DataLoss,
            Code::Unauthenticated,
        ];
        for code in codes {
            assert_eq!(grpc_code_from_name(grpc_code_name(code)), code);
        }
    }

    #[test]
    fn grpc_code_from_name_defaults_unknown_names_to_unknown() {
        assert_eq!(grpc_code_from_name(""), Code::Unknown);
        assert_eq!(grpc_code_from_name("NOT_A_REAL_CODE"), Code::Unknown);
    }

    #[test]
    fn reject_operation_id_with_dry_run_only_fires_when_both_are_set() {
        assert!(reject_operation_id_with_dry_run("", true).is_ok());
        assert!(reject_operation_id_with_dry_run("op-1", false).is_ok());
        let err = reject_operation_id_with_dry_run("op-1", true).unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn reject_nested_operation_id_only_fires_when_set() {
        assert!(reject_nested_operation_id("", 0).is_ok());
        let err = reject_nested_operation_id("op-1", 2).unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
        assert!(err.message().contains("ops[2]"));
    }
}
