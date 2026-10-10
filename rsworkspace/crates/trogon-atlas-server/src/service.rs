use std::{
    collections::{BTreeMap, HashMap},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use tokio_stream::{wrappers::ReceiverStream, Stream};
use tonic::{Request, Response, Status};
use trogon_atlas_core::{
    operation_id::{operation_claim_key, validate_operation_id},
    NamespaceId, NamespaceName, OwnerId, WriterRole,
};
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    store::{ChangeRecord, ListFilter, MutationOp, MutationOutcome, OperationStatus, WriterStatus},
    ChangeKind, ChangesetOp, ChangesetPage, ChangesetRecord, ChangesetRef, ChangesetScope,
    EntityRevisionRecord, NamespaceTenure, RevisionPage, Store, StoredEntity, WriteContext,
};

use crate::{
    analysis,
    conv::{
        entity_id, entity_kind, kind_from_i32, require_id, store_err, validate_branch_name,
        validate_entity_content,
    },
    diff,
    git_mirror::{self, GitMirror, MirrorAuthor, MirrorBatch, MirrorChange},
    graph,
    llm_analysis::LlmAnalyzer,
    operation_receipts,
    ownership::{
        Lens, NamespaceAuthorizer, NamespaceDirectory, RegistryAuthorizer, Visibility,
        WriteVerdict, DEFAULT_OWNER,
    },
    scope::{BaselineProtection, OutOfScope, WriteTargets},
    validation,
};

pub const MAX_PROJECTION_ENTITIES: usize = 5000;

/// Core of `get_latest_version`, parameterized on the entity cap so the
/// truncation boundary can be exercised with a far smaller entity count in
/// tests than production's `MAX_PROJECTION_ENTITIES`.
///
/// An empty `namespace` means every namespace the caller may see, not every
/// namespace that exists: the cap is applied by the store before the result
/// is sorted, so a bound caller's own visible namespaces must be the ones
/// handed to `store.list`, never `""` (every tenant's). Otherwise another
/// tenant's entities can exhaust the cap before the caller's own entity is
/// ever reached, and visibility would be consulted too late to matter.
pub async fn get_latest_version_with_cap(
    store: &Arc<dyn Store>,
    lens: &Lens,
    kind: pb::EntityKind,
    namespace: &str,
    slug: &str,
    cap: usize,
    branch: Option<&str>,
) -> Result<Option<pb::Entity>, Status> {
    if !namespace.is_empty() && !lens.admits(namespace) {
        return Ok(None);
    }
    let namespaces: Vec<String> = if namespace.is_empty() {
        match lens {
            Lens::Everything => vec![String::new()],
            Lens::Only(visible) => {
                if visible.is_empty() {
                    return Ok(None);
                }
                visible.iter().map(|id| id.as_str().to_owned()).collect()
            }
        }
    } else {
        vec![namespace.to_string()]
    };
    let kinds = [kind];
    let all = store
        .list(
            ListFilter {
                kinds: &kinds,
                namespaces: &namespaces,
                latest_versions_only: true,
            },
            Some(cap),
            branch,
        )
        .await
        .map_err(store_err)?;
    Ok(all
        .into_iter()
        .find(|s| {
            matches!(entity_id(&s.entity), Ok(id)
                if id.slug == slug && lens.admits(&id.namespace))
        })
        .map(|s| s.entity))
}

/// Hard ceiling on `batch_mutate.ops.len()`. A single batch is held in
/// memory and serialized through the global mutation lock, so an unbounded
/// request starves every other writer. 1024 is well above the largest seed
/// fixture (~150 ops) and small enough to bound worst-case latency.
pub const MAX_BATCH_OPS: usize = 1024;

/// Limits advertised in `GetServerInfo`. Defining them as consts ensures the
/// values in the response and the enforcement in each handler cannot drift.
pub const ADVERTISED_MAX_PAGE_SIZE: i32 = 500;
pub const ADVERTISED_DEFAULT_PAGE_SIZE: i32 = 100;
pub const ADVERTISED_MAX_IMPACT_DEPTH: u32 = 16;
pub const ADVERTISED_MAX_BATCH_GET_KEYS: usize = 500;
pub const ADVERTISED_MAX_CHANGE_FEED_PAGE: i32 = 500;
/// How long a settled operation receipt stays readable through
/// `GetOperation`. Must match `trogon_atlas_store::nats::DEFAULT_OPERATIONS_RETENTION`
/// (7 days): this is what the server advertises, that is what the backend
/// actually enforces, and the two must not drift.
pub const ADVERTISED_OPERATION_RETENTION_SECONDS: u32 = 604_800;
/// Maximum number of additional store pages fetched during a scoped
/// `ListChanges` call when all records in the initial page are filtered out.
/// Bounds the worst-case latency of a single RPC when the change feed contains
/// long runs of out-of-scope records.
pub const LIST_CHANGES_MAX_LOOKAHEAD: usize = 8;

/// Same bound as `LIST_CHANGES_MAX_LOOKAHEAD`, for a bound caller's scoped
/// `ListChangesets` page.
pub const LIST_CHANGESETS_MAX_LOOKAHEAD: usize = 8;

/// Default and maximum page sizes for `ListChangesets`.
const CHANGESETS_DEFAULT_PAGE_SIZE: i32 = 50;
const CHANGESETS_MAX_PAGE_SIZE: i32 = 500;

/// How many baseline changesets `DiffBranch` will enumerate behind a
/// branch's fork point before giving up and setting `behind_truncated`.
///
/// A branch that far behind has one useful answer ("very"), and the review
/// surface should not turn into an unbounded history dump.
const BRANCH_BEHIND_CAP: usize = 100;

/// Principal name used when a request carries no authenticated identity,
/// which happens only when the auth layer is switched off entirely. It is a
/// label for logs and for relationship subjects, never a grant: an
/// unauthenticated request is unbound, so it never reaches an authorizer that
/// would look this up.
const UNAUTHENTICATED_PRINCIPAL: &str = "unauthenticated";

/// The changeset one mutation RPC's writes belong to.
///
/// Minted once at the top of the handler, threaded into every store call the
/// handler makes (so each resulting change event carries the same
/// `changeset_id`), and appended to the durable changeset log once the writes
/// commit. One `PutEntity` is one changeset; so is one 40-op `BatchMutate`,
/// and so is a `MergeBranch` that lands twelve entities.
///
/// The author is the authenticated principal, taken from the same
/// already-principal-bound path the git mirror uses. It is never
/// caller-supplied.
struct PendingChangeset {
    /// UUIDv7: lexicographic order is chronological order, which is what
    /// makes paging the log by id work.
    id: String,
    author: String,
    /// Name of the RPC, for the durable record.
    rpc: &'static str,
    /// Branch the mutation applies to. `None` = baseline.
    branch: Option<String>,
    /// Entities the mutation actually changed. A no-op write adds nothing,
    /// so an empty `ops` means there is nothing to record.
    ops: Vec<ChangesetOp>,
    /// The `operation_id` this mutation carried, if any. Denormalised onto
    /// the durable changeset; see `ChangesetRef::operation_id`.
    operation_id: Option<String>,
    /// The operation receipt's claim key, if one was claimed for this
    /// mutation. Threaded onto `WriteContext::operation_key` so a crash
    /// that interrupts this write still leaves `nats_journal`'s recovery
    /// pass a way to find and settle the claim it left pending.
    operation_key: Option<String>,
}

impl PendingChangeset {
    fn mint(rpc: &'static str, author: &MirrorAuthor, branch: Option<&str>) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            author: author.name.clone(),
            rpc,
            branch: branch.map(str::to_owned),
            ops: Vec::new(),
            operation_id: None,
            operation_key: None,
        }
    }

    /// Attach the `operation_id` this mutation carried, so it ends up on
    /// the durable changeset.
    #[must_use]
    fn with_operation_id(mut self, operation_id: Option<&str>) -> Self {
        self.operation_id = operation_id.map(str::to_owned);
        self
    }

    /// Attach the operation receipt's claim key, so crash recovery can find
    /// and settle it. Only meaningful once a claim has actually been made;
    /// see `operation_receipts::claim`.
    #[must_use]
    fn with_operation_key(mut self, operation_key: Option<&str>) -> Self {
        self.operation_key = operation_key.map(str::to_owned);
        self
    }

    /// Write context for every store call this RPC makes: the branch scope
    /// plus the attribution.
    fn write_ctx(&self) -> WriteContext<'_> {
        let ctx = WriteContext::scoped(self.branch.as_deref()).attributed(ChangesetRef {
            id: &self.id,
            author: &self.author,
            rpc: self.rpc,
            operation_id: self.operation_id.as_deref(),
        });
        match self.operation_key.as_deref() {
            Some(key) => ctx.with_operation_key(key),
            None => ctx,
        }
    }

    fn record_put(&mut self, kind: pb::EntityKind, id: &pb::Id) {
        self.record(ChangeKind::Put, kind, id);
    }

    fn record_delete(&mut self, kind: pb::EntityKind, id: &pb::Id) {
        self.record(ChangeKind::Delete, kind, id);
    }

    fn record(&mut self, change: ChangeKind, kind: pb::EntityKind, id: &pb::Id) {
        self.ops.push(ChangesetOp {
            kind: change,
            entity_ref: pb::EntityRef {
                kind: kind as i32,
                id: Some(id.clone()),
            },
        });
    }

    /// Accumulate the ops a `batch_apply` actually landed, in input order.
    /// `Noop` outcomes contribute nothing: nothing changed, so there is
    /// nothing to attribute.
    fn record_batch(&mut self, metas: &[OpMetaLike], outcomes: &[MutationOutcome]) {
        for (meta, outcome) in metas.iter().zip(outcomes.iter()) {
            match outcome {
                MutationOutcome::Wrote { .. } => self.record_put(meta.kind, &meta.id),
                MutationOutcome::Deleted => self.record_delete(meta.kind, &meta.id),
                MutationOutcome::Noop { .. } => {}
            }
        }
    }

    fn into_record(self, message: String) -> ChangesetRecord {
        ChangesetRecord {
            id: self.id,
            author: self.author,
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            message,
            rpc: self.rpc.into(),
            branch: self.branch,
            ops: self.ops,
            operation_id: self.operation_id,
        }
    }
}

type OwnerSnapshotCache = tokio::sync::RwLock<HashMap<OwnerId, (Lens, Arc<Vec<StoredEntity>>)>>;

pub struct EventModelServiceImpl {
    pub store: Arc<dyn Store>,
    pub git_mirror: Option<Arc<GitMirror>>,
    pub llm_analyzer: Option<Arc<LlmAnalyzer>>,
    pub jev_analyzer: Option<Arc<crate::jev::JevAnalyzer>>,
    /// Serializes mutations within this process so batch apply (and its
    /// dry-run apply+rollback) can never interleave with other writers.
    /// Cross-instance atomicity still requires a single writer deployment.
    mutation_lock: tokio::sync::Mutex<()>,
    search_index: Arc<crate::search::SearchIndex>,
    search_ready: Arc<AtomicBool>,
    /// Indexes built for branch searches, keyed by branch and by a hash of
    /// the merged view they were built from. Branch writes never reach
    /// `search_index`, so a branch query must search something built from
    /// the merged view; without this cache it built one per request and
    /// threw it away. See `crate::search::BranchIndexCache`.
    branch_search_cache: Arc<crate::search::BranchIndexCache>,
    /// Same cache type, reused for a bound caller's own isolated index
    /// (keyed by owner rather than by branch). `search_index` spans every
    /// namespace, so its BM25 statistics (term document frequency, average
    /// document length) are shaped by every tenant's content; searching it
    /// directly would let an owner's result scores shift with another
    /// tenant's writes, which is itself a cross-tenant leak even though no
    /// other tenant's documents are ever returned. A bound caller is
    /// instead searched against an index built fresh from its own
    /// `snapshot`, so its statistics -- and therefore its scores -- depend
    /// only on what it can see.
    owner_search_cache: Arc<crate::search::BranchIndexCache>,
    /// In-process read-snapshot cache. Read RPCs that ask for "every
    /// stored entity" share the same `Arc<Vec<_>>` between calls so a
    /// burst of queries does not turn into a burst of full store scans
    /// (which is N `JetStream` KV list-scans for `NatsStore`, the killer at scale).
    ///
    /// Invalidated by every successful mutation through `invalidate_snapshot`.
    /// Other writers (different process) bypass this cache and may leave
    /// it stale until a local mutation refreshes it: single-writer
    /// deployments are the assumed posture (see `mutation_lock` above).
    ///
    /// Uses `tokio::sync::RwLock` so reads and writes are async-aware and
    /// cannot block the runtime (`parking_lot::RwLock` would block the thread).
    snapshot_cache: tokio::sync::RwLock<Option<Arc<Vec<StoredEntity>>>>,
    /// Cached `ReverseIndex` co-located with the snapshot. Invalidated in
    /// lockstep with `snapshot_cache` so reference queries do not rebuild
    /// the index from the full snapshot on every call.
    reverse_index_cache: tokio::sync::RwLock<Option<Arc<graph::ReverseIndex>>>,
    /// Single-flight guard around the snapshot cache miss path. Without
    /// this, two concurrent readers that both see `None` would each
    /// trigger a full `load_all` (a `JetStream` KV list scan on NATS, the
    /// dominant read-path cost). The mutex serializes the *miss* path
    /// only; cache *hits* still go through the lock-free read path.
    snapshot_load_lock: tokio::sync::Mutex<()>,
    /// Counts RPCs currently executing. Drives the graceful-shutdown
    /// drain in `main.rs` -- drop on `Drop` of `RequestGuard`. Snapshot
    /// cache hit/miss counts go through the Prometheus recorder
    /// (`record_snapshot_cache_hit/miss`); we deliberately do not keep
    /// a duplicate atomic here.
    pub inflight_requests: Arc<std::sync::atomic::AtomicI64>,
    /// Counts active `StreamChanges` subscribers. Capped at
    /// `MAX_STREAM_CHANGES_SUBSCRIBERS`; new streams are rejected with
    /// `resource_exhausted` when the ceiling is reached. Decremented via
    /// RAII guard (`StreamChangesGuard`) when any stream ends.
    stream_changes_count: Arc<std::sync::atomic::AtomicUsize>,
    /// Mints and resolves the opaque cursor `ListChanges`' unscoped poll
    /// falls back to when its bounded lookahead advances past a run of
    /// records a bound caller cannot see. See `change_cursor` for why a raw
    /// sequence number cannot be handed back in that case.
    change_cursor_seal: Arc<crate::change_cursor::ChangeCursorSeal>,
    /// True when the last snapshot load hit the entity cap exactly, meaning
    /// the in-process view may be incomplete. Destructive operations
    /// (`delete_by_query`) refuse to proceed on a truncated snapshot.
    snapshot_truncated: Arc<AtomicBool>,
    /// Which namespaces refuse direct baseline writes. Mutating RPCs
    /// (`PutEntity`, `DeleteEntity`, `BatchMutate`, `RetargetReferences`,
    /// `DeleteByQuery`) reject a request that carries no
    /// `x-trogon-atlas-branch` context and lands in a protected namespace,
    /// unless the caller is an Admin principal. Enforced by the single
    /// `authorize_write` helper (see `docs/explanation/branching.md`,
    /// "Baseline protection"). Defaults to `Off`, matching the behavior
    /// before protection existed.
    baseline_protection: BaselineProtection,
    /// Cached view of the namespace registry: which namespace belongs to
    /// which owner, and which human name resolves to which id.
    ///
    /// The registry is the source of truth for a namespace's *existence* and
    /// naming regardless of who decides visibility, which is why this stays
    /// here rather than moving behind `authorizer`.
    directory: Arc<NamespaceDirectory>,
    /// Who decides whether a caller may see or write a namespace.
    ///
    /// Defaults to [`RegistryAuthorizer`], which answers from `directory` and
    /// is the behaviour ownership shipped with. Swapping in
    /// [`crate::spicedb::SpiceDbAuthorizer`] moves that one decision to
    /// SpiceDB and changes nothing else.
    authorizer: Arc<dyn NamespaceAuthorizer>,
    type_libraries: crate::type_libraries::TypeLibraryGate,
    /// Per-owner projections of the baseline snapshot.
    ///
    /// Filtering is O(entities) and would otherwise run on every request of
    /// every scoped caller. Keyed by owner rather than by principal because
    /// visibility is a property of the owner, so two tokens under one tenant
    /// share the work. Invalidated in lockstep with `snapshot_cache`, and
    /// additionally whenever the registry changes: moving a namespace changes
    /// which entities an owner may see without changing any entity.
    ///
    /// The cached [`Lens`] is stored beside the entities and checked on every
    /// hit, because those two invalidations only cover the changes this
    /// process performs. An owner's view can also change from outside it: a
    /// grant written directly against SpiceDB, which
    /// `docs/explanation/authorization.md` says is how sharing is expressed
    /// and which this server never sees, or a registry move served by another
    /// replica. Keyed by owner alone, a revocation of either kind left the
    /// old entities being served indefinitely, since the fresh lens was never
    /// consulted on a hit. Comparing the lens costs one set comparison and
    /// makes the entry valid exactly as long as the decision behind it.
    owner_snapshot_cache: OwnerSnapshotCache,
    /// Per-owner reverse indexes, co-located with `owner_snapshot_cache` for
    /// the same reason `reverse_index_cache` sits beside `snapshot_cache`,
    /// and lens-keyed for the same reason.
    owner_reverse_index_cache:
        tokio::sync::RwLock<HashMap<OwnerId, (Lens, Arc<graph::ReverseIndex>)>>,
}

fn namespace_record_to_proto(record: &trogon_atlas_store::NamespaceRecord) -> pb::NamespaceRecord {
    pb::NamespaceRecord {
        id: record.id.to_string(),
        name: record.name.to_string(),
        parent: record.parent.to_string(),
        created_at: record.created_at.clone(),
        created_by: record.created_by.clone(),
        provisional_branch: record.tenure.branch().unwrap_or_default().to_owned(),
    }
}

/// Hard ceiling on concurrent `StreamChanges` subscribers. Each subscriber
/// holds a broadcast receiver and a spawned task; unbounded growth would
/// exhaust both goroutine stacks and broadcast buffer memory.
pub const MAX_STREAM_CHANGES_SUBSCRIBERS: usize = 500;

struct StreamChangesGuard(Arc<std::sync::atomic::AtomicUsize>);

impl Drop for StreamChangesGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

// Decode the LifecycleAnnotation status from an entity's metadata, returning
// None when the entity has no annotation. The returned string is already
// lowercased and trimmed. Malformed Any payloads are treated as absent.
fn entity_lifecycle_status(entity: &pb::Entity) -> Option<String> {
    let metadata: &[prost_types::Any] = match entity.kind.as_ref() {
        Some(pb::entity::Kind::Event(x)) => &x.metadata,
        Some(pb::entity::Kind::Command(x)) => &x.metadata,
        Some(pb::entity::Kind::ReadModel(x)) => &x.metadata,
        Some(pb::entity::Kind::Processor(x)) => &x.metadata,
        Some(pb::entity::Kind::Ui(x)) => &x.metadata,
        Some(pb::entity::Kind::Persona(x)) => &x.metadata,
        Some(pb::entity::Kind::Swimlane(x)) => &x.metadata,
        Some(pb::entity::Kind::CommandSlice(x)) => &x.metadata,
        Some(pb::entity::Kind::ReadModelSlice(x)) => &x.metadata,
        Some(pb::entity::Kind::AutomationSlice(x)) => &x.metadata,
        _ => &[],
    };
    for any in metadata {
        if !any
            .type_url
            .ends_with("trogonatlas.annotation.v1alpha1.LifecycleAnnotation")
        {
            continue;
        }
        if let Ok(ann) = <pb::LifecycleAnnotation as prost::Message>::decode(any.value.as_slice()) {
            return Some(ann.status.trim().to_lowercase());
        }
        return None;
    }
    None
}

// Returns true when entity passes the lifecycle filter composed from the two
// repeated fields. Empty status_in and empty status_not_in both match all
// entities. status_key is already lowercased and trimmed, or "" for no
// annotation.
fn passes_lifecycle_filter(
    status_key: &str,
    status_in: &[String],
    status_not_in: &[String],
) -> bool {
    if !status_in.is_empty() {
        let normalized: bool = status_in
            .iter()
            .any(|v| v.trim().to_lowercase() == status_key);
        if !normalized {
            return false;
        }
    }
    if !status_not_in.is_empty() {
        let excluded: bool = status_not_in
            .iter()
            .any(|v| v.trim().to_lowercase() == status_key);
        if excluded {
            return false;
        }
    }
    true
}

impl EventModelServiceImpl {
    pub fn try_new(store: Arc<dyn Store>) -> anyhow::Result<Self> {
        Self::try_new_with_directory(store, Arc::new(NamespaceDirectory::new()))
    }

