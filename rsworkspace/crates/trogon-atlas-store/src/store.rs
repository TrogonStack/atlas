use async_trait::async_trait;
use tokio::sync::broadcast;
use trogon_atlas_core::{Epoch, NamespaceId, NamespaceName, OwnerId, WriterRole};
use trogon_atlas_proto::{Entity, EntityKind, EntityRef, Id};

use crate::error::{StoreError, StoreResult};

/// One raw copy-on-write delta row belonging to a branch (Phase 2: Review
/// and merge). Unlike the merged view returned by `list`/`get` when
/// `branch` is set, this exposes the delta's full `base`/`base_etag`/
/// `ours`/`tombstone` payload so callers can classify it against the
/// current baseline (see `docs/explanation/branching.md`, "Merge
/// conflicts").
#[derive(Debug, Clone)]
pub struct BranchDeltaEntry {
    pub kind: EntityKind,
    pub id: Id,
    /// The baseline entity as it existed the first time this key was
    /// touched on the branch. `None` when the branch created the key.
    pub base: Option<Entity>,
    /// Etag of `base` at capture time. Empty when `base` is `None`.
    pub base_etag: String,
    /// The branch's current value. `None` when `tombstone` is true.
    pub ours: Option<Entity>,
    pub tombstone: bool,
    /// Revision of the delta row itself (the branch KV entry), not of
    /// baseline.
    pub etag: String,
}

/// One key to land on baseline as part of a branch merge, plus the
/// baseline etag it was diffed against (used as an optimistic-concurrency
/// guard so a concurrent baseline writer racing the merge is detected
/// rather than silently overwritten).
#[derive(Debug, Clone)]
pub enum BranchLandOp {
    /// Write `ours` to baseline verbatim -- no `system` re-stamping. Used
    /// for ADDED/CHANGED entries (and CONFLICT_DELETE_EDIT resolved via
    /// take-ours, if ever surfaced that way).
    Put {
        kind: EntityKind,
        ours: Box<Entity>,
        /// Baseline etag this was diffed against. `None` means baseline
        /// had no row for this key at diff time.
        expected_baseline_etag: Option<String>,
    },
    /// Delete the baseline row. Used for DELETED entries.
    Delete {
        kind: EntityKind,
        id: Id,
        expected_baseline_etag: String,
    },
}

#[derive(Debug, Clone)]
pub struct StoredEntity {
    pub entity: Entity,
    pub etag: String,
}

/// Metadata for a branch (see `docs/explanation/branching.md`, Phase 1:
/// Isolation). A branch is a named overlay of copy-on-write deltas over
/// the baseline store; this struct is the row persisted at the branch's
/// `meta.<name>` key.
#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub doc: String,
    /// RFC3339 timestamp.
    pub created_at: String,
    /// Opaque change-feed cursor captured at creation time.
    pub base_change_token: String,
    pub delta_count: u32,
    /// Id of the baseline changeset this branch forked from, or the one
    /// `UpdateBranch` last caught it up to. `None` when the changeset log
    /// held no baseline record at that moment.
    ///
    /// Changeset ids are UUIDv7, so this is a position in the log: every
    /// baseline changeset ordered above it landed after the fork. That is
    /// what makes branch-level ancestry a comparison of two positions
    /// instead of a walk over every per-key base.
    pub fork_changeset_id: Option<String>,
}

/// One namespace's registry row: who owns it, what it is called, and the
/// immutable id every storage key for it is built from.
///
/// This is administrative infrastructure, not model content. It sits in its
/// own bucket for the same reason [`BranchInfo`] does: it is not versioned,
/// not superseded, and not something a modeller edits. Putting ownership on a
/// `BoundedContext` instead would make re-owning a namespace a model edit that
/// lands in the changeset log and the git mirror, and would leave a namespace
/// unownable until someone wrote its first strategic entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceRecord {
    /// Immutable for the life of the namespace. Entity keys embed this, so
    /// changing it would be the big-bang re-key the registry exists to avoid.
    pub id: NamespaceId,
    /// Human label, unique within [`NamespaceRecord::parent`] only.
    pub name: NamespaceName,
    /// Ownership hierarchy position. The one mutable field.
    pub parent: OwnerId,
    /// RFC3339 timestamp.
    pub created_at: String,
    /// Authenticated principal that claimed it, or `migration` for rows the
    /// registry backfill created.
    pub created_by: String,
    /// Whether the row outlives the write that created it.
    pub tenure: NamespaceTenure,
}

/// How long a registry row lives.
///
/// A namespace enters the registry the first time somebody writes into it,
/// and that write is either on baseline or on a branch. The two cannot leave
/// the same trace. Baseline work is public the moment it lands, so the row
/// naming its owner is public too and outlives every branch. Branch work is
/// invisible outside its branch and disappears when the branch is deleted, so
/// a row created only to hold it has to disappear with it. Without the
/// distinction, starting a branch and abandoning it reserves a namespace name
/// globally and forever, which is precisely what a branch is not allowed to
/// do.
///
/// The row does exist while the branch is alive, and it has to. Every read is
/// filtered through the registry, so a namespace with no row is one its own
/// author cannot see: withholding the row until merge would make branch work
/// invisible to the person doing it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum NamespaceTenure {
    /// Outlives every branch. Explicitly registered, claimed by a baseline
    /// write, or backfilled by the registry migration.
    #[default]
    Permanent,
    /// Held open by one branch. Deleting that branch releases the row;
    /// merging it makes the row permanent.
    Provisional { branch: String },
}

impl NamespaceTenure {
    /// The tenure a write carrying `branch` creates. `None` is baseline.
    #[must_use]
    pub fn for_branch(branch: Option<&str>) -> Self {
        match branch {
            Some(name) => Self::Provisional {
                branch: name.to_owned(),
            },
            None => Self::Permanent,
        }
    }

