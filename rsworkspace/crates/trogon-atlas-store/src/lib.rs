pub mod error;
pub mod key;
pub mod nats;
pub mod recovery;
pub mod store;

// `refs` moved to the dedicated `trogon-atlas-core` crate so any third-party
// storage backend can reach the graph-walking primitives without depending
// on the storage layer. Re-exported here so existing call sites that
// reach for `trogon_atlas_store::refs` keep compiling -- new code should
// depend on `trogon-atlas-core` directly.
pub use error::{StoreError, StoreResult};
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
    ChangesetRef, ChangesetScope, ClaimOutcome, EntityRevisionRecord, NamespaceClaim,
    NamespaceRecord, NamespaceTenure, OperationRecord, OperationStatus, RevisionPage, Store,
    StoredEntity, WriteContext,
};
pub use trogon_atlas_core::refs;