    /// As [`Self::try_new`], but with the registry cache supplied by the
    /// caller, which is how the freshness bound gets configured.
    pub fn try_new_with_directory(
        store: Arc<dyn Store>,
        directory: Arc<NamespaceDirectory>,
    ) -> anyhow::Result<Self> {
        let store_for_authorizer = store.clone();
        let search_index = Arc::new(
            crate::search::SearchIndex::new()
                .map_err(|e| anyhow::anyhow!("initializing in-RAM search index: {e}"))?,
        );
        Ok(Self {
            store,
            git_mirror: None,
            llm_analyzer: None,
            jev_analyzer: None,
            mutation_lock: tokio::sync::Mutex::new(()),
            search_index,
            search_ready: Arc::new(AtomicBool::new(false)),
            branch_search_cache: Arc::new(crate::search::BranchIndexCache::default()),
            owner_search_cache: Arc::new(crate::search::BranchIndexCache::default()),
            snapshot_cache: tokio::sync::RwLock::new(None),
            reverse_index_cache: tokio::sync::RwLock::new(None),
            snapshot_load_lock: tokio::sync::Mutex::new(()),
            inflight_requests: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            stream_changes_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            change_cursor_seal: Arc::new(crate::change_cursor::ChangeCursorSeal::new(
                &crate::change_cursor::ChangeCursorKey::generate(),
            )),
            snapshot_truncated: Arc::new(AtomicBool::new(false)),
            baseline_protection: BaselineProtection::Off,
            directory: directory.clone(),
            authorizer: Arc::new(RegistryAuthorizer::new(directory, store_for_authorizer)),
            type_libraries: crate::type_libraries::TypeLibraryGate::default(),
            owner_snapshot_cache: tokio::sync::RwLock::new(HashMap::new()),
            owner_reverse_index_cache: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Returns a read snapshot of every stored entity. Cached in process,
    /// invalidated on every local mutation. Each call returns a fresh
    /// `Vec` cloned from the cached `Arc`, the cost we pay to keep the
    /// existing `&[StoredEntity]` consumer signatures untouched. The
    /// clone is dwarfed by the store-list call we used to issue per RPC.
    /// Returns the cached store snapshot as a shared `Arc<Vec<StoredEntity>>`.
    /// All downstream consumers accept `&[StoredEntity]`; pass via
    /// `all.as_slice()` or `&all[..]`. The Arc is shared across concurrent
    /// callers: refcount bumps replace the previous per-RPC Vec clone
    /// (~15 MB at 1500 entities × ~10 KB), which was the dominant
    /// allocation cost on the read path.
    async fn baseline_snapshot(&self) -> Result<Arc<Vec<StoredEntity>>, Status> {
        if let Some(snap) = self.snapshot_cache.read().await.clone() {
            crate::telemetry::record_snapshot_cache_hit();
            return Ok(snap);
        }
        crate::telemetry::record_snapshot_cache_miss();
        // Single-flight: only one task issues the load; the rest wait on
        // the mutex and re-check the cache when they acquire it.
        let _load_guard = self.snapshot_load_lock.lock().await;
        if let Some(snap) = self.snapshot_cache.read().await.clone() {
            return Ok(snap);
        }
        // Instrument the async load with a span. Using `.instrument()` keeps
        // the span active across the await without holding an `EnteredSpan`
        // guard (which is `!Send`) across the suspension point.
        let result = {
            use tracing::Instrument as _;
            graph::load_all(&self.store, None)
                .instrument(tracing::info_span!("snapshot_rebuild"))
                .await?
        };
        // Persist the truncation flag on the service so destructive callers
        // (delete_by_query) can refuse to proceed on a partial view.
        self.snapshot_truncated
            .store(result.truncated, std::sync::atomic::Ordering::Release);
        let all = Arc::new(result.entities);
        *self.snapshot_cache.write().await = Some(all.clone());
        Ok(all)
    }

    /// The caller's view of the store.
    ///
    /// [`Visibility::Everything`] hands back the shared baseline `Arc`
    /// untouched, so an unscoped deployment -- which is every deployment
    /// whose token registry predates ownership -- pays literally nothing for
    /// this indirection.
    async fn snapshot(&self, vis: &Visibility) -> Result<Arc<Vec<StoredEntity>>, Status> {
        let all = self.baseline_snapshot().await?;
        self.narrow_baseline(all, vis).await
    }

    /// Restrict a snapshot to what `vis` admits, memoized per owner.
    ///
    /// Filtering by namespace is sound only because a stored `Id.namespace`
    /// is a [`NamespaceId`] (see the `ownership` module docs). If it held a
    /// human name, two owners' `orders` namespaces would be indistinguishable
    /// here and this filter would be a coin flip.
    async fn narrow(
        &self,
        all: Arc<Vec<StoredEntity>>,
        vis: &Visibility,
    ) -> Result<Arc<Vec<StoredEntity>>, Status> {
        if vis.is_unrestricted() {
            return Ok(all);
        }
        let lens = self.lens(vis).await?;
        Ok(Self::apply_lens(&all, &lens))
    }

    fn apply_lens(all: &[StoredEntity], lens: &Lens) -> Arc<Vec<StoredEntity>> {
        let filtered: Vec<StoredEntity> = all
            .iter()
            .filter(|se| entity_id(&se.entity).is_ok_and(|id| lens.admits(&id.namespace)))
            .cloned()
            .collect();
        Arc::new(filtered)
    }

    /// [`narrow`](Self::narrow) for the baseline view only, with the result
    /// cached per owner. Branch views are not cached, matching
    /// `snapshot_for`'s treatment of the baseline cache.
    async fn narrow_baseline(
        &self,
        all: Arc<Vec<StoredEntity>>,
        vis: &Visibility,
    ) -> Result<Arc<Vec<StoredEntity>>, Status> {
        let Some(owner) = vis.owner() else {
            return Ok(all);
        };
        let lens = self.lens(vis).await?;
        if let Some((cached, hit)) = self.owner_snapshot_cache.read().await.get(owner) {
            if cached == &lens {
                return Ok(hit.clone());
            }
        }
        let filtered = Self::apply_lens(&all, &lens);
        self.owner_snapshot_cache
            .write()
            .await
            .insert(owner.clone(), (lens, filtered.clone()));
        Ok(filtered)
    }

    /// Branch-aware variant of `snapshot()`. When `branch` is `None`,
    /// delegates to the cached baseline `snapshot()` unchanged. When
    /// `branch` is `Some`, always issues a fresh `graph::load_all` scoped to
    /// that branch's merged view and never touches `snapshot_cache` --
    /// caching a branch's merged view alongside the baseline cache would
    /// require per-branch cache keys and per-branch invalidation, which
    /// Phase 1 does not need given branches are expected to be small,
    /// short-lived overlays (see `docs/explanation/branching.md`).
    async fn snapshot_for(
        &self,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<Arc<Vec<StoredEntity>>, Status> {
        let Some(branch) = branch else {
            return self.snapshot(vis).await;
        };
        use tracing::Instrument as _;
        let result = graph::load_all(&self.store, Some(branch))
            .instrument(tracing::info_span!("snapshot_rebuild_branch", branch))
            .await?;
        self.narrow(Arc::new(result.entities), vis).await
    }

    /// [`snapshot_for`](Self::snapshot_for) plus whether the view hit the
    /// entity cap.
    ///
    /// `snapshot_for` drops the branch load's truncation flag (only the
    /// baseline path persists one, on `snapshot_truncated`). Callers that
    /// must report completeness rather than merely tolerate it -- the
    /// snapshot id names a *state*, and naming a partial state as if it
    /// were whole is the failure mode -- need both branches to answer.
    async fn snapshot_with_truncation(
        &self,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<(Arc<Vec<StoredEntity>>, bool), Status> {
        let Some(branch) = branch else {
            let all = self.snapshot(vis).await?;
            let truncated = self.snapshot_truncated.load(Ordering::Acquire);
            return Ok((all, truncated));
        };
        use tracing::Instrument as _;
        let result = graph::load_all(&self.store, Some(branch))
            .instrument(tracing::info_span!("snapshot_rebuild_branch", branch))
            .await?;
        let entities = self.narrow(Arc::new(result.entities), vis).await?;
        Ok((entities, result.truncated))
    }

    async fn lock_mutations(&self) -> tokio::sync::MutexGuard<'_, ()> {
        let started = std::time::Instant::now();
        let guard = self.mutation_lock.lock().await;
        crate::telemetry::record_mutation_lock_wait(started.elapsed().as_secs_f64());
        guard
    }

    /// Marks the cached snapshot and reverse index stale. Every mutation path
    /// calls this after a successful apply so the next read re-fetches.
    async fn invalidate_snapshot(&self) {
        *self.snapshot_cache.write().await = None;
        *self.reverse_index_cache.write().await = None;
        self.owner_snapshot_cache.write().await.clear();
        self.owner_reverse_index_cache.write().await.clear();
        // Reset the truncation flag so the next load re-evaluates.
        self.snapshot_truncated
            .store(false, std::sync::atomic::Ordering::Release);
    }

    /// Returns a cached `ReverseIndex` built from the current snapshot. The index
    /// is rebuilt only when the snapshot was just loaded (cache miss path) or
    /// after a mutation invalidates both caches. Concurrent readers that hit the
    /// populated cache pay only a `RwLock` read-lock and an Arc clone.
    async fn reverse_index(&self, vis: &Visibility) -> Result<Arc<graph::ReverseIndex>, Status> {
        if let Some(owner) = vis.owner() {
            let lens = self.lens(vis).await?;
            if let Some((cached, idx)) = self.owner_reverse_index_cache.read().await.get(owner) {
                if cached == &lens {
                    return Ok(idx.clone());
                }
            }
            let snap = self.snapshot(vis).await?;
            let idx = Arc::new(graph::ReverseIndex::build(&snap));
            self.owner_reverse_index_cache
                .write()
                .await
                .insert(owner.clone(), (lens, idx.clone()));
            return Ok(idx);
        }
        if let Some(idx) = self.reverse_index_cache.read().await.clone() {
            return Ok(idx);
        }
        // Load the snapshot first (which may itself be cached).
        let snap = self.snapshot(vis).await?;
        // Re-check after acquiring write lock: another task may have built the
        // index while we were loading the snapshot.
        let mut guard = self.reverse_index_cache.write().await;
        if let Some(idx) = guard.clone() {
            return Ok(idx);
        }
        let idx = Arc::new(graph::ReverseIndex::build(&snap));
        *guard = Some(idx.clone());
        Ok(idx)
    }

    /// Branch-aware variant of `reverse_index()`. When `branch` is `None`,
    /// delegates to the cached baseline index. When `branch` is `Some`,
    /// builds a fresh index from `snapshot_for` and never touches
    /// `reverse_index_cache` (same rationale as `snapshot_for`).
    async fn reverse_index_for(
        &self,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<Arc<graph::ReverseIndex>, Status> {
        let Some(branch) = branch else {
            return self.reverse_index(vis).await;
        };
        let snap = self.snapshot_for(Some(branch), vis).await?;
        Ok(Arc::new(graph::ReverseIndex::build(&snap)))
    }

    /// Refuses ops that remove a type library an entity schema would still
    /// name once they apply. Scans every tenant, like the referrer gate on
    /// deletes, so a bound caller cannot strand another tenant's schema.
    async fn reject_type_library_users(
        &self,
        branch: Option<&str>,
        vis: &Visibility,
        metas: &[OpMetaLike],
    ) -> Result<(), Status> {
        let removed: std::collections::HashSet<pb::EntityKey> = metas
            .iter()
            .filter(|m| !m.is_put)
            .map(|m| pb::EntityKey::new(m.kind, &m.id))
            .collect();
        let libraries: std::collections::HashSet<pb::EntityKey> = removed
            .iter()
            .filter(|k| k.kind == pb::EntityKind::TypeLibrary)
            .cloned()
            .collect();
        if libraries.is_empty() {
            return Ok(());
        }
        let puts: Vec<&pb::Entity> = metas
            .iter()
            .filter(|m| m.is_put)
            .filter_map(|m| m.entity.as_ref())
            .collect();
        let unrestricted = Visibility::unrestricted(vis.principal().into());
        let before = self.snapshot_for(branch, &unrestricted).await?;
        crate::type_libraries::reject_schema_users(&graph::schema_users(
            &before, &puts, &removed, &libraries,
        ))
    }

    /// Hand ownership decisions to something other than the namespace
    /// registry.
    ///
    /// The default authorizer reads the registry's `parent` column, which can
    /// only ever express one owner per namespace. Replacing it is how a
    /// deployment gets sharing, delegation, or an owner hierarchy without any
    /// of the handlers below knowing that happened.
    #[must_use]
    pub fn with_authorizer(mut self, authorizer: Arc<dyn NamespaceAuthorizer>) -> Self {
        self.authorizer = authorizer;
        self
    }

    /// The registry cache this service reads namespace rows through.
    ///
    /// Exposed so a replacement authorizer shares it rather than opening a
    /// second one, which would let the two disagree about whether a namespace
    /// exists.
    #[must_use]
    pub fn directory(&self) -> Arc<NamespaceDirectory> {
        self.directory.clone()
    }

    pub fn with_git_mirror(mut self, mirror: Arc<GitMirror>) -> Self {
        self.git_mirror = Some(mirror);
        self
    }

    pub fn with_llm_analyzer(mut self, analyzer: Arc<LlmAnalyzer>) -> Self {
        self.llm_analyzer = Some(analyzer);
        self
    }

    pub fn with_jev_analyzer(mut self, analyzer: Arc<crate::jev::JevAnalyzer>) -> Self {
        self.jev_analyzer = Some(analyzer);
        self
    }

    /// Enable baseline write protection for every namespace
    /// (`--protect-baseline` / `TROGON_ATLAS_PROTECT_BASELINE`). See
    /// `authorize_write`.
    #[must_use]
    pub fn with_protect_baseline(self, protect: bool) -> Self {
        self.with_baseline_protection(if protect {
            BaselineProtection::All
        } else {
            BaselineProtection::Off
        })
    }

    /// Protect baseline in the named namespaces only
    /// (`--protect-baseline-namespaces` /
    /// `TROGON_ATLAS_PROTECT_BASELINE_NAMESPACES`). See `authorize_write`.
    #[must_use]
    pub fn with_baseline_protection(mut self, protection: BaselineProtection) -> Self {
        self.baseline_protection = protection;
        self
    }

    /// Seal `ListChanges` cursors under an explicitly configured key
    /// (`--change-cursor-key` / `TROGON_ATLAS_CHANGE_CURSOR_KEY`) instead of
    /// the random one generated at construction. Required for any
    /// deployment where more than one process can serve the same poll (a
    /// standby, a reader role): a cursor minted by one process must open on
    /// another holding the same key.
    #[must_use]
    pub fn with_change_cursor_key(mut self, key: &crate::change_cursor::ChangeCursorKey) -> Self {
        self.change_cursor_seal = Arc::new(crate::change_cursor::ChangeCursorSeal::new(key));
        self
    }

    fn author_from_metadata<T>(req: &Request<T>) -> MirrorAuthor {
        let md = req.metadata();
        let header_email = md
            .get("x-trogon-atlas-author-email")
            .and_then(|v| v.to_str().ok())
            .map(sanitize_author_field)
            .unwrap_or_default();

        // When the auth stack is present the Principal is in request
        // extensions and carries a verified name. Use it as the committed
        // author name so callers cannot spoof each other via headers.
        // The header email is still accepted as advisory (it carries
        // information the token registry does not hold); the header name is
        // silently ignored when a Principal is present.
        if let Some(principal) = req.extensions().get::<crate::auth::Principal>() {
            let principal_name = principal.name.as_ref().to_owned();
            // Derive a synthetic email from the principal name when no
            // advisory header email was supplied, keeping git commits
            // attributable without requiring operators to configure email.
            let email = if header_email.is_empty() {
                format!("{principal_name}@trogon-atlas")
            } else {
                header_email
            };
            return MirrorAuthor {
                name: principal_name,
                email,
            };
        }

        // No auth stack (e.g. unit tests that construct the service directly):
        // fall back to the self-asserted headers.
        let header_name = md
            .get("x-trogon-atlas-author-name")
            .and_then(|v| v.to_str().ok())
            .map(sanitize_author_field)
            .unwrap_or_default();
        if header_name.is_empty() || header_email.is_empty() {
            tracing::debug!(
                name_present = !header_name.is_empty(),
                email_present = !header_email.is_empty(),
                "x-trogon-atlas-author-name/email not provided; server default author will be used"
            );
        }
        MirrorAuthor {
            name: header_name,
            email: header_email,
        }
    }

    /// Extract and validate the optional `x-trogon-atlas-branch` request
    /// header (Phase 1: Isolation). Returns `Ok(None)` when the header is
    /// absent (the common case: baseline reads/writes), `Ok(Some(name))`
    /// when present and valid, or `Status::invalid_argument` when the header
    /// value is not valid UTF-8, fails the same charset rule as
    /// namespace/slug id components, or is the reserved name `"meta"`.
    fn branch_from_metadata<T>(req: &Request<T>) -> Result<Option<String>, Status> {
        let Some(value) = req.metadata().get("x-trogon-atlas-branch") else {
            return Ok(None);
        };
        let name = value.to_str().map_err(|_| {
            Status::invalid_argument("x-trogon-atlas-branch header is not valid UTF-8")
        })?;
        validate_branch_name(name)?;
        Ok(Some(name.to_string()))
    }

    /// The authenticated caller, cloned out of request extensions so the
    /// authorization checks can run after `into_inner()` consumed the
    /// request. `None` when no auth stack is mounted (unit tests that build
    /// the service directly), which authorizes everything: there is no
    /// identity to scope against.
    fn principal_from_metadata<T>(req: &Request<T>) -> Option<crate::auth::Principal> {
        req.extensions().get::<crate::auth::Principal>().cloned()
    }

    /// What this request is allowed to see.
    ///
    /// A principal with no `parent` -- which is every principal in a token
    /// registry written before ownership existed, and every caller when auth
    /// is off entirely -- sees everything, exactly as before.
    fn visibility_of<T>(req: &Request<T>) -> Visibility {
        let principal = Self::principal_from_metadata(req);
        let name = principal
            .as_ref()
            .map_or_else(|| Arc::from(UNAUTHENTICATED_PRINCIPAL), |p| p.name.clone());
        match principal.and_then(|p| p.parent) {
            Some(owner) => Visibility::owned_by(name, owner),
            None => Visibility::unrestricted(name),
        }
    }

    /// The namespace registry, indexed. Registry facts only: who may see
    /// what is [`Self::lens`]'s question, not this one.
    async fn directory_view(&self) -> Result<Arc<crate::ownership::DirectoryView>, Status> {
        self.directory.view(&*self.store).await
    }

    /// Gate a direct-by-key read on ownership.
    ///
    /// Answers `NOT_FOUND` rather than `PERMISSION_DENIED` on purpose.
    /// `PERMISSION_DENIED` distinguishes "exists but is not yours" from "does
    /// not exist", which turns any lookup into an existence oracle over
    /// another tenant's model. A caller who cannot see a namespace should not
    /// be able to learn what is in it by guessing slugs.
    async fn require_visible(&self, vis: &Visibility, namespace: &str) -> Result<(), Status> {
        if vis.is_unrestricted() {
            return Ok(());
        }
        // A namespace that is not even a legal id belongs to nobody, and
        // must not slip through on a technicality.
        let Ok(id) = NamespaceId::parse(namespace) else {
            return Err(Status::not_found("entity not found"));
        };
        if self.authorizer.admits(vis, &id).await? {
            return Ok(());
        }
        Err(Status::not_found("entity not found"))
    }

    /// The set of namespaces this request may read, resolved once so the
    /// per-record filters that follow stay synchronous.
    async fn lens(&self, vis: &Visibility) -> Result<Lens, Status> {
        self.authorizer.lens(vis).await
    }

    /// Decide whether `next` is safe to hand back as a plain `ListChanges`
    /// cursor, or must be sealed through `change_cursor_seal` instead.
    ///
    /// Safe to return plain exactly when: the caller has no tenancy boundary
    /// to protect (`vis.is_unrestricted()`), the response already disclosed
    /// `next` itself (`disclosed`, true whenever some returned event's own
    /// seq is what `next` holds), or `next` is unchanged from a `since_token`
    /// the caller supplied as a plain number it already knew. Every other
    /// case -- an empty `since_token` this handler resolved to the store's
    /// live tip, a previously sealed token, or genuine progress past
    /// invisible-only records -- would hand a bound caller a number it did
    /// not already possess, so it gets sealed instead.
    fn list_changes_next_token(
        &self,
        vis: &Visibility,
        raw_since_token: &str,
        since: u64,
        next: u64,
        disclosed: bool,
    ) -> String {
        if vis.is_unrestricted() || disclosed {
            return next.to_string();
        }
        let since_given_plain =
            !raw_since_token.is_empty() && raw_since_token.parse::<u64>().is_ok();
        if since_given_plain && next == since {
            return next.to_string();
        }
        self.change_cursor_seal.seal(vis.principal(), next)
    }

    /// Whether `principal` may see a branch literally named `branch`.
    ///
    /// `None` (no auth stack mounted) and an unrestricted principal (no
    /// `parent`) see every branch, exactly as before ownership existed: this
    /// is the same boundary [`Visibility::is_unrestricted`] draws for
    /// namespaces. A bound principal sees only a branch whose name is an
    /// owned branch ([`crate::ownership::OwnedBranchName`]) it is delegated
    /// for ([`crate::auth::Principal::authorizes_owner`]); a classic/global
    /// branch stays exactly as unpartitioned to a bound caller as these RPCs
    /// used to be outright, because its deltas can still span namespaces no
    /// single owner can vouch for.
    fn branch_visible(principal: Option<&crate::auth::Principal>, branch: &str) -> bool {
        let Some(principal) = principal else {
            return true;
        };
        if principal.parent.is_none() {
            return true;
        }
        match crate::ownership::OwnedBranchName::try_parse(branch) {
            Some(Ok(owned)) => principal.authorizes_owner(owned.owner()),
            _ => false,
        }
    }

    /// Whether `principal` may see one changeset: delegates to
    /// [`Self::branch_visible`] on the changeset's own `branch` field, since
    /// a changeset inherits its branch's owner. A baseline changeset
    /// (`branch: None`) is treated the same as a classic/global branch name:
    /// visible only to an unrestricted caller.
    fn changeset_visible(
        principal: Option<&crate::auth::Principal>,
        record: &trogon_atlas_store::store::ChangesetRecord,
    ) -> bool {
        Self::branch_visible(principal, record.branch.as_deref().unwrap_or(""))
    }

    /// Refuse an RPC whose response cannot be honestly narrowed to one owner.
    ///
    /// A changeset or a branch diff is a single unit of work that may touch
    /// several namespaces at once. Returning it with the other owners' rows
    /// stripped out would describe a change that never happened, and
    /// returning it whole would leak. Neither is acceptable, so a bound
    /// caller is told plainly that the surface is not partitioned yet rather
    /// than being handed a plausible answer.
    fn refuse_unpartitioned(vis: &Visibility, surface: &str) -> Result<(), Status> {
        let Some(owner) = vis.owner() else {
            return Ok(());
        };
        Err(Status::permission_denied(format!(
            "{surface} is not partitioned by owner, so it cannot be shown to a caller bound to \
             parent {owner} without either leaking another owner's changes or misrepresenting \
             its own"
        )))
    }

    /// Gate a branch-mutating RPC (`CreateBranch`, `UpdateBranch`,
    /// `ResolveBranchEntry`, `DeleteBranch`, `MergeBranch`) by ownership
    /// delegation instead of [`Self::refuse_unpartitioned`]'s blanket
    /// refusal: refusing every write leaves a delegated agent unable to do
    /// any work on its own owner's branch, which defeats the point of
    /// delegation.
    ///
    /// `None` (no auth stack mounted) and an unrestricted principal (no
    /// `parent`) may write any branch, exactly as before ownership existed.
    /// A bound principal may write only an owned branch
    /// ([`crate::ownership::OwnedBranchName`]) whose owner it
    /// [authorizes][crate::auth::Principal::authorizes_owner]; a
    /// classic/global branch, or one owned by someone else, is refused.
    ///
    /// This clears only the delegation gate. `MergeBranch` lands real
    /// namespace writes, so it separately runs
    /// [`Self::require_namespace_scope`] over the merge diff afterward;
    /// `RevertChangeset` touches a surface with no owner at all and keeps
    /// calling [`Self::refuse_unpartitioned`] unconditionally.
    fn authorize_branch_write(
        principal: Option<&crate::auth::Principal>,
        branch: &str,
        surface: &str,
    ) -> Result<(), Status> {
        let Some(principal) = principal else {
            return Ok(());
        };
        if principal.parent.is_none() {
            return Ok(());
        }
        match crate::ownership::OwnedBranchName::try_parse(branch) {
            Some(Ok(owned)) if principal.authorizes_owner(owned.owner()) => Ok(()),
            _ => Err(Status::permission_denied(format!(
                "{surface} on branch {branch:?} is not delegated to principal {}; only an \
                 owned branch (`@owner:name`) whose owner this principal authorizes may be \
                 written",
                principal.name
            ))),
        }
    }

    /// Drop every cache whose contents depend on the registry.
    ///
    /// A registry write changes which entities an owner may see without
    /// changing a single entity, so the per-owner projections go stale even
    /// though `snapshot_cache` is still perfectly valid.
    async fn invalidate_directory(&self) {
        self.directory.invalidate().await;
        self.owner_snapshot_cache.write().await.clear();
        self.owner_reverse_index_cache.write().await.clear();
    }

    /// Single enforcement point for the two write axes that the role check in
    /// `AuthzLayer` cannot see, because both need the request body:
    ///
    /// 1. **Namespace scope.** The caller's `Principal` may carry an
    ///    allow-list (`crate::scope::NamespaceScope`). It applies to every
    ///    write, branch or baseline: a branch is a staging area for baseline,
    ///    so letting a scoped principal write outside its namespaces on a
    ///    branch would only defer the violation to merge time. An empty
    ///    allow-list, which is what every registry written before the field
    ///    existed parses into, admits everything.
    /// 2. **Baseline protection** (opt-in via `--protect-baseline` /
    ///    `--protect-baseline-namespaces`; see
    ///    `docs/explanation/branching.md`, "Baseline protection"). A request
    ///    with branch context is never a baseline write and is always
    ///    allowed. Without branch context, a write into a protected namespace
    ///    requires a genuinely authenticated `Role::Admin` --
    ///    `Principal::is_anonymous` must be false.
    ///    `--insecure-allow-anonymous` inserts a synthetic `Role::Admin`
    ///    principal with `is_anonymous: true` so ordinary RBAC checks still
    ///    pass with auth off; that principal is deliberately excluded from
    ///    this escape hatch so an anonymous dev stack with protection on
    ///    genuinely enforces branching, with no admin bypass available to
    ///    anonymous callers.
    ///
    /// Callers pass the namespaces the RPC is about to write. Scope is
    /// checked first: "you may not write there at all" is a more fundamental
    /// answer than "not directly on baseline".
    async fn authorize_write(
        &self,
        principal: Option<&crate::auth::Principal>,
        branch: Option<&str>,
        targets: &WriteTargets,
    ) -> Result<(), Status> {
        self.require_namespace_scope(principal, targets, &NamespaceTenure::for_branch(branch))
            .await?;
        if branch.is_some() || !self.baseline_protection.protects(targets) {
            return Ok(());
        }
        let is_authenticated_admin =
            principal.is_some_and(|p| p.role == crate::auth::Role::Admin && !p.is_anonymous);
        if is_authenticated_admin {
            return Ok(());
        }
        Err(Status::failed_precondition(
            "baseline is protected: make changes on a branch (x-trogon-atlas-branch metadata / \
             trogon-atlas --branch) and merge them",
        ))
    }

    /// The namespace half of `authorize_write`, on its own. Used by
    /// `MergeBranch`, which lands on baseline by design and so must not
    /// consult baseline protection, but must still respect the caller's
    /// allow-list: merging is how a branch write reaches baseline, and a
    /// scoped principal cannot be allowed to launder a write through it.
    async fn require_namespace_scope(
        &self,
        principal: Option<&crate::auth::Principal>,
        targets: &WriteTargets,
        tenure: &NamespaceTenure,
    ) -> Result<(), Status> {
        let Some(principal) = principal else {
            return Ok(());
        };
        match principal.namespaces.deny_reason(targets) {
            None => {}
            Some(OutOfScope::Namespace(namespace)) => {
                return Err(Status::permission_denied(format!(
                    "principal {} may not write namespace {namespace:?}; its scope is {}",
                    principal.name, principal.namespaces,
                )))
            }
            Some(OutOfScope::Unbounded) => {
                return Err(Status::permission_denied(format!(
                    "principal {} is scoped to {} and this RPC rewrites entities it selects \
                     itself, which no allow-list can bound",
                    principal.name, principal.namespaces,
                )))
            }
        }
        self.require_namespace_ownership(principal, targets, tenure)
            .await
    }

    /// The ownership half of the namespace check, and the point where a
    /// namespace first enters the registry.
    ///
    /// Two things happen here, in this order and for the same reason.
    ///
    /// **Claim.** A target with no registry row gets one, owned by the
    /// caller's `parent` (or [`DEFAULT_OWNER`] when the caller has none).
    /// Without this, ownership would be a thing operators had to remember to
    /// declare before writing, and the first thing they would notice is a
    /// scoped principal locked out of namespaces it just created. Claiming is
    /// idempotent and the id equals the name, so it neither renames anything
    /// nor moves any key.
    ///
    /// **Deny.** A target owned by somebody else is refused, and the caller
    /// is told to register a name of its own. That is the collision boundary:
    /// claim-on-write is first-come-first-served on the bare name, and
    /// [`Store::register_namespace`] is the path that always succeeds because
    /// it mints an id instead of adopting one.
    ///
    /// An unscoped principal is never denied. It still claims, so the
    /// registry stays complete and a later tenancy rollout has nothing to
    /// backfill.
    async fn require_namespace_ownership(
        &self,
        principal: &crate::auth::Principal,
        targets: &WriteTargets,
        tenure: &NamespaceTenure,
    ) -> Result<(), Status> {
        let owner = match principal.parent.clone() {
            Some(owner) => owner,
            None => OwnerId::parse(DEFAULT_OWNER)
                .map_err(|e| Status::internal(format!("default owner is invalid: {e}")))?,
        };
        let enforced = principal.parent.is_some();

        let WriteTargets::Known(namespaces) = targets else {
            // Nothing to claim, because nothing is enumerable. For an
            // unscoped caller that is merely a gap in bookkeeping; for a
            // scoped one it is a request to rewrite entities chosen by a
            // query, which cannot be shown to stay inside one tenant.
            if enforced {
                return Err(Status::permission_denied(format!(
                    "principal {} is bound to parent {owner} and this RPC rewrites entities it \
                     selects itself, which cannot be shown to stay within one owner",
                    principal.name,
                )));
            }
            return Ok(());
        };

        let vis = Visibility::owned_by(principal.name.clone(), owner.clone());
        let mut claimed = false;
        for namespace in namespaces {
            let id = NamespaceId::parse(namespace).map_err(|e| {
                Status::invalid_argument(format!("invalid namespace {namespace:?}: {e}"))
            })?;
            match self.authorizer.may_write(&vis, &id).await? {
                WriteVerdict::Allowed => continue,
                WriteVerdict::Denied { owner: holder } => {
                    if enforced {
                        // The holder goes to the operator, never to the
                        // caller: whoever is debugging "why can't my client
                        // write" needs the name, and the client that could
                        // ask this question for any namespace would otherwise
                        // enumerate its neighbours one guess at a time.
                        // "Taken, pick another" is all the caller needs.
                        tracing::info!(
                            principal = %principal.name,
                            %owner,
                            namespace = %namespace,
                            %holder,
                            "refused a write into a namespace held by another owner",
                        );
                        return Err(Status::permission_denied(format!(
                            "principal {} is bound to parent {owner} and namespace \
                             {namespace:?} is not available to it; register a namespace of your \
                             own to get a distinct id",
                            principal.name,
                        )));
                    }
                    continue;
                }
                WriteVerdict::Unclaimed => {}
            }
            let name = NamespaceName::parse(namespace).map_err(|e| {
                Status::invalid_argument(format!("invalid namespace {namespace:?}: {e}"))
            })?;
            let claim = self
                .store
                .adopt_namespace(&name, &owner, tenure)
                .await
                .map_err(store_err)?;
            claimed = true;
            // Before the authorizer is told, so that an authorizer reading
            // the registry sees the row it is being told about.
            self.invalidate_directory().await;
            self.authorizer.on_registered(&claim.record).await?;
        }
        if claimed {
            self.invalidate_directory().await;
        }
        Ok(())
    }

    /// Settle the registry rows a branch was holding open, now that the
    /// branch has merged or been deleted.
    ///
    /// A row is released when its namespace holds nothing on baseline, and
    /// made permanent when it holds something. Which of the two applies is
    /// not decided by how the branch ended: a merge that landed nothing
    /// leaves the namespace as empty as an abandoned branch does, and a
    /// namespace somebody wrote on baseline while the branch was alive is
    /// public work whatever the branch went on to do. Asking baseline
    /// directly answers all of those without enumerating them.
    ///
    /// Reads the registry rather than the cached directory view, because the
    /// view is allowed to be stale by design and this decides what to delete.
    async fn settle_provisional_namespaces(&self, branch: &str) -> Result<(), Status> {
        let provisional: Vec<_> = self
            .store
            .list_namespaces()
            .await
            .map_err(store_err)?
            .into_iter()
            .filter(|record| record.tenure.branch() == Some(branch))
            .collect();
        if provisional.is_empty() {
            return Ok(());
        }
        let baseline = self.baseline_snapshot().await?;
        let occupied: std::collections::BTreeSet<&str> = baseline
            .iter()
            .filter_map(|se| trogon_atlas_store::refs::entity_id(&se.entity))
            .map(|id| id.namespace.as_str())
            .collect();
        for record in provisional {
            if occupied.contains(record.id.as_str()) {
                self.store
                    .promote_namespace(&record.id)
                    .await
                    .map_err(store_err)?;
                continue;
            }
            self.store
                .release_namespace(&record.id)
                .await
                .map_err(store_err)?;
            self.authorizer.on_released(&record).await?;
        }
        self.invalidate_directory().await;
        Ok(())
    }

    /// The owner a registration lands under.
    ///
    /// A bound caller may only register inside its own subtree, and may leave
    /// the field empty to mean exactly that. An unbound caller may name any
    /// owner, and gets [`DEFAULT_OWNER`] when it names none.
    fn resolve_write_parent(vis: &Visibility, requested: &str) -> Result<OwnerId, Status> {
        let requested = requested.trim();
        match vis.owner() {
            Some(owner) => {
                if requested.is_empty() || requested == owner.as_str() {
                    Ok(owner.clone())
                } else {
                    Err(Status::permission_denied(format!(
                        "caller is bound to parent {owner} and may not register under \
                         {requested:?}",
                    )))
                }
            }
            None if requested.is_empty() => OwnerId::parse(DEFAULT_OWNER)
                .map_err(|e| Status::internal(format!("default owner is invalid: {e}"))),
            None => OwnerId::parse(requested)
                .map_err(|e| Status::invalid_argument(format!("invalid parent: {e}"))),
        }
    }

    /// Where `info`'s branch sits relative to baseline as a whole: its fork
    /// point plus the baseline changesets recorded after it, newest first.
    ///
    /// Walks the changeset log backwards from newest until it reaches the
    /// fork point, so the cost is proportional to how far behind the branch
    /// is, not to the size of the log. A branch with no fork point (cut
    /// before the pointer existed, or from a store with no attributed write
    /// history) reports no ancestry rather than claiming the entire log
    /// landed after it.
    async fn branch_ancestry(
        &self,
        info: &trogon_atlas_store::store::BranchInfo,
    ) -> Result<pb::BranchAncestry, Status> {
        let Some(fork) = info.fork_changeset_id.as_deref() else {
            return Ok(pb::BranchAncestry::default());
        };
        let mut behind: Vec<pb::Changeset> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut behind_truncated = false;
        loop {
            let page = self
                .store
                .list_changesets(ChangesetPage {
                    before: cursor.as_deref(),
                    limit: BRANCH_BEHIND_CAP,
                    scope: ChangesetScope::Baseline,
                })
                .await
                .map_err(store_err)?;
            let exhausted = page.len() < BRANCH_BEHIND_CAP;
            cursor = page.last().map(|record| record.id.clone());
            for record in page {
                // Ids are UUIDv7: lexicographic order is chronological, so
                // reaching the fork point ends the walk. `<=` rather than
                // `==` so a fork point that was purged out from under us
                // still terminates instead of draining the whole log.
                if record.id.as_str() <= fork {
                    return Ok(pb::BranchAncestry {
                        fork_changeset_id: fork.to_owned(),
                        behind,
                        behind_truncated: false,
                    });
                }
                if behind.len() >= BRANCH_BEHIND_CAP {
                    behind_truncated = true;
                    break;
                }
                behind.push(changeset_to_proto(record));
            }
            if behind_truncated || exhausted || cursor.is_none() {
                break;
            }
        }
        Ok(pb::BranchAncestry {
            fork_changeset_id: fork.to_owned(),
            behind,
            behind_truncated,
        })
    }

    /// Move a merged-but-kept branch's fork point up to the merge it just
    /// produced (`landed`), or to the newest baseline changeset when the
    /// merge landed nothing because everything had already converged.
    ///
    /// Without this a branch kept across a merge immediately reports itself
    /// behind by its own merge, which is the one answer that is certainly
    /// wrong: its deltas were purged, so there is nothing left to reconcile.
    ///
    /// `landed` may name a changeset whose append was lost. That is harmless:
    /// the ancestry walk compares ids as positions and stops at the first one
    /// ordered at or below the fork point, so a fork point with no row behind
    /// it still terminates the walk.
    async fn catch_up_kept_branch(&self, name: &str, landed: Option<&str>) -> Result<(), Status> {
        let target = match landed {
            Some(id) => Some(id.to_owned()),
            None => self
                .store
                .newest_baseline_changeset()
                .await
                .map_err(store_err)?,
        };
        if let Some(target) = target {
            self.store
                .set_branch_fork_point(name, &target)
                .await
                .map_err(store_err)?;
        }
        Ok(())
    }

    /// Look one branch up by name, or `NotFound`.
    async fn require_branch(
        &self,
        name: &str,
    ) -> Result<trogon_atlas_store::store::BranchInfo, Status> {
        self.store
            .list_branches()
            .await
            .map_err(store_err)?
            .into_iter()
            .find(|b| b.name == name)
            .ok_or_else(|| branch_not_found(name))
    }

    /// Classify one raw branch delta row against the *current* baseline
    /// (Phase 2: Review and merge). See `docs/explanation/branching.md`,
    /// "Merge conflicts", for the full decision table this implements.
    /// Classify a single branch delta against the live baseline.
    ///
    /// Returns the diff entry plus the freshly-read baseline etag paired
    /// with `theirs` (`None` when the baseline has no such key). The etag
    /// is not part of `BranchDiffEntry` on the wire; `update_branch` needs
    /// it to rebase a delta's `base_etag` forward to the CURRENT baseline
    /// revision, not the stale one recorded when the delta was created.
    async fn classify_branch_delta(
        &self,
        delta: trogon_atlas_store::store::BranchDeltaEntry,
    ) -> Result<(pb::BranchDiffEntry, Option<String>), Status> {
        let entity_ref = pb::EntityRef {
            kind: delta.kind as i32,
            id: Some(delta.id.clone()),
        };
        let current = self.store.get(delta.kind, &delta.id, None).await;
        let (theirs, theirs_etag) = match current {
            Ok(stored) => (Some(stored.entity), Some(stored.etag)),
            Err(trogon_atlas_store::error::StoreError::NotFound) => (None, None),
            Err(e) => return Err(store_err(e)),
        };

        let baseline_moved = match (&delta.base, &theirs) {
            (None, None) => false,
            (None, Some(_)) | (Some(_), None) => true,
            (Some(base), Some(theirs_entity)) => {
                !trogon_atlas_core::semantic_eq::semantically_equal(base, theirs_entity)
            }
        };

        let status = if delta.tombstone {
            if !baseline_moved {
                pb::branch_diff_entry::Status::Deleted
            } else if theirs.is_some() {
                pb::branch_diff_entry::Status::ConflictEditDelete
            } else {
                // Baseline already deleted it too: converges to nothing.
                pb::branch_diff_entry::Status::Converged
            }
        } else if delta.base.is_none() {
            // First touch on the branch created this key.
            if theirs.is_none() {
                pb::branch_diff_entry::Status::Added
            } else {
                // Baseline gained the same key after the branch forked with
                // no base recorded (created independently); treat as an
                // edit/edit conflict unless content converged.
                let ours_entity = delta.ours.as_ref();
                let converged = ours_entity
                    .zip(theirs.as_ref())
                    .is_some_and(|(o, t)| trogon_atlas_core::semantic_eq::semantically_equal(o, t));
                if converged {
                    pb::branch_diff_entry::Status::Converged
                } else {
                    pb::branch_diff_entry::Status::ConflictEditEdit
                }
            }
        } else if !baseline_moved {
            pb::branch_diff_entry::Status::Changed
        } else {
            match (&delta.ours, &theirs) {
                (Some(ours), Some(theirs_entity)) => {
                    if trogon_atlas_core::semantic_eq::semantically_equal(ours, theirs_entity) {
                        pb::branch_diff_entry::Status::Converged
                    } else {
                        pb::branch_diff_entry::Status::ConflictEditEdit
                    }
                }
                (Some(_), None) => pb::branch_diff_entry::Status::ConflictDeleteEdit,
                (None, _) => {
                    // `ours` absent but not a tombstone is not a valid delta
                    // shape; treat conservatively as a conflict so it is
                    // never silently dropped or landed.
                    pb::branch_diff_entry::Status::ConflictEditEdit
                }
            }
        };

        let is_edit_edit = matches!(status, pb::branch_diff_entry::Status::ConflictEditEdit);
        let conflict_field_paths = if is_edit_edit {
            match (&delta.ours, &theirs) {
                (Some(ours), Some(theirs_entity)) => {
                    conflict_field_paths(ours, theirs_entity).unwrap_or_default()
                }
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };

        // A three-way merge needs all three sides. An edit/edit conflict
        // without a recorded `base` is the independent-creation case: both
        // sides authored the key from nothing, and there is no common
        // ancestor to attribute either side's fields to.
        let auto_merged = match (is_edit_edit, &delta.base, &delta.ours, &theirs) {
            (true, Some(base), Some(ours), Some(theirs_entity)) => {
                match crate::merge3::three_way(base, ours, theirs_entity) {
                    Ok(crate::merge3::Merge3::Merged(entity)) => Some(*entity),
                    Ok(crate::merge3::Merge3::Conflict(_)) => None,
                    Err(err) => {
                        tracing::debug!(error = %err, "structural three-way merge unavailable");
                        None
                    }
                }
            }
            _ => None,
        };

        let state = pb::BranchEntryState {
            base_etag: delta.base_etag.clone(),
            ours_etag: delta.etag,
            theirs_etag: theirs_etag.clone().unwrap_or_default(),
        };
        Ok((
            pb::BranchDiffEntry {
                r#ref: Some(entity_ref),
                status: status as i32,
                base: delta.base,
                base_etag: delta.base_etag,
                ours: delta.ours,
                theirs,
                conflict_field_paths,
                auto_merged,
                state: Some(state),
            },
            theirs_etag,
        ))
    }

    async fn mirror_apply(&self, batch: MirrorBatch) {
        if let Some(mirror) = self.git_mirror.as_ref() {
            use tracing::Instrument as _;
            let fut = async {
                if let Err(err) = mirror.apply(batch).await {
                    tracing::warn!(error = %err, "git mirror apply failed");
                    metrics::counter!("git_mirror_failures_total").increment(1);
                }
            };
            fut.instrument(tracing::info_span!("mirror_apply")).await;
        }
    }

    /// Seed the live search index from a full store snapshot on first use.
    /// The store scan happens entirely outside the mutation lock so writers are
    /// never blocked by index initialization. The ready flag is set with a
    /// `compare_exchange` so only the first concurrent winner publishes the index.
    pub async fn ensure_search_index(&self) -> Result<(), Status> {
        if self.search_ready.load(Ordering::Acquire) {
            return Ok(());
        }
        // Build the index without holding any lock; a concurrent caller may
        // do the same work and that is acceptable -- the last compare_exchange
        // winner's index is the one that gets used.
        let all = self
            .store
            .list(ListFilter::default(), Some(MAX_PROJECTION_ENTITIES), None)
            .await
            .map_err(store_err)?;
        // replace_all acquires a parking_lot mutex and calls writer.commit()
        // (blocking I/O); push onto the blocking pool so we do not stall the
        // async runtime for the duration of the Tantivy commit.
        let index = self.search_index.clone();
        tokio::task::spawn_blocking(move || index.replace_all(&all))
            .await
            .map_err(|e| Status::internal(format!("search index build task panicked: {e}")))?
            .map_err(|e| Status::internal(format!("search index build failed: {e}")))?;
        // Atomically publish. If another task already set the flag, this
        // is a no-op; either index is valid.
        let _ =
            self.search_ready
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire);
        Ok(())
    }

    /// Shared handles needed to warm up the search index in the background.
    /// `main.rs` clones these before consuming `self` into the tonic server
    /// so the first `SearchEntities` caller does not pay the full
    /// `list + replace_all` cost.
    pub fn search_warmup_handles(
        &self,
    ) -> (
        Arc<dyn Store>,
        Arc<crate::search::SearchIndex>,
        Arc<AtomicBool>,
    ) {
        (
            self.store.clone(),
            self.search_index.clone(),
            self.search_ready.clone(),
        )
    }

    /// Index maintenance must never fail a mutation; on error the index is
    /// marked stale and rebuilt on the next search. The Tantivy writer holds
    /// a `parking_lot` mutex and calls `commit()` which is blocking I/O; push the
    /// call onto the `spawn_blocking` pool so it does not stall the async runtime.
    fn search_apply_put(&self, entity: &pb::Entity) {
        if !self.search_ready.load(Ordering::Acquire) {
            return;
        }
        let index = self.search_index.clone();
        let ready = self.search_ready.clone();
        let entity = entity.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(err) = index.apply_put(&entity) {
                tracing::warn!(error = %err, "search index put failed; index will rebuild");
                ready.store(false, Ordering::Release);
                metrics::counter!("trogon_atlas_search_index_errors_total").increment(1);
            }
        });
    }

    fn search_apply_delete(&self, kind: pb::EntityKind, id: &pb::Id) {
        if !self.search_ready.load(Ordering::Acquire) {
            return;
        }
        let index = self.search_index.clone();
        let ready = self.search_ready.clone();
        let id = id.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(err) = index.apply_delete(kind, &id) {
                tracing::warn!(error = %err, "search index delete failed; index will rebuild");
                ready.store(false, Ordering::Release);
                metrics::counter!("trogon_atlas_search_index_errors_total").increment(1);
            }
        });
    }

    /// Apply each successful outcome of a `batch_apply` to the
    /// downstream side-effect channels: the search index and the git
    /// mirror. Change records are already written atomically by
    /// `batch_apply` itself; this method handles only the non-durable
    /// side effects. The lock is held by the caller.
    async fn apply_batch_side_effects(
        &self,
        metas: &[OpMetaLike],
        outcomes: &[MutationOutcome],
        author: MirrorAuthor,
    ) {
        let mut mirror_changes: Vec<MirrorChange> = Vec::with_capacity(metas.len());
        for (meta, oc) in metas.iter().zip(outcomes.iter()) {
            match oc {
                // Side effects see the entity AS STORED (system stamp
                // included) so the git mirror stays byte-faithful to the
                // store, not to whatever the client happened to send.
                MutationOutcome::Wrote { entity, .. } => {
                    self.search_apply_put(entity);
                    mirror_changes.push(MirrorChange::Put(entity.clone()));
                }
                // A skipped write changed nothing downstream.
                MutationOutcome::Noop { .. } => {}
                MutationOutcome::Deleted => {
                    self.search_apply_delete(meta.kind, &meta.id);
                    mirror_changes.push(MirrorChange::Delete {
                        kind: meta.kind,
                        id: meta.id.clone(),
                    });
                }
            }
        }
        if !mirror_changes.is_empty() {
            self.mirror_apply(MirrorBatch {
                message: git_mirror::batch_commit_message(mirror_changes.len()),
                changes: mirror_changes,
                author,
            })
            .await;
        }
    }

    /// Append the durable record of what this RPC landed.
    ///
    /// Called after the entity writes commit, with the ops that actually
    /// landed. A changeset that changed nothing (every write a no-op, or a
    /// dry run) is not recorded: an empty changeset is noise, not history.
    ///
    /// A failed append is logged and counted, not returned. The writes are
    /// already durable; failing the RPC here would tell the caller their
    /// mutation did not happen when it did. The window this leaves is
    /// narrower than the change feed's: losing a changeset takes a KV write
    /// failing on a store that just accepted the entity writes, whereas the
    /// change feed drops events whenever a publish exhausts its retries.
    /// Watch `trogon_atlas_changesets_lost_total` alongside
    /// `trogon_atlas_store_change_events_lost_total`.
    async fn record_changeset(&self, changeset: PendingChangeset, message: String) {
        if changeset.ops.is_empty() {
            return;
        }
        let record = changeset.into_record(message);
        if let Err(err) = self.store.append_changeset(&record).await {
            metrics::counter!("trogon_atlas_changesets_lost_total").increment(1);
            tracing::error!(
                changeset_id = %record.id,
                rpc = %record.rpc,
                op_count = record.ops.len(),
                error = %err,
                "changeset append failed after the writes committed; \
                 those changes have no durable attribution"
            );
        }
    }

    /// Builds the failure response for a `batch_apply` that rejected an
    /// op. Both the dry-run and real-apply branches of `batch_mutate`
    /// emit the same shape -- pulled out so the two stay aligned.
    ///
    /// Expects `err` to be `StoreError::BatchFailed`; any other variant
    /// is treated as an unknown batch failure at index 0. The issue
    /// message goes through `store_err` so backend detail stays in the
    /// server log. `code` is one of the reasons documented at
    /// `docs/reference/failure-reasons.md`, chosen by
    /// `batch_mutate_failure_code` from `mutations` and the failing op's
    /// own error, so a caller can branch on it instead of the prose in
    /// `message`.
    fn batch_failure_response(
        metas: &[OpMetaLike],
        mutations: &[MutationOp],
        err: trogon_atlas_store::error::StoreError,
    ) -> pb::BatchMutateResponse {
        let (index, source) = match err {
            trogon_atlas_store::error::StoreError::BatchFailed { index, source } => {
                (index, *source)
            }
            other => (0, other),
        };
        let code = Self::batch_mutate_failure_code(mutations, index, &source);
        let message = store_err(source).message().to_owned();
        // Bound-check `metas[index]`: defensively, a store that returns an
        // out-of-range index would panic the RPC. Fall back to a sentinel
        // subject; the message still carries `index` so the operator can
        // trace it.
        let subject = metas.get(index).map(|m| pb::EntityRef {
            kind: m.kind as i32,
            id: Some(m.id.clone()),
        });
        pb::BatchMutateResponse {
            status: pb::batch_mutate_response::Status::Failed as i32,
            results: Vec::new(),
            failed_op_index: i32::try_from(index).unwrap_or(i32::MAX),
            failure: vec![pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: code.into(),
                message,
                subject,
                subject_field: String::new(),
                ..Default::default()
            }],
            precondition_failure: None,
            operation_receipt: None,
        }
    }