    /// The branch holding this row open, or `None` when it stands on its own.
    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        match self {
            Self::Permanent => None,
            Self::Provisional { branch } => Some(branch.as_str()),
        }
    }
}

/// Outcome of a registration attempt.
///
/// Registration is idempotent, so "already yours" is a success, not an error.
/// A client that retries after a timeout must not be told the name is taken
/// when it is the one holding it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceClaim {
    pub record: NamespaceRecord,
    /// False when the row already existed under this same parent.
    pub created: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum ChangeKind {
    Put,
    Delete,
}

#[derive(Debug, Clone)]
pub struct ChangeRecord {
    pub seq: u64,
    pub kind: ChangeKind,
    pub entity_ref: EntityRef,
    pub at: String,
    /// Changeset this change belongs to, or empty when the write was not
    /// attributed. See [`ChangesetRef`].
    pub changeset_id: String,
    /// Authenticated principal that produced the change, or empty when the
    /// write was not attributed.
    pub author: String,
}

/// Server-minted identity of the changeset a write belongs to, threaded
/// into the store so the change events the write produces can be joined
/// back to the durable [`ChangesetRecord`].
///
/// Borrowed and `Copy`: one of these is minted per mutation RPC and passed
/// to every store call that RPC makes.
#[derive(Debug, Clone, Copy)]
pub struct ChangesetRef<'a> {
    /// UUIDv7, so lexicographic order is chronological order.
    pub id: &'a str,
    /// Authenticated principal name. Never caller-supplied.
    pub author: &'a str,
    /// Name of the RPC producing the write, e.g. `PutEntity`. Denormalised
    /// onto each revision row so an entity's history renders without a
    /// join back to the changeset log.
    pub rpc: &'a str,
    /// The `operation_id` the mutation carried, if any. Denormalised onto
    /// the durable changeset so `GetChangeset` can show which idempotency
    /// claim produced it; the claim record itself lives in the
    /// `trogon-atlas-operations` bucket, keyed separately.
    pub operation_id: Option<&'a str>,
}

/// Scope and attribution for one mutating call.
///
/// This replaces the bare `branch: Option<&str>` that mutating methods used
/// to take. It is still the ONE code path for baseline and branch writes;
/// there are no parallel attributed/unattributed methods. It just carries
/// the changeset the write belongs to alongside the branch scope.
///
/// The default is baseline scope with no attribution, which is what direct
/// store use outside a mutation RPC (fixtures, tools, tests) wants.
#[derive(Debug, Clone, Copy, Default)]
pub struct WriteContext<'a> {
    branch: Option<&'a str>,
    changeset: Option<ChangesetRef<'a>>,
    operation_key: Option<&'a str>,
}

impl<'a> WriteContext<'a> {
    /// Baseline scope, unattributed.
    #[must_use]
    pub fn baseline() -> Self {
        Self::default()
    }

    /// Branch scope, unattributed.
    #[must_use]
    pub fn branch(name: &'a str) -> Self {
        Self {
            branch: Some(name),
            changeset: None,
            operation_key: None,
        }
    }

    /// Baseline when `branch` is `None`, branch scope otherwise: the direct
    /// translation of a request's `x-trogon-atlas-branch` header.
    #[must_use]
    pub fn scoped(branch: Option<&'a str>) -> Self {
        Self {
            branch,
            changeset: None,
            operation_key: None,
        }
    }

    /// Attach the changeset this write belongs to.
    #[must_use]
    pub fn attributed(mut self, changeset: ChangesetRef<'a>) -> Self {
        self.changeset = Some(changeset);
        self
    }

    /// Attach the operation-receipt key this write claims, so a journaled
    /// batch can record it and a later recovery pass can settle it. See
    /// `trogon_atlas_store::operations`.
    #[must_use]
    pub fn with_operation_key(mut self, key: &'a str) -> Self {
        self.operation_key = Some(key);
        self
    }

    #[must_use]
    pub fn branch_name(self) -> Option<&'a str> {
        self.branch
    }

    #[must_use]
    pub fn changeset(self) -> Option<ChangesetRef<'a>> {
        self.changeset
    }

    #[must_use]
    pub fn operation_key(self) -> Option<&'a str> {
        self.operation_key
    }
}

/// One entity a changeset actually changed. A no-op write contributes
/// nothing.
#[derive(Debug, Clone)]
pub struct ChangesetOp {
    pub kind: ChangeKind,
    pub entity_ref: EntityRef,
}

/// Durable record of one mutation RPC: what landed together, who did it,
/// when.
///
/// Changesets are deliberately NOT stored on the change stream. That stream
/// is a transport: a publish that exhausts its retries is dropped
/// ([`StoreError::ChangeEventLost`]) and retention deletes old records.
/// Anything durable built on it would inherit both properties. Changesets
/// live in their own bucket with no retention instead.
#[derive(Debug, Clone)]
pub struct ChangesetRecord {
    /// UUIDv7, so lexicographic order is chronological order.
    pub id: String,
    pub author: String,
    /// RFC3339 timestamp.
    pub at: String,
    /// Server-generated summary of the mutation.
    pub message: String,
    /// Name of the RPC that produced the changeset, e.g. `PutEntity`.
    pub rpc: String,
    /// Branch the mutation applied to. `None` = baseline.
    pub branch: Option<String>,
    pub ops: Vec<ChangesetOp>,
    /// The `operation_id` the mutation carried, if any. See
    /// `ChangesetRef::operation_id`.
    pub operation_id: Option<String>,
}

