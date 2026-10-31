pub mod error;
pub mod key;
pub mod metrics;
pub mod nats;
pub mod recovery;
pub mod source;
pub mod store;

// `refs` moved to the dedicated `trogon-atlas-core` crate so any third-party
// storage backend can reach the graph-walking primitives without depending
// on the storage layer. Re-exported here so existing call sites that
// reach for `trogon_atlas_store::refs` keep compiling -- new code should
// depend on `trogon-atlas-core` directly.
pub use error::{StoreError, StoreResult};
pub use metrics::StoreMetrics;
pub use nats::{
    ConflictResolution, LegacyFieldsFinding, LegacyFieldsLocation, LegacyFieldsReport,
    LegacyFieldsShape, MigrationMode, NatsCredentials, NatsStore, NatsStoreConfig, StoreOpenMode,
    STORE_SCHEMA_MARKER,
};
pub use recovery::{
    BatchJournalId, BatchRecovery, BatchRecoveryReport, JournalPhase, RecoveryAction, RecoveryMode,
    RecoveryPolicy, RecoveryResult,
};
pub use store::{
    ChangeKind, ChangeRecord, ChangeStreamStats, ChangesetOp, ChangesetPage, ChangesetRecord,
    ChangesetRef, ChangesetScope, ChangesetValidation, ClaimOutcome, EntityRevisionRecord,
    IssueCounts, NamespaceClaim, NamespaceRecord, NamespaceTenure, OperationRecord,
    OperationStatus, ReceiptCoverage, RevisionPage, RuleCatalogDigest, Store, StoredEntity,
    UnvalidatedReason, ValidationReceipt, WriteContext,
};
pub use trogon_atlas_core::refs;