    /// The best available `Status` for a `batch_failure_response`, used only
    /// to settle an operation receipt. `batch_failure_response` already
    /// collapses every store error into one "BATCH_OP_FAILED" issue, so this
    /// settles with the same generic code rather than re-deriving the
    /// original error's exact gRPC code from information the response no
    /// longer carries.
    fn batch_failure_status(resp: &pb::BatchMutateResponse) -> Status {
        let message = resp
            .failure
            .first()
            .map_or_else(|| "batch_mutate failed".to_owned(), |f| f.message.clone());
        Status::failed_precondition(message)
    }

    /// The `ValidationIssue.code` for a batch op failure: `STALE_PRECONDITION`
    /// when the op was refused because planned-against state moved,
    /// `ENTITY_NOT_FOUND` / `INVALID_OP` when the store names a more specific
    /// cause, and `BATCH_OP_FAILED` only as the fallback for everything else
    /// (a backend error, a lost change event, and similar causes that are
    /// not actionable by the caller beyond retrying).
    fn batch_mutate_failure_code(
        mutations: &[MutationOp],
        index: usize,
        source: &trogon_atlas_store::error::StoreError,
    ) -> &'static str {
        use trogon_atlas_store::error::StoreError;
        if Self::refused_precondition(mutations, index, source).is_some() {
            return "STALE_PRECONDITION";
        }
        match source {
            StoreError::NotFound => "ENTITY_NOT_FOUND",
            StoreError::InvalidArgument(_) => "INVALID_OP",
            _ => "BATCH_OP_FAILED",
        }
    }

    /// `batch_failure_response`'s `ENTITY_REFERENCED` case: a batch delete
    /// op whose mode defaults to (or asks for) `FAIL_IF_REFERENCED` found a
    /// referrer, so the whole batch is rejected before any op commits --
    /// the same refusal `delete_entity` gives a standalone caller. Runs
    /// before any operation_id is claimed, so a retry simply re-derives the
    /// same deterministic rejection rather than needing a stored receipt.
    fn batch_referenced_response(metas: &[OpMetaLike], index: usize) -> pb::BatchMutateResponse {
        let subject = metas.get(index).map(|m| pb::EntityRef {
            kind: m.kind as i32,
            id: Some(m.id.clone()),
        });
        pb::BatchMutateResponse {
            status: pb::batch_mutate_response::Status::Failed as i32,
            results: Vec::new(),
            failed_op_index: i32::try_from(index).unwrap_or(i32::MAX),
            failure: vec![pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "ENTITY_REFERENCED".into(),
                message: "entity is referenced; pass mode=FORCE or repair referrers first".into(),
                subject,
                subject_field: String::new(),
                ..Default::default()
            }],
            precondition_failure: None,
            operation_receipt: None,
        }
    }

    /// `batch_failure_response`, plus the structured precondition failure
    /// when the failed op was refused because the state it was planned
    /// against moved. Must run under the mutation lock so the reported
    /// actual etag is the one that refused the op.
    async fn batch_failure(
        &self,
        metas: &[OpMetaLike],
        mutations: &[MutationOp],
        err: trogon_atlas_store::error::StoreError,
        branch: Option<&str>,
    ) -> Result<pb::BatchMutateResponse, Status> {
        use trogon_atlas_store::error::StoreError;
        let refused = match &err {
            StoreError::BatchFailed { index, source } => {
                Self::refused_precondition(mutations, *index, source)
            }
            _ => None,
        };
        let mut resp = Self::batch_failure_response(metas, mutations, err);
        let Some((index, expected_etag)) = refused else {
            return Ok(resp);
        };
        let Some(meta) = metas.get(index) else {
            return Ok(resp);
        };
        let actual_etag = match self.store.get(meta.kind, &meta.id, branch).await {
            Ok(stored) => stored.etag,
            Err(StoreError::NotFound) => String::new(),
            Err(e) => return Err(store_err(e)),
        };
        resp.precondition_failure = Some(pb::PreconditionFailure {
            subject: Some(pb::EntityRef {
                kind: meta.kind as i32,
                id: Some(meta.id.clone()),
            }),
            expected_etag,
            actual_etag,
        });
        Ok(resp)
    }

    /// The failed op's index and the etag it expected, when the op was
    /// refused because the state it was planned against moved.
    fn refused_precondition(
        mutations: &[MutationOp],
        index: usize,
        source: &trogon_atlas_store::error::StoreError,
    ) -> Option<(usize, String)> {
        use trogon_atlas_store::error::StoreError;
        let expected_etag = match (mutations.get(index), source) {
            (Some(MutationOp::Create { .. }), StoreError::AlreadyExists) => String::new(),
            (
                Some(
                    MutationOp::Put {
                        if_match: Some(expected),
                        ..
                    }
                    | MutationOp::Delete {
                        if_match: Some(expected),
                        ..
                    },
                ),
                StoreError::EtagMismatch { .. } | StoreError::NotFound,
            ) => expected.clone(),
            _ => return None,
        };
        Some((index, expected_etag))
    }
}

/// Strip control characters (including newlines, CR, and other C0/C1 controls)
/// from a git author field and cap its length, preventing header-injection
/// in git commits.
fn sanitize_author_field(s: &str) -> String {
    const MAX_LEN: usize = 256;
    s.chars()
        .filter(|c| !c.is_control())
        .take(MAX_LEN)
        .collect()
}

fn is_slice_kind(kind: pb::EntityKind) -> bool {
    matches!(
        kind,
        pb::EntityKind::CommandSlice
            | pb::EntityKind::ReadModelSlice
            | pb::EntityKind::AutomationSlice
            | pb::EntityKind::UiSlice
    )
}

fn build_op_results(
    metas: &[OpMetaLike],
    outcomes: &[MutationOutcome],
    validated: bool,
    per_op_validation: &[Vec<pb::ValidationIssue>],
    per_op_referrers: &[Vec<pb::Reference>],
) -> Vec<pb::BatchMutateOpResult> {
    metas
        .iter()
        .zip(outcomes.iter())
        .enumerate()
        .map(|(i, (m, oc))| {
            if m.is_put {
                let (etag, no_op, stored) = match oc {
                    MutationOutcome::Wrote { etag, entity } => {
                        let etag = if validated {
                            String::new()
                        } else {
                            etag.clone()
                        };
                        (etag, false, Some(entity.clone()))
                    }
                    MutationOutcome::Noop { etag, entity } => {
                        let etag = if validated {
                            String::new()
                        } else {
                            etag.clone()
                        };
                        (etag, true, Some(entity.clone()))
                    }
                    MutationOutcome::Deleted => (String::new(), false, None),
                };
                let validation = per_op_validation.get(i).cloned().unwrap_or_default();
                pb::BatchMutateOpResult {
                    result: Some(pb::batch_mutate_op_result::Result::Put(
                        pb::PutEntityResponse {
                            entity: stored.or_else(|| m.entity.clone()),
                            etag,
                            validation,
                            no_op,
                            operation_receipt: None,
                        },
                    )),
                }
            } else {
                let referrers = per_op_referrers.get(i).cloned().unwrap_or_default();
                pb::BatchMutateOpResult {
                    result: Some(pb::batch_mutate_op_result::Result::Delete(
                        pb::DeleteEntityResponse {
                            referrers,
                            operation_receipt: None,
                        },
                    )),
                }
            }
        })
        .collect()
}

struct OpMetaLike {
    kind: pb::EntityKind,
    id: pb::Id,
    is_put: bool,
    entity: Option<pb::Entity>,
}

// `PAGE_TOKEN_PREFIX` keeps clients out of the encoding's internals: a bare
// integer offset reads like an integer offset and clients will start to
// depend on the format. Wrapping the offset in an opaque-looking prefix +
// base64 makes the contract obvious: round-trip the string as-is.
const PAGE_TOKEN_PREFIX: &str = "v1.offset.";

// `tonic::Status` is ~176 bytes and cannot be shrunk (third-party type).
// The allows below are justified: boxing would add an allocation for no benefit.
#[allow(clippy::result_large_err)]
fn parse_page_token(token: &str) -> Result<usize, Status> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    if token.is_empty() {
        return Ok(0);
    }
    let body = token
        .strip_prefix(PAGE_TOKEN_PREFIX)
        .ok_or_else(|| Status::invalid_argument("invalid page_token"))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(body)
        .map_err(|_| Status::invalid_argument("invalid page_token"))?;
    let s =
        std::str::from_utf8(&bytes).map_err(|_| Status::invalid_argument("invalid page_token"))?;
    s.parse::<usize>()
        .map_err(|_| Status::invalid_argument("invalid page_token"))
}

fn encode_page_token(offset: usize) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let body = URL_SAFE_NO_PAD.encode(offset.to_string());
    format!("{PAGE_TOKEN_PREFIX}{body}")
}

fn slice_from_entity(entity: &pb::Entity) -> Option<pb::Slice> {
    use pb::entity::Kind as K;
    match entity.kind.clone()? {
        K::CommandSlice(s) => Some(pb::Slice {
            kind: Some(pb::slice::Kind::Command(s)),
        }),
        K::ReadModelSlice(s) => Some(pb::Slice {
            kind: Some(pb::slice::Kind::ReadModel(s)),
        }),
        K::AutomationSlice(s) => Some(pb::Slice {
            kind: Some(pb::slice::Kind::Automation(s)),
        }),
        K::UiSlice(s) => Some(pb::Slice {
            kind: Some(pb::slice::Kind::Ui(s)),
        }),
        _ => None,
    }
}

fn clamp_page_size(requested: i32, default: i32, max: i32) -> usize {
    let v = if requested <= 0 {
        default
    } else {
        requested.min(max)
    };
    // Both `default` and `max` are positive i32 consts; v >= 1 after the
    // max(1) call, so the sign-loss cast is always in range.
    usize::try_from(v.max(1)).unwrap_or(1)
}

fn format_scenario_issues(issues: &[pb::ValidationIssue]) -> String {
    let parts: Vec<String> = issues
        .iter()
        .filter(|i| i.severity == pb::validation_issue::Severity::Error as i32)
        .map(|i| format!("{}: {}", i.code, i.message))
        .collect();
    if parts.is_empty() {
        "scenario validation failed".into()
    } else {
        format!("scenario validation failed: {}", parts.join("; "))
    }
}

/// Convert this process's writer-lease status into the wire message
/// `GetServerInfoResponse` advertises it as.
fn writer_status_to_pb(status: WriterStatus) -> pb::get_server_info_response::WriterStatus {
    use pb::get_server_info_response::WriterRole as PbWriterRole;
    let role = match status.role {
        WriterRole::Writer => PbWriterRole::Writer,
        WriterRole::Standby => PbWriterRole::Standby,
        WriterRole::Reader => PbWriterRole::Reader,
    };
    pb::get_server_info_response::WriterStatus {
        role: role as i32,
        epoch: status.epoch.get(),
        lease_holder: status.lease_holder,
    }
}

#[tonic::async_trait]
impl pb::event_model_service_server::EventModelService for EventModelServiceImpl {
    async fn get_server_info(
        &self,
        _req: Request<pb::GetServerInfoRequest>,
    ) -> Result<Response<pb::GetServerInfoResponse>, Status> {
        Ok(Response::new(pb::GetServerInfoResponse {
            schema_version: pb::SCHEMA_VERSION.into(),
            server_version: env!("CARGO_PKG_VERSION").into(),
            features: Some(pb::get_server_info_response::Features {
                search: true,
                change_feed: true,
                infer_data_flow: true,
                check_information_completeness: true,
                diff_entities: true,
                impact_analysis: true,
                extract_subgraph: true,
                supersession_descendants: true,
                mutations: true,
                validate_only: true,
                branch_scoped_requests: true,
                state_preconditions: true,
                operation_receipts: true,
                single_writer_fencing: true,
                namespace_grants: true,
                owned_branches: true,
                who_am_i: true,
                type_libraries: true,
            }),
            limits: Some(pb::get_server_info_response::Limits {
                max_page_size: ADVERTISED_MAX_PAGE_SIZE,
                default_page_size: ADVERTISED_DEFAULT_PAGE_SIZE,
                max_impact_depth: ADVERTISED_MAX_IMPACT_DEPTH,
                max_projection_entities: i32::try_from(MAX_PROJECTION_ENTITIES).unwrap_or(i32::MAX),
                max_batch_get_keys: i32::try_from(ADVERTISED_MAX_BATCH_GET_KEYS)
                    .unwrap_or(i32::MAX),
                max_change_feed_page: ADVERTISED_MAX_CHANGE_FEED_PAGE,
                operation_retention_seconds: ADVERTISED_OPERATION_RETENTION_SECONDS,
            }),
            contract_revision: pb::CONTRACT_REVISION,
            min_client_contract_revision: pb::MIN_CLIENT_CONTRACT_REVISION,
            writer_status: Some(writer_status_to_pb(self.store.writer_status())),
        }))
    }

    async fn who_am_i(
        &self,
        req: Request<pb::WhoAmIRequest>,
    ) -> Result<Response<pb::WhoAmIResponse>, Status> {
        let principal = Self::principal_from_metadata(&req);
        let (name, role, kind, is_anonymous, namespaces, parent) = match principal {
            Some(p) => (
                p.name,
                p.role,
                p.kind,
                p.is_anonymous,
                p.namespaces,
                p.parent,
            ),
            // No auth stack mounted at all: unit tests that build the
            // service directly, or a server run without the BearerAuth
            // interceptor. Treated the same as the synthetic principal
            // `--insecure-allow-anonymous` authenticates every caller as,
            // since neither case has a real identity to report.
            None => (
                Arc::from(UNAUTHENTICATED_PRINCIPAL),
                crate::auth::Role::Admin,
                crate::auth::PrincipalKind::User,
                true,
                crate::scope::NamespaceScope::unrestricted(),
                None,
            ),
        };
        Ok(Response::new(pb::WhoAmIResponse {
            name: name.to_string(),
            kind: pb::PrincipalKind::from(kind) as i32,
            role: pb::Role::from(role) as i32,
            namespaces: namespaces.to_strings(),
            owner: parent
                .map(|owner| owner.as_str().to_owned())
                .unwrap_or_default(),
            anonymous: is_anonymous,
        }))
    }