/// One entity as it stood on either side of one changeset's write.
///
/// A changeset says what landed together; a revision says what one entity
/// looked like before and after. Together they answer `git log <path>` and
/// they are what makes revert a *derivation* rather than a guess.
///
/// Both images are stored rather than only the pre-image. Chaining
/// `after(n) == before(n+1)` would be one row cheaper and would be wrong the
/// moment an unattributed write slips between two attributed ones, which is
/// a supported path (`import`, direct `Store` use). A self-describing row
/// cannot drift.
#[derive(Debug, Clone)]
pub struct EntityRevisionRecord {
    /// Changeset this revision belongs to. UUIDv7.
    pub changeset_id: String,
    pub author: String,
    /// RFC3339 timestamp.
    pub at: String,
    /// Name of the RPC that produced the write.
    pub rpc: String,
    /// Branch the write applied to. `None` = baseline.
    pub branch: Option<String>,
    pub entity_ref: EntityRef,
    pub kind: ChangeKind,
    /// The entity immediately before the write. `None` when it created the key.
    pub before: Option<Entity>,
    /// The entity immediately after. `None` when the write deleted the key.
    pub after: Option<Entity>,
}

/// Paging request for [`Store::list_entity_revisions`].
#[derive(Debug, Clone, Copy, Default)]
pub struct RevisionPage<'a> {
    /// Exclusive cursor: return only revisions whose changeset id orders
    /// strictly before this one. `None` starts from the newest.
    pub before: Option<&'a str>,
    /// Maximum records to return. Implementations clamp this.
    pub limit: usize,
    /// Whose history to read. `None` = baseline. A branch write never
    /// touches baseline, so the two logs are separate by construction.
    pub branch: Option<&'a str>,
}

/// Which changesets a [`Store::list_changesets`] page covers.
///
/// A bare `Option<&str>` could say "this branch" or "everything" but never
/// "baseline only", which is exactly the question branch ancestry asks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChangesetScope<'a> {
    /// Every changeset, baseline and branch alike.
    #[default]
    All,
    /// Only changesets that landed on baseline.
    Baseline,
    /// Only changesets that applied to this branch.
    Branch(&'a str),
}

impl ChangesetScope<'_> {
    /// True when `branch` (the changeset's own scope, `None` = baseline) is
    /// inside this scope.
    #[must_use]
    pub fn admits(&self, branch: Option<&str>) -> bool {
        match self {
            Self::All => true,
            Self::Baseline => branch.is_none(),
            Self::Branch(name) => branch == Some(*name),
        }
    }
}

/// Paging request for [`Store::list_changesets`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ChangesetPage<'a> {
    /// Exclusive cursor: return only changesets ordered strictly before
    /// this id. `None` starts from the newest.
    pub before: Option<&'a str>,
    /// Maximum records to return. Implementations clamp this.
    pub limit: usize,
    /// Which changesets the page covers.
    pub scope: ChangesetScope<'a>,
}

/// Backend-agnostic persistence interface. The bundled implementation is
/// `NatsStore` (`JetStream` KV). Third-party backends implement this trait
/// directly; see the crate README for the contract.
///
/// # Concurrency contract
///
/// All mutating methods are safe to call concurrently from multiple tasks or
/// threads.
///
/// ## Write semantics
///
/// - `create`: inserts exactly once. Returns `AlreadyExists` if a row with the
///   same (kind, namespace, slug, version) already exists. Never overwrites.
/// - `put`: unconditional overwrite. The `system` stamp on the stored entity is
///   derived from the entity that was actually present before the write (or
///   `None` for first write), not from any value supplied by the caller.
/// - `update`: compare-and-swap on etag. Returns `NotFound` if no row exists,
///   or `EtagMismatch { expected, found }` if the current etag differs. The
///   `system` stamp is derived from the entity being replaced.
/// - `delete`: removes the row. `expected_etag = None` is an unconditional
///   delete; a `Some(etag)` fails with `EtagMismatch` if the stored etag
///   differs. Returns `NotFound` if no row exists.
///
/// ## Error mapping
///
/// Implementations must map errors consistently: missing rows are always
/// `NotFound`, etag conflicts are always `EtagMismatch`, and every other
/// backend failure is `Backend`. `InvalidArgument` is used for caller-supplied
/// values that cannot be represented (e.g. `since` overflow in
/// `read_changes`). `Unavailable` signals transient connectivity failures.
///
/// ## Branch overlays (Phase 1: Isolation)
///
/// Every read method below takes `branch: Option<&str>`; every mutating
/// method takes a [`WriteContext`], which carries the same branch scope plus
/// the changeset the write belongs to. This is the ONE code path for both
/// baseline and branch-scoped access; there are no parallel
/// `branch_get`/`branch_put`/etc. methods. `branch = None` (equivalently
/// `WriteContext::baseline()`) means baseline, exactly as before this
/// parameter existed.
///
/// When `branch = Some(name)`, the store overlays copy-on-write deltas for
/// that branch on top of the baseline data:
/// - `get`/`batch_get`/`list`: a delta for a key wins over baseline. A
///   tombstone delta hides the key (`NotFound` from `get`, omitted from
///   `list`). A live delta's entity is returned with its etag set to the
///   branch KV row's revision (not the baseline etag).
/// - `create`/`put`/update`/`delete`: validated and stamped against the
///   *merged view* (the branch's delta if one exists for the key, else
///   baseline) rather than baseline alone. On first touch of a key on a
///   branch, the baseline entity and its etag (if any) are captured
///   alongside the new delta so the branch remembers what it forked from.
/// - Change events (`record_change`) are never emitted for branch writes;
///   branch writes are invisible to the change feed entirely. This is
///   enforced by implementations internally in their write paths, not by
///   this trait's change-feed methods (`record_change`, `read_changes`,
///   `current_change_seq`, `subscribe`, `prune_changes`,
///   `change_stream_stats` remain branch-agnostic; they only ever see
///   baseline history).
///
/// See `create_branch`/`list_branches`/`delete_branch` for branch lifecycle
/// management, and `docs/explanation/branching.md` for the full design.
#[async_trait]
pub trait Store: Send + Sync {
    // ---- Single-writer fencing --------------------------------------------
    //
    // Only one process may mutate at a time; see `docs/explanation/single-writer.md`.
    // Backends without a writer lease accept the default, which reports
    // every process as the sole writer -- the behavior every caller saw
    // before this fencing existed.

