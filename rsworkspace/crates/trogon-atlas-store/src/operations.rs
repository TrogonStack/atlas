//! Operation receipts: idempotency claims for mutations that carry a
//! client-supplied `operation_id`, backed by the `trogon-atlas-operations`
//! KV bucket. See `OperationReceipt` / `GetOperation` in `service.proto`,
//! and `Store::claim_operation` for the vocabulary this implements.
//!
//! A claim is settled at most once per attempt: the RPC handler settles it
//! after the mutation it guards finishes (success or failure), and a crash
//! recovery pass settles it again for a batch that crashed before the
//! handler could. Settling is best-effort: the changeset, not the receipt,
//! is the source of truth, so a failed settle only costs a future retry a
//! fresh claim instead of a replay.

use bytes::Bytes;
use prost::Message as _;

use super::{kv_entry_timeout, live_entry, now_micros, NatsStore, CAS_MAX_ATTEMPTS, KV_OP_TIMEOUT};
use crate::{
    error::{StoreError, StoreResult},
    store::{ClaimOutcome, OperationRecord, OperationStatus},
};

/// On-disk encoding of an [`OperationRecord`] in the `trogon-atlas-operations`
/// KV bucket. Kept private and hand-rolled (same pattern as `JournalHeader`
/// in `nats_journal.rs`) rather than reusing a public `service.proto`
/// message, since this row carries fields (`digest`) that are never part of
/// the client-facing contract.
#[derive(Clone, PartialEq, ::prost::Message)]
struct OperationRow {
    #[prost(string, tag = "1")]
    digest: String,
    #[prost(string, tag = "2")]
    rpc: String,
    #[prost(string, tag = "3")]
    branch: String,
    #[prost(int32, tag = "4")]
    status: i32,
    #[prost(string, tag = "5")]
    changeset_id: String,
    #[prost(string, tag = "6")]
    rejection_code: String,
    #[prost(string, tag = "7")]
    rejection_message: String,
    #[prost(int64, tag = "8")]
    updated_at_micros: i64,
}

fn status_code(status: OperationStatus) -> i32 {
    match status {
        OperationStatus::Pending => 0,
        OperationStatus::Applied => 1,
        OperationStatus::Rejected => 2,
        OperationStatus::NotApplied => 3,
    }
}

fn status_from_code(code: i32) -> OperationStatus {
    match code {
        1 => OperationStatus::Applied,
        2 => OperationStatus::Rejected,
        3 => OperationStatus::NotApplied,
        _ => OperationStatus::Pending,
    }
}

impl From<OperationRow> for OperationRecord {
    fn from(row: OperationRow) -> Self {
        Self {
            digest: row.digest,
            rpc: row.rpc,
            branch: (!row.branch.is_empty()).then_some(row.branch),
            status: status_from_code(row.status),
            changeset_id: row.changeset_id,
            rejection_code: row.rejection_code,
            rejection_message: row.rejection_message,
        }
    }
}

fn fresh_row(digest: &str, rpc: &str, branch: Option<&str>) -> OperationRow {
    OperationRow {
        digest: digest.to_owned(),
        rpc: rpc.to_owned(),
        branch: branch.unwrap_or_default().to_owned(),
        status: status_code(OperationStatus::Pending),
        changeset_id: String::new(),
        rejection_code: String::new(),
        rejection_message: String::new(),
        updated_at_micros: now_micros(),
    }
}