    async fn compile_type_library(
        &self,
        req: Request<pb::CompileTypeLibraryRequest>,
    ) -> Result<Response<pb::CompileTypeLibraryResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let library = req
            .into_inner()
            .library
            .ok_or_else(|| Status::invalid_argument("library is required"))?;
        let id = require_id(library.id.as_ref(), "library")?;
        self.require_visible(&vis, &id.namespace).await?;
        if let Some(name) = branch.as_deref() {
            self.require_branch(name).await?;
        }
        let response = self
            .type_libraries
            .dry_run(&*self.store, &library, branch.as_deref())
            .await?;
        Ok(Response::new(response))
    }

    async fn resolve_type(
        &self,
        req: Request<pb::ResolveTypeRequest>,
    ) -> Result<Response<pb::ResolveTypeResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let type_url = trogon_atlas_types::TypeUrl::parse(&req.type_url)
            .map_err(|e| Status::invalid_argument(format!("type_url: {e}")))?;
        self.require_visible(&vis, &req.namespace).await?;
        if let Some(name) = branch.as_deref() {
            self.require_branch(name).await?;
        }
        let response = self
            .type_libraries
            .resolve(&*self.store, &req.namespace, &type_url, branch.as_deref())
            .await?;
        Ok(Response::new(response))
    }

    async fn get_type_library_descriptor_set(
        &self,
        req: Request<pb::GetTypeLibraryDescriptorSetRequest>,
    ) -> Result<Response<pb::GetTypeLibraryDescriptorSetResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        self.require_visible(&vis, &req.namespace).await?;
        if let Some(name) = branch.as_deref() {
            self.require_branch(name).await?;
        }
        let response = self
            .type_libraries
            .descriptor_set(&*self.store, &req.namespace, branch.as_deref())
            .await?;
        Ok(Response::new(response))
    }

    async fn list_namespaces(
        &self,
        req: Request<pb::ListNamespacesRequest>,
    ) -> Result<Response<pb::ListNamespacesResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        // Route through `snapshot_for()` so this read shares the cached
        // baseline scan with the other read RPCs instead of issuing its
        // own full list on every call, and so a branch that introduces a
        // namespace lists it. The registry half below is deliberately not
        // branched: ownership is not copy-on-write.
        let all = self.snapshot_for(branch.as_deref(), &vis).await?;
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for se in all.iter() {
            let id = entity_id(&se.entity)?;
            *counts.entry(id.namespace.clone()).or_default() += 1;
        }

        // Join the entity counts onto the registry. The two sources disagree
        // in both directions and both disagreements are legitimate: a
        // registered namespace with nothing in it yet has a row and no
        // entities, and a namespace whose entities predate the registry has
        // entities and no row. Listing the union keeps this RPC usable
        // before, during, and after the backfill.
        let view = self.directory_view().await?;
        let lens = self.lens(&vis).await?;
        let mut namespaces: Vec<pb::list_namespaces_response::Namespace> = Vec::new();
        for record in view.records() {
            if !lens.admits(record.id.as_str()) {
                continue;
            }
            namespaces.push(pb::list_namespaces_response::Namespace {
                id: record.id.to_string(),
                name: record.name.to_string(),
                parent: record.parent.to_string(),
                entity_count: counts.remove(record.id.as_str()).unwrap_or(0),
            });
        }
        // Whatever the snapshot still holds has no registry row. `snapshot`
        // already dropped these for a scoped caller, so anything left here
        // belongs to an unscoped one.
        for (namespace, entity_count) in counts {
            namespaces.push(pb::list_namespaces_response::Namespace {
                id: namespace.clone(),
                name: namespace,
                parent: String::new(),
                entity_count,
            });
        }
        namespaces.sort_by(|a, b| (&a.parent, &a.name).cmp(&(&b.parent, &b.name)));
        Ok(Response::new(pb::ListNamespacesResponse { namespaces }))
    }

    async fn register_namespace(
        &self,
        req: Request<pb::RegisterNamespaceRequest>,
    ) -> Result<Response<pb::RegisterNamespaceResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let principal = Self::principal_from_metadata(&req);
        let msg = req.into_inner();

        let name = NamespaceName::parse(msg.name.trim())
            .map_err(|e| Status::invalid_argument(format!("invalid namespace name: {e}")))?;
        let parent = Self::resolve_write_parent(&vis, &msg.parent)?;

        let claim = self
            .store
            .register_namespace(
                &name,
                &parent,
                principal.as_ref().map_or("anonymous", |p| p.name.as_ref()),
            )
            .await
            .map_err(store_err)?;
        if claim.created {
            self.invalidate_directory().await;
        }
        // Told on every call, not only on creation. A retry after a failed
        // grant is the only way back from a namespace whose row exists but
        // whose grant did not land, and `RegisterNamespace` is idempotent
        // precisely so that retry is available.
        self.authorizer.on_registered(&claim.record).await?;
        Ok(Response::new(pb::RegisterNamespaceResponse {
            namespace: Some(namespace_record_to_proto(&claim.record)),
            created: claim.created,
        }))
    }

    async fn move_namespace(
        &self,
        req: Request<pb::MoveNamespaceRequest>,
    ) -> Result<Response<pb::MoveNamespaceResponse>, Status> {
        // Deliberately no `visibility_of` gate on the source: a move is the
        // one operation that crosses an ownership boundary, so a caller who
        // could only act inside their own could never complete one. It is
        // restricted to Admin by `required_role` instead, and a caller bound
        // to an owner -- however it got that role, since the token registry
        // is only one way to produce a `Principal` -- is refused outright:
        // binding and boundary-crossing are mutually exclusive by
        // definition, so this cannot rely on the registry alone having
        // refused the combination at load time.
        if let Some(principal) = Self::principal_from_metadata(&req) {
            if principal.parent.is_some() {
                return Err(Status::permission_denied(
                    "a principal bound to an owner may not move a namespace between owners",
                ));
            }
        }
        let msg = req.into_inner();
        let id = NamespaceId::parse(msg.id.trim())
            .map_err(|e| Status::invalid_argument(format!("invalid namespace id: {e}")))?;
        let parent = OwnerId::parse(msg.parent.trim())
            .map_err(|e| Status::invalid_argument(format!("invalid parent: {e}")))?;

        let previous = self
            .directory_view()
            .await?
            .get(&id)
            .map(|record| record.parent.clone());
        let record = self
            .store
            .move_namespace(&id, &parent)
            .await
            .map_err(store_err)?;
        self.invalidate_directory().await;
        if let Some(previous) = previous {
            self.authorizer.on_moved(&record, &previous).await?;
        }
        Ok(Response::new(pb::MoveNamespaceResponse {
            namespace: Some(namespace_record_to_proto(&record)),
        }))
    }

    async fn get_snapshot_id(
        &self,
        req: Request<pb::GetSnapshotIdRequest>,
    ) -> Result<Response<pb::GetSnapshotIdResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let (all, truncated) = self
            .snapshot_with_truncation(branch.as_deref(), &vis)
            .await?;

        // Empty means every namespace. A set rather than a scan per entity so
        // a wide filter stays linear.
        let scope: std::collections::BTreeSet<&str> =
            req.namespaces.iter().map(String::as_str).collect();

        let mut hasher = trogon_atlas_core::SnapshotHasher::new();
        let mut entries = Vec::new();
        for se in all.iter() {
            let id = entity_id(&se.entity)?;
            if !scope.is_empty() && !scope.contains(id.namespace.as_str()) {
                continue;
            }
            let kind = entity_kind(&se.entity)?;
            let key = trogon_atlas_store::key::entity_key(kind, id);
            let hash = trogon_atlas_core::entity_content_hash(&se.entity).map_err(|e| {
                // An entity that cannot render as canonical JSON has no
                // content identity, so the snapshot has none either. Failing
                // is the honest answer: silently skipping it would produce an
                // id for a model that is missing an entity.
                Status::failed_precondition(format!(
                    "entity {key} has no content identity, so no snapshot id can be computed: {e}"
                ))
            })?;
            if req.include_entries {
                entries.push(pb::SnapshotEntry {
                    key: key.clone(),
                    entity: Some(pb::EntityRef {
                        kind: kind as i32,
                        id: Some(id.clone()),
                    }),
                    content_hash: hash.to_hex(),
                });
            }
            hasher.insert(key, hash).map_err(|e| {
                // Two rows for one key means the store enumeration is
                // broken; hashing on would name a state that never existed.
                Status::internal(format!("snapshot enumeration produced {e}"))
            })?;
        }
        // `SnapshotHasher` keeps its own sorted order; sort the response
        // entries to match, since the proto promises key order.
        entries.sort_by(|a, b| a.key.cmp(&b.key));

        Ok(Response::new(pb::GetSnapshotIdResponse {
            snapshot_id: hasher.finish().to_hex(),
            entity_count: u32::try_from(hasher.len()).unwrap_or(u32::MAX),
            truncated,
            entries,
        }))
    }

    async fn get_entity(
        &self,
        req: Request<pb::GetEntityRequest>,
    ) -> Result<Response<pb::GetEntityResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let kind = kind_from_i32(req.kind, "kind")?;
        let id = require_id(req.id.as_ref(), "")?;
        // Direct-by-key reads never touch the snapshot, so the visibility
        // filter that covers every other read path does not reach them.
        self.require_visible(&vis, &id.namespace).await?;
        let stored = self
            .store
            .get(kind, id, branch.as_deref())
            .await
            .map_err(store_err)?;
        Ok(Response::new(pb::GetEntityResponse {
            entity: Some(stored.entity),
            etag: stored.etag,
        }))
    }

    async fn batch_get_entities(
        &self,
        req: Request<pb::BatchGetEntitiesRequest>,
    ) -> Result<Response<pb::BatchGetEntitiesResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        if req.keys.len() > ADVERTISED_MAX_BATCH_GET_KEYS {
            return Err(Status::invalid_argument("keys exceeds max_batch_get_keys"));
        }
        let mut parsed: Vec<(pb::EntityKind, pb::Id)> = Vec::with_capacity(req.keys.len());
        // Which requested keys the caller may see. An invisible key reads as
        // absent rather than as an error, so one out-of-scope key in a batch
        // of 500 does not fail the other 499, and dense mode reports it the
        // same way it reports a key that genuinely does not exist.
        let mut visible: Vec<bool> = Vec::with_capacity(req.keys.len());
        let lens = self.lens(&vis).await?;
        for er in &req.keys {
            let kind = kind_from_i32(er.kind, "keys[].kind")?;
            let id = require_id(er.id.as_ref(), "keys[]")?;
            let admitted = lens.admits(&id.namespace);
            visible.push(admitted);
            if admitted {
                parsed.push((kind, id.clone()));
            }
        }
        let fetched = self
            .store
            .batch_get(&parsed, branch.as_deref())
            .await
            .map_err(store_err)?;
        // Re-expand to the caller's key order, filling the skipped positions
        // with `None`. Dense mode's `found` array is positional, so dropping
        // keys silently would misalign every entry after the first skip.
        let result: Vec<Option<StoredEntity>> = if lens.is_unrestricted() {
            fetched
        } else {
            let mut fetched = fetched.into_iter();
            visible
                .iter()
                .map(|admitted| {
                    if *admitted {
                        fetched.next().flatten()
                    } else {
                        None
                    }
                })
                .collect()
        };
        let (entities, found, etags) = if req.dense {
            let mut entities = Vec::with_capacity(result.len());
            let mut found = Vec::with_capacity(result.len());
            let mut etags = Vec::with_capacity(result.len());
            for opt in result {
                found.push(opt.is_some());
                let (entity, etag) = opt.map(|s| (s.entity, s.etag)).unwrap_or_default();
                entities.push(entity);
                etags.push(etag);
            }
            (entities, found, etags)
        } else {
            // found is only meaningful in dense mode; leave it empty in sparse mode
            let (entities, etags) = result
                .into_iter()
                .flatten()
                .map(|s| (s.entity, s.etag))
                .unzip();
            (entities, Vec::new(), etags)
        };
        Ok(Response::new(pb::BatchGetEntitiesResponse {
            entities,
            found,
            etags,
        }))
    }

    async fn list_entities(
        &self,
        req: Request<pb::ListEntitiesRequest>,
    ) -> Result<Response<pb::ListEntitiesResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let page_size = clamp_page_size(
            req.page_size,
            ADVERTISED_DEFAULT_PAGE_SIZE,
            ADVERTISED_MAX_PAGE_SIZE,
        );
        let offset = parse_page_token(&req.page_token)?;

        let mut kinds: Vec<pb::EntityKind> = Vec::with_capacity(req.kinds.len());
        for k in &req.kinds {
            kinds.push(kind_from_i32(*k, "kinds[]")?);
        }
        for ns in &req.namespaces {
            crate::conv::validate_id_component("namespaces[]", ns)?;
        }
        let all = self.snapshot_for(branch.as_deref(), &vis).await?;
        // Apply kind/namespace filters and latest_versions_only in-process
        // rather than issuing a full store scan on every call. The snapshot
        // is sorted (kind, namespace, slug, version asc) by graph::load_all,
        // which matches the ordering the store would return.
        let mut filtered: Vec<&trogon_atlas_store::StoredEntity> = all
            .iter()
            .filter(|s| {
                if !kinds.is_empty() {
                    match entity_kind(&s.entity) {
                        Ok(k) => {
                            if !kinds.contains(&k) {
                                return false;
                            }
                        }
                        Err(_) => return false,
                    }
                }
                if !req.namespaces.is_empty() {
                    match entity_id(&s.entity) {
                        Ok(id) => {
                            if !req.namespaces.iter().any(|ns| ns == &id.namespace) {
                                return false;
                            }
                        }
                        Err(_) => return false,
                    }
                }
                true
            })
            .collect();
        if req.latest_versions_only {
            use std::collections::BTreeMap;
            let mut latest: BTreeMap<(i32, String, String), &trogon_atlas_store::StoredEntity> =
                BTreeMap::new();
            for s in filtered {
                let (Ok(k), Ok(id)) = (entity_kind(&s.entity), entity_id(&s.entity)) else {
                    continue;
                };
                let group = (k as i32, id.namespace.clone(), id.slug.clone());
                let keep = latest
                    .get(&group)
                    .and_then(|cur| {
                        entity_id(&cur.entity)
                            .ok()
                            .map(|cid| cid.version < id.version)
                    })
                    .unwrap_or(true);
                if keep {
                    latest.insert(group, s);
                }
            }
            filtered = latest.into_values().collect();
        }
        // Apply lifecycle_status_in / lifecycle_status_not_in filters before
        // pagination so page offsets remain stable for the same filter params.
        // "" in either list matches entities with no LifecycleAnnotation.
        if !req.lifecycle_status_in.is_empty() || !req.lifecycle_status_not_in.is_empty() {
            filtered.retain(|s| {
                let status = entity_lifecycle_status(&s.entity);
                let status_key: &str = status.as_deref().unwrap_or("");
                passes_lifecycle_filter(
                    status_key,
                    &req.lifecycle_status_in,
                    &req.lifecycle_status_not_in,
                )
            });
        }
        let total = filtered.len();
        let slice: Vec<pb::Entity> = filtered
            .into_iter()
            .skip(offset)
            .take(page_size)
            .map(|s| s.entity.clone())
            .collect();
        let next_offset = offset + slice.len();
        let next_page_token = if next_offset >= total {
            String::new()
        } else {
            encode_page_token(next_offset)
        };
        Ok(Response::new(pb::ListEntitiesResponse {
            entities: slice,
            next_page_token,
        }))
    }

    async fn search_entities(
        &self,
        req: Request<pb::SearchEntitiesRequest>,
    ) -> Result<Response<pb::SearchEntitiesResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let q = req.query.trim();
        if q.is_empty() {
            return Ok(Response::new(pb::SearchEntitiesResponse {
                results: Vec::new(),
            }));
        }
        const MAX_QUERY_BYTES: usize = 2048;
        if q.len() > MAX_QUERY_BYTES {
            return Err(Status::invalid_argument(format!(
                "query exceeds maximum length of {MAX_QUERY_BYTES} bytes"
            )));
        }
        let mut kinds: Vec<pb::EntityKind> = Vec::with_capacity(req.kinds.len());
        for k in &req.kinds {
            kinds.push(kind_from_i32(*k, "kinds[]")?);
        }
        for ns in &req.namespaces {
            crate::conv::validate_id_component("namespaces[]", ns)?;
        }
        #[allow(clippy::cast_sign_loss)] // guarded: <= 0 branch returns 50, else branch is positive
        let limit = if req.limit <= 0 {
            50_usize
        } else {
            req.limit.min(ADVERTISED_MAX_PAGE_SIZE) as usize
        };
        let kind_labels: Vec<String> = kinds
            .iter()
            .map(|k| crate::search::kind_label_for(*k).to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let query = q.to_string();
        let namespaces = req.namespaces.clone();

        // Every other read path narrows through `snapshot`/`snapshot_for`.
        // This one queries a tantivy index that spans all namespaces, so the
        // lens has to be folded into the namespace filter by hand or a bound
        // caller sees other owners' titles and excerpts. The filter is
        // applied before the limit is taken, so a bound caller still gets a
        // full page of its own hits rather than a page trimmed after the
        // fact. An empty allow-list means "no namespaces", never "all", so
        // it returns early instead of falling through to an unfiltered
        // search.
        let namespaces = if vis.is_unrestricted() {
            namespaces
        } else {
            match self.lens(&vis).await? {
                Lens::Everything => namespaces,
                Lens::Only(visible) => {
                    let allowed: Vec<String> = if namespaces.is_empty() {
                        visible.iter().map(|id| id.as_str().to_owned()).collect()
                    } else {
                        namespaces
                            .into_iter()
                            .filter(|ns| {
                                NamespaceId::parse(ns).is_ok_and(|id| visible.contains(&id))
                            })
                            .collect()
                    };
                    if allowed.is_empty() {
                        return Ok(Response::new(pb::SearchEntitiesResponse {
                            results: Vec::new(),
                        }));
                    }
                    allowed
                }
            }
        };

        // Branch writes are not indexed (see branching.md). When a branch
        // header is present, search an index over the merged view instead of
        // the baseline tantivy index. That index is cached per branch and
        // keyed by a hash of the view it was built from, so a branch that
        // has not moved is searched, not re-indexed. Hash, build, and query
        // all run off the async runtime so a large snapshot cannot stall
        // other RPCs (and so a tantivy panic surfaces as Internal via
        // JoinError, matching the baseline path).
        let hits = if let Some(branch_name) = branch.clone() {
            let all = self.snapshot_for(Some(branch_name.as_str()), &vis).await?;
            let query = query.clone();
            let kind_labels = kind_labels.clone();
            let namespaces = namespaces.clone();
            let cache = self.branch_search_cache.clone();
            tokio::task::spawn_blocking(move || -> Result<Vec<_>, Status> {
                let _span = tracing::info_span!("search_query_branch").entered();
                let version = crate::search::ViewVersion::of(all.as_ref());
                let index = if let Some(cached) = cache.get(&branch_name, &version) {
                    crate::telemetry::record_branch_search_index_hit();
                    cached
                } else {
                    crate::telemetry::record_branch_search_index_miss();
                    let built = Arc::new(crate::search::SearchIndex::new().map_err(|e| {
                        Status::internal(format!("branch search index init failed: {e}"))
                    })?);
                    built.replace_all(all.as_ref()).map_err(|e| {
                        Status::internal(format!("branch search index build failed: {e}"))
                    })?;
                    cache.insert(branch_name, version, built.clone());
                    built
                };
                index
                    .search(&query, &kind_labels, &namespaces, limit)
                    .map_err(|e| match e {
                        tantivy::TantivyError::InvalidArgument(msg) => {
                            Status::invalid_argument(msg)
                        }
                        other => Status::internal(format!("search failed: {other}")),
                    })
            })
            .await
            .map_err(|e| Status::internal(format!("branch search task panicked: {e}")))??
        } else if let Some(owner) = vis.owner() {
            // `self.search_index` spans every namespace, so its BM25
            // statistics are shaped by every tenant's content. Searching an
            // index built fresh from this caller's own (already cached)
            // `snapshot` instead keeps scoring confined to what it can see,
            // the same way the branch path above is confined to one branch's
            // merged view.
            let all = self.snapshot(&vis).await?;
            let owner_key = owner.as_str().to_string();
            let query = query.clone();
            let kind_labels = kind_labels.clone();
            let namespaces = namespaces.clone();
            let cache = self.owner_search_cache.clone();
            tokio::task::spawn_blocking(move || -> Result<Vec<_>, Status> {
                let _span = tracing::info_span!("search_query_owner").entered();
                let version = crate::search::ViewVersion::of(all.as_ref());
                let index = if let Some(cached) = cache.get(&owner_key, &version) {
                    cached
                } else {
                    let built = Arc::new(crate::search::SearchIndex::new().map_err(|e| {
                        Status::internal(format!("owner search index init failed: {e}"))
                    })?);
                    built.replace_all(all.as_ref()).map_err(|e| {
                        Status::internal(format!("owner search index build failed: {e}"))
                    })?;
                    cache.insert(owner_key, version, built.clone());
                    built
                };
                index
                    .search(&query, &kind_labels, &namespaces, limit)
                    .map_err(|e| match e {
                        tantivy::TantivyError::InvalidArgument(msg) => {
                            Status::invalid_argument(msg)
                        }
                        other => Status::internal(format!("search failed: {other}")),
                    })
            })
            .await
            .map_err(|e| Status::internal(format!("owner search task panicked: {e}")))??
        } else {
            self.ensure_search_index().await?;
            let search_index = self.search_index.clone();
            tokio::task::spawn_blocking(move || {
                let _span = tracing::info_span!("search_query").entered();
                search_index.search(&query, &kind_labels, &namespaces, limit)
            })
            .await
            .map_err(|e| Status::internal(format!("search task panicked: {e}")))?
            .map_err(|e| match e {
                tantivy::TantivyError::InvalidArgument(msg) => Status::invalid_argument(msg),
                other => Status::internal(format!("search failed: {other}")),
            })?
        };
        let results = hits
            .into_iter()
            .map(|(entity, score, excerpt)| pb::SearchResult {
                entity: Some(entity),
                score,
                excerpt,
            })
            .collect();
        Ok(Response::new(pb::SearchEntitiesResponse { results }))
    }

    async fn list_versions(
        &self,
        req: Request<pb::ListVersionsRequest>,
    ) -> Result<Response<pb::ListVersionsResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let kind = kind_from_i32(req.kind, "kind")?;
        if req.slug.is_empty() {
            return Err(Status::invalid_argument("slug is required"));
        }
        if !req.namespace.is_empty() {
            crate::conv::validate_id_component("namespace", &req.namespace)?;
        }
        crate::conv::validate_id_component("slug", &req.slug)?;
        let page_size = clamp_page_size(
            req.page_size,
            ADVERTISED_DEFAULT_PAGE_SIZE,
            ADVERTISED_MAX_PAGE_SIZE,
        );
        let offset = parse_page_token(&req.page_token)?;
        let all = self.snapshot_for(branch.as_deref(), &vis).await?;
        let mut all_versions: Vec<pb::Entity> = all
            .iter()
            .filter_map(|s| {
                let Ok(k) = entity_kind(&s.entity) else {
                    return None;
                };
                if k != kind {
                    return None;
                }
                let Ok(id) = entity_id(&s.entity) else {
                    return None;
                };
                if !req.namespace.is_empty() && id.namespace != req.namespace {
                    return None;
                }
                if id.slug != req.slug {
                    return None;
                }
                Some(s.entity.clone())
            })
            .collect();
        all_versions.sort_by_key(|e| entity_id(e).map_or(0, |id| id.version));
        let total = all_versions.len();
        let versions: Vec<pb::Entity> = all_versions
            .into_iter()
            .skip(offset)
            .take(page_size)
            .collect();
        let next_offset = offset + versions.len();
        let next_page_token = if next_offset >= total {
            String::new()
        } else {
            encode_page_token(next_offset)
        };
        Ok(Response::new(pb::ListVersionsResponse {
            versions,
            next_page_token,
        }))
    }

    async fn get_latest_version(
        &self,
        req: Request<pb::GetLatestVersionRequest>,
    ) -> Result<Response<pb::GetLatestVersionResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let kind = kind_from_i32(req.kind, "kind")?;
        if req.slug.is_empty() {
            return Err(Status::invalid_argument("slug is required"));
        }
        if !req.namespace.is_empty() {
            crate::conv::validate_id_component("namespace", &req.namespace)?;
        }
        crate::conv::validate_id_component("slug", &req.slug)?;
        let lens = self.lens(&vis).await?;
        let entity = get_latest_version_with_cap(
            &self.store,
            &lens,
            kind,
            &req.namespace,
            &req.slug,
            MAX_PROJECTION_ENTITIES,
            branch.as_deref(),
        )
        .await?;
        Ok(Response::new(pb::GetLatestVersionResponse { entity }))
    }

    async fn get_supersession_chain(
        &self,
        req: Request<pb::GetSupersessionChainRequest>,
    ) -> Result<Response<pb::GetSupersessionChainResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let kind = kind_from_i32(req.kind, "kind")?;
        let id = require_id(req.id.as_ref(), "")?.clone();
        let direction = pb::get_supersession_chain_request::Direction::try_from(req.direction)
            .unwrap_or(pb::get_supersession_chain_request::Direction::Both);
        let page_size = clamp_page_size(
            req.page_size,
            ADVERTISED_DEFAULT_PAGE_SIZE,
            ADVERTISED_MAX_PAGE_SIZE,
        );
        let offset = parse_page_token(&req.page_token)?;
        let all = self.snapshot_for(branch.as_deref(), &vis).await?;

        let mut ancestors = graph::supersession_history(&all, kind, &id); // newest..oldest, incl. self
        let descendants = graph::supersession_descendants(&all, kind, &id);

        let want_anc = matches!(
            direction,
            pb::get_supersession_chain_request::Direction::Ancestors
                | pb::get_supersession_chain_request::Direction::Both
                | pb::get_supersession_chain_request::Direction::Unspecified
        );
        let want_desc = matches!(
            direction,
            pb::get_supersession_chain_request::Direction::Descendants
                | pb::get_supersession_chain_request::Direction::Both
                | pb::get_supersession_chain_request::Direction::Unspecified
        );

        let mut refs: Vec<pb::EntityRef> = Vec::new();
        if want_anc {
            ancestors.reverse(); // now oldest..self
            refs.extend(ancestors);
        } else {
            refs.push(pb::EntityRef {
                kind: kind as i32,
                id: Some(id.clone()),
            });
        }
        if want_desc {
            refs.extend(descendants);
        }

        let all_chain = graph::resolve_entities(&all, &refs);
        let total = all_chain.len();
        let chain: Vec<pb::Entity> = all_chain.into_iter().skip(offset).take(page_size).collect();
        let next_offset = offset + chain.len();
        let next_page_token = if next_offset >= total {
            String::new()
        } else {
            encode_page_token(next_offset)
        };
        Ok(Response::new(pb::GetSupersessionChainResponse {
            chain,
            next_page_token,
        }))
    }

    async fn get_incoming_references(
        &self,
        req: Request<pb::GetReferencesRequest>,
    ) -> Result<Response<pb::GetReferencesResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let kind = kind_from_i32(req.kind, "kind")?;
        let id = require_id(req.id.as_ref(), "")?.clone();
        let mut filter_kinds: Vec<pb::EntityKind> = Vec::with_capacity(req.filter_kinds.len());
        for k in &req.filter_kinds {
            filter_kinds.push(kind_from_i32(*k, "filter_kinds[]")?);
        }
        let page_size = clamp_page_size(
            req.page_size,
            ADVERTISED_DEFAULT_PAGE_SIZE,
            ADVERTISED_MAX_PAGE_SIZE,
        );
        let offset = parse_page_token(&req.page_token)?;
        let index = self.reverse_index_for(branch.as_deref(), &vis).await?;
        let refs = index.incoming(kind, &id).to_vec();
        // Incoming refs have the queried entity in `to`; the referrer kind
        // lives in `from`.
        let all_references = graph::filter_refs_by_kinds(refs, &filter_kinds, graph::RefSide::From);
        let total = all_references.len();
        let references: Vec<pb::Reference> = all_references
            .into_iter()
            .skip(offset)
            .take(page_size)
            .collect();
        let next_offset = offset + references.len();
        let next_page_token = if next_offset >= total {
            String::new()
        } else {
            encode_page_token(next_offset)
        };
        Ok(Response::new(pb::GetReferencesResponse {
            references,
            next_page_token,
        }))
    }

    async fn get_outgoing_references(
        &self,
        req: Request<pb::GetReferencesRequest>,
    ) -> Result<Response<pb::GetReferencesResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let kind = kind_from_i32(req.kind, "kind")?;
        let id = require_id(req.id.as_ref(), "")?;
        // The root is fetched by key, so the lensed snapshot below
        // narrows the neighbourhood but never the root itself.
        self.require_visible(&vis, &id.namespace).await?;
        let mut filter_kinds: Vec<pb::EntityKind> = Vec::with_capacity(req.filter_kinds.len());
        for k in &req.filter_kinds {
            filter_kinds.push(kind_from_i32(*k, "filter_kinds[]")?);
        }
        let page_size = clamp_page_size(
            req.page_size,
            ADVERTISED_DEFAULT_PAGE_SIZE,
            ADVERTISED_MAX_PAGE_SIZE,
        );
        let offset = parse_page_token(&req.page_token)?;
        let stored = self
            .store
            .get(kind, id, branch.as_deref())
            .await
            .map_err(store_err)?;
        let snap = self.snapshot_for(branch.as_deref(), &vis).await?;
        let known: std::collections::HashSet<_> = snap
            .iter()
            .filter_map(|se| {
                let k = trogon_atlas_store::refs::entity_kind(&se.entity)?;
                let i = trogon_atlas_store::refs::entity_id(&se.entity)?;
                Some(pb::EntityKey::new(k, i))
            })
            .collect();
        let mut refs = graph::outgoing_of_resolved(&stored.entity, &known);
        refs.extend(graph::TypeOwners::build(&snap).schema_ref(&stored.entity));
        // Outgoing refs have the queried entity in `from`; the target kind
        // lives in `to`.
        let all_references = graph::filter_refs_by_kinds(refs, &filter_kinds, graph::RefSide::To);
        let total = all_references.len();
        let references: Vec<pb::Reference> = all_references
            .into_iter()
            .skip(offset)
            .take(page_size)
            .collect();
        let next_offset = offset + references.len();
        let next_page_token = if next_offset >= total {
            String::new()
        } else {
            encode_page_token(next_offset)
        };
        Ok(Response::new(pb::GetReferencesResponse {
            references,
            next_page_token,
        }))
    }

    async fn get_impact(
        &self,
        req: Request<pb::GetImpactRequest>,
    ) -> Result<Response<pb::GetImpactResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let root = req
            .root
            .ok_or_else(|| Status::invalid_argument("root is required"))?;
        let kind = kind_from_i32(root.kind, "root.kind")?;
        let id = require_id(root.id.as_ref(), "root")?.clone();
        let max_depth = if req.max_depth == 0 {
            ADVERTISED_MAX_IMPACT_DEPTH
        } else {
            req.max_depth.min(ADVERTISED_MAX_IMPACT_DEPTH)
        };
        let page_size = clamp_page_size(
            req.page_size,
            ADVERTISED_DEFAULT_PAGE_SIZE,
            ADVERTISED_MAX_PAGE_SIZE,
        );
        let offset = parse_page_token(&req.page_token)?;

        let all = self.snapshot_for(branch.as_deref(), &vis).await?;
        let mut all_nodes = graph::impact(&all, kind, &id, max_depth, true);

        if !req.filter_kinds.is_empty() {
            let allowed: std::collections::HashSet<i32> =
                req.filter_kinds.iter().copied().collect();
            all_nodes.retain(|n| {
                n.depth == 0 || n.entity.as_ref().is_some_and(|e| allowed.contains(&e.kind))
            });
        }

        let total = all_nodes.len();
        let nodes: Vec<pb::ImpactNode> =
            all_nodes.into_iter().skip(offset).take(page_size).collect();
        let next_offset = offset + nodes.len();
        let next_page_token = if next_offset >= total {
            String::new()
        } else {
            encode_page_token(next_offset)
        };
        Ok(Response::new(pb::GetImpactResponse {
            nodes,
            next_page_token,
        }))
    }

    async fn retarget_references(
        &self,
        req: Request<pb::RetargetReferencesRequest>,
    ) -> Result<Response<pb::RetargetReferencesResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let author = Self::author_from_metadata(&req);
        // Under a branch header the sweep runs over the merged view and every
        // rewrite lands as a branch delta, so a version migration -- mint
        // `@2`, retarget every referrer, drain the stale references -- is
        // stageable as one reviewable change set. That is the change set
        // branches exist for.
        //
        // A referrer visible only on baseline is still a referrer this branch
        // sees, so rewriting it copies it into the branch (copy-on-write, the
        // same thing an ordinary branch put does). Skipping those instead
        // would leave the branch's own merged view holding references to the
        // entity being migrated away from, which is the state the migration
        // exists to remove.
        let branch = Self::branch_from_metadata(&req)?;
        // Every referrer of `from` is rewritten, wherever it lives, and which
        // entities those are is only known after scanning the reverse index.
        // `Unbounded` is the honest description: a namespace-scoped principal
        // cannot use this RPC at all. Under baseline protection it requires
        // Admin unless it carries a branch, which is the ordinary way in.
        self.authorize_write(
            Self::principal_from_metadata(&req).as_ref(),
            branch.as_deref(),
            &WriteTargets::Unbounded,
        )
        .await?;
        let req = req.into_inner();
        if let Some(name) = branch.as_deref() {
            self.require_branch(name).await?;
        }

        let from = req
            .from
            .ok_or_else(|| Status::invalid_argument("from is required"))?;
        let to = req
            .to
            .ok_or_else(|| Status::invalid_argument("to is required"))?;

        let from_kind = kind_from_i32(from.kind, "from.kind")?;
        let to_kind = kind_from_i32(to.kind, "to.kind")?;

        if from_kind != to_kind {
            return Err(Status::invalid_argument(
                "from and to must be the same EntityKind",
            ));
        }

        let from_id = require_id(from.id.as_ref(), "from")?.clone();
        let to_id = require_id(to.id.as_ref(), "to")?.clone();

        let mut filter_kinds: Vec<pb::EntityKind> = Vec::with_capacity(req.filter_kinds.len());
        for k in &req.filter_kinds {
            filter_kinds.push(kind_from_i32(*k, "filter_kinds[]")?);
        }

        // Verify `to` exists before touching any referrer. NOT_FOUND here
        // means the caller is pointing to a nonexistent entity. Resolved
        // through the branch, since the migration target is usually minted on
        // the branch and does not exist on baseline yet.
        self.store
            .get(to_kind, &to_id, branch.as_deref())
            .await
            .map_err(store_err)?;

        // Acquire the mutation lock; concurrent batches must not interleave.
        let _guard = self.lock_mutations().await;
        if branch.is_none() {
            self.invalidate_snapshot().await;
        }

        let index = self.reverse_index_for(branch.as_deref(), &vis).await?;
        let referrers: Vec<pb::Reference> = index.incoming(from_kind, &from_id).to_vec();

        let referrers: Vec<pb::Reference> = if filter_kinds.is_empty() {
            referrers
        } else {
            referrers
                .into_iter()
                .filter(|r| {
                    r.from
                        .as_ref()
                        .and_then(|f| pb::EntityKind::try_from(f.kind).ok())
                        .is_some_and(|k| filter_kinds.contains(&k))
                })
                .collect()
        };

        // Load every referrer, apply the in-memory rewrite, and stage the
        // rewritten entity for a batch write. Per-referrer load errors are
        // captured in results and do not abort the rest of the referrers.
        let mut results: Vec<pb::retarget_references_response::Result> =
            Vec::with_capacity(referrers.len());
        let mut staged: Vec<(pb::EntityKind, pb::Entity)> = Vec::new();
        let mut load_fail_count: u32 = 0;

        for referrer_ref in &referrers {
            let referrer_from = referrer_ref
                .from
                .as_ref()
                .ok_or_else(|| Status::internal("reverse index entry missing from"))?;
            let r_kind = kind_from_i32(referrer_from.kind, "referrer.kind")?;
            let r_id = require_id(referrer_from.id.as_ref(), "referrer")?.clone();

            let stored = match self.store.get(r_kind, &r_id, branch.as_deref()).await {
                Ok(s) => s,
                Err(e) => {
                    results.push(pb::retarget_references_response::Result {
                        referrer: Some(referrer_from.clone()),
                        fields: Vec::new(),
                        error: format!("load failed: {e}"),
                    });
                    load_fail_count += 1;
                    continue;
                }
            };

            let mut entity = stored.entity.clone();
            let fields =
                trogon_atlas_core::refs::retarget_refs(&mut entity, from_kind, &from_id, &to_id);

            results.push(pb::retarget_references_response::Result {
                referrer: Some(referrer_from.clone()),
                fields: fields.clone(),
                error: String::new(),
            });

            if !fields.is_empty() {
                staged.push((r_kind, entity));
            }
        }

        let staged_count = u32::try_from(staged.len()).unwrap_or(u32::MAX);

        if req.dry_run || staged.is_empty() {
            return Ok(Response::new(pb::RetargetReferencesResponse {
                rewritten: staged_count,
                failed: load_fail_count,
                results,
            }));
        }

        // Apply staged writes through the existing batch machinery.
        use trogon_atlas_store::store::MutationOp;
        let ops: Vec<MutationOp> = staged
            .iter()
            .map(|(kind, entity)| MutationOp::Put {
                kind: *kind,
                entity: entity.clone(),
                if_match: None,
                force: false,
            })
            .collect();

        let metas: Vec<OpMetaLike> = staged
            .iter()
            .map(|(kind, entity)| {
                let id = entity_id(entity).cloned().unwrap_or_default();
                OpMetaLike {
                    kind: *kind,
                    id,
                    is_put: true,
                    entity: Some(entity.clone()),
                }
            })
            .collect();

        let mut changeset =
            PendingChangeset::mint("RetargetReferences", &author, branch.as_deref());
        match self.store.batch_apply(&ops, changeset.write_ctx()).await {
            Ok(outcomes) => {
                changeset.record_batch(&metas, &outcomes);
                self.record_changeset(changeset, git_mirror::batch_commit_message(outcomes.len()))
                    .await;
                // A branch rewrite touched no baseline row and no baseline
                // cache, so it skips every baseline side effect (git mirror,
                // search index, snapshot invalidation) exactly as an ordinary
                // branch put does.
                if branch.is_none() {
                    self.apply_batch_side_effects(&metas, &outcomes, author)
                        .await;
                    self.invalidate_snapshot().await;
                }
                Ok(Response::new(pb::RetargetReferencesResponse {
                    rewritten: u32::try_from(outcomes.len()).unwrap_or(u32::MAX),
                    failed: load_fail_count,
                    results,
                }))
            }
            Err(e) => {
                tracing::warn!(error = %e, "retarget_references batch_apply failed");
                for r in &mut results {
                    if r.error.is_empty() && !r.fields.is_empty() {
                        r.error = format!("write failed: {e}");
                    }
                }
                Ok(Response::new(pb::RetargetReferencesResponse {
                    rewritten: 0,
                    failed: load_fail_count + staged_count,
                    results,
                }))
            }
        }
    }

    async fn get_slice_projection(
        &self,
        req: Request<pb::GetSliceProjectionRequest>,
    ) -> Result<Response<pb::GetSliceProjectionResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let kind = kind_from_i32(req.kind, "kind")?;
        if !matches!(
            kind,
            pb::EntityKind::CommandSlice
                | pb::EntityKind::ReadModelSlice
                | pb::EntityKind::AutomationSlice
                | pb::EntityKind::UiSlice
        ) {
            return Err(Status::invalid_argument("kind must be a *_SLICE kind"));
        }
        let id = require_id(req.id.as_ref(), "")?;
        // The root is fetched by key, so the lensed snapshot below
        // narrows the neighbourhood but never the root itself.
        self.require_visible(&vis, &id.namespace).await?;
        let stored = self
            .store
            .get(kind, id, branch.as_deref())
            .await
            .map_err(store_err)?;
        let slice = slice_from_entity(&stored.entity)
            .ok_or_else(|| Status::internal("slice entity kind mismatch"))?;

        let root = pb::EntityRef {
            kind: kind as i32,
            id: Some(id.clone()),
        };
        let all = self.snapshot_for(branch.as_deref(), &vis).await?;
        let graph::Closure {
            mut entities,
            truncated,
        } = graph::closure(&all, &[root], MAX_PROJECTION_ENTITIES);
        // The closure includes the slice itself; the projection ships the
        // slice separately on `slice`.
        entities.remove(&trogon_atlas_proto::canonical::id_string(kind, id));

        Ok(Response::new(pb::GetSliceProjectionResponse {
            projection: Some(pb::SliceProjection {
                slice: Some(slice),
                entities,
            }),
            truncated,
        }))
    }

    async fn get_storyboard_projection(
        &self,
        req: Request<pb::GetStoryboardProjectionRequest>,
    ) -> Result<Response<pb::GetStoryboardProjectionResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let id = require_id(req.id.as_ref(), "")?;
        // The root is fetched by key, so the lensed snapshot below
        // narrows the neighbourhood but never the root itself.
        self.require_visible(&vis, &id.namespace).await?;
        let stored = self
            .store
            .get(pb::EntityKind::Storyboard, id, branch.as_deref())
            .await
            .map_err(store_err)?;
        let Some(pb::entity::Kind::Storyboard(storyboard)) = stored.entity.kind.clone() else {
            return Err(Status::internal("storyboard entity kind mismatch"));
        };

        let all = self.snapshot_for(branch.as_deref(), &vis).await?;
        let by_key = graph::entity_lookup(&all);

        // Inline slices that the storyboard names.
        let mut slices: Vec<pb::Slice> = Vec::new();
        for sr in &storyboard.slices {
            let Some(sid) = sr.id.as_ref() else { continue };
            for k in [
                pb::EntityKind::CommandSlice,
                pb::EntityKind::ReadModelSlice,
                pb::EntityKind::AutomationSlice,
                pb::EntityKind::UiSlice,
            ] {
                let key = pb::EntityKey::new(k, sid);
                if let Some(ent) = by_key.get(&key) {
                    if let Some(s) = slice_from_entity(ent) {
                        slices.push(s);
                        break;
                    }
                }
            }
        }

        let root = pb::EntityRef {
            kind: pb::EntityKind::Storyboard as i32,
            id: Some(id.clone()),
        };
        let graph::Closure {
            mut entities,
            truncated,
        } = graph::closure(&all, &[root], MAX_PROJECTION_ENTITIES);
        entities.remove(&trogon_atlas_proto::canonical::id_string(
            pb::EntityKind::Storyboard,
            id,
        ));

        Ok(Response::new(pb::GetStoryboardProjectionResponse {
            projection: Some(pb::StoryboardProjection {
                storyboard: Some(storyboard),
                slices,
                entities,
            }),
            truncated,
        }))
    }

    async fn get_event_model_projection(
        &self,
        req: Request<pb::GetEventModelProjectionRequest>,
    ) -> Result<Response<pb::GetEventModelProjectionResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let id = require_id(req.id.as_ref(), "")?;
        // The root is fetched by key, so the lensed snapshot below
        // narrows the neighbourhood but never the root itself.
        self.require_visible(&vis, &id.namespace).await?;
        let stored = self
            .store
            .get(pb::EntityKind::EventModel, id, branch.as_deref())
            .await
            .map_err(store_err)?;
        let Some(pb::entity::Kind::EventModel(em)) = stored.entity.kind.clone() else {
            return Err(Status::internal("event_model entity kind mismatch"));
        };
        let all = self.snapshot_for(branch.as_deref(), &vis).await?;
        let root_refs: Vec<pb::EntityRef> = em.members.clone();
        let graph::Closure {
            mut entities,
            truncated,
        } = graph::closure(&all, &root_refs, MAX_PROJECTION_ENTITIES);
        if !req.include_cross_model {
            let own_ns = id.namespace.clone();
            entities.retain(|_, e| {
                trogon_atlas_store::refs::entity_id(e).is_some_and(|i| i.namespace == own_ns)
            });
        }
        Ok(Response::new(pb::GetEventModelProjectionResponse {
            projection: Some(pb::EventModelProjection {
                event_model: Some(em),
                entities,
            }),
            truncated,
        }))
    }

    async fn extract_subgraph(
        &self,
        req: Request<pb::ExtractSubgraphRequest>,
    ) -> Result<Response<pb::ExtractSubgraphResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let scope = req
            .scope
            .ok_or_else(|| Status::invalid_argument("scope is required"))?;
        let Some(
            pb::analysis_scope::Scope::Slice(root_ref)
            | pb::analysis_scope::Scope::Storyboard(root_ref)
            | pb::analysis_scope::Scope::EventModel(root_ref),
        ) = scope.scope
        else {
            return Err(Status::invalid_argument("scope.scope is required"));
        };
        let _ = kind_from_i32(root_ref.kind, "scope")?;
        let _ = require_id(root_ref.id.as_ref(), "scope")?;

        let all = self.snapshot_for(branch.as_deref(), &vis).await?;
        let graph::Closure {
            entities,
            truncated,
        } = graph::closure(
            &all,
            std::slice::from_ref(&root_ref),
            MAX_PROJECTION_ENTITIES,
        );
        let own_ns = root_ref
            .id
            .as_ref()
            .map(|i| i.namespace.clone())
            .unwrap_or_default();

        let filtered: std::collections::HashMap<String, pb::Entity> = if req.include_cross_model {
            entities
        } else {
            entities
                .into_iter()
                .filter(|(_, e)| {
                    trogon_atlas_store::refs::entity_id(e).is_some_and(|i| i.namespace == own_ns)
                })
                .collect()
        };

        let new_id = req.new_event_model_id.unwrap_or_else(|| pb::Id {
            namespace: own_ns.clone(),
            slug: format!(
                "extracted-{}",
                root_ref
                    .id
                    .as_ref()
                    .map_or_else(|| "subgraph".into(), |i| i.slug.clone())
            ),
            version: 1,
        });

        let members: Vec<pb::EntityRef> = filtered
            .values()
            .filter_map(|e| {
                let k = trogon_atlas_store::refs::entity_kind(e)?;
                let i = trogon_atlas_store::refs::entity_id(e)?.clone();
                Some(pb::EntityRef {
                    kind: k as i32,
                    id: Some(i),
                })
            })
            .collect();

        Ok(Response::new(pb::ExtractSubgraphResponse {
            event_model: Some(pb::EventModel {
                id: Some(new_id),
                title: "Extracted subgraph".into(),
                doc: String::new(),
                members,
                metadata: Vec::new(),
                supersedes: None,
            }),
            entities: filtered,
            truncated,
        }))
    }

    async fn diff_entities(
        &self,
        req: Request<pb::DiffEntitiesRequest>,
    ) -> Result<Response<pb::DiffEntitiesResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let a = req
            .a
            .ok_or_else(|| Status::invalid_argument("a is required"))?;
        let b = req
            .b
            .ok_or_else(|| Status::invalid_argument("b is required"))?;
        let a_kind = kind_from_i32(a.kind, "a.kind")?;
        let b_kind = kind_from_i32(b.kind, "b.kind")?;
        let a_id = require_id(a.id.as_ref(), "a")?;
        let b_id = require_id(b.id.as_ref(), "b")?;
        // Two direct-by-key reads wearing a diff for a hat.
        self.require_visible(&vis, &a_id.namespace).await?;
        self.require_visible(&vis, &b_id.namespace).await?;
        let a_stored = self
            .store
            .get(a_kind, a_id, branch.as_deref())
            .await
            .map_err(store_err)?;
        let b_stored = self
            .store
            .get(b_kind, b_id, branch.as_deref())
            .await
            .map_err(store_err)?;
        let ops = diff::diff_entities(&a_stored.entity, &b_stored.entity);
        Ok(Response::new(pb::DiffEntitiesResponse { ops }))
    }

    async fn put_entity(
        &self,
        req: Request<pb::PutEntityRequest>,
    ) -> Result<Response<pb::PutEntityResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let author = Self::author_from_metadata(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();
        operation_receipts::reject_operation_id_with_dry_run(&req.operation_id, req.validate_only)?;
        let entity = req
            .entity
            .ok_or_else(|| Status::invalid_argument("entity is required"))?;
        let kind = entity_kind(&entity)?;
        let id = entity_id(&entity)?.clone();
        self.authorize_write(
            principal.as_ref(),
            branch.as_deref(),
            &WriteTargets::of(&id.namespace),
        )
        .await?;
        if let Some(name) = branch.as_deref() {
            self.require_branch(name).await?;
        }
        validate_entity_content(&entity)?;
        check_project_namespace_conflict(&entity)?;
        let type_library = self
            .type_libraries
            .admit(&*self.store, &entity, branch.as_deref())
            .await?;
        let schema_types = self
            .type_libraries
            .check_schema(&*self.store, &entity, &[], branch.as_deref())
            .await?;

        let scenario_issues = if matches!(
            kind,
            pb::EntityKind::CommandSlice
                | pb::EntityKind::ReadModelSlice
                | pb::EntityKind::AutomationSlice
        ) {
            let all = self.snapshot_for(branch.as_deref(), &vis).await?;
            validation::validate_scenarios(&entity, &all)
        } else {
            Vec::new()
        };
        if scenario_issues
            .iter()
            .any(|i| i.severity == pb::validation_issue::Severity::Error as i32)
        {
            return Err(Status::invalid_argument(format_scenario_issues(
                &scenario_issues,
            )));
        }

        if req.validate_only {
            return Ok(Response::new(pb::PutEntityResponse {
                entity: Some(entity),
                etag: String::new(),
                validation: scenario_issues,
                no_op: false,
                operation_receipt: None,
            }));
        }

        let _guard = self.lock_mutations().await;
        for admission in type_library.iter().chain(&schema_types) {
            self.type_libraries
                .confirm(&*self.store, admission, branch.as_deref())
                .await?;
        }

        let attempt = if req.operation_id.is_empty() {
            None
        } else {
            let principal_name = principal
                .as_ref()
                .map_or(UNAUTHENTICATED_PRINCIPAL, |p| p.name.as_ref());
            let digest_req = pb::PutEntityRequest {
                entity: Some(entity.clone()),
                create_only: req.create_only,
                if_match: req.if_match.clone(),
                validate_only: false,
                force: req.force,
                operation_id: String::new(),
            };
            Some(operation_receipts::OperationAttempt::new(
                principal_name,
                &req.operation_id,
                "PutEntity",
                branch.as_deref(),
                "trogonatlas.api.eventmodel.v1alpha1.PutEntityRequest",
                &digest_req,
            )?)
        };

        let claim_key = match &attempt {
            None => None,
            Some(attempt) => {
                match operation_receipts::claim(
                    &*self.store,
                    attempt,
                    "PutEntity",
                    branch.as_deref(),
                )
                .await?
                {
                    operation_receipts::Claim::Proceed => Some(attempt.claim_key.clone()),
                    operation_receipts::Claim::Replay(record) => {
                        if record.status == OperationStatus::Rejected {
                            return Err(operation_receipts::replay_rejected_status(&record));
                        }
                        // Applied: the replay reconstructs the current state
                        // rather than replaying a byte-identical response --
                        // the entity may have moved on since the original
                        // call, and GetEntity-shaped data is the only honest
                        // answer to "what does it look like now".
                        let current = self
                            .store
                            .get(kind, &id, branch.as_deref())
                            .await
                            .map_err(store_err)?;
                        return Ok(Response::new(pb::PutEntityResponse {
                            no_op: false,
                            etag: current.etag,
                            entity: Some(current.entity),
                            validation: Vec::new(),
                            operation_receipt: Some(operation_receipts::replay_receipt(
                                &req.operation_id,
                                &record,
                            )),
                        }));
                    }
                }
            }
        };

        let mut changeset = PendingChangeset::mint("PutEntity", &author, branch.as_deref())
            .with_operation_id(claim_key.as_deref().map(|_| req.operation_id.as_str()))
            .with_operation_key(claim_key.as_deref());
        let create_only = req.create_only;
        let write_result = if create_only {
            self.store
                .create(kind, &entity, changeset.write_ctx())
                .await
        } else if !req.if_match.is_empty() {
            self.store
                .update(
                    kind,
                    &entity,
                    &req.if_match,
                    req.force,
                    changeset.write_ctx(),
                )
                .await
        } else {
            self.store
                .put(kind, &entity, req.force, changeset.write_ctx())
                .await
        };
        let written = match write_result {
            Ok(w) => w,
            Err(e) => {
                let status = store_err(e);
                if let Some(key) = &claim_key {
                    operation_receipts::settle_for_status(&*self.store, key, &status).await;
                }
                return Err(status);
            }
        };
        // A no-op write (e.g. `put` with content identical to what is
        // already stored) never gets recorded as a changeset, so a receipt
        // for it must not point at a changeset id nothing ever persisted.
        let receipt_changeset_id = if written.wrote {
            changeset.id.clone()
        } else {
            String::new()
        };
        if let Some(key) = &claim_key {
            operation_receipts::settle_applied(&*self.store, key, &receipt_changeset_id).await;
        }

        // A skipped write changed nothing: no side effects, no mirror
        // commit. When it did write, every side effect sees the entity AS
        // STORED (system stamp included) so the git mirror stays
        // byte-faithful to the store.
        //
        // Branch writes (branch.is_some()) never reach here with side
        // effects: they never touch the search index, the git mirror, or
        // the baseline snapshot cache. A branch is a small, short-lived
        // server-side overlay (see docs/explanation/branching.md); none of
        // those baseline-only projections are meant to observe it in
        // Phase 1.
        let verb = if create_only { "create" } else { "update" };
        // A branch write gets a changeset too. It never reaches the change
        // feed, the search index, or the mirror, but "who changed what on
        // this branch" is exactly the question the changeset log answers,
        // and the record's `branch` field keeps it distinguishable.
        if written.wrote {
            changeset.record_put(kind, &id);
            self.record_changeset(
                changeset,
                git_mirror::commit_message(kind, &id.namespace, &id.slug, id.version, verb),
            )
            .await;
        }

        if written.wrote && branch.is_none() {
            self.search_apply_put(&written.entity);
            self.invalidate_snapshot().await;

            self.mirror_apply(MirrorBatch {
                changes: vec![MirrorChange::Put(written.entity.clone())],
                author,
                message: git_mirror::commit_message(
                    kind,
                    &id.namespace,
                    &id.slug,
                    id.version,
                    verb,
                ),
            })
            .await;
        }

        Ok(Response::new(pb::PutEntityResponse {
            no_op: !written.wrote,
            etag: written.etag,
            entity: Some(written.entity),
            validation: scenario_issues,
            operation_receipt: claim_key.as_ref().map(|_| {
                operation_receipts::fresh_receipt(&req.operation_id, &receipt_changeset_id)
            }),
        }))
    }

    async fn delete_entity(
        &self,
        req: Request<pb::DeleteEntityRequest>,
    ) -> Result<Response<pb::DeleteEntityResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let author = Self::author_from_metadata(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();
        let kind = kind_from_i32(req.kind, "kind")?;
        let id = require_id(req.id.as_ref(), "")?.clone();
        self.authorize_write(
            principal.as_ref(),
            branch.as_deref(),
            &WriteTargets::of(&id.namespace),
        )
        .await?;
        if let Some(name) = branch.as_deref() {
            self.require_branch(name).await?;
        }
        let mode = pb::delete_entity_request::Mode::try_from(req.mode)
            .unwrap_or(pb::delete_entity_request::Mode::FailIfReferenced);
        let mode = if matches!(mode, pb::delete_entity_request::Mode::Unspecified) {
            pb::delete_entity_request::Mode::FailIfReferenced
        } else {
            mode
        };
        operation_receipts::reject_operation_id_with_dry_run(
            &req.operation_id,
            matches!(mode, pb::delete_entity_request::Mode::DryRun),
        )?;

        let if_match = if req.if_match.is_empty() {
            None
        } else {
            Some(req.if_match.as_str())
        };

        let _guard = self.lock_mutations().await;
        // Drop any snapshot cached from before we acquired the mutation lock:
        // that snapshot may not reflect a mutation that landed elsewhere
        // (other process, or another tokio task that beat us into the
        // lock). FailIfReferenced is only safe if the referrer scan happens
        // against a post-lock view of the store. Branch deletes never
        // touch the baseline snapshot cache in the first place, so this is
        // a no-op for them.
        if branch.is_none() {
            self.invalidate_snapshot().await;
        }

        // Claimed before the existence check, not after: a successful
        // delete removes the very thing a retry would otherwise need to
        // confirm still exists, so checking existence first would turn
        // every replay of an already-applied delete into a spurious
        // NOT_FOUND instead of the receipt it is entitled to. Every
        // deterministic rejection below this point must settle the claim
        // it just consumed rather than leave it stuck Pending.
        let attempt = if req.operation_id.is_empty() {
            None
        } else {
            let principal_name = principal
                .as_ref()
                .map_or(UNAUTHENTICATED_PRINCIPAL, |p| p.name.as_ref());
            let digest_req = pb::DeleteEntityRequest {
                kind: req.kind,
                id: req.id.clone(),
                mode: req.mode,
                if_match: req.if_match.clone(),
                operation_id: String::new(),
            };
            Some(operation_receipts::OperationAttempt::new(
                principal_name,
                &req.operation_id,
                "DeleteEntity",
                branch.as_deref(),
                "trogonatlas.api.eventmodel.v1alpha1.DeleteEntityRequest",
                &digest_req,
            )?)
        };

        let claim_key = match &attempt {
            None => None,
            Some(attempt) => {
                match operation_receipts::claim(
                    &*self.store,
                    attempt,
                    "DeleteEntity",
                    branch.as_deref(),
                )
                .await?
                {
                    operation_receipts::Claim::Proceed => Some(attempt.claim_key.clone()),
                    operation_receipts::Claim::Replay(record) => {
                        if record.status == OperationStatus::Rejected {
                            return Err(operation_receipts::replay_rejected_status(&record));
                        }
                        // Applied: the write already landed, most likely
                        // removing the entity this call would otherwise
                        // scan referrers against, so there is nothing
                        // current to report; an empty list is the honest
                        // answer, not a stale one.
                        return Ok(Response::new(pb::DeleteEntityResponse {
                            referrers: Vec::new(),
                            operation_receipt: Some(operation_receipts::replay_receipt(
                                &req.operation_id,
                                &record,
                            )),
                        }));
                    }
                }
            }
        };

        // Confirm existence (and etag) up front so we can return NOT_FOUND
        // before considering referrers.
        let _stored = match self.store.get(kind, &id, branch.as_deref()).await {
            Ok(s) => s,
            Err(e) => {
                let status = store_err(e);
                if let Some(key) = &claim_key {
                    operation_receipts::settle_for_status(&*self.store, key, &status).await;
                }
                return Err(status);
            }
        };
        if let Some(exp) = if_match {
            if _stored.etag != exp {
                tracing::debug!(expected = %exp, found = %_stored.etag, "etag mismatch on delete_entity");
                let status = Status::aborted("etag mismatch");
                if let Some(key) = &claim_key {
                    operation_receipts::settle_for_status(&*self.store, key, &status).await;
                }
                return Err(status);
            }
        }

        if kind == pb::EntityKind::TypeLibrary
            && !matches!(mode, pb::delete_entity_request::Mode::DryRun)
        {
            let mut changes = crate::type_libraries::TypeLibraryChanges::default();
            changes.delete(kind, &id);
            if let Err(status) = self
                .type_libraries
                .verify(&*self.store, &changes, branch.as_deref())
                .await
            {
                if let Some(key) = &claim_key {
                    operation_receipts::settle_for_status(&*self.store, key, &status).await;
                }
                return Err(status);
            }
            let removal = [OpMetaLike {
                kind,
                id: id.clone(),
                is_put: false,
                entity: None,
            }];
            if let Err(status) = self
                .reject_type_library_users(branch.as_deref(), &vis, &removal)
                .await
            {
                if let Some(key) = &claim_key {
                    operation_receipts::settle_for_status(&*self.store, key, &status).await;
                }
                return Err(status);
            }
        }

        // Referrer scan must match the client's write scope: baseline for
        // baseline deletes, merged branch view for branch deletes. Using the
        // baseline-only index here would let a branch-only referrer silently
        // lose its target (GetIncomingReferences already uses the branch
        // index; deletes must agree).
        let index = self.reverse_index_for(branch.as_deref(), &vis).await?;
        let referrers: Vec<pb::Reference> = index.incoming(kind, &id).to_vec();

        // The gate below must see referrers in every tenant, not just this
        // caller's own, or a bound caller can delete an entity another
        // tenant still depends on and silently break that tenant's
        // reference. Scanning the unrestricted index for the existence
        // check alone -- never for the `referrers` field above, which stays
        // scoped to what this caller can see -- closes that hole without
        // telling a bound caller anything about another tenant's data.
        let is_referenced = if vis.is_unrestricted() {
            !referrers.is_empty()
        } else {
            let unrestricted = Visibility::unrestricted(vis.principal().into());
            let global_index = self
                .reverse_index_for(branch.as_deref(), &unrestricted)
                .await?;
            !global_index.incoming(kind, &id).is_empty()
        };

        match mode {
            pb::delete_entity_request::Mode::DryRun => {
                // operation_id and dry-run never coexist (rejected above),
                // so claim_key is always None here.
                return Ok(Response::new(pb::DeleteEntityResponse {
                    referrers,
                    operation_receipt: None,
                }));
            }
            pb::delete_entity_request::Mode::FailIfReferenced if is_referenced => {
                let status = Status::failed_precondition(
                    "entity is referenced; pass mode=FORCE or repair referrers first",
                );
                if let Some(key) = &claim_key {
                    operation_receipts::settle_for_status(&*self.store, key, &status).await;
                }
                return Err(status);
            }
            pb::delete_entity_request::Mode::Unspecified => {
                let status = Status::invalid_argument(
                    "delete mode must not be Unspecified (server defaults to FAIL_IF_REFERENCED)",
                );
                if let Some(key) = &claim_key {
                    operation_receipts::settle_for_status(&*self.store, key, &status).await;
                }
                return Err(status);
            }
            pb::delete_entity_request::Mode::FailIfReferenced
            | pb::delete_entity_request::Mode::Force => {}
        }

        // From here, mode is FailIfReferenced-with-no-referrers or Force:
        // exactly one real write either way, settled the same way
        // regardless of which mode asked for it.
        let mut changeset = PendingChangeset::mint("DeleteEntity", &author, branch.as_deref())
            .with_operation_id(claim_key.as_deref().map(|_| req.operation_id.as_str()))
            .with_operation_key(claim_key.as_deref());
        if let Err(e) = self
            .store
            .delete(kind, &id, if_match, changeset.write_ctx())
            .await
        {
            let status = store_err(e);
            if let Some(key) = &claim_key {
                operation_receipts::settle_for_status(&*self.store, key, &status).await;
            }
            return Err(status);
        }
        let receipt_changeset_id = changeset.id.clone();
        if let Some(key) = &claim_key {
            operation_receipts::settle_applied(&*self.store, key, &receipt_changeset_id).await;
        }
        changeset.record_delete(kind, &id);
        self.record_changeset(
            changeset,
            git_mirror::commit_message(kind, &id.namespace, &id.slug, id.version, "delete"),
        )
        .await;
        if branch.is_none() {
            self.search_apply_delete(kind, &id);
            self.invalidate_snapshot().await;
            self.mirror_apply(MirrorBatch {
                changes: vec![MirrorChange::Delete {
                    kind,
                    id: id.clone(),
                }],
                author,
                message: git_mirror::commit_message(
                    kind,
                    &id.namespace,
                    &id.slug,
                    id.version,
                    "delete",
                ),
            })
            .await;
        }
        Ok(Response::new(pb::DeleteEntityResponse {
            referrers,
            operation_receipt: claim_key.as_ref().map(|_| {
                operation_receipts::fresh_receipt(&req.operation_id, &receipt_changeset_id)
            }),
        }))
    }

    async fn batch_mutate(
        &self,
        req: Request<pb::BatchMutateRequest>,
    ) -> Result<Response<pb::BatchMutateResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let author = Self::author_from_metadata(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();

        if req.ops.len() > MAX_BATCH_OPS {
            return Err(Status::invalid_argument(format!(
                "batch_mutate.ops length {} exceeds MAX_BATCH_OPS={MAX_BATCH_OPS}",
                req.ops.len(),
            )));
        }

        // Authorize before validating, so an unauthorized caller does not get
        // a free validation pass over the whole batch. Ops that carry no
        // usable id contribute no namespace here; the validation loop below
        // rejects them on their own terms.
        let mut targets = WriteTargets::none();
        for op in &req.ops {
            let id = match &op.op {
                Some(pb::batch_mutate_op::Op::Put(p)) => {
                    p.entity.as_ref().and_then(|e| entity_id(e).ok())
                }
                Some(pb::batch_mutate_op::Op::Delete(d)) => d.id.as_ref(),
                None => None,
            };
            if let Some(id) = id {
                targets.insert(&id.namespace);
            }
        }
        self.authorize_write(principal.as_ref(), branch.as_deref(), &targets)
            .await?;
        if let Some(name) = branch.as_deref() {
            self.require_branch(name).await?;
        }

        // Validate every op up front and translate to store-level mutations.
        // Build per-op metadata so we can map outcomes back to per-op responses.
        //
        // Scenario validation needs the full store as context; we defer it
        // until *after* `lock_mutations()` so an entity referenced by a
        // scenario cannot be deleted between the snapshot we validate
        // against and the lock that admits the apply. The first pass below
        // only does op-level checks (entity_kind, entity_id, slice kind);
        // slice entities are queued for post-lock scenario validation.
        let mut mutations: Vec<MutationOp> = Vec::with_capacity(req.ops.len());
        let mut metas: Vec<OpMetaLike> = Vec::with_capacity(req.ops.len());
        let mut dry_run = req.validate_only;
        let mut has_slice_puts = false;

        for (i, op) in req.ops.iter().enumerate() {
            match &op.op {
                Some(pb::batch_mutate_op::Op::Put(p)) => {
                    operation_receipts::reject_nested_operation_id(&p.operation_id, i)?;
                    let entity = p.entity.clone().ok_or_else(|| {
                        Status::invalid_argument(format!("ops[{i}].put.entity is required"))
                    })?;
                    let kind = entity_kind(&entity)?;
                    let id = entity_id(&entity)?.clone();
                    validate_entity_content(&entity)?;
                    check_project_namespace_conflict(&entity)?;
                    if is_slice_kind(kind) {
                        has_slice_puts = true;
                    }
                    if p.validate_only {
                        dry_run = true;
                    }
                    metas.push(OpMetaLike {
                        kind,
                        id: id.clone(),
                        is_put: true,
                        entity: Some(entity.clone()),
                    });
                    let mut_op = if p.create_only {
                        MutationOp::Create { kind, entity }
                    } else if !p.if_match.is_empty() {
                        MutationOp::Put {
                            kind,
                            entity,
                            if_match: Some(p.if_match.clone()),
                            force: p.force,
                        }
                    } else {
                        MutationOp::Put {
                            kind,
                            entity,
                            if_match: None,
                            force: p.force,
                        }
                    };
                    mutations.push(mut_op);
                }
                Some(pb::batch_mutate_op::Op::Delete(d)) => {
                    operation_receipts::reject_nested_operation_id(&d.operation_id, i)?;
                    let kind = kind_from_i32(d.kind, &format!("ops[{i}].delete.kind"))?;
                    let id = require_id(d.id.as_ref(), &format!("ops[{i}].delete"))?.clone();
                    let mode = pb::delete_entity_request::Mode::try_from(d.mode)
                        .unwrap_or(pb::delete_entity_request::Mode::FailIfReferenced);
                    if matches!(mode, pb::delete_entity_request::Mode::DryRun) {
                        dry_run = true;
                    }
                    metas.push(OpMetaLike {
                        kind,
                        id: id.clone(),
                        is_put: false,
                        entity: None,
                    });
                    let if_match = if d.if_match.is_empty() {
                        None
                    } else {
                        Some(d.if_match.clone())
                    };
                    mutations.push(MutationOp::Delete { kind, id, if_match });
                }
                None => {
                    return Err(Status::invalid_argument(format!("ops[{i}].op is required")));
                }
            }
        }
        // `dry_run` is only fully known once every op's own validate_only /
        // MODE_DRY_RUN flag has been folded in above, so the exclusion check
        // against operation_id has to wait until here.
        operation_receipts::reject_operation_id_with_dry_run(&req.operation_id, dry_run)?;

        let mut type_library_namespaces = std::collections::HashSet::new();
        let mut type_library_admissions = Vec::new();
        for (i, m) in metas.iter().enumerate() {
            let Some(entity) = m.entity.as_ref().filter(|_| m.is_put) else {
                continue;
            };
            if m.kind != pb::EntityKind::TypeLibrary {
                continue;
            }
            if !type_library_namespaces.insert(m.id.namespace.clone()) {
                return Err(Status::invalid_argument(format!(
                    "ops[{i}]: a batch may put at most one type library per namespace"
                )));
            }
            if let Some(admission) = self
                .type_libraries
                .admit(&*self.store, entity, branch.as_deref())
                .await?
            {
                type_library_admissions.push(admission);
            }
        }
        let pending_libraries: Vec<&pb::TypeLibrary> = metas
            .iter()
            .filter(|m| m.is_put)
            .filter_map(|m| match m.entity.as_ref()?.kind.as_ref()? {
                pb::entity::Kind::TypeLibrary(library) => Some(library),
                _ => None,
            })
            .collect();
        for m in metas.iter().filter(|m| m.is_put) {
            if let Some(entity) = m.entity.as_ref() {
                if let Some(admission) = self
                    .type_libraries
                    .check_schema(&*self.store, entity, &pending_libraries, branch.as_deref())
                    .await?
                {
                    if !type_library_admissions.contains(&admission) {
                        type_library_admissions.push(admission);
                    }
                }
            }
        }

        // Serialize against every other writer: dry-run applies and rolls
        // back, so nothing may interleave with it, and concurrent batches
        // must not interleave with each other.
        let _guard = self.lock_mutations().await;
        for admission in &type_library_admissions {
            self.type_libraries
                .confirm(&*self.store, admission, branch.as_deref())
                .await?;
        }
        if metas
            .iter()
            .any(|m| !m.is_put && m.kind == pb::EntityKind::TypeLibrary)
        {
            let mut changes = crate::type_libraries::TypeLibraryChanges::default();
            for m in &metas {
                match m.entity.as_ref().filter(|_| m.is_put) {
                    Some(entity) => changes.put(entity),
                    None => changes.delete(m.kind, &m.id),
                }
            }
            self.type_libraries
                .verify(&*self.store, &changes, branch.as_deref())
                .await?;
            self.reject_type_library_users(branch.as_deref(), &vis, &metas)
                .await?;
        }

        // Scenario validation runs against a post-lock snapshot so an
        // entity referenced by a scenario cannot be deleted between the
        // check and the apply. We invalidate first to drop any stale
        // pre-lock view from another process.
        //
        // The batch validates against the world it is about to create: its
        // own puts overlay the snapshot, so a slice may reference entities
        // created (or updated) alongside it in the same atomic request.
        // Appending after the snapshot lets batch versions win in the
        // per-key maps validation builds. The mutation lock is held, so
        // nothing can remove an overlaid entity between check and apply.
        //
        // Per-op issues (warnings/info) are retained for PutEntityResponse
        // on success; errors fail the batch with the first offending index.
        let mut per_op_validation: Vec<Vec<pb::ValidationIssue>> =
            std::iter::repeat_with(Vec::new).take(metas.len()).collect();
        if has_slice_puts {
            if branch.is_none() {
                self.invalidate_snapshot().await;
            }
            let pre_all_for_validation = self.snapshot_for(branch.as_deref(), &vis).await?;
            let mut validation_ctx: Vec<StoredEntity> = (*pre_all_for_validation).clone();
            validation_ctx.extend(metas.iter().filter(|m| m.is_put).filter_map(|m| {
                m.entity.as_ref().map(|entity| StoredEntity {
                    entity: entity.clone(),
                    etag: String::new(),
                })
            }));
            let mut scenario_errors: Vec<pb::ValidationIssue> = Vec::new();
            let mut first_error_index: Option<usize> = None;
            for (i, m) in metas.iter().enumerate() {
                if !m.is_put || !is_slice_kind(m.kind) {
                    continue;
                }
                let Some(entity) = m.entity.as_ref() else {
                    continue;
                };
                let issues = validation::validate_scenarios(entity, &validation_ctx);
                let (errors, non_errors): (Vec<_>, Vec<_>) = issues
                    .into_iter()
                    .partition(|iss| iss.severity == pb::validation_issue::Severity::Error as i32);
                if errors.is_empty() {
                    per_op_validation[i] = non_errors;
                } else {
                    if first_error_index.is_none() {
                        first_error_index = Some(i);
                    }
                    scenario_errors.extend(errors);
                }
            }
            if let Some(failed_op_index) = first_error_index {
                return Ok(Response::new(pb::BatchMutateResponse {
                    status: pb::batch_mutate_response::Status::Failed as i32,
                    results: Vec::new(),
                    failed_op_index: i32::try_from(failed_op_index).unwrap_or(i32::MAX),
                    failure: scenario_errors,
                    precondition_failure: None,
                    operation_receipt: None,
                }));
            }
        }

        // A batch delete defaults to the same FAIL_IF_REFERENCED contract as
        // the standalone delete_entity RPC: embedding a DeleteEntityRequest
        // in a batch op must not silently relax it. Checked once, up front,
        // under the mutation lock, so a dry-run batch reports the same
        // outcome a real apply would reach -- and so this also runs for the
        // non-dry-run branch below, which has no referrer check of its own.
        //
        // The same scan also feeds `per_op_referrers`: the standalone
        // delete_entity RPC always returns its referrers on success (FORCE
        // included) so the caller knows what else now points at nothing;
        // a batch delete result should carry the same awareness instead of
        // the empty list `build_op_results` used to hand back unconditionally.
        let delete_modes: Vec<Option<pb::delete_entity_request::Mode>> = req
            .ops
            .iter()
            .map(|op| match &op.op {
                Some(pb::batch_mutate_op::Op::Delete(d)) => Some(
                    pb::delete_entity_request::Mode::try_from(d.mode)
                        .unwrap_or(pb::delete_entity_request::Mode::FailIfReferenced),
                ),
                _ => None,
            })
            .collect();
        let mut per_op_referrers: Vec<Vec<pb::Reference>> =
            std::iter::repeat_with(Vec::new).take(metas.len()).collect();
        if delete_modes.iter().any(Option::is_some) {
            let index = self.reverse_index_for(branch.as_deref(), &vis).await?;
            for (i, mode) in delete_modes.iter().enumerate() {
                let Some(mode) = mode else { continue };
                let Some(meta) = metas.get(i) else { continue };
                let referrers = index.incoming(meta.kind, &meta.id).to_vec();
                if matches!(
                    mode,
                    pb::delete_entity_request::Mode::FailIfReferenced
                        | pb::delete_entity_request::Mode::Unspecified
                ) && !referrers.is_empty()
                {
                    return Ok(Response::new(Self::batch_referenced_response(&metas, i)));
                }
                per_op_referrers[i] = referrers;
            }
        }

        if dry_run {
            let dry_run_keys: Vec<String> = mutations
                .iter()
                .map(|op| match op {
                    MutationOp::Create { kind, entity } | MutationOp::Put { kind, entity, .. } => {
                        let id = trogon_atlas_store::refs::entity_id(entity).map_or_else(
                            || "<unknown>".into(),
                            |i| format!("{}/{}", i.namespace, i.slug),
                        );
                        format!("{kind:?}:{id}")
                    }
                    MutationOp::Delete { kind, id, .. } => {
                        format!("{kind:?}:{}/{}", id.namespace, id.slug)
                    }
                })
                .collect();
            tracing::debug!(keys = ?dry_run_keys, "dry-run batch_mutate: validating against in-memory overlay");
            // Validate against an in-memory overlay seeded from the current
            // snapshot. The real store is never touched, so concurrent readers
            // can never observe intermediate dry-run state.
            //
            // We take the snapshot under the mutation lock so the overlay
            // starts from a consistent post-lock view.
            if branch.is_none() {
                self.invalidate_snapshot().await;
            }
            let pre_all_arc = self.snapshot_for(branch.as_deref(), &vis).await?;
            // The overlay's etags are not the live store's, so if_match
            // guards must be checked against the snapshot's etags here and
            // stripped before the overlay apply.
            let live_etags: std::collections::HashMap<(i32, pb::IdKey), String> = pre_all_arc
                .iter()
                .filter_map(|se| {
                    let k = trogon_atlas_store::refs::entity_kind(&se.entity)?;
                    let id = trogon_atlas_store::refs::entity_id(&se.entity)?;
                    Some(((k as i32, pb::IdKey::new(id)), se.etag.clone()))
                })
                .collect();
            let mut overlay_mutations: Vec<MutationOp> = Vec::with_capacity(mutations.len());
            for (i, op) in mutations.iter().enumerate() {
                let (kind, id, if_match) = match op {
                    MutationOp::Put {
                        kind,
                        entity,
                        if_match: Some(exp),
                        ..
                    } => {
                        if let Some(id) = trogon_atlas_store::refs::entity_id(entity) {
                            (*kind, id.clone(), exp.clone())
                        } else {
                            overlay_mutations.push(op.clone());
                            continue;
                        }
                    }
                    MutationOp::Delete {
                        kind,
                        id,
                        if_match: Some(exp),
                    } => (*kind, id.clone(), exp.clone()),
                    other => {
                        overlay_mutations.push(other.clone());
                        continue;
                    }
                };
                let found = live_etags.get(&(kind as i32, pb::IdKey::new(&id)));
                match found {
                    Some(etag) if *etag == if_match => {}
                    Some(etag) => {
                        let err = trogon_atlas_store::error::StoreError::BatchFailed {
                            index: i,
                            source: Box::new(trogon_atlas_store::error::StoreError::EtagMismatch {
                                expected: if_match,
                                found: etag.clone(),
                            }),
                        };
                        return Ok(Response::new(
                            self.batch_failure(&metas, &mutations, err, branch.as_deref())
                                .await?,
                        ));
                    }
                    None => {
                        let err = trogon_atlas_store::error::StoreError::BatchFailed {
                            index: i,
                            source: Box::new(trogon_atlas_store::error::StoreError::NotFound),
                        };
                        return Ok(Response::new(
                            self.batch_failure(&metas, &mutations, err, branch.as_deref())
                                .await?,
                        ));
                    }
                }
                overlay_mutations.push(match op {
                    MutationOp::Put { kind, entity, .. } => MutationOp::Put {
                        kind: *kind,
                        entity: entity.clone(),
                        if_match: None,
                        force: false,
                    },
                    MutationOp::Delete { kind, id, .. } => MutationOp::Delete {
                        kind: *kind,
                        id: id.clone(),
                        if_match: None,
                    },
                    MutationOp::Create { kind, entity } => MutationOp::Create {
                        kind: *kind,
                        entity: entity.clone(),
                    },
                });
            }
            match dry_run::apply(&pre_all_arc, &overlay_mutations) {
                Ok(outcomes) => {
                    let results = build_op_results(
                        &metas,
                        &outcomes,
                        /*validated=*/ true,
                        &per_op_validation,
                        &per_op_referrers,
                    );
                    Ok(Response::new(pb::BatchMutateResponse {
                        status: pb::batch_mutate_response::Status::Validated as i32,
                        results,
                        failed_op_index: 0,
                        failure: Vec::new(),
                        precondition_failure: None,
                        operation_receipt: None,
                    }))
                }
                Err(err) => Ok(Response::new(
                    self.batch_failure(&metas, &mutations, err, branch.as_deref())
                        .await?,
                )),
            }
        } else {
            let attempt = if req.operation_id.is_empty() {
                None
            } else {
                let principal_name = principal
                    .as_ref()
                    .map_or(UNAUTHENTICATED_PRINCIPAL, |p| p.name.as_ref());
                let digest_req = pb::BatchMutateRequest {
                    ops: req.ops.clone(),
                    validate_only: false,
                    operation_id: String::new(),
                };
                Some(operation_receipts::OperationAttempt::new(
                    principal_name,
                    &req.operation_id,
                    "BatchMutate",
                    branch.as_deref(),
                    "trogonatlas.api.eventmodel.v1alpha1.BatchMutateRequest",
                    &digest_req,
                )?)
            };

            let claim_key = match &attempt {
                None => None,
                Some(attempt) => {
                    match operation_receipts::claim(
                        &*self.store,
                        attempt,
                        "BatchMutate",
                        branch.as_deref(),
                    )
                    .await?
                    {
                        operation_receipts::Claim::Proceed => Some(attempt.claim_key.clone()),
                        operation_receipts::Claim::Replay(record) => {
                            if record.status == OperationStatus::Rejected {
                                return Err(operation_receipts::replay_rejected_status(&record));
                            }
                            // Applied: a replayed batch is reported Applied
                            // with no per-op results rather than
                            // reconstructing one result per original op --
                            // the batch is one unit of idempotency, and
                            // `changeset_id` is enough for the caller to
                            // confirm it already landed.
                            return Ok(Response::new(pb::BatchMutateResponse {
                                status: pb::batch_mutate_response::Status::Applied as i32,
                                results: Vec::new(),
                                failed_op_index: 0,
                                failure: Vec::new(),
                                precondition_failure: None,
                                operation_receipt: Some(operation_receipts::replay_receipt(
                                    &req.operation_id,
                                    &record,
                                )),
                            }));
                        }
                    }
                }
            };

            let mut changeset = PendingChangeset::mint("BatchMutate", &author, branch.as_deref())
                .with_operation_id(claim_key.as_deref().map(|_| req.operation_id.as_str()))
                .with_operation_key(claim_key.as_deref());
            match self
                .store
                .batch_apply(&mutations, changeset.write_ctx())
                .await
            {
                Ok(outcomes) => {
                    let results = build_op_results(
                        &metas,
                        &outcomes,
                        /*validated=*/ false,
                        &per_op_validation,
                        &per_op_referrers,
                    );
                    let receipt_changeset_id = changeset.id.clone();
                    if let Some(key) = &claim_key {
                        operation_receipts::settle_applied(
                            &*self.store,
                            key,
                            &receipt_changeset_id,
                        )
                        .await;
                    }
                    changeset.record_batch(&metas, &outcomes);
                    self.record_changeset(
                        changeset,
                        git_mirror::batch_commit_message(outcomes.len()),
                    )
                    .await;
                    // Branch batches skip every baseline side effect (git
                    // mirror, search index, snapshot-cache invalidation):
                    // they never touched the baseline store or its cache in
                    // the first place (see the "Branch overlays" note on
                    // `put_entity`/`delete_entity`).
                    if branch.is_none() {
                        self.apply_batch_side_effects(&metas, &outcomes, author)
                            .await;
                        self.invalidate_snapshot().await;
                    }
                    Ok(Response::new(pb::BatchMutateResponse {
                        status: pb::batch_mutate_response::Status::Applied as i32,
                        results,
                        failed_op_index: 0,
                        failure: Vec::new(),
                        precondition_failure: None,
                        operation_receipt: claim_key.as_ref().map(|_| {
                            operation_receipts::fresh_receipt(
                                &req.operation_id,
                                &receipt_changeset_id,
                            )
                        }),
                    }))
                }
                // A partial apply leaves the claim deliberately unsettled:
                // `nats_journal`'s crash-recovery pass re-settles it (via the
                // `operation_key` on this changeset's `WriteContext`) once it
                // resolves what actually landed, which this handler cannot
                // know yet.
                Err(err) if err.is_partial_apply() => Err(store_err(err)),
                Err(err) => {
                    let resp = self
                        .batch_failure(&metas, &mutations, err, branch.as_deref())
                        .await?;
                    if let Some(key) = &claim_key {
                        let status = Self::batch_failure_status(&resp);
                        operation_receipts::settle_for_status(&*self.store, key, &status).await;
                    }
                    Ok(Response::new(resp))
                }
            }
        }
    }

    /// Poll the change feed starting after `since_token`.
    ///
    /// When `since_token` references a sequence beyond the store's current
    /// maximum (e.g. a token from a future write), the store will find no
    /// records at or after that position and return an empty page. For an
    /// unrestricted caller, or one that supplied that same future sequence
    /// itself as a plain `since_token`, the token comes back unchanged. This
    /// is intentional and not an error: clients that poll ahead of the write
    /// frontier simply receive empty pages until new writes arrive.
    ///
    /// When `scopes` is non-empty, only change events whose entity belongs to
    /// the transitive closure of at least one of the requested scopes are
    /// returned. The closure is the same set computed by `ExtractSubgraph`:
    /// the scope root entity plus everything it transitively references.
    ///
    /// Both branches share one rule for what `next_token` may say. A bound
    /// caller's bounded lookahead (up to `LIST_CHANGES_MAX_LOOKAHEAD` further
    /// store pages) keeps a poll making progress even when an entire page is
    /// records the caller cannot see or that filtering drops, so it never
    /// stalls echoing the same `since_token` forever. But a raw sequence
    /// number is only ever handed back when it is something the caller
    /// already knows: a visible or matched record's own seq that this
    /// response just disclosed, or the exact plain `since_token` the caller
    /// supplied, unchanged. Whenever the lookahead had to advance past
    /// records the caller cannot see or that are outside its requested
    /// scopes to make progress, the token is sealed instead (see
    /// `change_cursor`), so its bytes never tell a bound caller how far other
    /// tenants, or data outside its scopes, have been written. An
    /// unrestricted caller has no tenancy boundary to protect and always
    /// gets a plain sequence. A sealed token that fails to open -- tampered,
    /// minted for a different principal, or sealed under a key this process
    /// does not hold -- is refused with the classified `CURSOR_REJECTED`
    /// error rather than silently restarting the poll from 0 or from the
    /// live tip.
    async fn list_changes(
        &self,
        req: Request<pb::ListChangesRequest>,
    ) -> Result<Response<pb::ListChangesResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let req = req.into_inner();
        let page_size = clamp_page_size(
            req.page_size,
            ADVERTISED_DEFAULT_PAGE_SIZE,
            ADVERTISED_MAX_CHANGE_FEED_PAGE,
        );
        let since = if req.since_token.is_empty() {
            self.store.current_change_seq().await.map_err(store_err)?
        } else if let Ok(seq) = req.since_token.parse::<u64>() {
            seq
        } else {
            match self
                .change_cursor_seal
                .resolve(vis.principal(), &req.since_token)
            {
                Ok(seq) => seq,
                Err(crate::change_cursor::ResolveError::NotSealed) => {
                    return Err(Status::invalid_argument("invalid since_token"));
                }
                Err(crate::change_cursor::ResolveError::FailedToOpen) => {
                    return Err(crate::conv::cursor_rejected_err(
                        "since_token could not be verified; restart the listing with an empty since_token",
                    ));
                }
            }
        };

        if req.scopes.is_empty() {
            // The scoped branch below inherits the owner filter through the
            // snapshot its closure is computed from. This one reads the
            // change log directly, so it has to apply the filter itself.
            //
            // Mirrors the scoped branch's own bounded lookahead: a page that
            // is entirely invisible to this caller must not stall the
            // cursor, so the handler keeps reading further pages (bounded by
            // `LIST_CHANGES_MAX_LOOKAHEAD`) until something visible turns up
            // or the budget runs out.
            let lens = self.lens(&vis).await?;
            let mut cursor = since;
            let mut events: Vec<pb::ChangeEvent> = Vec::new();
            let mut next = since;

            for _ in 0..LIST_CHANGES_MAX_LOOKAHEAD {
                let records = self
                    .store
                    .read_changes(cursor, page_size)
                    .await
                    .map_err(store_err)?;
                if records.is_empty() {
                    break;
                }
                next = records.last().map_or(cursor, |r| r.seq);
                for record in records {
                    if change_record_visible(&record, &lens) {
                        next = record.seq;
                        events.push(change_record_to_event(record));
                        if events.len() >= page_size {
                            break;
                        }
                    }
                }
                if events.len() >= page_size || !events.is_empty() {
                    break;
                }
                cursor = next;
            }

            // A raw seq is safe to hand back only when the response already
            // disclosed it, the caller already supplied it, or there is no
            // tenancy boundary to protect; see `list_changes_next_token`.
            let next_token = self.list_changes_next_token(
                &vis,
                &req.since_token,
                since,
                next,
                !events.is_empty(),
            );

            return Ok(Response::new(pb::ListChangesResponse {
                events,
                next_token,
            }));
        }

        // Resolve each scope to its transitive entity closure (same computation
        // as ExtractSubgraph). Done once per RPC call; per-poll staleness is
        // acceptable for long-polling consumers.
        let all = self.snapshot(&vis).await?;
        let closure_keys = resolve_scope_closure(&req.scopes, &all)?;

        // Fetch up to LIST_CHANGES_MAX_LOOKAHEAD store pages to reduce the
        // probability of returning an empty page when all records in a batch
        // are filtered out. The cursor always advances by the last store-read
        // sequence so the caller never stalls. Mirrors the unscoped branch
        // above: `next` tracks the last *matched* record's own seq whenever a
        // scan turns one up, so a response never advances the cursor past a
        // record it did not actually disclose.
        let mut cursor = since;
        let mut matched: Vec<pb::ChangeEvent> = Vec::new();
        let mut next = since;

        for _ in 0..LIST_CHANGES_MAX_LOOKAHEAD {
            let records = self
                .store
                .read_changes(cursor, page_size)
                .await
                .map_err(store_err)?;

            if records.is_empty() {
                break;
            }

            next = records.last().map_or(cursor, |c| c.seq);

            for record in records {
                if record_in_scope_closure(&record, &closure_keys) {
                    next = record.seq;
                    matched.push(change_record_to_event(record));
                    if matched.len() >= page_size {
                        break;
                    }
                }
            }

            if matched.len() >= page_size || !matched.is_empty() {
                break;
            }

            cursor = next;
        }

        let next_token =
            self.list_changes_next_token(&vis, &req.since_token, since, next, !matched.is_empty());

        Ok(Response::new(pb::ListChangesResponse {
            events: matched,
            next_token,
        }))
    }

    type StreamChangesStream =
        Pin<Box<dyn Stream<Item = Result<pb::ChangeEvent, Status>> + Send + 'static>>;

    /// Page the durable changeset log, newest first.
    ///
    /// `page_token` is the id of the oldest changeset already seen; the next
    /// page starts strictly before it. Because changeset ids are UUIDv7 their
    /// lexicographic order is their chronological order, so the cursor needs
    /// no separate timestamp.
    ///
    /// An empty `next_page_token` means the caller reached the end of the log.
    /// Unlike the change feed this log does not expire, so a consumer can walk
    /// it back to the first write the server ever attributed.
    async fn list_changesets(
        &self,
        req: Request<pb::ListChangesetsRequest>,
    ) -> Result<Response<pb::ListChangesetsResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();
        let limit = clamp_page_size(
            req.page_size,
            CHANGESETS_DEFAULT_PAGE_SIZE,
            CHANGESETS_MAX_PAGE_SIZE,
        );
        let scope = if req.branch.is_empty() {
            ChangesetScope::All
        } else {
            ChangesetScope::Branch(req.branch.as_str())
        };

        if vis.is_unrestricted() {
            let before = (!req.page_token.is_empty()).then_some(req.page_token.as_str());
            let records = self
                .store
                .list_changesets(ChangesetPage {
                    before,
                    limit,
                    scope,
                })
                .await
                .map_err(store_err)?;
            // A short page is the end of the log: the store only returns fewer
            // than `limit` when it exhausted the keyspace.
            let next_page_token = if records.len() < limit {
                String::new()
            } else {
                records.last().map(|r| r.id.clone()).unwrap_or_default()
            };
            return Ok(Response::new(pb::ListChangesetsResponse {
                changesets: records.into_iter().map(changeset_to_proto).collect(),
                next_page_token,
            }));
        }

        // A bound caller sees only changesets attributed to an owner it is
        // delegated for (`Self::changeset_visible`); every other changeset
        // stays as unpartitioned as the whole RPC used to be. Mirrors
        // `list_changes`'s own bounded lookahead: a store page that turns out
        // entirely invisible to this caller must not stall the cursor or
        // prematurely report "end of log", so this keeps reading further
        // pages (bounded by `LIST_CHANGESETS_MAX_LOOKAHEAD`) until something
        // visible turns up or the store itself runs out.
        let mut cursor = (!req.page_token.is_empty()).then(|| req.page_token.clone());
        let mut matched = Vec::new();
        let mut next_page_token = String::new();

        for _ in 0..LIST_CHANGESETS_MAX_LOOKAHEAD {
            let records = self
                .store
                .list_changesets(ChangesetPage {
                    before: cursor.as_deref(),
                    limit,
                    scope,
                })
                .await
                .map_err(store_err)?;

            if records.is_empty() {
                next_page_token = String::new();
                break;
            }

            let store_exhausted = records.len() < limit;
            cursor = records.last().map(|r| r.id.clone());
            next_page_token = if store_exhausted {
                String::new()
            } else {
                cursor.clone().unwrap_or_default()
            };

            for record in records {
                if Self::changeset_visible(principal.as_ref(), &record) {
                    matched.push(changeset_to_proto(record));
                    if matched.len() >= limit {
                        break;
                    }
                }
            }

            if matched.len() >= limit || !matched.is_empty() || store_exhausted {
                break;
            }
        }

        Ok(Response::new(pb::ListChangesetsResponse {
            changesets: matched,
            next_page_token,
        }))
    }

    async fn get_changeset(
        &self,
        req: Request<pb::GetChangesetRequest>,
    ) -> Result<Response<pb::GetChangesetResponse>, Status> {
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();
        if req.id.is_empty() {
            return Err(Status::invalid_argument("id is required"));
        }
        let record = self.store.get_changeset(&req.id).await.map_err(store_err)?;
        // NOT_FOUND, not PERMISSION_DENIED: a changeset id a bound caller
        // cannot see must not be distinguishable from one that never
        // existed, the same doctrine `require_visible` applies to entities.
        if !Self::changeset_visible(principal.as_ref(), &record) {
            return Err(Status::not_found("entity not found"));
        }
        Ok(Response::new(pb::GetChangesetResponse {
            changeset: Some(changeset_to_proto(record)),
        }))
    }

    /// Page one entity's edit log, newest first: `git log -- <path>`.
    ///
    /// Paged exactly like `ListChangesets` -- `page_token` is the changeset
    /// id of the oldest revision already seen, and an empty
    /// `next_page_token` means the log is exhausted.
    ///
    /// The branch header selects *which* log. A branch keeps its own
    /// revisions, so under `x-trogon-atlas-branch` this answers "what has this
    /// branch done to the entity", not "how did the entity get here"; the
    /// baseline history behind the fork point is one call away without the
    /// header. Writes that carried no changeset (`trogon-atlas-server import-git`,
    /// direct `Store` use) recorded no revision and so are absent from both.
    async fn get_entity_history(
        &self,
        req: Request<pb::GetEntityHistoryRequest>,
    ) -> Result<Response<pb::GetEntityHistoryResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let entity_ref = req
            .r#ref
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("ref is required"))?;
        let kind = kind_from_i32(entity_ref.kind, "ref.kind")?;
        let id = require_id(entity_ref.id.as_ref(), "ref")?.clone();
        // One entity's log, so the entity's own namespace settles it.
        self.require_visible(&vis, &id.namespace).await?;
        let limit = clamp_page_size(
            req.page_size,
            CHANGESETS_DEFAULT_PAGE_SIZE,
            CHANGESETS_MAX_PAGE_SIZE,
        );
        let before = (!req.page_token.is_empty()).then_some(req.page_token.as_str());
        let records = self
            .store
            .list_entity_revisions(
                kind,
                &id,
                RevisionPage {
                    before,
                    limit,
                    branch: branch.as_deref(),
                },
            )
            .await
            .map_err(store_err)?;
        let next_page_token = if records.len() < limit {
            String::new()
        } else {
            records
                .last()
                .map(|r| r.changeset_id.clone())
                .unwrap_or_default()
        };
        Ok(Response::new(pb::GetEntityHistoryResponse {
            revisions: records.into_iter().map(entity_revision_to_proto).collect(),
            next_page_token,
        }))
    }

    /// Undo a changeset by writing its inverse.
    ///
    /// Not a rollback: nothing is erased. The inverse lands as an ordinary
    /// batch through the ordinary write path, so it validates, records
    /// change events, and is itself a changeset that can in turn be
    /// reverted. Both the original and its undo stay in the log.
    ///
    /// Refuses rather than guesses, in two cases:
    ///
    /// - **No revision row.** A write that carried no changeset id recorded
    ///   no pre-image, and the entities bucket keeps one revision, so the
    ///   pre-image is genuinely gone. Inventing one would silently write
    ///   fabricated content.
    /// - **The entity moved on.** Reverting means restoring the pre-image,
    ///   which discards everything written after it. This RPC works at
    ///   whole-entity granularity, so *any* later edit to the same entity
    ///   overlaps -- the same condition git reports as a revert conflict.
    ///   Revert the newer changesets first.
    ///
    /// The branch header chooses where the inverse lands, which is not
    /// necessarily where the original did: reverting a baseline changeset
    /// under a branch stages the undo for review and merge, exactly as any
    /// other branch write. The pre-images always come from the original
    /// changeset's own log.
    async fn revert_changeset(
        &self,
        req: Request<pb::RevertChangesetRequest>,
    ) -> Result<Response<pb::RevertChangesetResponse>, Status> {
        // A changeset has no owner, so a revert cannot be bounded to one.
        Self::refuse_unpartitioned(&Self::visibility_of(&req), "reverting a changeset")?;
        let author = Self::author_from_metadata(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();
        if req.id.is_empty() {
            return Err(Status::invalid_argument("id is required"));
        }
        operation_receipts::reject_operation_id_with_dry_run(&req.operation_id, req.dry_run)?;
        if let Some(name) = branch.as_deref() {
            self.require_branch(name).await?;
        }
        let record = self.store.get_changeset(&req.id).await.map_err(store_err)?;
        let origin = record.branch.clone();

        // Newest op first: a changeset's ops landed in order, so its inverse
        // unwinds in reverse. Deduplicated because two ops on one key share
        // a single folded revision, and applying its inverse twice would
        // restore the same pre-image twice.
        let mut seen: std::collections::HashSet<pb::EntityKey> = std::collections::HashSet::new();
        let mut inverse: Vec<(pb::EntityKind, pb::Id, Option<pb::Entity>)> = Vec::new();
        for op in record.ops.iter().rev() {
            let kind = kind_from_i32(op.entity_ref.kind, "changeset op kind")?;
            let id = require_id(op.entity_ref.id.as_ref(), "changeset op")?.clone();
            if !seen.insert(pb::EntityKey::new(kind, &id)) {
                continue;
            }
            let revision = self
                .store
                .get_entity_revision(kind, &id, &record.id, origin.as_deref())
                .await
                .map_err(|e| match e {
                    trogon_atlas_store::error::StoreError::NotFound => {
                        Status::failed_precondition(format!(
                            "changeset {} recorded no revision for {}/{}: it predates revision \
                             recording or was written without changeset attribution, so its \
                             prior content is not recoverable",
                            record.id, id.namespace, id.slug
                        ))
                    }
                    other => store_err(other),
                })?;
            let current = match self.store.get(kind, &id, branch.as_deref()).await {
                Ok(stored) => Some(stored.entity),
                Err(trogon_atlas_store::error::StoreError::NotFound) => None,
                Err(e) => return Err(store_err(e)),
            };
            let unchanged = match (current.as_ref(), revision.after.as_ref()) {
                (None, None) => true,
                (Some(now), Some(then)) => {
                    trogon_atlas_core::semantic_eq::semantically_equal(now, then)
                }
                _ => false,
            };
            if !unchanged {
                return Err(Status::failed_precondition(format!(
                    "{}/{} changed after changeset {}; revert the newer changesets first",
                    id.namespace, id.slug, record.id
                )));
            }
            inverse.push((kind, id, revision.before));
        }

        let mut targets = WriteTargets::none();
        for (_, id, _) in &inverse {
            targets.insert(&id.namespace);
        }
        self.authorize_write(principal.as_ref(), branch.as_deref(), &targets)
            .await?;

        let ops: Vec<MutationOp> = inverse
            .iter()
            .map(|(kind, id, before)| match before {
                Some(entity) => MutationOp::Put {
                    kind: *kind,
                    entity: entity.clone(),
                    if_match: None,
                    force: false,
                },
                None => MutationOp::Delete {
                    kind: *kind,
                    id: id.clone(),
                    if_match: None,
                },
            })
            .collect();
        let metas: Vec<OpMetaLike> = inverse
            .iter()
            .map(|(kind, id, before)| OpMetaLike {
                kind: *kind,
                id: id.clone(),
                is_put: before.is_some(),
                entity: before.clone(),
            })
            .collect();
        let reverted_ops: Vec<pb::ChangesetOp> = metas
            .iter()
            .map(|meta| pb::ChangesetOp {
                kind: if meta.is_put {
                    pb::change_event::Kind::Put as i32
                } else {
                    pb::change_event::Kind::Deleted as i32
                },
                entity: Some(pb::EntityRef {
                    kind: meta.kind as i32,
                    id: Some(meta.id.clone()),
                }),
            })
            .collect();

        let attempt = if req.operation_id.is_empty() {
            None
        } else {
            let principal_name = principal
                .as_ref()
                .map_or(UNAUTHENTICATED_PRINCIPAL, |p| p.name.as_ref());
            let digest_req = pb::RevertChangesetRequest {
                id: req.id.clone(),
                dry_run: false,
                operation_id: String::new(),
            };
            Some(operation_receipts::OperationAttempt::new(
                principal_name,
                &req.operation_id,
                "RevertChangeset",
                branch.as_deref(),
                "trogonatlas.api.eventmodel.v1alpha1.RevertChangesetRequest",
                &digest_req,
            )?)
        };

        // Claimed here, ahead of the dry-run-or-nothing-to-revert check
        // below: "nothing to revert" is a sibling success outcome to the
        // real write, not a validation failure, so it is settled Applied
        // (with an empty changeset_id) the same as the real write is.
        let claim_key = match &attempt {
            None => None,
            Some(attempt) => {
                match operation_receipts::claim(
                    &*self.store,
                    attempt,
                    "RevertChangeset",
                    branch.as_deref(),
                )
                .await?
                {
                    operation_receipts::Claim::Proceed => Some(attempt.claim_key.clone()),
                    operation_receipts::Claim::Replay(record) => {
                        if record.status == OperationStatus::Rejected {
                            return Err(operation_receipts::replay_rejected_status(&record));
                        }
                        return Ok(Response::new(pb::RevertChangesetResponse {
                            ops: reverted_ops,
                            dry_run: record.changeset_id.is_empty(),
                            changeset_id: record.changeset_id.clone(),
                            operation_receipt: Some(operation_receipts::replay_receipt(
                                &req.operation_id,
                                &record,
                            )),
                        }));
                    }
                }
            }
        };

        if req.dry_run || ops.is_empty() {
            if let Some(key) = &claim_key {
                operation_receipts::settle_applied(&*self.store, key, "").await;
            }
            return Ok(Response::new(pb::RevertChangesetResponse {
                ops: reverted_ops,
                changeset_id: String::new(),
                dry_run: true,
                operation_receipt: claim_key
                    .as_ref()
                    .map(|_| operation_receipts::fresh_receipt(&req.operation_id, "")),
            }));
        }

        let _guard = self.lock_mutations().await;
        let mut changeset = PendingChangeset::mint("RevertChangeset", &author, branch.as_deref())
            .with_operation_id(claim_key.as_deref().map(|_| req.operation_id.as_str()))
            .with_operation_key(claim_key.as_deref());
        let changeset_id = changeset.id.clone();
        let outcomes = match self.store.batch_apply(&ops, changeset.write_ctx()).await {
            Ok(o) => o,
            // Same partial-apply exception as `batch_mutate`: leave the
            // claim Pending and let `nats_journal`'s crash-recovery pass
            // settle it once it knows what actually landed.
            Err(err) if err.is_partial_apply() => return Err(store_err(err)),
            Err(err) => {
                let status = store_err(err);
                if let Some(key) = &claim_key {
                    operation_receipts::settle_for_status(&*self.store, key, &status).await;
                }
                return Err(status);
            }
        };
        if let Some(key) = &claim_key {
            operation_receipts::settle_applied(&*self.store, key, &changeset_id).await;
        }
        changeset.record_batch(&metas, &outcomes);
        self.record_changeset(changeset, format!("revert {}", record.id))
            .await;
        if branch.is_none() {
            self.apply_batch_side_effects(&metas, &outcomes, author)
                .await;
            self.invalidate_snapshot().await;
        }
        Ok(Response::new(pb::RevertChangesetResponse {
            ops: reverted_ops,
            changeset_id: changeset_id.clone(),
            dry_run: false,
            operation_receipt: claim_key
                .as_ref()
                .map(|_| operation_receipts::fresh_receipt(&req.operation_id, &changeset_id)),
        }))
    }

    async fn stream_changes(
        &self,
        req: Request<pb::StreamChangesRequest>,
    ) -> Result<Response<Self::StreamChangesStream>, Status> {
        // A subscription outlives the directory view it would be filtered
        // with, so a namespace moved away mid-stream would keep feeding its
        // former owner until the connection dropped. `ListChanges` re-reads
        // the registry on every poll and has no such window; this resolves
        // the lens once at subscribe time instead and accepts that one
        // window of staleness for the life of the connection.
        let vis = Self::visibility_of(&req);
        let lens = self.lens(&vis).await?;
        let req = req.into_inner();
        let after_seq = if req.after_token.is_empty() {
            self.store.current_change_seq().await.map_err(store_err)?
        } else {
            req.after_token
                .parse::<u64>()
                .map_err(|_| Status::invalid_argument("invalid after_token"))?
        };
        let namespace_filter = req.namespace.clone();
        // `Unspecified` (0) is the documented "no filter" sentinel; any
        // other integer that does not decode to a known kind is a client
        // typo and must fail loudly rather than silently widen the stream.
        let kind_filter = if req.kind == 0 {
            pb::EntityKind::Unspecified
        } else {
            pb::EntityKind::try_from(req.kind)
                .map_err(|_| Status::invalid_argument(format!("unknown EntityKind {}", req.kind)))?
        };

        // Compare-exchange loop: only admit the subscriber if the current count
        // is strictly below the cap. fetch_add-then-check would let concurrent
        // callers all read N-1 and all increment past the cap simultaneously.
        let mut current = self
            .stream_changes_count
            .load(std::sync::atomic::Ordering::Relaxed);
        loop {
            if current >= MAX_STREAM_CHANGES_SUBSCRIBERS {
                return Err(Status::resource_exhausted(format!(
                    "too many concurrent StreamChanges subscribers (limit {MAX_STREAM_CHANGES_SUBSCRIBERS})"
                )));
            }
            match self.stream_changes_count.compare_exchange_weak(
                current,
                current + 1,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        let stream_counter = StreamChangesGuard(self.stream_changes_count.clone());

        let subscriber = self.store.subscribe();
        // Bounded backlog page so a long-gap reconnect does not allocate the
        // whole change log in one shot. The spawned task pages through the
        // backlog and pushes each record into the bounded mpsc; the client
        // applies natural backpressure via the channel's capacity.
        const BACKLOG_PAGE: usize = 256;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<pb::ChangeEvent, Status>>(64);
        let store_for_backlog = self.store.clone();
        let tx_panic = tx.clone();
        let inner = tokio::spawn(async move {
            let _guard = stream_counter;
            let mut termination = crate::telemetry::StreamTerminationGuard::new("StreamChanges");
            let mut last_sent = after_seq;
            let mut subscriber = subscriber;
            // Drain the backlog one page at a time. A page that returns
            // fewer than BACKLOG_PAGE records is the end of the backlog.
            loop {
                let page = match store_for_backlog
                    .read_changes(last_sent, BACKLOG_PAGE)
                    .await
                {
                    Ok(p) => p,
                    Err(err) => {
                        let _ = tx.send(Err(store_err(err))).await;
                        termination.mark("backlog_error");
                        return;
                    }
                };
                let page_len = page.len();
                for record in page {
                    if record.seq <= last_sent {
                        continue;
                    }
                    let next_seq = record.seq;
                    if change_matches_filter(&record, &namespace_filter, kind_filter)
                        && change_record_visible(&record, &lens)
                        && tx.send(Ok(change_record_to_event(record))).await.is_err()
                    {
                        termination.mark("client_disconnected_backlog");
                        return;
                    }
                    last_sent = next_seq;
                }
                if page_len < BACKLOG_PAGE {
                    break;
                }
            }
            loop {
                match subscriber.recv().await {
                    Ok(record) => {
                        if record.seq <= last_sent {
                            continue;
                        }
                        // Gap detection: if the first live record's seq is
                        // > last_sent + 1, a record fell out of the
                        // broadcast buffer between the backlog drain and
                        // the live tail (or was skipped by the tailer).
                        // Re-drain via `read_changes` for the gap
                        // [last_sent + 1, record.seq) before emitting this
                        // record so nothing in `(after_seq, record.seq]`
                        // is silently lost.
                        if record.seq > last_sent + 1 {
                            let mut gap_start = last_sent;
                            'gap: loop {
                                let page = match store_for_backlog
                                    .read_changes(gap_start, BACKLOG_PAGE)
                                    .await
                                {
                                    Ok(p) => p,
                                    Err(err) => {
                                        let _ = tx.send(Err(store_err(err))).await;
                                        termination.mark("gap_error");
                                        return;
                                    }
                                };
                                let page_len = page.len();
                                // Retention holes / skipped undecodable
                                // records can make the next durable seq
                                // already >= live. The previous loop
                                // `continue`d those rows without advancing
                                // `gap_start`, which spun forever on a full
                                // page. `gap_fill_page_outcome` detects that.
                                match gap_fill_page_outcome(
                                    page.iter().map(|r| r.seq),
                                    gap_start,
                                    record.seq,
                                    BACKLOG_PAGE,
                                    page_len,
                                ) {
                                    GapFillPageOutcome::Done => break 'gap,
                                    GapFillPageOutcome::Emit {
                                        new_gap_start,
                                        page_exhausted,
                                    } => {
                                        for gap_rec in page {
                                            if gap_rec.seq <= gap_start || gap_rec.seq >= record.seq
                                            {
                                                continue;
                                            }
                                            let next_seq = gap_rec.seq;
                                            if change_matches_filter(
                                                &gap_rec,
                                                &namespace_filter,
                                                kind_filter,
                                            ) && change_record_visible(&gap_rec, &lens)
                                                && tx
                                                    .send(Ok(change_record_to_event(gap_rec)))
                                                    .await
                                                    .is_err()
                                            {
                                                termination.mark("client_disconnected_gap");
                                                return;
                                            }
                                            gap_start = next_seq;
                                        }
                                        gap_start = new_gap_start;
                                        if page_exhausted
                                            || gap_start >= record.seq.saturating_sub(1)
                                        {
                                            break 'gap;
                                        }
                                    }
                                }
                            }
                        }
                        if !change_matches_filter(&record, &namespace_filter, kind_filter)
                            || !change_record_visible(&record, &lens)
                        {
                            last_sent = record.seq;
                            continue;
                        }
                        last_sent = record.seq;
                        if tx.send(Ok(change_record_to_event(record))).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        // Broadcast overflow dropped `missed` in-process
                        // copies. Do NOT terminate: the next Ok(record)
                        // arrives with seq > last_sent + 1 and the gap-fill
                        // path above recovers the durable JetStream log via
                        // `read_changes`. Terminating with DataLoss forced
                        // every slow subscriber to reconnect and re-list
                        // even though the store still held the records.
                        metrics::counter!("broadcast_lag_events_total").increment(missed);
                        tracing::warn!(
                            missed,
                            last_sent,
                            "StreamChanges subscriber lagged; will reconcile via read_changes on next record"
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        termination.mark("source_closed");
                        return;
                    }
                }
            }
        });
        // Catch panics in the spawned task and forward an Internal error so
        // the client sees a clean stream termination instead of a silent close.
        tokio::spawn(async move {
            if let Err(join_err) = inner.await {
                if join_err.is_panic() {
                    tracing::error!("StreamChanges task panicked; notifying client");
                    let _ = tx_panic
                        .send(Err(Status::internal("stream processing task panicked")))
                        .await;
                }
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn validate_event_model(
        &self,
        req: Request<pb::ValidateEventModelRequest>,
    ) -> Result<Response<pb::ValidateEventModelResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let model = if let Some(m) = req.event_model {
            m
        } else if let Some(id) = req.event_model_id.as_ref() {
            require_id(Some(id), "event_model_id")?;
            // Loading a model by id is a by-key read; the snapshot the
            // validator runs against is lensed, but this fetch is not.
            self.require_visible(&vis, &id.namespace).await?;
            let stored = self
                .store
                .get(pb::EntityKind::EventModel, id, branch.as_deref())
                .await
                .map_err(store_err)?;
            match stored.entity.kind {
                Some(pb::entity::Kind::EventModel(em)) => em,
                _ => return Err(Status::internal("stored entity is not an EventModel")),
            }
        } else {
            return Err(Status::invalid_argument(
                "either event_model or event_model_id must be set",
            ));
        };

        let all = self.snapshot_for(branch.as_deref(), &vis).await?;
        let issues = validation::validate_model(&model, &all);
        Ok(Response::new(pb::ValidateEventModelResponse { issues }))
    }

    async fn list_validation_rules(
        &self,
        _req: Request<pb::ListValidationRulesRequest>,
    ) -> Result<Response<pb::ListValidationRulesResponse>, Status> {
        // Static catalog, no I/O, no store lookup. The validator owns
        // the source of truth; the response is a snapshot of every code
        // it can emit with its canonical severity and the "why".
        let rules = validation::rule_catalog();
        Ok(Response::new(pb::ListValidationRulesResponse { rules }))
    }

    async fn validate_project(
        &self,
        req: Request<pb::ValidateProjectRequest>,
    ) -> Result<Response<pb::ValidateProjectResponse>, Status> {
        let vis = Self::visibility_of(&req);
        use pb::{validate_project_request::Scope, validate_project_response::ModelReport};
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let scope = req.scope.ok_or_else(|| {
            Status::invalid_argument("scope is required (project_id or domain_id)")
        })?;

        let all = self.snapshot_for(branch.as_deref(), &vis).await?;

        // Project-level hygiene runs once for the whole call.
        let mut store_issues = validation::validate_store(all.as_ref());

        // Resolve which Domain(s) belong to the scope, then iterate
        // every stored EventModel whose namespace traces back to one of
        // those Domains. Today scoping by project requires walking the
        // Domain.project chain; scoping by domain_id is direct.
        let scoped_domains: std::collections::HashSet<(String, String)> = match &scope {
            Scope::DomainId(id) => {
                let mut s = std::collections::HashSet::new();
                s.insert((id.namespace.clone(), id.slug.clone()));
                s
            }
            Scope::ProjectId(pid) => all
                .iter()
                .filter_map(|se| match se.entity.kind.as_ref()? {
                    pb::entity::Kind::Domain(d) => {
                        let did = d.id.as_ref()?;
                        let proj = d.project.as_ref()?.id.as_ref()?;
                        (proj.namespace == pid.namespace && proj.slug == pid.slug)
                            .then(|| (did.namespace.clone(), did.slug.clone()))
                    }
                    _ => None,
                })
                .collect(),
        };

        let (_subdomain_domain, bc_domain) = build_domain_maps(all.as_ref());

        let mut reports: Vec<ModelReport> = Vec::new();
        for se in all.iter() {
            let Some(pb::entity::Kind::EventModel(em)) = se.entity.kind.as_ref() else {
                continue;
            };
            let Some(id) = em.id.as_ref() else { continue };
            let owning = bc_domain.get(&id.namespace);
            let in_scope = match owning {
                Some(did) => scoped_domains.contains(did),
                None => false,
            };
            if !in_scope {
                continue;
            }
            let issues = validation::validate_model(em, all.as_ref());
            reports.push(ModelReport {
                event_model_id: Some(id.clone()),
                issues,
            });
        }

        // Surface the store-level issues by attaching them to a
        // synthetic "store" report, keeping the response shape uniform
        // without inventing a new field.
        let (mut total_errors, mut total_warnings, mut total_info) = (0i32, 0i32, 0i32);
        for r in &reports {
            for i in &r.issues {
                match pb::validation_issue::Severity::try_from(i.severity) {
                    Ok(pb::validation_issue::Severity::Error) => total_errors += 1,
                    Ok(pb::validation_issue::Severity::Warning) => total_warnings += 1,
                    Ok(pb::validation_issue::Severity::Info) => total_info += 1,
                    _ => {}
                }
            }
        }
        for i in &store_issues {
            match pb::validation_issue::Severity::try_from(i.severity) {
                Ok(pb::validation_issue::Severity::Error) => total_errors += 1,
                Ok(pb::validation_issue::Severity::Warning) => total_warnings += 1,
                Ok(pb::validation_issue::Severity::Info) => total_info += 1,
                _ => {}
            }
        }
        if !store_issues.is_empty() {
            let synthetic = ModelReport {
                event_model_id: None, // store-wide; not tied to a single EM
                issues: std::mem::take(&mut store_issues),
            };
            reports.insert(0, synthetic);
        }

        Ok(Response::new(pb::ValidateProjectResponse {
            reports,
            total_errors,
            total_warnings,
            total_info,
        }))
    }

    /// Delete entities matching a query.
    ///
    /// When only `project` is set it is treated as a namespace alias: entities
    /// whose namespace equals the project name pass the filter. Setting both
    /// `project` and `namespace` to different non-empty values is ambiguous and
    /// returns `invalid_argument`.
    async fn delete_by_query(
        &self,
        req: Request<pb::DeleteByQueryRequest>,
    ) -> Result<Response<pb::DeleteByQueryResponse>, Status> {
        let vis = Self::visibility_of(&req);
        use pb::delete_by_query_response::Deleted;
        let author = Self::author_from_metadata(&req);
        // Under a branch header the victim scan and the referrer gate both
        // read the merged view, and each delete lands as a tombstone delta.
        // A victim that exists only on baseline is therefore hidden from the
        // branch and removed from baseline on merge, never before: bulk
        // cleanup becomes a reviewable change set like any other.
        let branch = Self::branch_from_metadata(&req)?;
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();

        if !req.project.is_empty() && !req.namespace.is_empty() && req.project != req.namespace {
            return Err(Status::invalid_argument(
                "project and namespace are both set but differ; \
                 set only one or ensure they are equal",
            ));
        }

        if !req.namespace.is_empty() {
            crate::conv::validate_id_component("namespace", &req.namespace)?;
        }
        if !req.slug.is_empty() {
            crate::conv::validate_id_component("slug", &req.slug)?;
        }
        if !req.project.is_empty() {
            crate::conv::validate_id_component("project", &req.project)?;
        }

        // A namespace filter is the one case where the victims are bounded
        // before the scan: nothing outside it can match. Without one the
        // query selects its own victims from the whole store, which no
        // allow-list can bound.
        let targets = if req.namespace.is_empty() {
            WriteTargets::Unbounded
        } else {
            WriteTargets::of(&req.namespace)
        };
        self.authorize_write(principal.as_ref(), branch.as_deref(), &targets)
            .await?;
        if let Some(name) = branch.as_deref() {
            self.require_branch(name).await?;
        }

        // Reject a fully-unscoped request: kind=Unspecified AND no namespace,
        // slug, or project means "delete everything", which is almost certainly
        // an accidental call.
        let kind_raw = pb::EntityKind::try_from(req.kind).unwrap_or(pb::EntityKind::Unspecified);
        if kind_raw == pb::EntityKind::Unspecified
            && req.namespace.is_empty()
            && req.slug.is_empty()
            && req.project.is_empty()
        {
            return Err(Status::invalid_argument(
                "delete_by_query requires at least one filter (kind, namespace, slug, or project); \
                 an all-default request would delete the entire store",
            ));
        }

        if req.max_deletes < 0 {
            return Err(Status::invalid_argument(
                "max_deletes must be a positive integer; \
                 set it to the maximum number of entities you expect this call to remove",
            ));
        }
        // max_deletes=0 means "refuse to delete anything" -- the caller must
        // provide a positive cap. This prevents silent unlimited deletes when
        // a caller forgets to set the field.
        if req.max_deletes == 0 {
            return Err(Status::failed_precondition(
                "max_deletes must be a positive integer; \
                 set it to the maximum number of entities you expect this call to remove",
            ));
        }

        // Hold the mutation lock for the entire query->delete sequence so a
        // concurrent put cannot create entities matching the query mid-loop
        // (which would escape the max_deletes cap) or delete entities that
        // are already on our victim list (which would surface as NotFound).
        // Drop the snapshot first so the victim scan reads post-lock state.
        let _guard = self.lock_mutations().await;
        if branch.is_none() {
            self.invalidate_snapshot().await;
        }
        let (all, truncated) = self
            .snapshot_with_truncation(branch.as_deref(), &vis)
            .await?;

        // Refuse to proceed if the snapshot is truncated: we cannot guarantee
        // we have seen every entity matching the filter, so the operation
        // could leave behind entities the caller intended to remove. The
        // branch view answers for itself rather than borrowing the baseline
        // flag, which a branch load never sets.
        if truncated {
            return Err(Status::failed_precondition(format!(
                "delete_by_query refused: the entity snapshot is truncated at {MAX_PROJECTION_ENTITIES} entries; \
                 the operation cannot guarantee completeness: reduce the store size or \
                 contact the administrator",
            )));
        }

        let kind_filter = pb::EntityKind::try_from(req.kind).ok();
        let target_kind = kind_filter.filter(|k| *k != pb::EntityKind::Unspecified);

        let mut victims: Vec<(pb::EntityKind, pb::Id)> = Vec::new();
        for se in all.iter() {
            let Ok(k) = entity_kind(&se.entity) else {
                continue;
            };
            let Ok(id) = entity_id(&se.entity) else {
                continue;
            };
            if let Some(want) = target_kind {
                if k != want {
                    continue;
                }
            }
            if !req.namespace.is_empty() && id.namespace != req.namespace {
                continue;
            }
            if !req.slug.is_empty() && id.slug != req.slug {
                continue;
            }
            // project filter: match if any Domain in id's namespace
            // chain belongs to the requested project. For now we treat
            // project as a namespace-style filter: entities whose
            // namespace equals the project name pass. The Project
            // entity itself adds richer scoping later.
            if !req.project.is_empty() && id.namespace != req.project {
                continue;
            }
            victims.push((k, id.clone()));
        }

        // max_deletes=0 is rejected above; any positive value acts as a cap.
        // max_deletes is validated to be > 0 earlier in this handler, so the
        // try_from conversion always succeeds for well-formed requests.
        let max_deletes_usize = usize::try_from(req.max_deletes).unwrap_or(usize::MAX);
        if victims.len() > max_deletes_usize {
            return Err(Status::failed_precondition(format!(
                "delete_by_query would remove {} entities; limit is max_deletes={}",
                victims.len(),
                req.max_deletes,
            )));
        }

        // Same default as DeleteEntity: MODE_UNSPECIFIED (protobuf 0) means
        // FAIL_IF_REFERENCED. Without this remap, mode=0 skips the referrer
        // gate and force-deletes (DeleteByQueryRequest.mode documents the
        // shared DeleteEntityRequest.Mode semantics).
        let mode = pb::delete_entity_request::Mode::try_from(req.mode)
            .unwrap_or(pb::delete_entity_request::Mode::FailIfReferenced);
        let mode = if matches!(mode, pb::delete_entity_request::Mode::Unspecified) {
            pb::delete_entity_request::Mode::FailIfReferenced
        } else {
            mode
        };
        let dry_run = mode == pb::delete_entity_request::Mode::DryRun;

        // For FAIL_IF_REFERENCED mode, find any entities outside the victim
        // set that hold references into it. Intra-set references are
        // intentionally ignored: we are deleting those too.
        if mode == pb::delete_entity_request::Mode::FailIfReferenced {
            let victim_set: std::collections::HashSet<(i32, String, String)> = victims
                .iter()
                .map(|(k, id)| (*k as i32, id.namespace.clone(), id.slug.clone()))
                .collect();
            let index = graph::ReverseIndex::build(&all);
            let mut external_referrers: Vec<String> = Vec::new();
            for (k, id) in &victims {
                for r in index.incoming(*k, id) {
                    let Some(from) = r.from.as_ref() else {
                        continue;
                    };
                    let Some(from_id) = from.id.as_ref() else {
                        continue;
                    };
                    let key = (from.kind, from_id.namespace.clone(), from_id.slug.clone());
                    if !victim_set.contains(&key) {
                        external_referrers.push(format!("{}/{}", from_id.namespace, from_id.slug));
                    }
                }
            }
            external_referrers.sort();
            external_referrers.dedup();
            if !external_referrers.is_empty() {
                return Err(Status::failed_precondition(format!(
                    "delete_by_query: {} entities outside the victim set reference victims: {}; \
                     pass mode=FORCE to delete anyway",
                    external_referrers.len(),
                    external_referrers.join(", ")
                )));
            }
        }

        let deleted: Vec<Deleted> = victims
            .iter()
            .map(|(k, id)| Deleted {
                kind: *k as i32,
                id: Some(id.clone()),
            })
            .collect();

        if !dry_run && !victims.is_empty() {
            let ops: Vec<MutationOp> = victims
                .iter()
                .map(|(k, id)| MutationOp::Delete {
                    kind: *k,
                    id: id.clone(),
                    if_match: None,
                })
                .collect();
            let metas: Vec<OpMetaLike> = victims
                .iter()
                .map(|(k, id)| OpMetaLike {
                    kind: *k,
                    id: id.clone(),
                    is_put: false,
                    entity: None,
                })
                .collect();
            let mut changeset = PendingChangeset::mint("DeleteByQuery", &author, branch.as_deref());
            self.store
                .batch_apply(&ops, changeset.write_ctx())
                .await
                .map_err(store_err)?;
            let outcomes: Vec<MutationOutcome> =
                victims.iter().map(|_| MutationOutcome::Deleted).collect();
            changeset.record_batch(&metas, &outcomes);
            self.record_changeset(changeset, git_mirror::batch_commit_message(outcomes.len()))
                .await;
            // Tombstones on a branch touched no baseline row and no baseline
            // cache, so they skip every baseline side effect exactly as an
            // ordinary branch delete does.
            if branch.is_none() {
                self.apply_batch_side_effects(&metas, &outcomes, author)
                    .await;
                self.invalidate_snapshot().await;
            }
        }

        // Saturating cast: deleted.len() is bounded by victims.len() which we
        // checked against `max_deletes: i32` above, but be explicit so the
        // proto count cannot wrap into a negative number.
        let count = i32::try_from(deleted.len()).unwrap_or(i32::MAX);
        Ok(Response::new(pb::DeleteByQueryResponse {
            deleted,
            deleted_count: count,
            dry_run,
        }))
    }

    async fn list_entity_kinds(
        &self,
        _req: Request<pb::ListEntityKindsRequest>,
    ) -> Result<Response<pb::ListEntityKindsResponse>, Status> {
        Ok(Response::new(pb::ListEntityKindsResponse {
            kinds: validation::entity_kind_catalog(),
        }))
    }

    async fn list_entities_by_domain(
        &self,
        req: Request<pb::ListEntitiesByDomainRequest>,
    ) -> Result<Response<pb::ListEntitiesByDomainResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let dom_id = req
            .domain_id
            .ok_or_else(|| Status::invalid_argument("domain_id is required"))?;
        let kinds: std::collections::HashSet<i32> = req.kinds.iter().copied().collect();
        let page_size = clamp_page_size(
            req.page_size,
            ADVERTISED_DEFAULT_PAGE_SIZE,
            ADVERTISED_MAX_PAGE_SIZE,
        );
        let offset = parse_page_token(&req.page_token)?;

        let all = self.snapshot_for(branch.as_deref(), &vis).await?;

        let (subdomain_domain, ns_to_dom) = build_domain_maps(all.as_ref());
        let target = (dom_id.namespace.clone(), dom_id.slug.clone());

        let mut matched: Vec<pb::Entity> = Vec::new();
        for se in all.iter() {
            let Ok(id) = entity_id(&se.entity) else {
                continue;
            };
            let Ok(k) = entity_kind(&se.entity) else {
                continue;
            };
            if !kinds.is_empty() && !kinds.contains(&(k as i32)) {
                continue;
            }
            // The Domain itself, its Subdomains, and its BCs always belong;
            // everything else has to chain back via the BC namespace.
            let belongs = match k {
                pb::EntityKind::Domain => id.namespace == target.0 && id.slug == target.1,
                pb::EntityKind::Subdomain => subdomain_domain
                    .get(&(id.namespace.clone(), id.slug.clone()))
                    .is_some_and(|d| d == &target),
                pb::EntityKind::BoundedContext => {
                    ns_to_dom.get(&id.namespace).is_some_and(|d| d == &target)
                }
                _ => ns_to_dom.get(&id.namespace).is_some_and(|d| d == &target),
            };
            if belongs {
                matched.push(se.entity.clone());
            }
        }
        let total = matched.len();
        let entities: Vec<pb::Entity> = matched.into_iter().skip(offset).take(page_size).collect();
        let next_offset = offset + entities.len();
        let next_page_token = if next_offset >= total {
            String::new()
        } else {
            encode_page_token(next_offset)
        };
        Ok(Response::new(pb::ListEntitiesByDomainResponse {
            entities,
            next_page_token,
        }))
    }

    async fn infer_data_flow(
        &self,
        req: Request<pb::InferDataFlowRequest>,
    ) -> Result<Response<pb::InferDataFlowResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let scope = req
            .scope
            .ok_or_else(|| Status::invalid_argument("scope is required"))?;
        // Validate the scope's entity reference before loading a snapshot so
        // malformed or hostile ids are rejected at the boundary.
        let Some(
            pb::analysis_scope::Scope::Slice(scope_ref)
            | pb::analysis_scope::Scope::Storyboard(scope_ref)
            | pb::analysis_scope::Scope::EventModel(scope_ref),
        ) = scope.scope.as_ref()
        else {
            return Err(Status::invalid_argument("scope.scope is required"));
        };
        let _ = kind_from_i32(scope_ref.kind, "scope.kind")?;
        let _ = require_id(scope_ref.id.as_ref(), "scope")?;
        let all = self.snapshot_for(branch.as_deref(), &vis).await?;
        let jev_fallback = if let Some(analyzer) = self.jev_analyzer.as_ref() {
            use tracing::Instrument as _;
            match analyzer
                .infer_data_flow(&scope, all.as_ref())
                .instrument(tracing::info_span!("jev_evaluate", rpc = "infer_data_flow"))
                .await
            {
                Ok(result) => {
                    let provenance = if result.evaluated {
                        pb::AnalysisProvenance::Llm
                    } else {
                        pb::AnalysisProvenance::Deterministic
                    };
                    return Ok(Response::new(pb::InferDataFlowResponse {
                        mappings: result.mappings,
                        provenance: provenance as i32,
                        fallback_reason: String::new(),
                    }));
                }
                Err(error) => {
                    let reason = error.reason();
                    tracing::warn!(reason, "Jev inference unavailable; using analysis fallback");
                    Some(reason)
                }
            }
        } else {
            None
        };
        let (mappings, provenance, mut fallback_reason) = match self.llm_analyzer.as_ref() {
            Some(analyzer) => {
                use tracing::Instrument as _;
                let result = analyzer
                    .infer_data_flow(&scope, all.as_ref())
                    .instrument(tracing::info_span!("llm_call", rpc = "infer_data_flow"))
                    .await;
                match result {
                    Ok(m) => (m, pb::AnalysisProvenance::Llm, String::new()),
                    Err(err) => {
                        let reason = llm_fallback_reason(&err);
                        tracing::warn!(error = %err, reason, "llm infer_data_flow failed, falling back to deterministic");
                        (
                            analysis::infer_data_flow(&scope, all.as_ref()),
                            pb::AnalysisProvenance::Deterministic,
                            reason.to_owned(),
                        )
                    }
                }
            }
            None => (
                analysis::infer_data_flow(&scope, all.as_ref()),
                pb::AnalysisProvenance::Deterministic,
                String::new(),
            ),
        };
        if let Some(reason) = jev_fallback {
            fallback_reason = if fallback_reason.is_empty() {
                reason.to_owned()
            } else {
                format!("{reason},{fallback_reason}")
            };
        }
        Ok(Response::new(pb::InferDataFlowResponse {
            mappings,
            provenance: provenance as i32,
            fallback_reason,
        }))
    }

    async fn check_information_completeness(
        &self,
        req: Request<pb::CheckInformationCompletenessRequest>,
    ) -> Result<Response<pb::CheckInformationCompletenessResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let branch = Self::branch_from_metadata(&req)?;
        let req = req.into_inner();
        let scope = req
            .scope
            .ok_or_else(|| Status::invalid_argument("scope is required"))?;
        // Validate the scope's entity reference before loading a snapshot.
        let Some(
            pb::analysis_scope::Scope::Slice(scope_ref)
            | pb::analysis_scope::Scope::Storyboard(scope_ref)
            | pb::analysis_scope::Scope::EventModel(scope_ref),
        ) = scope.scope.as_ref()
        else {
            return Err(Status::invalid_argument("scope.scope is required"));
        };
        let _ = kind_from_i32(scope_ref.kind, "scope.kind")?;
        let _ = require_id(scope_ref.id.as_ref(), "scope")?;
        let all = self.snapshot_for(branch.as_deref(), &vis).await?;
        let ((gaps, overall_score), provenance, fallback_reason) = match self.llm_analyzer.as_ref()
        {
            Some(analyzer) => {
                use tracing::Instrument as _;
                let result = analyzer
                    .check_completeness(&scope, all.as_ref())
                    .instrument(tracing::info_span!("llm_call", rpc = "check_completeness"))
                    .await;
                match result {
                    Ok(out) => (out, pb::AnalysisProvenance::Llm, String::new()),
                    Err(err) => {
                        let reason = llm_fallback_reason(&err);
                        tracing::warn!(error = %err, reason, "llm check_completeness failed, falling back to deterministic");
                        (
                            analysis::check_completeness(&scope, all.as_ref()),
                            pb::AnalysisProvenance::Deterministic,
                            reason.to_owned(),
                        )
                    }
                }
            }
            None => (
                analysis::check_completeness(&scope, all.as_ref()),
                pb::AnalysisProvenance::Deterministic,
                String::new(),
            ),
        };
        Ok(Response::new(pb::CheckInformationCompletenessResponse {
            gaps,
            overall_score,
            provenance: provenance as i32,
            fallback_reason,
        }))
    }

    // ---------- Branching (Phase 1: Isolation) -----------------------------
    //
    // Branch lifecycle only; branch *scope* for every other RPC travels via
    // the `x-trogon-atlas-branch` metadata header (see
    // `branch_from_metadata`), never through these request messages.

    async fn create_branch(
        &self,
        req: Request<pb::CreateBranchRequest>,
    ) -> Result<Response<pb::CreateBranchResponse>, Status> {
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();
        validate_branch_name(&req.name)?;
        Self::authorize_branch_write(principal.as_ref(), &req.name, "creating a branch")?;
        let info = self
            .store
            .create_branch(&req.name, &req.doc)
            .await
            .map_err(store_err)?;
        Ok(Response::new(pb::CreateBranchResponse {
            branch: Some(branch_info_to_pb(info)),
        }))
    }

    // ---------- Branching (Phase 2: Review and merge) ----------------------

    async fn diff_branch(
        &self,
        req: Request<pb::DiffBranchRequest>,
    ) -> Result<Response<pb::DiffBranchResponse>, Status> {
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();
        validate_branch_name(&req.name)?;
        if !Self::branch_visible(principal.as_ref(), &req.name) {
            return Err(branch_not_found(&req.name));
        }
        let info = self.require_branch(&req.name).await?;
        let deltas = self
            .store
            .list_branch_deltas(&req.name)
            .await
            .map_err(store_err)?;
        let mut entries = Vec::with_capacity(deltas.len());
        for delta in deltas {
            let (entry, _theirs_etag) = self.classify_branch_delta(delta).await?;
            entries.push(entry);
        }
        let ancestry = self.branch_ancestry(&info).await?;
        Ok(Response::new(pb::DiffBranchResponse {
            entries,
            ancestry: Some(ancestry),
        }))
    }

    async fn merge_branch(
        &self,
        req: Request<pb::MergeBranchRequest>,
    ) -> Result<Response<pb::MergeBranchResponse>, Status> {
        let vis = Self::visibility_of(&req);
        let author = Self::author_from_metadata(&req);
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();
        validate_branch_name(&req.name)?;
        // Clears only the ownership-delegation gate: every namespace the
        // merge diff touches still has to clear this principal's own write
        // scope below, via `require_namespace_scope` over `targets`.
        Self::authorize_branch_write(principal.as_ref(), &req.name, "merging a branch")?;
        operation_receipts::reject_operation_id_with_dry_run(&req.operation_id, req.dry_run)?;
        if !self
            .store
            .list_branches()
            .await
            .map_err(store_err)?
            .iter()
            .any(|b| b.name == req.name)
        {
            return Err(branch_not_found(&req.name));
        }

        let _guard = self.lock_mutations().await;

        let deltas = self
            .store
            .list_branch_deltas(&req.name)
            .await
            .map_err(store_err)?;
        let mut classified = Vec::with_capacity(deltas.len());
        for delta in deltas {
            let (mut entry, theirs_etag) = self.classify_branch_delta(delta).await?;
            if req.auto_merge {
                accept_structural_merge(&mut entry, theirs_etag.as_deref());
            }
            classified.push(entry);
        }

        let conflicts: Vec<pb::BranchDiffEntry> = classified
            .iter()
            .filter(|e| is_conflict_status(e.status))
            .cloned()
            .collect();
        if !conflicts.is_empty() {
            return Ok(Response::new(pb::MergeBranchResponse {
                status: pb::merge_branch_response::Status::Conflicts as i32,
                conflicts,
                validation: Vec::new(),
                applied_count: 0,
                operation_receipt: None,
            }));
        }

        // CONVERGED entries are dropped silently: nothing to land, and
        // `UpdateBranch`/a future diff will see them collapse once the
        // delta rows are cleaned up. Only ADDED/CHANGED/DELETED land.
        let landable: Vec<&pb::BranchDiffEntry> = classified
            .iter()
            .filter(|e| {
                matches!(
                    branch_diff_entry_status(e.status),
                    pb::branch_diff_entry::Status::Added
                        | pb::branch_diff_entry::Status::Changed
                        | pb::branch_diff_entry::Status::Deleted
                )
            })
            .collect();

        let mut ops: Vec<trogon_atlas_store::store::BranchLandOp> =
            Vec::with_capacity(landable.len());
        let mut op_metas: Vec<OpMetaLike> = Vec::with_capacity(landable.len());
        for entry in &landable {
            let entity_ref = entry
                .r#ref
                .as_ref()
                .ok_or_else(|| Status::internal("branch diff entry missing ref"))?;
            let kind = kind_from_i32(entity_ref.kind, "conflicts[].ref.kind")?;
            let id = require_id(entity_ref.id.as_ref(), "conflicts[].ref")?.clone();
            if branch_diff_entry_status(entry.status) == pb::branch_diff_entry::Status::Deleted {
                ops.push(trogon_atlas_store::store::BranchLandOp::Delete {
                    kind,
                    id: id.clone(),
                    expected_baseline_etag: entry.base_etag.clone(),
                });
                op_metas.push(OpMetaLike {
                    kind,
                    id,
                    is_put: false,
                    entity: None,
                });
            } else {
                let ours = entry
                    .ours
                    .clone()
                    .ok_or_else(|| Status::internal("branch diff entry missing ours"))?;
                let expected_baseline_etag = if entry.base.is_some() {
                    Some(entry.base_etag.clone())
                } else {
                    None
                };
                ops.push(trogon_atlas_store::store::BranchLandOp::Put {
                    kind,
                    ours: Box::new(ours.clone()),
                    expected_baseline_etag,
                });
                op_metas.push(OpMetaLike {
                    kind,
                    id,
                    is_put: true,
                    entity: Some(ours),
                });
            }
        }

        // Merging is the sanctioned way onto baseline, so it deliberately
        // does not consult baseline protection. The caller's namespace scope
        // still applies: a merge is a baseline write, and a scoped principal
        // must not be able to launder one through a branch.
        let mut targets = WriteTargets::none();
        for meta in &op_metas {
            targets.insert(&meta.id.namespace);
        }
        self.require_namespace_scope(principal.as_ref(), &targets, &NamespaceTenure::Permanent)
            .await?;

        let mut type_library_changes = crate::type_libraries::TypeLibraryChanges::default();
        for meta in &op_metas {
            match meta.entity.as_ref().filter(|_| meta.is_put) {
                Some(entity) => type_library_changes.put(entity),
                None => type_library_changes.delete(meta.kind, &meta.id),
            }
        }
        self.type_libraries
            .verify(&*self.store, &type_library_changes, None)
            .await?;
        self.reject_type_library_users(None, &vis, &op_metas)
            .await?;
        let merged_libraries: Vec<&pb::TypeLibrary> = op_metas
            .iter()
            .filter(|m| m.is_put)
            .filter_map(|m| match m.entity.as_ref()?.kind.as_ref()? {
                pb::entity::Kind::TypeLibrary(library) => Some(library),
                _ => None,
            })
            .collect();
        for meta in op_metas.iter().filter(|m| m.is_put) {
            if let Some(entity) = meta.entity.as_ref() {
                self.type_libraries
                    .check_schema(&*self.store, entity, &merged_libraries, None)
                    .await?;
            }
        }

        if req.dry_run {
            // Full detection already ran above; validate the post-merge
            // world via an in-memory overlay without persisting anything.
            let pre_all = self.snapshot(&vis).await?;
            let overlay_mutations: Vec<MutationOp> = op_metas
                .iter()
                .map(|meta| match &meta.entity {
                    Some(entity) if meta.is_put => MutationOp::Put {
                        kind: meta.kind,
                        entity: entity.clone(),
                        if_match: None,
                        force: true,
                    },
                    _ => MutationOp::Delete {
                        kind: meta.kind,
                        id: meta.id.clone(),
                        if_match: None,
                    },
                })
                .collect();
            let post_world: Vec<StoredEntity> = match dry_run::apply(&pre_all, &overlay_mutations) {
                Ok(outcomes) => {
                    let mut world: Vec<StoredEntity> = (*pre_all).clone();
                    let mut by_key: std::collections::HashMap<pb::EntityKey, usize> =
                        std::collections::HashMap::new();
                    for (i, se) in world.iter().enumerate() {
                        if let (Some(k), Some(id)) = (
                            trogon_atlas_store::refs::entity_kind(&se.entity),
                            trogon_atlas_store::refs::entity_id(&se.entity),
                        ) {
                            by_key.insert(pb::EntityKey::new(k, id), i);
                        }
                    }
                    for (meta, oc) in op_metas.iter().zip(outcomes.iter()) {
                        let key = pb::EntityKey::new(meta.kind, &meta.id);
                        match oc {
                            MutationOutcome::Wrote { entity, etag } => {
                                let se = StoredEntity {
                                    entity: entity.clone(),
                                    etag: etag.clone(),
                                };
                                if let Some(&i) = by_key.get(&key) {
                                    world[i] = se;
                                } else {
                                    by_key.insert(key, world.len());
                                    world.push(se);
                                }
                            }
                            MutationOutcome::Noop { .. } => {}
                            MutationOutcome::Deleted => {
                                if let Some(&i) = by_key.get(&key) {
                                    world.remove(i);
                                    by_key.remove(&key);
                                    for v in by_key.values_mut() {
                                        if *v > i {
                                            *v -= 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    world
                }
                Err(_) => (*pre_all).clone(),
            };
            let validation_issues = validation::validate_store(&post_world);
            let has_error = validation_issues
                .iter()
                .any(|i| i.severity == pb::validation_issue::Severity::Error as i32);
            if has_error {
                return Ok(Response::new(pb::MergeBranchResponse {
                    status: pb::merge_branch_response::Status::Invalid as i32,
                    conflicts: Vec::new(),
                    validation: validation_issues,
                    applied_count: 0,
                    operation_receipt: None,
                }));
            }
            return Ok(Response::new(pb::MergeBranchResponse {
                status: pb::merge_branch_response::Status::Validated as i32,
                conflicts: Vec::new(),
                validation: Vec::new(),
                applied_count: u32::try_from(landable.len()).unwrap_or(u32::MAX),
                operation_receipt: None,
            }));
        }

        // Real merge: land verbatim, then validate the *actual* post-merge
        // world. If invalid, nothing was persisted -- `land_branch_merge`
        // has not been called yet, so there is nothing to roll back.
        {
            let pre_all = self.snapshot(&vis).await?;
            let mut projected: Vec<StoredEntity> = (*pre_all).clone();
            let mut by_key: std::collections::HashMap<pb::EntityKey, usize> =
                std::collections::HashMap::new();
            for (i, se) in projected.iter().enumerate() {
                if let (Some(k), Some(id)) = (
                    trogon_atlas_store::refs::entity_kind(&se.entity),
                    trogon_atlas_store::refs::entity_id(&se.entity),
                ) {
                    by_key.insert(pb::EntityKey::new(k, id), i);
                }
            }
            for meta in &op_metas {
                let key = pb::EntityKey::new(meta.kind, &meta.id);
                let Some(entity) = meta.entity.clone().filter(|_| meta.is_put) else {
                    if let Some(&i) = by_key.get(&key) {
                        projected.remove(i);
                        by_key.remove(&key);
                        for v in by_key.values_mut() {
                            if *v > i {
                                *v -= 1;
                            }
                        }
                    }
                    continue;
                };
                let se = StoredEntity {
                    entity,
                    etag: String::new(),
                };
                if let Some(&i) = by_key.get(&key) {
                    projected[i] = se;
                } else {
                    by_key.insert(key, projected.len());
                    projected.push(se);
                }
            }
            let validation_issues = validation::validate_store(&projected);
            let has_error = validation_issues
                .iter()
                .any(|i| i.severity == pb::validation_issue::Severity::Error as i32);
            if has_error {
                return Ok(Response::new(pb::MergeBranchResponse {
                    status: pb::merge_branch_response::Status::Invalid as i32,
                    conflicts: Vec::new(),
                    validation: validation_issues,
                    applied_count: 0,
                    operation_receipt: None,
                }));
            }
        }

        // Claimed here, past every validation failure above (conflicts,
        // dry-run/real invalid post-merge world) and ahead of the
        // nothing-to-land check below: "nothing to land" is a sibling
        // success outcome to the real merge, not a validation failure, so
        // it is settled Applied (with an empty changeset_id) the same way.
        let attempt = if req.operation_id.is_empty() {
            None
        } else {
            let principal_name = principal
                .as_ref()
                .map_or(UNAUTHENTICATED_PRINCIPAL, |p| p.name.as_ref());
            let digest_req = pb::MergeBranchRequest {
                name: req.name.clone(),
                dry_run: false,
                auto_merge: req.auto_merge,
                keep_branch: req.keep_branch,
                operation_id: String::new(),
            };
            Some(operation_receipts::OperationAttempt::new(
                principal_name,
                &req.operation_id,
                "MergeBranch",
                None,
                "trogonatlas.api.eventmodel.v1alpha1.MergeBranchRequest",
                &digest_req,
            )?)
        };

        let claim_key = match &attempt {
            None => None,
            Some(attempt) => {
                match operation_receipts::claim(&*self.store, attempt, "MergeBranch", None).await? {
                    operation_receipts::Claim::Proceed => Some(attempt.claim_key.clone()),
                    operation_receipts::Claim::Replay(record) => {
                        if record.status == OperationStatus::Rejected {
                            return Err(operation_receipts::replay_rejected_status(&record));
                        }
                        // Applied: the merge already landed (or there was
                        // nothing to land). `applied_count` from the
                        // original call is not retained on the receipt, so
                        // it is reported as 0 here; `changeset_id` is the
                        // caller's proof the merge is settled.
                        return Ok(Response::new(pb::MergeBranchResponse {
                            status: pb::merge_branch_response::Status::Applied as i32,
                            conflicts: Vec::new(),
                            validation: Vec::new(),
                            applied_count: 0,
                            operation_receipt: Some(operation_receipts::replay_receipt(
                                &req.operation_id,
                                &record,
                            )),
                        }));
                    }
                }
            }
        };

        if ops.is_empty() {
            // Nothing to land (everything was CONVERGED). Still a
            // successful merge: clean up the branch per keep_branch.
            if req.keep_branch {
                self.catch_up_kept_branch(&req.name, None).await?;
            } else {
                self.store
                    .delete_branch(&req.name)
                    .await
                    .map_err(store_err)?;
            }
            self.settle_provisional_namespaces(&req.name).await?;
            if let Some(key) = &claim_key {
                operation_receipts::settle_applied(&*self.store, key, "").await;
            }
            return Ok(Response::new(pb::MergeBranchResponse {
                status: pb::merge_branch_response::Status::Applied as i32,
                conflicts: Vec::new(),
                validation: Vec::new(),
                applied_count: 0,
                operation_receipt: claim_key
                    .as_ref()
                    .map(|_| operation_receipts::fresh_receipt(&req.operation_id, "")),
            }));
        }

        // The merge lands on the baseline, so the changeset is a baseline
        // changeset: `branch` stays empty and the ops are the entities that
        // actually moved, not the branch's whole overlay.
        let mut changeset = PendingChangeset::mint("MergeBranch", &author, None)
            .with_operation_id(claim_key.as_deref().map(|_| req.operation_id.as_str()))
            .with_operation_key(claim_key.as_deref());
        let merge_changeset_id = changeset.id.clone();
        let outcomes = match self
            .store
            .land_branch_merge(&req.name, &ops, changeset.write_ctx())
            .await
        {
            Ok(o) => o,
            // Same partial-apply exception as `batch_mutate`: leave the
            // claim Pending and let `nats_journal`'s crash-recovery pass
            // settle it once it knows what actually landed.
            Err(err) if err.is_partial_apply() => return Err(store_err(err)),
            Err(err) => {
                let status = store_err(err);
                if let Some(key) = &claim_key {
                    operation_receipts::settle_for_status(&*self.store, key, &status).await;
                }
                return Err(status);
            }
        };
        if let Some(key) = &claim_key {
            operation_receipts::settle_applied(&*self.store, key, &merge_changeset_id).await;
        }
        changeset.record_batch(&op_metas, &outcomes);
        self.record_changeset(
            changeset,
            format!(
                "merge branch {name}: {count} entities",
                name = req.name,
                count = outcomes.len()
            ),
        )
        .await;
        self.apply_batch_side_effects(&op_metas, &outcomes, author)
            .await;
        self.invalidate_snapshot().await;

        if req.keep_branch {
            self.catch_up_kept_branch(&req.name, Some(&merge_changeset_id))
                .await?;
        } else {
            self.store
                .delete_branch(&req.name)
                .await
                .map_err(store_err)?;
        }
        self.settle_provisional_namespaces(&req.name).await?;

        Ok(Response::new(pb::MergeBranchResponse {
            status: pb::merge_branch_response::Status::Applied as i32,
            conflicts: Vec::new(),
            validation: Vec::new(),
            applied_count: u32::try_from(ops.len()).unwrap_or(u32::MAX),
            operation_receipt: claim_key
                .as_ref()
                .map(|_| operation_receipts::fresh_receipt(&req.operation_id, &merge_changeset_id)),
        }))
    }

    async fn update_branch(
        &self,
        req: Request<pb::UpdateBranchRequest>,
    ) -> Result<Response<pb::UpdateBranchResponse>, Status> {
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();
        validate_branch_name(&req.name)?;
        Self::authorize_branch_write(principal.as_ref(), &req.name, "rebasing a branch")?;
        let info = self.require_branch(&req.name).await?;

        let _guard = self.lock_mutations().await;

        // Read the target BEFORE rebasing, so the pointer never advances past
        // a baseline write that landed while this call was running.
        let caught_up_to = self
            .store
            .newest_baseline_changeset()
            .await
            .map_err(store_err)?;

        let deltas = self
            .store
            .list_branch_deltas(&req.name)
            .await
            .map_err(store_err)?;
        let mut classified: Vec<(pb::BranchDiffEntry, Option<String>)> =
            Vec::with_capacity(deltas.len());
        for delta in deltas {
            classified.push(self.classify_branch_delta(delta).await?);
        }

        let conflicts: Vec<pb::BranchDiffEntry> = classified
            .iter()
            .map(|(e, _)| e)
            .filter(|e| is_conflict_status(e.status))
            .cloned()
            .collect();

        let mut rebases: Vec<(pb::EntityKind, pb::Id, Option<(pb::Entity, String)>)> = Vec::new();
        for (entry, theirs_etag) in &classified {
            if is_conflict_status(entry.status) {
                continue;
            }
            let entity_ref = entry
                .r#ref
                .as_ref()
                .ok_or_else(|| Status::internal("branch diff entry missing ref"))?;
            let kind = kind_from_i32(entity_ref.kind, "entry.ref.kind")?;
            let id = require_id(entity_ref.id.as_ref(), "entry.ref")?.clone();
            match branch_diff_entry_status(entry.status) {
                pb::branch_diff_entry::Status::Converged => {
                    rebases.push((kind, id, None));
                }
                pb::branch_diff_entry::Status::Added => {
                    // No baseline row yet; nothing to rebase against.
                }
                _ => {
                    // Rebase to the CURRENT baseline etag (read fresh in
                    // `classify_branch_delta`), not the stale one recorded
                    // when the delta was created -- otherwise the delta's
                    // `base_etag` would go out of sync with `theirs` and
                    // corrupt the optimistic-concurrency guard on the next
                    // `MergeBranch`.
                    if let (Some(theirs), Some(etag)) = (entry.theirs.clone(), theirs_etag.clone())
                    {
                        rebases.push((kind, id, Some((theirs, etag))));
                    }
                }
            }
        }

        let rebased_count = if rebases.is_empty() {
            0
        } else {
            self.store
                .rebase_branch_deltas(&req.name, &rebases)
                .await
                .map_err(store_err)?
        };

        // The fork point describes the branch as a whole, so it advances only
        // when the whole branch caught up. With a conflict still open some
        // baseline write has NOT been reconciled, and moving the pointer past
        // it would erase the evidence.
        let fork_changeset_id = match (&caught_up_to, conflicts.is_empty()) {
            (Some(target), true) => {
                self.store
                    .set_branch_fork_point(&req.name, target)
                    .await
                    .map_err(store_err)?;
                target.clone()
            }
            _ => info.fork_changeset_id.unwrap_or_default(),
        };

        Ok(Response::new(pb::UpdateBranchResponse {
            rebased_count,
            conflicts,
            fork_changeset_id,
        }))
    }

    async fn resolve_branch_entry(
        &self,
        req: Request<pb::ResolveBranchEntryRequest>,
    ) -> Result<Response<pb::ResolveBranchEntryResponse>, Status> {
        let author = Self::author_from_metadata(&req);
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();
        validate_branch_name(&req.name)?;
        Self::authorize_branch_write(principal.as_ref(), &req.name, "resolving a branch conflict")?;
        if !self
            .store
            .list_branches()
            .await
            .map_err(store_err)?
            .iter()
            .any(|b| b.name == req.name)
        {
            return Err(branch_not_found(&req.name));
        }
        let entity_ref = req
            .r#ref
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("ref is required"))?;
        let kind = kind_from_i32(entity_ref.kind, "ref.kind")?;
        let id = require_id(entity_ref.id.as_ref(), "ref")?.clone();
        let resolution = pb::resolve_branch_entry_request::Resolution::try_from(req.resolution)
            .map_err(|_| Status::invalid_argument("unknown resolution"))?;
        if matches!(
            resolution,
            pb::resolve_branch_entry_request::Resolution::Unspecified
        ) {
            return Err(Status::invalid_argument(
                "resolution must not be UNSPECIFIED",
            ));
        }
        let take_theirs = matches!(
            resolution,
            pb::resolve_branch_entry_request::Resolution::TakeTheirs
        );

        let _guard = self.lock_mutations().await;

        if let Some(expected) = req.expected_state.as_ref() {
            self.require_branch_entry_state(&req.name, kind, &id, expected)
                .await?;
        }

        let attempt = if req.operation_id.is_empty() {
            None
        } else {
            let principal_name = principal
                .as_ref()
                .map_or(UNAUTHENTICATED_PRINCIPAL, |p| p.name.as_ref());
            let digest_req = pb::ResolveBranchEntryRequest {
                name: req.name.clone(),
                r#ref: req.r#ref.clone(),
                resolution: req.resolution,
                expected_state: req.expected_state.clone(),
                operation_id: String::new(),
            };
            Some(operation_receipts::OperationAttempt::new(
                principal_name,
                &req.operation_id,
                "ResolveBranchEntry",
                Some(req.name.as_str()),
                "trogonatlas.api.eventmodel.v1alpha1.ResolveBranchEntryRequest",
                &digest_req,
            )?)
        };

        let claim_key = match &attempt {
            None => None,
            Some(attempt) => {
                match operation_receipts::claim(
                    &*self.store,
                    attempt,
                    "ResolveBranchEntry",
                    Some(req.name.as_str()),
                )
                .await?
                {
                    operation_receipts::Claim::Proceed => Some(attempt.claim_key.clone()),
                    operation_receipts::Claim::Replay(record) => {
                        if record.status == OperationStatus::Rejected {
                            return Err(operation_receipts::replay_rejected_status(&record));
                        }
                        return Ok(Response::new(pb::ResolveBranchEntryResponse {
                            operation_receipt: Some(operation_receipts::replay_receipt(
                                &req.operation_id,
                                &record,
                            )),
                        }));
                    }
                }
            }
        };

        // Taking the merge is "keep ours" with a different `ours`: write the
        // merged entity onto the branch first, then let the existing rebase
        // carry `base`/`base_etag` forward so the conflict clears. Both steps
        // are branch-local, so nothing here is visible from baseline.
        let mut receipt_changeset_id = String::new();
        if matches!(
            resolution,
            pb::resolve_branch_entry_request::Resolution::TakeMerged
        ) {
            let delta = self
                .store
                .list_branch_deltas(&req.name)
                .await
                .map_err(store_err)?
                .into_iter()
                .find(|d| d.kind == kind && d.id == id)
                .ok_or_else(|| {
                    Status::not_found(format!("branch {:?} has no delta for this key", req.name))
                })?;
            let (entry, _theirs_etag) = self.classify_branch_delta(delta).await?;
            let merged = entry.auto_merged.ok_or_else(|| {
                Status::failed_precondition(
                    "this entry has no structural merge to take: either the two sides changed the \
                     same field paths, or it is not an edit/edit conflict",
                )
            })?;
            let changeset =
                PendingChangeset::mint("ResolveBranchEntry", &author, Some(req.name.as_str()))
                    .with_operation_id(claim_key.as_deref().map(|_| req.operation_id.as_str()))
                    .with_operation_key(claim_key.as_deref());
            receipt_changeset_id = changeset.id.clone();
            if let Err(err) = self
                .store
                .put(kind, &merged, true, changeset.write_ctx())
                .await
            {
                let status = store_err(err);
                if let Some(key) = &claim_key {
                    operation_receipts::settle_for_status(&*self.store, key, &status).await;
                }
                return Err(status);
            }
        }

        if let Err(err) = self
            .store
            .resolve_branch_entry(&req.name, kind, &id, take_theirs)
            .await
        {
            let status = store_err(err);
            if let Some(key) = &claim_key {
                if receipt_changeset_id.is_empty() {
                    operation_receipts::settle_for_status(&*self.store, key, &status).await;
                } else {
                    // TakeMerged's write already landed; this step only
                    // clears the conflict marker. Settling not-applied
                    // (rather than rejected, which would replay a write
                    // that already happened on every retry) lets a retry
                    // redo the cheap half instead of getting stuck.
                    tracing::warn!(
                        error = %status,
                        changeset_id = %receipt_changeset_id,
                        "resolve_branch_entry: merged entity was written but clearing the \
                         conflict failed; settling operation as not-applied"
                    );
                    operation_receipts::settle_not_applied(&*self.store, key).await;
                }
            }
            return Err(status);
        }

        if let Some(key) = &claim_key {
            operation_receipts::settle_applied(&*self.store, key, &receipt_changeset_id).await;
        }
        Ok(Response::new(pb::ResolveBranchEntryResponse {
            operation_receipt: claim_key.as_ref().map(|_| {
                operation_receipts::fresh_receipt(&req.operation_id, &receipt_changeset_id)
            }),
        }))
    }

    async fn get_operation(
        &self,
        req: Request<pb::GetOperationRequest>,
    ) -> Result<Response<pb::GetOperationResponse>, Status> {
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();
        validate_operation_id(&req.operation_id)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let principal_name = principal
            .as_ref()
            .map_or(UNAUTHENTICATED_PRINCIPAL, |p| p.name.as_ref());
        let key = operation_claim_key(principal_name, &req.operation_id);
        let record = self.store.get_operation(&key).await.map_err(store_err)?;
        let Some(record) = record else {
            return Ok(Response::new(pb::GetOperationResponse {
                status: pb::OperationStatus::Unspecified as i32,
                changeset_id: String::new(),
                rejection_code: String::new(),
                rejection_message: String::new(),
            }));
        };
        // Branch-scoped mutations keep no durable receipt: GetOperation
        // reports UNKNOWN regardless of the claim's real status. See
        // ResolveBranchEntryRequest.operation_id.
        if record.branch.is_some() {
            return Ok(Response::new(pb::GetOperationResponse {
                status: pb::OperationStatus::Unknown as i32,
                changeset_id: String::new(),
                rejection_code: String::new(),
                rejection_message: String::new(),
            }));
        }
        Ok(Response::new(pb::GetOperationResponse {
            status: operation_receipts::pb_operation_status(record.status) as i32,
            changeset_id: if record.status == OperationStatus::Applied {
                record.changeset_id
            } else {
                String::new()
            },
            rejection_code: if record.status == OperationStatus::Rejected {
                record.rejection_code
            } else {
                String::new()
            },
            rejection_message: if record.status == OperationStatus::Rejected {
                record.rejection_message
            } else {
                String::new()
            },
        }))
    }

    async fn list_branches(
        &self,
        req: Request<pb::ListBranchesRequest>,
    ) -> Result<Response<pb::ListBranchesResponse>, Status> {
        let principal = Self::principal_from_metadata(&req);
        let branches = self
            .store
            .list_branches()
            .await
            .map_err(store_err)?
            .into_iter()
            .filter(|b| Self::branch_visible(principal.as_ref(), &b.name))
            .map(branch_info_to_pb)
            .collect();
        Ok(Response::new(pb::ListBranchesResponse { branches }))
    }

    async fn delete_branch(
        &self,
        req: Request<pb::DeleteBranchRequest>,
    ) -> Result<Response<pb::DeleteBranchResponse>, Status> {
        let principal = Self::principal_from_metadata(&req);
        let req = req.into_inner();
        validate_branch_name(&req.name)?;
        Self::authorize_branch_write(principal.as_ref(), &req.name, "deleting a branch")?;
        if !self
            .store
            .list_branches()
            .await
            .map_err(store_err)?
            .iter()
            .any(|b| b.name == req.name)
        {
            return Err(branch_not_found(&req.name));
        }
        self.store
            .delete_branch(&req.name)
            .await
            .map_err(store_err)?;
        self.settle_provisional_namespaces(&req.name).await?;
        Ok(Response::new(pb::DeleteBranchResponse {}))
    }
}

impl EventModelServiceImpl {
    /// Refuse a resolution decided against an entry state that no longer
    /// holds. Callers hold the mutation lock, so the state compared here is
    /// the state the resolution would act on.
    async fn require_branch_entry_state(
        &self,
        branch: &str,
        kind: pb::EntityKind,
        id: &pb::Id,
        expected: &pb::BranchEntryState,
    ) -> Result<(), Status> {
        let delta = self
            .store
            .list_branch_deltas(branch)
            .await
            .map_err(store_err)?
            .into_iter()
            .find(|d| d.kind == kind && d.id == *id);
        let actual = match delta {
            None => None,
            Some(delta) => {
                let theirs_etag = match self.store.get(kind, id, None).await {
                    Ok(stored) => stored.etag,
                    Err(trogon_atlas_store::error::StoreError::NotFound) => String::new(),
                    Err(e) => return Err(store_err(e)),
                };
                Some(pb::BranchEntryState {
                    base_etag: delta.base_etag,
                    ours_etag: delta.etag,
                    theirs_etag,
                })
            }
        };
        if actual.as_ref() == Some(expected) {
            return Ok(());
        }
        let now = actual.as_ref().map_or_else(
            || "no entry for this key".to_string(),
            |state| format!("now {}", branch_entry_state_label(state)),
        );
        Err(Status::aborted(format!(
            "stale resolution for {} on branch {branch:?}: decided against {}, {now}; diff the \
             branch again and decide from the current entry",
            pb::canonical::id_string(kind, id),
            branch_entry_state_label(expected),
        )))
    }
}

fn branch_entry_state_label(state: &pb::BranchEntryState) -> String {
    let side = |etag: &str| {
        if etag.is_empty() {
            "absent".to_string()
        } else {
            etag.to_string()
        }
    };
    format!(
        "base={} ours={} theirs={}",
        side(&state.base_etag),
        side(&state.ours_etag),
        side(&state.theirs_etag)
    )
}

fn branch_not_found(name: &str) -> Status {
    Status::not_found(format!("branch not found: {name:?}"))
}

fn branch_info_to_pb(info: trogon_atlas_store::store::BranchInfo) -> pb::BranchInfo {
    pb::BranchInfo {
        name: info.name,
        doc: info.doc,
        created_at: info.created_at,
        base_change_token: info.base_change_token,
        delta_count: info.delta_count,
        fork_changeset_id: info.fork_changeset_id.unwrap_or_default(),
    }
}

/// Decode a raw `i32` status field back into the generated enum, defaulting
/// to `Unspecified` for an out-of-range value (defensive; every value we
/// construct ourselves is always one of the named variants).
fn branch_diff_entry_status(raw: i32) -> pb::branch_diff_entry::Status {
    pb::branch_diff_entry::Status::try_from(raw)
        .unwrap_or(pb::branch_diff_entry::Status::Unspecified)
}

/// True for any of the four conflict classes (see
/// `docs/explanation/branching.md`, "Merge conflicts"). CONVERGED is
/// deliberately excluded: it is not a conflict, it is a no-op entry that is
/// dropped silently on merge.
fn is_conflict_status(raw: i32) -> bool {
    matches!(
        branch_diff_entry_status(raw),
        pb::branch_diff_entry::Status::ConflictEditEdit
            | pb::branch_diff_entry::Status::ConflictEditDelete
            | pb::branch_diff_entry::Status::ConflictDeleteEdit
    )
}

/// Fold an entry's structural three-way merge into the entry itself, so the
/// rest of the merge path treats it as an ordinary edit rather than a
/// conflict. A no-op unless the entry is an edit/edit conflict that actually
/// carries a merge, which leaves every other conflict class untouched.
///
/// The rewritten `base_etag` is the baseline revision the merge was computed
/// against, not the stale one the delta recorded when it forked. That is the
/// whole safety story for landing a merged entity: `land_branch_merge` guards
/// each put on that etag, so a baseline write slipping in between here and
/// the land fails the whole call instead of overwriting a revision nobody
/// merged against.
fn accept_structural_merge(entry: &mut pb::BranchDiffEntry, theirs_etag: Option<&str>) {
    if !matches!(
        branch_diff_entry_status(entry.status),
        pb::branch_diff_entry::Status::ConflictEditEdit
    ) {
        return;
    }
    let (Some(merged), Some(etag)) = (entry.auto_merged.clone(), theirs_etag) else {
        return;
    };
    entry.status = pb::branch_diff_entry::Status::Changed as i32;
    entry.ours = Some(merged);
    entry.base = entry.theirs.clone();
    etag.clone_into(&mut entry.base_etag);
    entry.conflict_field_paths.clear();
}

/// Recursive canonical-JSON diff walk between `ours` and `theirs`, returning
/// every field path (dot-separated, array indices in `[n]`) whose leaf value
/// differs. Used to populate `BranchDiffEntry.conflict_field_paths` for
/// STATUS_CONFLICT_EDIT_EDIT so a reviewer can see exactly what collided
/// without diffing the full entity by hand.
///
/// Depth-limited to guard against pathological nesting; paths beyond the
/// limit are reported at the truncation point rather than recursed into
/// further. Traversal order is deterministic: object keys are visited in
/// sorted order (canonical JSON from `entity_json_value` already uses a
/// `serde_json::Map`, which is insertion-ordered, so we sort explicitly to
/// keep the result stable regardless of proto field-encoding order).
fn conflict_field_paths(
    ours: &pb::Entity,
    theirs: &pb::Entity,
) -> Result<Vec<String>, trogon_atlas_core::transcode::TranscodeError> {
    const MAX_DEPTH: usize = 32;
    let ours_json = trogon_atlas_core::transcode::entity_json_value(ours)?;
    let theirs_json = trogon_atlas_core::transcode::entity_json_value(theirs)?;
    let mut paths = Vec::new();
    walk_json_diff(
        &ours_json,
        &theirs_json,
        String::new(),
        0,
        MAX_DEPTH,
        &mut paths,
    );
    Ok(paths)
}

fn walk_json_diff(
    a: &serde_json::Value,
    b: &serde_json::Value,
    prefix: String,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<String>,
) {
    if a == b {
        return;
    }
    if depth >= max_depth {
        out.push(if prefix.is_empty() {
            "<root>".to_string()
        } else {
            prefix
        });
        return;
    }
    match (a, b) {
        (serde_json::Value::Object(oa), serde_json::Value::Object(ob)) => {
            let mut keys: std::collections::BTreeSet<&String> = std::collections::BTreeSet::new();
            keys.extend(oa.keys());
            keys.extend(ob.keys());
            for key in keys {
                let next_prefix = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                let av = oa.get(key).unwrap_or(&serde_json::Value::Null);
                let bv = ob.get(key).unwrap_or(&serde_json::Value::Null);
                walk_json_diff(av, bv, next_prefix, depth + 1, max_depth, out);
            }
        }
        (serde_json::Value::Array(aa), serde_json::Value::Array(ba)) => {
            let len = aa.len().max(ba.len());
            for i in 0..len {
                let next_prefix = format!("{prefix}[{i}]");
                let av = aa.get(i).unwrap_or(&serde_json::Value::Null);
                let bv = ba.get(i).unwrap_or(&serde_json::Value::Null);
                walk_json_diff(av, bv, next_prefix, depth + 1, max_depth, out);
            }
        }
        _ => {
            out.push(if prefix.is_empty() {
                "<root>".to_string()
            } else {
                prefix
            });
        }
    }
}

#[cfg(test)]
mod batch_mutate_failure_code_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use trogon_atlas_proto as pb;
    use trogon_atlas_store::{error::StoreError, store::MutationOp};

    use super::EventModelServiceImpl;

    fn entity(ns: &str, slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: ns.into(),
                    slug: slug.into(),
                    version: 1,
                }),
                title: "t".into(),
                ..Default::default()
            })),
        }
    }

    #[test]
    fn etag_mismatch_on_a_planned_put_is_stale_precondition() {
        let mutations = vec![MutationOp::Put {
            kind: pb::EntityKind::Event,
            entity: entity("ns", "a"),
            if_match: Some("1".into()),
            force: false,
        }];
        let source = StoreError::EtagMismatch {
            expected: "1".into(),
            found: "2".into(),
        };
        assert_eq!(
            EventModelServiceImpl::batch_mutate_failure_code(&mutations, 0, &source),
            "STALE_PRECONDITION"
        );
    }

    #[test]
    fn not_found_on_an_unconditional_put_is_entity_not_found() {
        let mutations = vec![MutationOp::Put {
            kind: pb::EntityKind::Event,
            entity: entity("ns", "a"),
            if_match: None,
            force: false,
        }];
        assert_eq!(
            EventModelServiceImpl::batch_mutate_failure_code(&mutations, 0, &StoreError::NotFound),
            "ENTITY_NOT_FOUND"
        );
    }

    #[test]
    fn not_found_on_a_planned_put_is_stale_precondition_not_entity_not_found() {
        // A Put with `if_match` means the caller planned against a specific
        // revision; the entity vanishing since then is the same refusal as
        // an etag mismatch, not a plain "it never existed".
        let mutations = vec![MutationOp::Put {
            kind: pb::EntityKind::Event,
            entity: entity("ns", "a"),
            if_match: Some("1".into()),
            force: false,
        }];
        assert_eq!(
            EventModelServiceImpl::batch_mutate_failure_code(&mutations, 0, &StoreError::NotFound),
            "STALE_PRECONDITION"
        );
    }

    #[test]
    fn invalid_argument_is_invalid_op() {
        let mutations = vec![MutationOp::Create {
            kind: pb::EntityKind::Event,
            entity: entity("ns", "a"),
        }];
        let source = StoreError::InvalidArgument("bad field".into());
        assert_eq!(
            EventModelServiceImpl::batch_mutate_failure_code(&mutations, 0, &source),
            "INVALID_OP"
        );
    }

    #[test]
    fn backend_error_falls_back_to_batch_op_failed() {
        let mutations = vec![MutationOp::Create {
            kind: pb::EntityKind::Event,
            entity: entity("ns", "a"),
        }];
        let source = StoreError::Backend("nats down".into());
        assert_eq!(
            EventModelServiceImpl::batch_mutate_failure_code(&mutations, 0, &source),
            "BATCH_OP_FAILED"
        );
    }
}