    /// Whether this process currently holds the writer lease (or has no
    /// lease to hold, for backends that do not support fencing). Every
    /// mutating method must refuse with `StoreError::NotWriter` when this
    /// is false.
    fn is_writer(&self) -> bool {
        true
    }

    /// This process's writer role, lease epoch, and the last known lease
    /// holder, for `GetServerInfo`. Backends without a writer lease report
    /// `WriterRole::Writer` at `Epoch::NONE` with no named holder.
    fn writer_status(&self) -> WriterStatus {
        WriterStatus {
            role: WriterRole::Writer,
            epoch: Epoch::NONE,
            lease_holder: String::new(),
        }
    }

    async fn get(
        &self,
        kind: EntityKind,
        id: &Id,
        branch: Option<&str>,
    ) -> StoreResult<StoredEntity>;

    async fn batch_get(
        &self,
        keys: &[(EntityKind, Id)],
        branch: Option<&str>,
    ) -> StoreResult<Vec<Option<StoredEntity>>>;

    /// Insert iff no entity at this key in the merged view. Fails with
    /// `AlreadyExists` otherwise.
    async fn create(
        &self,
        kind: EntityKind,
        entity: &Entity,
        ctx: WriteContext<'_>,
    ) -> StoreResult<Written>;

    /// Replace. The system stamp is derived from the entity actually
    /// replaced in the merged view (or `None` for a first write), not from
    /// caller input.
    ///
    /// Idempotency is a store property: when the merged-view entity is
    /// semantically identical to the proposed one
    /// (`trogon_atlas_core::semantically_equal`), the write is skipped:
    /// no new revision, no change event, no delta recorded on a branch;
    /// and the result has `wrote == false` with the existing etag.
    /// `force = true` overwrites regardless (deliberate re-stamp /
    /// byte-level repair).
    async fn put(
        &self,
        kind: EntityKind,
        entity: &Entity,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> StoreResult<Written>;

    /// Replace iff the merged view's current etag matches. Fails with
    /// `EtagMismatch` otherwise. Same no-op semantics as `put` when the
    /// content is semantically identical (the etag must still match).
    async fn update(
        &self,
        kind: EntityKind,
        entity: &Entity,
        expected_etag: &str,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> StoreResult<Written>;

    async fn delete(
        &self,
        kind: EntityKind,
        id: &Id,
        expected_etag: Option<&str>,
        ctx: WriteContext<'_>,
    ) -> StoreResult<()>;

    /// All entities matching the filter in the merged view, sorted by
    /// (kind, namespace, slug, version asc).
    ///
    /// `limit` caps the number of results returned. When `None` the store
    /// applies an internal hard cap (`10_000`) to prevent unbounded scans.
    /// Callers should always pass the bound they actually need.
    ///
    /// # Memory
    ///
    /// `NatsStore` streams the full KV key set before applying filters. In the
    /// worst case (no filter, large store) it materialises up to `10_000` keys
    /// into a `Vec<String>` before fetching entries. Callers should pass
    /// a small explicit `limit` to keep peak memory predictable.
    async fn list(
        &self,
        filter: ListFilter<'_>,
        limit: Option<usize>,
        branch: Option<&str>,
    ) -> StoreResult<Vec<StoredEntity>>;

    /// Append a change record. Returns the assigned sequence number.
    ///
    /// `changeset` attributes the event to the mutation that produced it;
    /// pass `None` for an unattributed write.
    ///
    /// # Error semantics
    ///
    /// Returns `Err(StoreError::InvalidArgument)` when `entity_ref.id` is
    /// absent; this is always a programming error.
    ///
    /// When the corresponding KV write has already committed but publish
    /// retries are exhausted, returns `Err(StoreError::ChangeEventLost)`.
    /// Internal callers in `NatsStore` catch this variant specifically,
    /// increment the `change_events_lost` counter, log at error level, and
    /// complete the surrounding mutation successfully so the caller's write
    /// is not rolled back. A non-zero `change_events_lost` counter means the
    /// change feed has gaps that must be reconciled via `read_changes`.
    async fn record_change(
        &self,
        kind: ChangeKind,
        entity_ref: &EntityRef,
        changeset: Option<ChangesetRef<'_>>,
    ) -> StoreResult<u64>;

    /// Fetch change records with `seq > since` up to `limit`.
    async fn read_changes(&self, since: u64, limit: usize) -> StoreResult<Vec<ChangeRecord>>;

    /// Current change-log head sequence.
    async fn current_change_seq(&self) -> StoreResult<u64>;

    /// Apply a batch of mutations atomically. On the first failed op,
    /// no writes persist and the error includes the zero-based op index.
    /// On success, returns one outcome per op in input order.
    ///
    /// # Atomicity contract
    ///
    /// `NatsStore`: recoverable rather than transactional, because NATS KV
    /// has no multi-key transactions. A baseline batch journals the prior
    /// image of every key it touches before its first write. On first
    /// failure the store rolls back every key written so far. If any
    /// rollback step fails, the error returned is
    /// `StoreError::BatchFailed { index, source: Box::new(StoreError::PartialApply { .. }) }`
    /// naming the journal entry, and [`Self::recover_batches`] finishes the
    /// rollback. A fully successful rollback returns
    /// `StoreError::BatchFailed { index, source }` where `source` is the
    /// original op error.
    ///
    /// # Change-feed and history gaps
    ///
    /// After all KV writes succeed the batch is committed and the result is
    /// `Ok`. Change events and revision rows are written next; any that
    /// fail, and a changeset the caller never appends, are written by
    /// [`Self::recover_batches`] rolling the batch forward.
    ///
    /// When `branch` is set, every op is validated and applied against the
    /// merged view and no change events are published for any op in the
    /// batch (see the trait-level "Branch overlays" section).
    async fn batch_apply(
        &self,
        ops: &[MutationOp],
        ctx: WriteContext<'_>,
    ) -> StoreResult<Vec<MutationOutcome>>;

    /// Subscribe to live change records. Each `record_change` call emits
    /// once on every active receiver. Slow consumers may observe lagged
    /// errors from `broadcast::Receiver::recv`; they should reconcile by
    /// re-reading from `read_changes` using the last seen seq.
    ///
    /// # Precondition: subscribe first, then drain historical records
    ///
    /// The `NatsStore` change tail starts from `DeliverPolicy::New` on the
    /// first connection. Callers must `subscribe()` **before** paging through
    /// `read_changes(last_seen_seq, ...)` so records published during the
    /// backlog drain are buffered on the receiver and can be de-duplicated by
    /// sequence against the backlog. Draining first and then subscribing
    /// races a startup gap.
    fn subscribe(&self) -> broadcast::Receiver<ChangeRecord>;

    /// Delete change records older than `before_seq`.
    ///
    /// `NatsStore` `JetStream` retention policies handle pruning server-side;
    /// this method is a no-op that returns `Ok(())`. Third-party backends
    /// should issue an equivalent sweep against their change log store.
    async fn prune_changes(&self, before_seq: u64) -> StoreResult<()>;

    /// Backend-specific stats about the change stream, used to drive
    /// retention-pressure metrics. Returns `Ok(None)` for backends where
    /// the question is meaningless. `NatsStore` reports the `JetStream`
    /// stream's message count, byte count, and the timestamp of the oldest
    /// message so an SRE can detect imminent retention pressure before it
    /// bites.
    async fn change_stream_stats(&self) -> StoreResult<Option<ChangeStreamStats>> {
        Ok(None)
    }

    /// Find baseline batches that stopped before finishing and, in
    /// [`RecoveryMode::Repair`], drive each to a converged state: fully
    /// applied with its revisions, notifications, and changeset, or fully
    /// absent. Backends whose batches are transactional have nothing to
    /// recover and keep the default.
    ///
    /// [`RecoveryMode::Repair`]: crate::recovery::RecoveryMode::Repair
    async fn recover_batches(
        &self,
        policy: crate::recovery::RecoveryPolicy,
    ) -> StoreResult<crate::recovery::BatchRecoveryReport> {
        let _ = policy;
        Ok(crate::recovery::BatchRecoveryReport::default())
    }

    /// Create a new branch. Fails with `AlreadyExists` if a branch with
    /// this name is already registered. `base_change_token` is captured
    /// from `current_change_seq` at creation time, and `fork_changeset_id`
    /// from the newest baseline changeset.
    async fn create_branch(&self, name: &str, doc: &str) -> StoreResult<BranchInfo>;

    /// Move a branch's fork point forward to `fork_changeset_id`, leaving
    /// every other field of its metadata row alone. `NotFound` when no
    /// branch with this name is registered.
    ///
    /// Callers advance the pointer only once the branch is genuinely caught
    /// up with baseline; see `UpdateBranch`.
    async fn set_branch_fork_point(&self, name: &str, fork_changeset_id: &str) -> StoreResult<()>;

    /// List every registered branch, in implementation-defined order.
    async fn list_branches(&self) -> StoreResult<Vec<BranchInfo>>;

    /// Delete a branch: removes every delta row plus its metadata row.
    /// Returns `NotFound` if no branch with this name is registered.
    /// Entities previously visible only through this branch's deltas
    /// become unreachable (or fall through to baseline, if baseline has
    /// the key) once this returns.
    async fn delete_branch(&self, name: &str) -> StoreResult<()>;

    /// Enumerate every raw delta row for `branch` (Phase 2: Review and
    /// merge), in implementation-defined order. Unlike `list`/`get` with
    /// `branch = Some(name)`, this returns the delta's full `base`/
    /// `base_etag`/`ours`/`tombstone` payload rather than the merged view,
    /// so callers can classify each entry against current baseline.
    /// Returns an empty vec (not `NotFound`) for a branch with zero deltas.
    async fn list_branch_deltas(&self, branch: &str) -> StoreResult<Vec<BranchDeltaEntry>>;

    /// Land a set of branch deltas onto baseline verbatim: `BranchLandOp::Put`
    /// writes `ours` bytes as-is (no `system` re-stamp, no idempotency
    /// short-circuit -- identity is preserved exactly as authored on the
    /// branch); `BranchLandOp::Delete` removes the baseline row. Each op is
    /// guarded by the baseline etag captured at diff time: if baseline moved
    /// since then, the whole call fails without applying anything (the
    /// caller is expected to have already re-diffed under the mutation lock
    /// so this should not happen in practice).
    ///
    /// All-or-nothing like `batch_apply`: on the first failed op, a
    /// best-effort compensating rollback restores every key already
    /// written. Ordinary baseline change events are recorded for every
    /// applied op. On success, the corresponding delta rows for the landed
    /// keys are purged from the branch (the caller purges/keeps the branch
    /// itself via `delete_branch` or by leaving it registered).
    ///
    /// `ctx` supplies the changeset the landed writes belong to. Its branch
    /// scope is ignored: the ops land on baseline by definition, and the
    /// branch they came from is the `branch` argument.
    async fn land_branch_merge(
        &self,
        branch: &str,
        ops: &[BranchLandOp],
        ctx: WriteContext<'_>,
    ) -> StoreResult<Vec<MutationOutcome>>;

    /// Rebase every non-conflicting delta row on `branch` forward: replace
    /// `base`/`base_etag` with the entity/etag given for that key (the
    /// current baseline value), or drop the delta row entirely when
    /// `new_base` is `None` (CONVERGED: the branch's `ours` already matches
    /// the new baseline, so the delta collapses to nothing). Used by
    /// `UpdateBranch` for incremental reconciliation; conflicting entries
    /// are left untouched by the caller (they are never passed here).
    async fn rebase_branch_deltas(
        &self,
        branch: &str,
        rebases: &[(EntityKind, Id, Option<(Entity, String)>)],
    ) -> StoreResult<u32>;

    /// Resolve a single conflicting entry on `branch`:
    /// - `take_theirs = true`: overwrite the delta with the current
    ///   baseline value (or remove the delta entirely if baseline no
    ///   longer has the key), clearing the conflict by making `ours`
    ///   converge with baseline.
    /// - `take_theirs = false` (keep ours): rebase `base`/`base_etag`
    ///   forward to the current baseline, keeping the branch's `ours`
    ///   unchanged.
    ///
    /// Returns `NotFound` if the branch has no delta for this key.
    async fn resolve_branch_entry(
        &self,
        branch: &str,
        kind: EntityKind,
        id: &Id,
        take_theirs: bool,
    ) -> StoreResult<()>;

    // ---- Durable changeset log -------------------------------------------
    //
    // Separate from the change feed on purpose. `record_change` publishes to
    // a stream that drops events when a publish fails and deletes them when
    // retention bites; these three methods read and write a durable,
    // unexpiring log. Learn that something changed from the feed; read what
    // changed from here.

    /// Durably record a changeset. Idempotent on `record.id`: appending the
    /// same id twice leaves one row, so a retried mutation cannot duplicate
    /// history.
    ///
    /// Callers append AFTER the entity writes commit, with the ops that
    /// actually landed. A crash between the two loses attribution for that
    /// one mutation, never the writes themselves.
    async fn append_changeset(&self, record: &ChangesetRecord) -> StoreResult<()>;

    /// Fetch one changeset. `NotFound` when no changeset carries this id.
    async fn get_changeset(&self, id: &str) -> StoreResult<ChangesetRecord>;

    /// Page over changesets, newest first.
    ///
    /// # Cost
    ///
    /// Backends without ordered range scans (`NatsStore` over `JetStream` KV
    /// is one) enumerate the changeset key set on every call. Memory stays
    /// bounded by `page.limit`; time is linear in total changeset count. A
    /// deployment whose history outgrows that should project changesets into
    /// a read-optimised store rather than paging this method.
    async fn list_changesets(&self, page: ChangesetPage<'_>) -> StoreResult<Vec<ChangesetRecord>>;

    /// Page over one entity's revisions, newest first.
    ///
    /// Revisions are appended by the store itself on every attributed write,
    /// so there is no `append` counterpart on this trait: a backend that
    /// records changesets records revisions with them or neither.
    ///
    /// # Cost
    ///
    /// The same caveat [`Self::list_changesets`] carries, and worse: a
    /// backend without ordered range scans enumerates the whole revision
    /// keyspace to answer for one entity. Memory stays bounded by
    /// `page.limit`; time is linear in the store's total recorded history.
    async fn list_entity_revisions(
        &self,
        kind: EntityKind,
        id: &Id,
        page: RevisionPage<'_>,
    ) -> StoreResult<Vec<EntityRevisionRecord>>;

    /// Fetch the revision one changeset made to one entity. `NotFound` when
    /// that changeset never touched this key, or when the write predates
    /// revision recording.
    async fn get_entity_revision(
        &self,
        kind: EntityKind,
        id: &Id,
        changeset_id: &str,
        branch: Option<&str>,
    ) -> StoreResult<EntityRevisionRecord>;

    /// Id of the newest changeset that landed on baseline, or `None` when
    /// none has been recorded yet.
    ///
    /// This is the value a branch's fork point is captured from. Writes that
    /// mint no changeset (`trogon-atlas-server import-git`, direct `Store` use)
    /// do not move it, which is the same unattributed-write caveat the
    /// changeset log carries everywhere else.
    async fn newest_baseline_changeset(&self) -> StoreResult<Option<String>> {
        let newest = self
            .list_changesets(ChangesetPage {
                before: None,
                limit: 1,
                scope: ChangesetScope::Baseline,
            })
            .await?;
        Ok(newest.into_iter().next().map(|record| record.id))
    }

    // -- Namespace registry -------------------------------------------------
    //
    // The registry is what makes namespace names collision-free between
    // owners. Everything below the registry sees only `NamespaceId`, which is
    // globally unique; the human name is resolved to one at the request
    // boundary, within the caller's own ownership node.

    /// Claim `name` under `parent`, minting a fresh [`NamespaceId`].
    ///
    /// Idempotent: re-registering a name already held by this same `parent`
    /// returns the existing row with `created: false`. Claiming a name held by
    /// a *different* parent is impossible by construction, because the
    /// uniqueness constraint is keyed by `(parent, name)`.
    ///
    /// Implementations must make the claim atomic against concurrent callers.
    /// A read-then-write would let two racing registrations both believe they
    /// won, which is the `BC_PROJECT_COLLISION` failure mode in a new place.
    async fn register_namespace(
        &self,
        name: &NamespaceName,
        parent: &OwnerId,
        created_by: &str,
    ) -> StoreResult<NamespaceClaim>;

    /// Register a namespace under an id equal to its name.
    ///
    /// Two callers need this, and both need it for the same reason: the
    /// entity keys already exist (or are about to) under the bare name, so
    /// the registry has to describe reality rather than mint a new id and
    /// orphan them. The backfill adopts every namespace that predates the
    /// registry; the server's claim-on-first-write adopts a namespace the
    /// moment a caller writes into one nobody has registered.
    ///
    /// Separate from [`Store::register_namespace`] because it is the one path
    /// that does not mint, and unminted ids are the only ones that can
    /// collide. Adoption therefore fails when the id is already taken by
    /// another owner, which is what pushes a second claimant to
    /// [`Store::register_namespace`] and a minted `ns_…` id.
    ///
    /// `tenure` records whether the write doing the adopting was on baseline
    /// or on a branch; see [`NamespaceTenure`]. An adoption that finds the row
    /// already there returns it unchanged, so a branch write never demotes a
    /// permanent row.
    async fn adopt_namespace(
        &self,
        name: &NamespaceName,
        parent: &OwnerId,
        tenure: &NamespaceTenure,
    ) -> StoreResult<NamespaceClaim>;

    /// Put a registry row back exactly as it was recorded.
    ///
    /// The restore path, and the only one that writes an id it did not mint
    /// or derive from a name. A mirror carries entities keyed by
    /// [`NamespaceRecord::id`], so bringing a store back means reproducing
    /// those ids verbatim: [`Store::register_namespace`] would mint fresh ones
    /// and orphan every entity, and [`Store::adopt_namespace`] can only
    /// reproduce the ids that happen to equal their name.
    ///
    /// `created_at` and `created_by` are taken from the record rather than
    /// stamped, so a restore does not rewrite history into "claimed by the
    /// restore, at the moment of the restore".
    ///
    /// Idempotent, and safe against a partially-restored registry: a row this
    /// owner already holds under this name is returned with `created: false`.
    /// Fails with `AlreadyExists` when the id is held by a different owner or
    /// under a different name, because landing it would silently re-own
    /// somebody else's namespace.
    async fn restore_namespace(&self, record: &NamespaceRecord) -> StoreResult<NamespaceClaim>;

    /// Drop a row and its name index outright.
    ///
    /// Only ever called for a provisional row whose branch has just been
    /// deleted and whose namespace holds nothing on baseline. Idempotent:
    /// releasing a row that is already gone succeeds.
    async fn release_namespace(&self, id: &NamespaceId) -> StoreResult<()>;

    /// Make a provisional row permanent, because its branch merged.
    ///
    /// `None` when no such row exists. A row that is already permanent is
    /// returned unchanged.
    async fn promote_namespace(&self, id: &NamespaceId) -> StoreResult<Option<NamespaceRecord>>;

    /// Look up one row by id. `None` when no such namespace is registered.
    async fn get_namespace(&self, id: &NamespaceId) -> StoreResult<Option<NamespaceRecord>>;

    /// Resolve a human name to its id *within one owner*. `None` when that
    /// owner has no namespace by that name, even if another owner does.
    ///
    /// This is the function that makes two tenants able to both say `orders`.
    async fn resolve_namespace(
        &self,
        parent: &OwnerId,
        name: &NamespaceName,
    ) -> StoreResult<Option<NamespaceId>>;

    /// Every registered namespace, in implementation-defined order.
    async fn list_namespaces(&self) -> StoreResult<Vec<NamespaceRecord>>;

    /// Re-parent a namespace.
    ///
    /// Writes the registry row and the name index, and nothing else. No entity
    /// is read, rewritten, or re-keyed: that is the entire point of holding the
    /// id in the key and the owner in the registry.
    ///
    /// Fails with `AlreadyExists` when the destination owner already has a
    /// namespace by this name, because landing it there would break the
    /// per-owner uniqueness the resolver depends on. `NotFound` when `id` is
    /// not registered.
    async fn move_namespace(
        &self,
        id: &NamespaceId,
        new_parent: &OwnerId,
    ) -> StoreResult<NamespaceRecord>;

    // ---- Operation receipts -----------------------------------------------
    //
    // Idempotency for mutations that carry a client-supplied `operation_id`.
    // See `OperationReceipt` / `GetOperation` in `service.proto`.

    /// Claim `key` (derived from the caller principal and the mutation's
    /// `operation_id`) for a request whose canonical digest is `digest`.
    /// Must be called under the mutation lock, before any write the
    /// mutation would otherwise perform: claiming is what makes two
    /// concurrent retries of the same `operation_id` agree on exactly one
    /// winner.
    ///
    /// Backends without a durable receipt store accept the default, which
    /// fails closed: callers must treat `Err` as "operation receipts are
    /// not available here," never as a successful claim.
    async fn claim_operation(
        &self,
        key: &str,
        digest: &str,
        rpc: &str,
        branch: Option<&str>,
    ) -> StoreResult<ClaimOutcome> {
        let _ = (key, digest, rpc, branch);
        Err(StoreError::Unavailable(
            "operation receipts are not supported by this backend".into(),
        ))
    }

    /// Settle a claim as applied, recording the changeset it produced.
    /// Settling is best-effort bookkeeping, never the source of truth for
    /// whether the mutation happened, so a backend that cannot settle still
    /// returns `Ok`.
    async fn settle_operation_applied(&self, key: &str, changeset_id: &str) -> StoreResult<()> {
        let _ = (key, changeset_id);
        Ok(())
    }

    /// Settle a claim as rejected, recording why so a replay can return the
    /// same failure instead of a generic one.
    async fn settle_operation_rejected(
        &self,
        key: &str,
        code: &str,
        message: &str,
    ) -> StoreResult<()> {
        let _ = (key, code, message);
        Ok(())
    }

    /// Settle a claim as not-applied: the mutation never happened (e.g. the
    /// batch it belonged to rolled back), so a retry with the same
    /// `operation_id` and digest is free to claim it again.
    async fn settle_operation_not_applied(&self, key: &str) -> StoreResult<()> {
        let _ = key;
        Ok(())
    }

    /// Look up a claim by key, for `GetOperation`. `None` means no record:
    /// never claimed, or past the backend's retention.
    async fn get_operation(&self, key: &str) -> StoreResult<Option<OperationRecord>> {
        let _ = key;
        Ok(None)
    }
}

/// This process's writer role, lease epoch, and the last known lease
/// holder. See [`Store::writer_status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterStatus {
    pub role: WriterRole,
    pub epoch: Epoch,
    /// Opaque id of the process that last held (or currently holds) the
    /// lease. Empty when no backend-tracked holder exists.
    pub lease_holder: String,
}

/// Status of one claim on an `operation_id`. Mirrors
/// `trogonatlas.api.eventmodel.v1alpha1.OperationStatus` minus `UNSPECIFIED`/`UNKNOWN`, which are
/// server-level concepts (no record found at all) rather than states a
/// claim is ever stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationStatus {
    /// Claimed, mutation not yet settled.
    Pending,
    /// The mutation landed; `OperationRecord::changeset_id` names it.
    Applied,
    /// The mutation was rejected; `OperationRecord::rejection_code` /
    /// `rejection_message` say why.
    Rejected,
    /// The mutation never landed (e.g. its batch rolled back). A retry
    /// with the same `operation_id` and digest is free to claim again.
    NotApplied,
}

/// One claim on an `operation_id`: what request it was made for, and what
/// happened to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRecord {
    /// Canonical digest of the request this claim was made for. Compared
    /// against a new request's digest to tell a genuine retry from
    /// `operation_id` reuse.
    pub digest: String,
    /// Name of the RPC that claimed this key, e.g. `PutEntity`.
    pub rpc: String,
    /// Branch the mutation applied to. `None` = baseline.
    pub branch: Option<String>,
    pub status: OperationStatus,
    /// Set when `status == Applied`.
    pub changeset_id: String,
    /// Set when `status == Rejected`.
    pub rejection_code: String,
    /// Set when `status == Rejected`.
    pub rejection_message: String,
}