impl NatsStore {
    /// Claim `key` for a mutation whose canonical request digest is
    /// `digest`. See [`crate::store::Store::claim_operation`].
    pub(crate) async fn claim_operation(
        &self,
        key: &str,
        digest: &str,
        rpc: &str,
        branch: Option<&str>,
    ) -> StoreResult<ClaimOutcome> {
        for _ in 0..CAS_MAX_ATTEMPTS {
            let current = kv_entry_timeout(&self.operations_kv, key.to_owned()).await?;
            match live_entry(current) {
                None => {
                    let bytes = Bytes::from(fresh_row(digest, rpc, branch).encode_to_vec());
                    match tokio::time::timeout(KV_OP_TIMEOUT, self.operations_kv.create(key, bytes))
                        .await
                        .map_err(|_| {
                            StoreError::Backend(format!("operation {key} create: timeout"))
                        })? {
                        Ok(_) => return Ok(ClaimOutcome::Claimed),
                        Err(e) => {
                            use async_nats::jetstream::kv::CreateErrorKind;
                            if matches!(e.kind(), CreateErrorKind::AlreadyExists) {
                                continue;
                            }
                            return Err(StoreError::Backend(format!(
                                "operation {key} create: {e}"
                            )));
                        }
                    }
                }
                Some(entry) => {
                    let row = OperationRow::decode(entry.value.as_ref())
                        .map_err(|e| StoreError::Backend(format!("decode operation {key}: {e}")))?;
                    if row.digest != digest {
                        return Ok(ClaimOutcome::DigestMismatch);
                    }
                    match status_from_code(row.status) {
                        OperationStatus::Pending => return Ok(ClaimOutcome::InProgress),
                        OperationStatus::Applied | OperationStatus::Rejected => {
                            return Ok(ClaimOutcome::Replay(row.into()));
                        }
                        OperationStatus::NotApplied => {
                            let bytes = Bytes::from(fresh_row(digest, rpc, branch).encode_to_vec());
                            match tokio::time::timeout(
                                KV_OP_TIMEOUT,
                                self.operations_kv.update(key, bytes, entry.revision),
                            )
                            .await
                            .map_err(|_| {
                                StoreError::Backend(format!("operation {key} update: timeout"))
                            })? {
                                Ok(_) => return Ok(ClaimOutcome::Claimed),
                                Err(e) => {
                                    use async_nats::jetstream::kv::UpdateErrorKind;
                                    if matches!(e.kind(), UpdateErrorKind::WrongLastRevision) {
                                        continue;
                                    }
                                    return Err(StoreError::Backend(format!(
                                        "operation {key} update: {e}"
                                    )));
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(StoreError::Unavailable(format!(
            "operation {key}: claim CAS loop exhausted after {CAS_MAX_ATTEMPTS} attempts"
        )))
    }

    async fn settle_operation(
        &self,
        key: &str,
        mutate: impl Fn(&mut OperationRow),
    ) -> StoreResult<()> {
        for attempt in 0..CAS_MAX_ATTEMPTS {
            let Some(entry) =
                live_entry(kv_entry_timeout(&self.operations_kv, key.to_owned()).await?)
            else {
                // Nothing to settle: the claim already expired (retention)
                // or was never made under this key. Settling is best-effort
                // bookkeeping, never the source of truth, so this is not an
                // error.
                return Ok(());
            };
            let mut row = OperationRow::decode(entry.value.as_ref())
                .map_err(|e| StoreError::Backend(format!("decode operation {key}: {e}")))?;
            mutate(&mut row);
            row.updated_at_micros = now_micros();
            let bytes = Bytes::from(row.encode_to_vec());
            match tokio::time::timeout(
                KV_OP_TIMEOUT,
                self.operations_kv.update(key, bytes, entry.revision),
            )
            .await
            .map_err(|_| StoreError::Backend(format!("operation {key} settle: timeout")))?
            {
                Ok(_) => return Ok(()),
                Err(e) => {
                    use async_nats::jetstream::kv::UpdateErrorKind;
                    if matches!(e.kind(), UpdateErrorKind::WrongLastRevision)
                        && attempt + 1 < CAS_MAX_ATTEMPTS
                    {
                        continue;
                    }
                    return Err(StoreError::Backend(format!("operation {key} settle: {e}")));
                }
            }
        }
        Err(StoreError::Unavailable(format!(
            "operation {key}: settle CAS loop exhausted after {CAS_MAX_ATTEMPTS} attempts"
        )))
    }

    /// Settle a claim as applied. See
    /// [`crate::store::Store::settle_operation_applied`].
    pub(crate) async fn settle_operation_applied(
        &self,
        key: &str,
        changeset_id: &str,
    ) -> StoreResult<()> {
        self.settle_operation(key, |row| {
            row.status = status_code(OperationStatus::Applied);
            changeset_id.clone_into(&mut row.changeset_id);
        })
        .await
    }

    /// Settle a claim as rejected. See
    /// [`crate::store::Store::settle_operation_rejected`].
    pub(crate) async fn settle_operation_rejected(
        &self,
        key: &str,
        code: &str,
        message: &str,
    ) -> StoreResult<()> {
        self.settle_operation(key, |row| {
            row.status = status_code(OperationStatus::Rejected);
            code.clone_into(&mut row.rejection_code);
            message.clone_into(&mut row.rejection_message);
        })
        .await
    }

    /// Settle a claim as not-applied. See
    /// [`crate::store::Store::settle_operation_not_applied`].
    pub(crate) async fn settle_operation_not_applied(&self, key: &str) -> StoreResult<()> {
        self.settle_operation(key, |row| {
            row.status = status_code(OperationStatus::NotApplied);
        })
        .await
    }

    /// Look up a claim by key. See [`crate::store::Store::get_operation`].
    pub(crate) async fn get_operation(&self, key: &str) -> StoreResult<Option<OperationRecord>> {
        let Some(entry) = live_entry(kv_entry_timeout(&self.operations_kv, key.to_owned()).await?)
        else {
            return Ok(None);
        };
        OperationRow::decode(entry.value.as_ref())
            .map(|row| Some(row.into()))
            .map_err(|e| StoreError::Backend(format!("decode operation {key}: {e}")))
    }
}