#[cfg(test)]
mod conflict_field_paths_property_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use proptest::prelude::*;
    use trogon_atlas_core::semantic_eq::semantically_equal;
    use trogon_atlas_proto as pb;

    use super::conflict_field_paths;

    fn bounded_text() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 _.-]{0,24}"
    }

    fn event_with(title: String, doc: String) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: "ns".into(),
                    slug: "a".into(),
                    version: 1,
                }),
                title,
                doc,
                ..Default::default()
            })),
        }
    }

    /// Every reported path must address a point where `ours`/`theirs`
    /// actually differ: walk the path's dot segments (ignoring `[n]`
    /// index components, which this generator's flat entities never
    /// produce) down into the canonical JSON of each side, and assert the
    /// values found there are not equal.
    fn assert_paths_address_real_differences(
        paths: &[String],
        ours: &pb::Entity,
        theirs: &pb::Entity,
    ) {
        let ours_json = trogon_atlas_core::transcode::entity_json_value(ours).unwrap();
        let theirs_json = trogon_atlas_core::transcode::entity_json_value(theirs).unwrap();
        for path in paths {
            if path == "<root>" {
                assert_ne!(
                    ours_json, theirs_json,
                    "path {path:?} claims a root-level difference but json is equal"
                );
                continue;
            }
            let mut a = &ours_json;
            let mut b = &theirs_json;
            for segment in path.split('.') {
                // This generator only produces flat object fields, so a
                // bracketed array index never appears; strip defensively
                // in case future generators add array fields.
                let key = segment.split('[').next().unwrap_or(segment);
                a = a.get(key).unwrap_or(&serde_json::Value::Null);
                b = b.get(key).unwrap_or(&serde_json::Value::Null);
            }
            assert_ne!(a, b, "path {path:?} does not address an actual difference");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Empty path list iff semantically equal, in both directions.
        #[test]
        fn empty_paths_iff_semantically_equal(
            title_a in bounded_text(),
            doc_a in bounded_text(),
            title_b in bounded_text(),
            doc_b in bounded_text(),
        ) {
            let ours = event_with(title_a, doc_a);
            let theirs = event_with(title_b, doc_b);
            let paths = conflict_field_paths(&ours, &theirs).unwrap();
            prop_assert_eq!(paths.is_empty(), semantically_equal(&ours, &theirs));
        }

        /// Every non-empty path list addresses at least one real
        /// difference point between the two entities.
        #[test]
        fn every_path_addresses_a_real_difference(
            title_a in bounded_text(),
            doc_a in bounded_text(),
            title_b in bounded_text(),
            doc_b in bounded_text(),
        ) {
            let ours = event_with(title_a, doc_a);
            let theirs = event_with(title_b, doc_b);
            let paths = conflict_field_paths(&ours, &theirs).unwrap();
            assert_paths_address_real_differences(&paths, &ours, &theirs);
        }

        /// Deterministic: calling twice on the same input yields the same
        /// output, in the same order.
        #[test]
        fn output_is_deterministic(
            title_a in bounded_text(),
            doc_a in bounded_text(),
            title_b in bounded_text(),
            doc_b in bounded_text(),
        ) {
            let ours = event_with(title_a, doc_a);
            let theirs = event_with(title_b, doc_b);
            let first = conflict_field_paths(&ours, &theirs).unwrap();
            let second = conflict_field_paths(&ours, &theirs).unwrap();
            prop_assert_eq!(first, second);
        }
    }
}