/// What claiming an `operation_id` found.
#[derive(Debug, Clone)]
pub enum ClaimOutcome {
    /// No prior claim existed (or the prior one settled `NotApplied`);
    /// this call now owns the operation and must settle it before
    /// returning.
    Claimed,
    /// Same digest, already settled: replay the stored outcome instead of
    /// repeating the mutation.
    Replay(OperationRecord),
    /// Same digest, still `Pending`: another call (or a crashed one not
    /// yet recovered) owns it.
    InProgress,
    /// A different digest is claiming the same key: the caller reused an
    /// `operation_id` for a different request.
    DigestMismatch,
}

/// Snapshot of the change stream's retention state.
#[derive(Debug, Clone)]
pub struct ChangeStreamStats {
    /// Total messages currently in the stream.
    pub messages: u64,
    /// Total bytes currently in the stream.
    pub bytes: u64,
    /// Unix epoch seconds of the oldest message still retained, or
    /// `None` if the stream is empty.
    pub oldest_message_unix_seconds: Option<i64>,
}

#[derive(Debug, Clone)]
pub enum MutationOp {
    /// Create with `AlreadyExists` failure on duplicate.
    Create { kind: EntityKind, entity: Entity },
    /// Replace, optionally guarded by an expected etag (None = unconditional put).
    Put {
        kind: EntityKind,
        entity: Entity,
        if_match: Option<String>,
        /// Overwrite even when the stored entity is semantically identical.
        force: bool,
    },
    /// Delete, optionally guarded by etag.
    Delete {
        kind: EntityKind,
        id: Id,
        if_match: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub enum MutationOutcome {
    Wrote {
        etag: String,
        /// The entity as stored (system stamp included): what mirrors,
        /// search indexes, and responses must see.
        entity: Entity,
    },
    /// Write skipped: the stored entity is semantically identical
    /// (content equality ignoring `system`). `etag` is the existing
    /// revision; no change event was recorded.
    Noop {
        etag: String,
        entity: Entity,
    },
    Deleted,
}

/// Result of a single-entity write (`create` / `put` / `update`).
#[derive(Debug, Clone)]
pub struct Written {
    /// Etag of the row after the call (existing revision when `!wrote`).
    pub etag: String,
    /// False when the write was skipped because the stored entity is
    /// semantically identical.
    pub wrote: bool,
    /// The entity as stored (system stamp included).
    pub entity: Entity,
}

#[derive(Debug, Clone, Default)]
pub struct ListFilter<'a> {
    pub kinds: &'a [EntityKind],
    pub namespaces: &'a [String],
    pub latest_versions_only: bool,
}

// Historical home for `stamp_system`. The policy lives in
// `trogon-atlas-core::system_meta::stamp_system` so third-party backends
// can call it without depending on this crate. Re-exported here so the
// existing `trogon_atlas_store::store::stamp_system` import path keeps
// working until call sites migrate.
pub use trogon_atlas_core::system_meta::stamp_system;
