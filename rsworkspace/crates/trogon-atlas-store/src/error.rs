use thiserror::Error;
use trogon_atlas_core::{Epoch, WriterRole};

use crate::recovery::BatchJournalId;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("entity not found")]
    NotFound,
    #[error("entity already exists")]
    AlreadyExists,
    #[error("etag mismatch: expected {expected}, found {found}")]
    EtagMismatch { expected: String, found: String },
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("backend unavailable: {0}")]
    Unavailable(String),
    #[error("backend error: {0}")]
    Backend(String),
    /// Returned by `batch_apply` when the batch could not be undone in
    /// line, so `keys` may be in an intermediate state. A journaled batch
    /// names its `journal`, which a recovery pass rolls back. Branch
    /// batches and branch merges are not journaled, so their keys need a
    /// manual repair.
    #[error("partial batch apply; keys left in an intermediate state: {keys:?}")]
    PartialApply {
        keys: Vec<String>,
        journal: Option<BatchJournalId>,
    },
    /// The entity write landed in the KV store but the corresponding change
    /// event could not be published to the `JetStream` stream after all retries.
    /// Callers that need a consistent change feed must decide whether to
    /// retry the whole operation or accept the gap.
    #[error("change event publish failed after retries; entity write succeeded but change feed has a gap")]
    ChangeEventLost,
    /// A stored KV entry could not be decoded as a valid `Entity`. The entry
    /// is skipped rather than served, so callers observe it as absent.
    #[error("corrupt entity in store for key {key}: {reason}")]
    CorruptEntry { key: String, reason: String },
    /// Returned by `batch_apply` when op `index` failed. The zero-based
    /// `index` identifies which op failed; `source` carries the original
    /// error so callers can map it to a response without losing type
    /// information. When rollback is incomplete, `source` is
    /// `PartialApply`; otherwise it is the error from the failing op.
    #[error("batch op {index} failed: {source}")]
    BatchFailed {
        index: usize,
        #[source]
        source: Box<StoreError>,
    },
    /// The KV bucket's recorded schema version does not match the version
    /// compiled into this binary. A data migration is required before the
    /// store can be opened.
    #[error(
        "schema version mismatch: bucket contains \"{found}\", binary expects \"{expected}\"; \
         a migration is required before opening this store"
    )]
    SchemaMismatch { expected: String, found: String },
    /// The store still holds Event, Command or ReadModel rows in the retired
    /// `fields` layout. Serving it would drop those lists on the first write.
    #[error(
        "{rows} stored entity image(s) still use the retired `fields` layout; stop every \
         trogon-atlas-server process on this store, run `trogon-atlas-server \
         migrate-legacy-fields` to preview and `trogon-atlas-server migrate-legacy-fields \
         --apply` to rewrite them, then start this version again"
    )]
    LegacyFieldsUnmigrated { rows: usize },
    /// This process attempted a mutation without holding the writer lease.
    /// `role` is this process's configured role and `epoch` is the lease
    /// epoch it last observed; neither proves who currently holds it.
    #[error("this process is not the current writer (role: {role}, epoch: {epoch})")]
    NotWriter { role: WriterRole, epoch: Epoch },
}

pub type StoreResult<T> = Result<T, StoreError>;

impl StoreError {
    /// Whether this error reports a batch left partially applied, which
    /// callers must not present as "nothing was persisted".
    #[must_use]
    pub fn is_partial_apply(&self) -> bool {
        match self {
            Self::PartialApply { .. } => true,
            Self::BatchFailed { source, .. } => source.is_partial_apply(),
            _ => false,
        }
    }
}