type SubdomainDomainMap = std::collections::HashMap<(String, String), (String, String)>;
type BcNamespaceDomainMap = std::collections::HashMap<String, (String, String)>;

/// Build maps needed to resolve namespace->domain relationships. Returns:
/// - `subdomain_domain`: (subdomain ns, slug) -> (domain ns, slug)
/// - `bc_domain`: BC namespace -> (domain ns, slug)
///
/// Used by `validate_project` and `list_entities_by_domain`.
fn build_domain_maps(
    all: &[trogon_atlas_store::StoredEntity],
) -> (SubdomainDomainMap, BcNamespaceDomainMap) {
    let mut subdomain_domain: SubdomainDomainMap = std::collections::HashMap::new();
    for se in all {
        if let Some(pb::entity::Kind::Subdomain(s)) = se.entity.kind.as_ref() {
            if let (Some(id), Some(did)) =
                (s.id.as_ref(), s.domain.as_ref().and_then(|r| r.id.as_ref()))
            {
                subdomain_domain.insert(
                    (id.namespace.clone(), id.slug.clone()),
                    (did.namespace.clone(), did.slug.clone()),
                );
            }
        }
    }
    let mut bc_domain: BcNamespaceDomainMap = std::collections::HashMap::new();
    for se in all {
        if let Some(pb::entity::Kind::BoundedContext(bc)) = se.entity.kind.as_ref() {
            let Some(id) = bc.id.as_ref() else { continue };
            if let Some(sd_id) = bc.realizes.iter().find_map(|r| r.id.as_ref()) {
                if let Some(did) =
                    subdomain_domain.get(&(sd_id.namespace.clone(), sd_id.slug.clone()))
                {
                    bc_domain.insert(id.namespace.clone(), did.clone());
                }
            }
        }
    }
    (subdomain_domain, bc_domain)
}

/// Reject writes where a Domain's declared project lives in a different namespace.
///
/// Project identity follows the convention `project.id.slug == project.id.namespace`.
/// A Domain that declares a project in a different namespace than its own would create
/// an unresolvable cross-project association at the mutation boundary. Entity kinds
/// other than Domain carry no project field, so their project membership can only
/// be determined after write via the BC -> Subdomain -> Domain -> Project chain;
/// enforcement for those kinds is left to the validator (`DOMAIN_MISSING_PROJECT`,
/// `BC_PROJECT_COLLISION`, `PROJECT_SLUG_MISMATCH`).
#[allow(clippy::result_large_err)]
fn check_project_namespace_conflict(entity: &pb::Entity) -> Result<(), Status> {
    let Some(pb::entity::Kind::Domain(domain)) = entity.kind.as_ref() else {
        return Ok(());
    };
    let Some(domain_id) = domain.id.as_ref() else {
        return Ok(());
    };
    let Some(proj_ref) = domain.project.as_ref() else {
        return Ok(());
    };
    let Some(proj_id) = proj_ref.id.as_ref() else {
        return Ok(());
    };
    // Project identity: namespace IS the project (slug == namespace by convention).
    // A Domain whose project.id.namespace differs from its own namespace is
    // declaring a cross-namespace project association, which is not permitted.
    if !proj_id.namespace.is_empty() && proj_id.namespace != domain_id.namespace {
        return Err(Status::failed_precondition(format!(
            "domain {}/{} declares project in namespace '{}' which differs from the domain's own namespace '{}'; \
             a Domain must be owned by a Project whose namespace matches the Domain's namespace",
            domain_id.namespace,
            domain_id.slug,
            proj_id.namespace,
            domain_id.namespace,
        )));
    }
    Ok(())
}

/// Classify an LLM error into a stable short reason code for client visibility.
/// Keeps internal error details server-side; only the category crosses the wire.
fn llm_fallback_reason(err: &anyhow::Error) -> &'static str {
    let msg = format!("{err:#}").to_lowercase();
    if msg.contains("timeout") || msg.contains("timed out") || msg.contains("deadline") {
        "llm_timeout"
    } else {
        "llm_error"
    }
}

/// Returns `true` when the `record`'s entity belongs to `closure_keys`.
///
/// `closure_keys` is a set of canonical ID strings of the form
/// `"<kind>:<namespace>/<slug>@<version>"` as produced by
/// `trogon_atlas_proto::canonical::id_string`. An entity whose kind or id is
/// unresolvable is considered outside all closures and returns `false`.
/// Delete records whose entity was a member at resolution time pass the filter;
/// the caller must not exclude them.
/// Whether a change record's entity is visible to this caller.
///
/// A record whose entity ref is malformed is dropped rather than passed
/// through: it cannot be attributed to an owner, and a caller bound to one
/// must not receive rows nobody can vouch for.
fn change_record_visible(record: &ChangeRecord, lens: &Lens) -> bool {
    if lens.is_unrestricted() {
        return true;
    }
    record
        .entity_ref
        .id
        .as_ref()
        .is_some_and(|id| lens.admits(&id.namespace))
}

pub fn record_in_scope_closure<S: std::hash::BuildHasher>(
    record: &ChangeRecord,
    closure_keys: &std::collections::HashSet<String, S>,
) -> bool {
    let Ok(kind) = pb::EntityKind::try_from(record.entity_ref.kind) else {
        return false;
    };
    if kind == pb::EntityKind::Unspecified {
        return false;
    }
    let Some(id) = record.entity_ref.id.as_ref() else {
        return false;
    };
    let key = trogon_atlas_proto::canonical::id_string(kind, id);
    closure_keys.contains(&key)
}

/// Resolves a list of `AnalysisScope` values to the union of their transitive
/// entity closures, returning the result as a set of canonical ID strings.
///
/// Each scope must have its `scope` oneof set to a concrete variant (`Slice`,
/// `Storyboard`, or `EventModel`) with a valid, non-empty `EntityRef.id`.
/// Returns `INVALID_ARGUMENT` for a missing or unset scope, consistent with
/// how `ExtractSubgraph` and neighbouring RPCs handle the same condition.
pub fn resolve_scope_closure(
    scopes: &[pb::AnalysisScope],
    all: &[StoredEntity],
) -> Result<std::collections::HashSet<String>, Status> {
    let mut roots: Vec<pb::EntityRef> = Vec::with_capacity(scopes.len());
    for (i, scope) in scopes.iter().enumerate() {
        let field = format!("scopes[{i}]");
        let Some(
            pb::analysis_scope::Scope::Slice(root_ref)
            | pb::analysis_scope::Scope::Storyboard(root_ref)
            | pb::analysis_scope::Scope::EventModel(root_ref),
        ) = scope.scope.as_ref()
        else {
            return Err(Status::invalid_argument(format!(
                "{field}.scope is required"
            )));
        };
        let _ = kind_from_i32(root_ref.kind, &format!("{field}.kind"))?;
        let _ = require_id(root_ref.id.as_ref(), &field)?;
        roots.push(root_ref.clone());
    }
    let graph::Closure { entities, .. } = graph::closure(all, &roots, MAX_PROJECTION_ENTITIES);
    Ok(entities.into_keys().collect())
}

fn change_kind_to_proto(kind: ChangeKind) -> i32 {
    match kind {
        ChangeKind::Put => pb::change_event::Kind::Put as i32,
        ChangeKind::Delete => pb::change_event::Kind::Deleted as i32,
    }
}

fn changeset_to_proto(record: ChangesetRecord) -> pb::Changeset {
    pb::Changeset {
        id: record.id,
        author: record.author,
        at: record.at,
        message: record.message,
        branch: record.branch.unwrap_or_default(),
        rpc: record.rpc,
        ops: record
            .ops
            .into_iter()
            .map(|op| pb::ChangesetOp {
                kind: change_kind_to_proto(op.kind),
                entity: Some(op.entity_ref),
            })
            .collect(),
        operation_id: record.operation_id.unwrap_or_default(),
    }
}

fn entity_revision_to_proto(record: EntityRevisionRecord) -> pb::EntityRevision {
    pb::EntityRevision {
        changeset_id: record.changeset_id,
        author: record.author,
        at: record.at,
        rpc: record.rpc,
        branch: record.branch.unwrap_or_default(),
        entity: Some(record.entity_ref),
        kind: change_kind_to_proto(record.kind),
        before: record.before,
        after: record.after,
    }
}

fn change_record_to_event(record: ChangeRecord) -> pb::ChangeEvent {
    let timestamp_micros = if let Ok(d) = chrono::DateTime::parse_from_rfc3339(&record.at) {
        d.timestamp_micros()
    } else {
        tracing::warn!(raw = %record.at, "change record timestamp could not be parsed as RFC 3339; using epoch 0");
        0
    };
    pb::ChangeEvent {
        kind: change_kind_to_proto(record.kind),
        entity: Some(record.entity_ref),
        at: record.at,
        token: record.seq.to_string(),
        timestamp_micros,
        changeset_id: record.changeset_id,
        author: record.author,
    }
}

fn change_matches_filter(record: &ChangeRecord, namespace: &str, kind: pb::EntityKind) -> bool {
    if kind != pb::EntityKind::Unspecified && record.entity_ref.kind != kind as i32 {
        return false;
    }
    if !namespace.is_empty() {
        match record.entity_ref.id.as_ref() {
            Some(id) if id.namespace == namespace => {}
            _ => return false,
        }
    }
    true
}

/// Decision for one `read_changes` page while filling a live-tail gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GapFillPageOutcome {
    /// No records remain in `(gap_start, live_seq)`: empty page, retention
    /// hole whose next durable seq is already `>= live_seq`, or only
    /// duplicate seqs at/below `gap_start`.
    Done,
    /// Emit records in `(gap_start, live_seq)` from this page. When
    /// `page_exhausted` is true the durable log has no further messages
    /// after this page, so the caller must stop after emitting.
    Emit {
        new_gap_start: u64,
        page_exhausted: bool,
    },
}

/// Classify a gap-fill page from its sequence numbers alone.
///
/// Critical invariant: when the next durable seq is already `>= live_seq`
/// (retention hole), this returns [`GapFillPageOutcome::Done`] instead of
/// asking the caller to retry the same `read_changes(gap_start, ...)`.
/// Retrying a full page of post-live records with a stuck cursor loops
/// forever.
fn gap_fill_page_outcome(
    page_seqs: impl IntoIterator<Item = u64>,
    gap_start: u64,
    live_seq: u64,
    page_capacity: usize,
    page_len: usize,
) -> GapFillPageOutcome {
    if page_len == 0 {
        return GapFillPageOutcome::Done;
    }
    let mut cursor = gap_start;
    let mut progressed = false;
    for seq in page_seqs {
        if seq <= cursor {
            continue;
        }
        if seq >= live_seq {
            break;
        }
        cursor = seq;
        progressed = true;
    }
    if !progressed {
        return GapFillPageOutcome::Done;
    }
    GapFillPageOutcome::Emit {
        new_gap_start: cursor,
        page_exhausted: page_len < page_capacity,
    }
}

/// Attempt to reserve a `StreamChanges` slot using a compare-exchange loop.
/// Returns `true` if the slot was reserved (counter incremented), `false` if
/// the cap was already reached. Extracted from `stream_changes` so the cap
/// enforcement logic can be tested without a live store or tonic request.
#[cfg(test)]
fn try_reserve_stream_slot(counter: &std::sync::atomic::AtomicUsize, cap: usize) -> bool {
    let mut current = counter.load(std::sync::atomic::Ordering::Relaxed);
    loop {
        if current >= cap {
            return false;
        }
        match counter.compare_exchange_weak(
            current,
            current + 1,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Relaxed,
        ) {
            Ok(_) => return true,
            Err(actual) => current = actual,
        }
    }
}

#[cfg(test)]
mod subscriber_cap_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::atomic::AtomicUsize;

    use super::*;

    #[test]
    fn reserves_up_to_cap() {
        let counter = AtomicUsize::new(0);
        for _ in 0..3 {
            assert!(try_reserve_stream_slot(&counter, 3));
        }
        assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 3);
    }

    #[test]
    fn rejects_at_cap() {
        let counter = AtomicUsize::new(3);
        assert!(!try_reserve_stream_slot(&counter, 3));
        assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 3);
    }

    #[test]
    fn rejects_above_cap() {
        let counter = AtomicUsize::new(10);
        assert!(!try_reserve_stream_slot(&counter, 3));
    }

    #[test]
    fn zero_cap_always_rejects() {
        let counter = AtomicUsize::new(0);
        assert!(!try_reserve_stream_slot(&counter, 0));
    }

    #[test]
    fn guard_decrements_on_drop() {
        let counter = Arc::new(AtomicUsize::new(1));
        {
            let _guard = StreamChangesGuard(counter.clone());
        }
        assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 0);
    }
}

#[cfg(test)]
mod gap_fill_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{gap_fill_page_outcome, GapFillPageOutcome};

    #[test]
    fn empty_page_is_done() {
        assert_eq!(
            gap_fill_page_outcome(std::iter::empty(), 5, 10, 256, 0),
            GapFillPageOutcome::Done
        );
    }

    #[test]
    fn retention_hole_full_page_is_done_not_retry() {
        // PROVE: old loop `continue`d every seq >= live_seq without advancing
        // gap_start. A full page of post-live seqs would spin forever.
        let page: Vec<u64> = (10..266).collect();
        assert_eq!(
            gap_fill_page_outcome(page.iter().copied(), 5, 10, 256, 256),
            GapFillPageOutcome::Done,
            "a full page whose first durable seq is already >= live must stop"
        );
    }

    #[test]
    fn contiguous_gap_emits_and_continues() {
        let page: Vec<u64> = (6..=261).collect();
        assert_eq!(
            gap_fill_page_outcome(page.iter().copied(), 5, 1000, 256, 256),
            GapFillPageOutcome::Emit {
                new_gap_start: 261,
                page_exhausted: false,
            }
        );
    }

    #[test]
    fn short_page_marks_exhausted() {
        let page: Vec<u64> = vec![6, 7, 8];
        assert_eq!(
            gap_fill_page_outcome(page.iter().copied(), 5, 100, 256, 3),
            GapFillPageOutcome::Emit {
                new_gap_start: 8,
                page_exhausted: true,
            }
        );
    }

    #[test]
    fn stops_classifying_at_live_seq() {
        let page: Vec<u64> = vec![6, 7, 10, 11];
        assert_eq!(
            gap_fill_page_outcome(page.iter().copied(), 5, 10, 256, 4),
            GapFillPageOutcome::Emit {
                new_gap_start: 7,
                page_exhausted: true,
            }
        );
    }
}

#[cfg(test)]
mod page_token_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn roundtrip() {
        for offset in [0_usize, 1, 100, 9999, usize::MAX / 2] {
            let token = encode_page_token(offset);
            assert!(token.starts_with(PAGE_TOKEN_PREFIX));
            assert_eq!(parse_page_token(&token).unwrap(), offset);
        }
    }

    #[test]
    fn empty_token_is_zero_offset() {
        assert_eq!(parse_page_token("").unwrap(), 0);
    }

    #[test]
    fn rejects_legacy_bare_integer() {
        assert!(parse_page_token("42").is_err());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_page_token("not-a-token").is_err());
        assert!(parse_page_token("v1.offset.!!!").is_err());
    }
}

#[cfg(test)]
mod lifecycle_filter_tests {
    use super::passes_lifecycle_filter;

    #[test]
    fn both_empty_passes_all() {
        assert!(passes_lifecycle_filter("draft", &[], &[]));
        assert!(passes_lifecycle_filter("", &[], &[]));
        assert!(passes_lifecycle_filter("accepted", &[], &[]));
    }

    #[test]
    fn status_in_matches_present_status() {
        let status_in = vec!["draft".to_string(), "proposed".to_string()];
        assert!(passes_lifecycle_filter("draft", &status_in, &[]));
        assert!(passes_lifecycle_filter("proposed", &status_in, &[]));
        assert!(!passes_lifecycle_filter("accepted", &status_in, &[]));
        assert!(!passes_lifecycle_filter("", &status_in, &[]));
    }

    #[test]
    fn status_in_with_empty_string_matches_no_annotation() {
        let status_in = vec![String::new()];
        assert!(passes_lifecycle_filter("", &status_in, &[]));
        assert!(!passes_lifecycle_filter("draft", &status_in, &[]));
    }

    #[test]
    fn status_not_in_excludes_listed() {
        let not_in = vec!["draft".to_string(), "proposed".to_string()];
        assert!(!passes_lifecycle_filter("draft", &[], &not_in));
        assert!(!passes_lifecycle_filter("proposed", &[], &not_in));
        assert!(passes_lifecycle_filter("accepted", &[], &not_in));
        assert!(passes_lifecycle_filter("", &[], &not_in));
    }

    #[test]
    fn both_filters_compose_as_and() {
        let status_in = vec!["draft".to_string(), "accepted".to_string()];
        let not_in = vec!["draft".to_string()];
        // accepted passes status_in and is not in not_in: passes
        assert!(passes_lifecycle_filter("accepted", &status_in, &not_in));
        // draft passes status_in but is in not_in: rejected
        assert!(!passes_lifecycle_filter("draft", &status_in, &not_in));
        // proposed fails status_in: rejected
        assert!(!passes_lifecycle_filter("proposed", &status_in, &not_in));
    }

    #[test]
    fn filter_is_case_insensitive() {
        let not_in = vec!["Draft".to_string()];
        assert!(!passes_lifecycle_filter("draft", &[], &not_in));
        let status_in = vec!["ACCEPTED".to_string()];
        assert!(passes_lifecycle_filter("accepted", &status_in, &[]));
    }
}

mod dry_run {
    use std::collections::HashSet;

    use trogon_atlas_proto as pb;
    use trogon_atlas_store::{
        error::{StoreError, StoreResult},
        refs as r,
        store::{MutationOp, MutationOutcome},
        StoredEntity,
    };

    type OverlayKey = (i32, pb::IdKey);

    pub struct Overlay {
        map: HashSet<OverlayKey>,
    }

    impl Overlay {
        pub fn from_snapshot(snapshot: &[StoredEntity]) -> Self {
            let mut map = HashSet::new();
            for se in snapshot {
                let (Some(k), Some(id)) = (r::entity_kind(&se.entity), r::entity_id(&se.entity))
                else {
                    continue;
                };
                map.insert((k as i32, pb::IdKey::new(id)));
            }
            Self { map }
        }

        pub fn batch_apply(mut self, ops: &[MutationOp]) -> StoreResult<Vec<MutationOutcome>> {
            let mut outcomes: Vec<MutationOutcome> = Vec::with_capacity(ops.len());
            for (i, op) in ops.iter().enumerate() {
                match op {
                    MutationOp::Create { kind, entity } => {
                        let id = r::entity_id(entity).cloned().ok_or_else(|| {
                            StoreError::BatchFailed {
                                index: i,
                                source: Box::new(StoreError::InvalidArgument(
                                    "entity.id is required".into(),
                                )),
                            }
                        })?;
                        let key: OverlayKey = (*kind as i32, pb::IdKey::new(&id));
                        if self.map.contains(&key) {
                            return Err(StoreError::BatchFailed {
                                index: i,
                                source: Box::new(StoreError::AlreadyExists),
                            });
                        }
                        self.map.insert(key);
                        outcomes.push(MutationOutcome::Wrote {
                            etag: "dry-run".into(),
                            entity: entity.clone(),
                        });
                    }
                    MutationOp::Put { kind, entity, .. } => {
                        let id = r::entity_id(entity).cloned().ok_or_else(|| {
                            StoreError::BatchFailed {
                                index: i,
                                source: Box::new(StoreError::InvalidArgument(
                                    "entity.id is required".into(),
                                )),
                            }
                        })?;
                        let key: OverlayKey = (*kind as i32, pb::IdKey::new(&id));
                        self.map.insert(key);
                        outcomes.push(MutationOutcome::Wrote {
                            etag: "dry-run".into(),
                            entity: entity.clone(),
                        });
                    }
                    MutationOp::Delete { kind, id, .. } => {
                        let key: OverlayKey = (*kind as i32, pb::IdKey::new(id));
                        if !self.map.remove(&key) {
                            return Err(StoreError::BatchFailed {
                                index: i,
                                source: Box::new(StoreError::NotFound),
                            });
                        }
                        outcomes.push(MutationOutcome::Deleted);
                    }
                }
            }
            Ok(outcomes)
        }
    }

    pub fn apply(
        snapshot: &[StoredEntity],
        ops: &[MutationOp],
    ) -> StoreResult<Vec<MutationOutcome>> {
        Overlay::from_snapshot(snapshot).batch_apply(ops)
    }
}

#[cfg(test)]
mod scope_filter_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashSet;

    use trogon_atlas_proto as pb;
    use trogon_atlas_store::{store::ChangeRecord, ChangeKind, StoredEntity};

    use super::{record_in_scope_closure, resolve_scope_closure};

    fn make_id(ns: &str, slug: &str) -> pb::Id {
        pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn make_entity_ref(kind: pb::EntityKind, ns: &str, slug: &str) -> pb::EntityRef {
        pb::EntityRef {
            kind: kind as i32,
            id: Some(make_id(ns, slug)),
        }
    }

    fn make_change_record(
        kind: ChangeKind,
        entity_kind: pb::EntityKind,
        ns: &str,
        slug: &str,
    ) -> ChangeRecord {
        ChangeRecord {
            seq: 1,
            kind,
            entity_ref: make_entity_ref(entity_kind, ns, slug),
            at: "2024-01-01T00:00:00Z".into(),
            changeset_id: String::new(),
            author: String::new(),
        }
    }

    fn closure_from_refs(refs: &[pb::EntityRef]) -> HashSet<String> {
        refs.iter()
            .filter_map(|r| {
                let id = r.id.as_ref()?;
                let k = pb::EntityKind::try_from(r.kind).ok()?;
                if k == pb::EntityKind::Unspecified {
                    return None;
                }
                Some(trogon_atlas_proto::canonical::id_string(k, id))
            })
            .collect()
    }

    #[test]
    fn matching_entity_passes_filter() {
        let closure =
            closure_from_refs(&[make_entity_ref(pb::EntityKind::Event, "orders", "placed")]);
        let record = make_change_record(ChangeKind::Put, pb::EntityKind::Event, "orders", "placed");
        assert!(record_in_scope_closure(&record, &closure));
    }

    #[test]
    fn non_member_entity_is_filtered() {
        let closure =
            closure_from_refs(&[make_entity_ref(pb::EntityKind::Event, "orders", "placed")]);
        let record = make_change_record(
            ChangeKind::Put,
            pb::EntityKind::Event,
            "orders",
            "cancelled",
        );
        assert!(!record_in_scope_closure(&record, &closure));
    }

    #[test]
    fn delete_record_for_member_passes_filter() {
        let closure =
            closure_from_refs(&[make_entity_ref(pb::EntityKind::Command, "orders", "place")]);
        let record = make_change_record(
            ChangeKind::Delete,
            pb::EntityKind::Command,
            "orders",
            "place",
        );
        assert!(record_in_scope_closure(&record, &closure));
    }

    #[test]
    fn empty_closure_filters_everything() {
        let closure: HashSet<String> = HashSet::new();
        let record = make_change_record(ChangeKind::Put, pb::EntityKind::Event, "orders", "placed");
        assert!(!record_in_scope_closure(&record, &closure));
    }

    #[test]
    fn resolve_scope_closure_returns_root_when_no_outbound_refs() {
        let root = make_entity_ref(pb::EntityKind::Event, "orders", "placed");
        let stored = StoredEntity {
            entity: pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(make_id("orders", "placed")),
                    title: "placed".into(),
                    ..Default::default()
                })),
            },
            etag: String::new(),
        };
        let all = vec![stored];
        let scope = pb::AnalysisScope {
            scope: Some(pb::analysis_scope::Scope::Slice(root)),
        };
        let keys = resolve_scope_closure(&[scope], &all).unwrap();
        let expected_key = trogon_atlas_proto::canonical::id_string(
            pb::EntityKind::Event,
            &make_id("orders", "placed"),
        );
        assert!(
            keys.contains(&expected_key),
            "root must be in closure: {keys:?}"
        );
    }

    #[test]
    fn resolve_scope_closure_rejects_unset_scope_oneof() {
        let scope = pb::AnalysisScope { scope: None };
        let result = resolve_scope_closure(&[scope], &[]);
        assert!(
            result.is_err(),
            "unset scope oneof must be INVALID_ARGUMENT"
        );
        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn empty_scopes_list_produces_empty_closure_keys() {
        let keys = resolve_scope_closure(&[], &[]).unwrap();
        assert!(keys.is_empty());
    }
}
