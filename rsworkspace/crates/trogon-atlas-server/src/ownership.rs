//! Ownership: who may see and write which namespaces.
//!
//! This is the third authorization axis, after the role gate in
//! [`crate::auth`] and the write allow-list in [`crate::scope`]. The other
//! two are static: they are answered entirely from the token registry file.
//! Ownership cannot be, because the mapping from a namespace to its owner
//! lives in the namespace registry and changes at runtime, which is the
//! whole point (moving a namespace must take effect without a restart).
//!
//! # `Id.namespace` is a namespace id, not a name
//!
//! The one invariant everything here rests on. A stored `Id.namespace`, a
//! namespace segment in a storage key, and a namespace in any reference are
//! all [`NamespaceId`] values. The human name exists only in the registry.
//!
//! It is worth being explicit about why, because the alternative is
//! tempting. If entities held the human *name* while keys held the id, then
//! two owners who each call a namespace `orders` would produce entities whose
//! references are textually identical. Every consumer that joins references
//! by `(namespace, slug, version)` -- the reverse index, impact analysis,
//! validation, the projections -- would silently splice the two owners'
//! models together. Holding the id in both places means a reference either
//! resolves to exactly one entity or to none.
//!
//! The cost is that a minted id is not readable. That cost is paid at the
//! edges, by whatever renders a namespace to a person: the registry maps
//! `(parent, name)` to an id, and the CLI, MCP, and Studio layers resolve a
//! typed name into one before calling. The server's contract stays
//! unambiguous.
//!
//! Every namespace that predates the registry has `id == name`, so today
//! this distinction is invisible: the backfill adopts each existing name as
//! its own id precisely so no entity has to be rewritten.

use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
    time::Instant,
};

use async_trait::async_trait;
use tonic::Status;
use trogon_atlas_core::{NamespaceId, NamespaceName, OwnerId};
use trogon_atlas_store::{NamespaceRecord, Store};

use crate::token_reload::ReloadInterval;

/// Owner assigned to a namespace nobody has claimed.
///
/// Every namespace in a store that predates ownership lands here, and so does
/// every namespace an unscoped caller creates. It exists so the registry can
/// be complete from day one: a namespace with no row has no owner, and the
/// read filter has to refuse those, so leaving rows unwritten would turn
/// switching a principal to owner-scoped into a silent outage.
pub const DEFAULT_OWNER: &str = "default";

/// How far one caller's world extends.
///
/// Read scope is deliberately all-or-nothing per owner rather than a pattern
/// list. A pattern list is the right shape for "which of my own namespaces
/// may this agent write", which is what [`crate::scope::NamespaceScope`]
/// already does. It is the wrong shape for a tenancy boundary, where the
/// answer must come from the registry and must not be something an operator
/// can widen with a typo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// No owner boundary: every namespace in the store.
    ///
    /// This is what a principal with no `parent` gets, which is every
    /// principal in a token registry written before ownership existed. That
    /// is what makes the feature opt-in, the same property
    /// `namespaces = []` has.
    Everything,
    /// Bound to one owner.
    Owner(OwnerId),
}

/// What one caller is allowed to see, and who that caller is.
///
/// The identity rides along because the two authorizers disagree about what
/// the subject of an authorization question is. The registry only knows
/// owners, so for it the identity is dead weight. A relationship-based
/// authorizer wants the principal, so that a namespace can be shared with one
/// API key without inventing an owner for it. Carrying both keeps that a
/// choice the authorizer makes rather than a rewrite of every handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visibility {
    principal: Arc<str>,
    scope: Scope,
}

impl Visibility {
    #[must_use]
    pub fn unrestricted(principal: Arc<str>) -> Self {
        Self {
            principal,
            scope: Scope::Everything,
        }
    }

    #[must_use]
    pub fn owned_by(principal: Arc<str>, owner: OwnerId) -> Self {
        Self {
            principal,
            scope: Scope::Owner(owner),
        }
    }

    /// The authenticated principal name, or the anonymous placeholder when
    /// auth is switched off.
    #[must_use]
    pub fn principal(&self) -> &str {
        &self.principal
    }

    #[must_use]
    pub fn is_unrestricted(&self) -> bool {
        matches!(self.scope, Scope::Everything)
    }

    #[must_use]
    pub fn owner(&self) -> Option<&OwnerId> {
        match &self.scope {
            Scope::Everything => None,
            Scope::Owner(owner) => Some(owner),
        }
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.scope {
            Scope::Everything => f.write_str("*"),
            Scope::Owner(owner) => write!(f, "parent={owner}"),
        }
    }
}

/// An immutable, fully indexed view of the namespace registry.
///
/// Built by one scan of the registry bucket and then shared by every request
/// until something invalidates it. Requests ask it three questions and all
/// three must be O(1), because the visibility filter runs once per entity on
/// the read path.
#[derive(Debug, Default)]
#[allow(clippy::struct_field_names)]
pub struct DirectoryView {
    by_id: HashMap<NamespaceId, NamespaceRecord>,
    by_owner: HashMap<OwnerId, Arc<BTreeSet<NamespaceId>>>,
    by_owner_name: HashMap<(OwnerId, NamespaceName), NamespaceId>,
}

impl DirectoryView {
    fn build(records: Vec<NamespaceRecord>) -> Self {
        let mut by_id = HashMap::with_capacity(records.len());
        let mut owned: HashMap<OwnerId, BTreeSet<NamespaceId>> = HashMap::new();
        let mut by_owner_name = HashMap::with_capacity(records.len());
        for record in records {
            owned
                .entry(record.parent.clone())
                .or_default()
                .insert(record.id.clone());
            by_owner_name.insert(
                (record.parent.clone(), record.name.clone()),
                record.id.clone(),
            );
            by_id.insert(record.id.clone(), record);
        }
        Self {
            by_id,
            by_owner: owned
                .into_iter()
                .map(|(owner, ids)| (owner, Arc::new(ids)))
                .collect(),
            by_owner_name,
        }
    }

    #[must_use]
    pub fn get(&self, id: &NamespaceId) -> Option<&NamespaceRecord> {
        self.by_id.get(id)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Resolve a human name to an id within one owner. The function that lets
    /// two owners each have a namespace called `orders`.
    #[must_use]
    pub fn resolve(&self, parent: &OwnerId, name: &NamespaceName) -> Option<&NamespaceId> {
        self.by_owner_name.get(&(parent.clone(), name.clone()))
    }

    /// Every namespace id under one owner. Empty set when the owner holds
    /// nothing, which is the correct answer for a freshly provisioned tenant.
    #[must_use]
    pub fn owned_by(&self, parent: &OwnerId) -> Arc<BTreeSet<NamespaceId>> {
        self.by_owner
            .get(parent)
            .cloned()
            .unwrap_or_else(|| Arc::new(BTreeSet::new()))
    }

    /// Whether `visibility` admits this namespace.
    ///
    /// An unregistered namespace is visible to an unrestricted caller and
    /// to nobody else. Failing closed matters here: a namespace with no
    /// registry row has no owner, and guessing one would hand a tenant
    /// somebody else's data.
    #[must_use]
    pub fn admits(&self, visibility: &Visibility, namespace: &str) -> bool {
        let Some(owner) = visibility.owner() else {
            return true;
        };
        let Ok(id) = NamespaceId::parse(namespace) else {
            return false;
        };
        self.by_id
            .get(&id)
            .is_some_and(|record| &record.parent == owner)
    }

    pub fn records(&self) -> impl Iterator<Item = &NamespaceRecord> {
        self.by_id.values()
    }
}

/// A registry view together with the moment it was read, so a cached copy
/// can be judged stale without asking the store.
#[derive(Debug, Clone)]
struct CachedView {
    view: Arc<DirectoryView>,
    loaded_at: Instant,
}

impl CachedView {
    fn is_fresh(&self, freshness: ReloadInterval, now: Instant) -> bool {
        match freshness.as_duration() {
            None => true,
            Some(ttl) => now.saturating_duration_since(self.loaded_at) < ttl,
        }
    }
}

/// Cached, invalidatable access to the namespace registry.
///
/// Cached because the visibility filter consults it per entity on the read
/// path and a registry round-trip there would be absurd. Invalidatable
/// because a move must take effect on the next request, rather than making
/// "move this namespace" an operations task instead of an API call.
///
/// # Why the cache also expires
///
/// `invalidate` only reaches the process that served the write. That is the
/// whole story on one server, and wrong on two: a `MoveNamespace` handled by
/// replica A leaves replica B answering from a view that says the namespace
/// still belongs to the previous owner, with nothing that would ever correct
/// it. Ownership is the tenancy boundary -- the field that makes a key safe
/// to hand to a client -- so that is a boundary failure, and a permanent and
/// silent one, in exactly the deployment
/// `docs/explanation/authorization.md` tells operators to run.
///
/// SpiceDB does not have the problem, because there the lens comes from
/// SpiceDB on every request and the move is a relationship write both
/// replicas read. [`RegistryAuthorizer`] is the default and does have it.
///
/// So the view expires as well as being invalidated, which bounds the
/// disagreement to one interval and makes it the same kind of quantity as
/// the token registry's reload interval: a revocation latency, not a polling
/// knob. `0` pins the view to whatever was last loaded, which is the
/// behaviour before this and is only safe on a single replica.
#[derive(Debug)]
pub struct NamespaceDirectory {
    cache: tokio::sync::RwLock<Option<CachedView>>,
    /// Single-flight guard on the miss path, mirroring the snapshot cache:
    /// without it, a cold cache under concurrent load issues one full
    /// registry scan per in-flight request.
    load_lock: tokio::sync::Mutex<()>,
    freshness: ReloadInterval,
}

impl Default for NamespaceDirectory {
    fn default() -> Self {
        Self::new()
    }
}

impl NamespaceDirectory {
    #[must_use]
    pub fn new() -> Self {
        Self::with_freshness(ReloadInterval::default())
    }

    #[must_use]
    pub fn with_freshness(freshness: ReloadInterval) -> Self {
        Self {
            cache: tokio::sync::RwLock::new(None),
            load_lock: tokio::sync::Mutex::new(()),
            freshness,
        }
    }

    #[must_use]
    pub fn freshness(&self) -> ReloadInterval {
        self.freshness
    }

    /// Drop the cached view. Called after any registry write.
    pub async fn invalidate(&self) {
        *self.cache.write().await = None;
    }

    /// The current view, loading it if the cache is cold or expired.
    pub async fn view(&self, store: &dyn Store) -> Result<Arc<DirectoryView>, Status> {
        self.view_at(store, Instant::now()).await
    }

    async fn view_at(&self, store: &dyn Store, now: Instant) -> Result<Arc<DirectoryView>, Status> {
        if let Some(cached) = self.cache.read().await.clone() {
            if cached.is_fresh(self.freshness, now) {
                return Ok(cached.view);
            }
        }
        let _guard = self.load_lock.lock().await;
        if let Some(cached) = self.cache.read().await.clone() {
            if cached.is_fresh(self.freshness, now) {
                return Ok(cached.view);
            }
        }
        let records = store
            .list_namespaces()
            .await
            .map_err(|e| Status::unavailable(format!("namespace registry unavailable: {e}")))?;
        let view = Arc::new(DirectoryView::build(records));
        *self.cache.write().await = Some(CachedView {
            view: view.clone(),
            loaded_at: now,
        });
        Ok(view)
    }
}

/// The set of namespaces one request may read.
///
/// Resolved once, at the top of a handler, and then consulted synchronously
/// for every candidate entity. That split is not an optimisation detail, it
/// is what lets the decision move off this process at all: a per-entity
/// `await` against a remote authorization service would be thousands of round
/// trips on a single `ListEntities`, while a per-request one is a single
/// `LookupResources`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lens {
    /// No boundary. What an unbound caller gets, and what keeps ownership
    /// opt-in.
    Everything,
    /// Exactly these namespace ids and no others.
    ///
    /// A namespace absent from the set is refused whether it is owned by
    /// somebody else or does not exist at all, which is the same fail-closed
    /// answer [`DirectoryView::admits`] gives.
    Only(Arc<BTreeSet<NamespaceId>>),
}

impl Lens {
    #[must_use]
    pub fn is_unrestricted(&self) -> bool {
        matches!(self, Self::Everything)
    }

    #[must_use]
    pub fn admits(&self, namespace: &str) -> bool {
        match self {
            Self::Everything => true,
            Self::Only(ids) => NamespaceId::parse(namespace).is_ok_and(|id| ids.contains(&id)),
        }
    }
}

/// The answer to "may this caller write here", with the third case that makes
/// claim-on-first-write possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteVerdict {
    Allowed,
    /// No registry row exists, so nobody owns it yet and the caller may claim
    /// it.
    Unclaimed,
    /// Owned by somebody else. The owner is carried for the server's own log,
    /// which is the only place it may appear: a refusal that named the holder
    /// would answer "who else is on this deployment" for any name the caller
    /// cares to guess. Knowing a namespace exists is not the same as being
    /// entitled to know whose it is.
    Denied {
        owner: OwnerId,
    },
}

/// Where the ownership decision is made.
///
/// Extracted as a trait for exactly one reason: the decision is the only part
/// of ownership that a central authorization service would take over. Naming,
/// id minting, and existence stay in the namespace registry either way,
/// because they are storage concerns and a permission system has no business
/// being the source of truth for whether a row exists.
///
/// [`RegistryAuthorizer`] answers from the registry itself and is the default.
/// [`crate::spicedb::SpiceDbAuthorizer`] answers from SpiceDB.
#[async_trait]
pub trait NamespaceAuthorizer: Send + Sync + std::fmt::Debug {
    /// Whether `visibility` may read one named namespace.
    ///
    /// Split from [`lens`](Self::lens) because the two questions cost
    /// wildly different amounts to answer remotely, and the call sites
    /// already know which one they are asking: a lookup by key asks about
    /// one namespace it was handed, while a listing asks about all of them
    /// at once.
    async fn admits(
        &self,
        visibility: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<bool, Status>;

    /// Everything `visibility` may read, for this request.
    async fn lens(&self, visibility: &Visibility) -> Result<Lens, Status>;

    /// Whether `visibility` may write into `namespace`.
    async fn may_write(
        &self,
        visibility: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<WriteVerdict, Status>;

    /// The registry gained a namespace. Fails the RPC if the authorizer
    /// cannot record it, because a namespace its owner cannot see is an
    /// outage, and a loud one is better than a quiet one.
    async fn on_registered(&self, record: &NamespaceRecord) -> Result<(), Status>;

    /// A namespace changed owner. `from` is the previous owner, which a
    /// relationship-based authorizer needs in order to remove the old edge.
    async fn on_moved(&self, record: &NamespaceRecord, from: &OwnerId) -> Result<(), Status>;

    /// The registry lost a namespace, because the branch that was holding its
    /// row open was deleted. A grant that outlived its row would let its owner
    /// keep writing into a namespace nobody is recorded as owning.
    async fn on_released(&self, record: &NamespaceRecord) -> Result<(), Status>;
}

/// The registry is the authority: a caller sees a namespace when its row says
/// so.
///
/// This is the behaviour ownership shipped with, unchanged, and the default.
pub struct RegistryAuthorizer {
    directory: Arc<NamespaceDirectory>,
    store: Arc<dyn Store>,
}

impl std::fmt::Debug for RegistryAuthorizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegistryAuthorizer")
    }
}

impl RegistryAuthorizer {
    #[must_use]
    pub fn new(directory: Arc<NamespaceDirectory>, store: Arc<dyn Store>) -> Self {
        Self { directory, store }
    }

    async fn view(&self) -> Result<Arc<DirectoryView>, Status> {
        self.directory.view(&*self.store).await
    }
}

#[async_trait]
impl NamespaceAuthorizer for RegistryAuthorizer {
    async fn admits(
        &self,
        visibility: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<bool, Status> {
        if visibility.is_unrestricted() {
            return Ok(true);
        }
        Ok(self.view().await?.admits(visibility, namespace.as_str()))
    }

    async fn lens(&self, visibility: &Visibility) -> Result<Lens, Status> {
        match visibility.owner() {
            None => Ok(Lens::Everything),
            Some(owner) => Ok(Lens::Only(self.view().await?.owned_by(owner))),
        }
    }

    async fn may_write(
        &self,
        visibility: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<WriteVerdict, Status> {
        let Some(owner) = visibility.owner() else {
            return Ok(WriteVerdict::Allowed);
        };
        let view = self.view().await?;
        Ok(match view.get(namespace) {
            None => WriteVerdict::Unclaimed,
            Some(record) if &record.parent == owner => WriteVerdict::Allowed,
            Some(record) => WriteVerdict::Denied {
                owner: record.parent.clone(),
            },
        })
    }

    /// Nothing to record: the registry write the caller already performed
    /// *is* the grant.
    async fn on_registered(&self, _record: &NamespaceRecord) -> Result<(), Status> {
        Ok(())
    }

    async fn on_moved(&self, _record: &NamespaceRecord, _from: &OwnerId) -> Result<(), Status> {
        Ok(())
    }

    async fn on_released(&self, _record: &NamespaceRecord) -> Result<(), Status> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// OwnedBranchName
// ---------------------------------------------------------------------------

/// A branch name attributed to one owner: `@<owner>:<name>`, e.g.
/// `@acme:feature-x`.
///
/// Unlike a namespace, a branch has no registry row to resolve its owner
/// from: `CreateBranch` takes a bare string, and the deltas it accumulates
/// may span namespaces owned by several different tenants (that fan-out is
/// exactly why `refuse_unpartitioned` exists). An owned branch sidesteps the
/// problem by carrying its owner in the name itself, so a caller's rights
/// over it never depend on resolving anything beyond the string.
///
/// `@` and `:` are both outside `trogon_atlas_core::validate_id_component`'s
/// charset (its own property tests enumerate them as always-invalid), so
/// this cannot delegate to the existing branch-name validator on the whole
/// string. It splits on the two delimiters first and validates each half
/// with the existing per-component rules instead: [`OwnerId::parse`] for the
/// owner, [`crate::conv::validate_classic_branch_name`] for the name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnedBranchName {
    owner: OwnerId,
    name: String,
}

impl OwnedBranchName {
    /// Parse `@<owner>:<name>`.
    ///
    /// # Errors
    ///
    /// Returns `None` for any branch name that does not start with `@` or
    /// has no `:` after it -- the signal a caller uses to tell an owned
    /// branch from an ordinary global one, not a malformed-input error.
    /// Returns `Some(Err(_))` when the `@`/`:` shape is present but the owner
    /// or name half fails its own validation.
    #[must_use]
    pub fn try_parse(branch: &str) -> Option<Result<Self, Status>> {
        let rest = branch.strip_prefix('@')?;
        let (owner, name) = rest.split_once(':')?;
        Some(Self::build(owner, name))
    }

    fn build(owner: &str, name: &str) -> Result<Self, Status> {
        let owner = OwnerId::parse(owner)
            .map_err(|e| Status::invalid_argument(format!("branch owner {e}")))?;
        crate::conv::validate_classic_branch_name(name)?;
        Ok(Self {
            owner,
            name: name.to_owned(),
        })
    }

    #[must_use]
    pub fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Display for OwnedBranchName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@{}:{}", self.owner, self.name)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn record(id: &str, name: &str, parent: &str) -> NamespaceRecord {
        NamespaceRecord {
            id: NamespaceId::parse(id).unwrap(),
            name: NamespaceName::parse(name).unwrap(),
            parent: OwnerId::parse(parent).unwrap(),
            created_at: "2026-01-01T00:00:00Z".into(),
            created_by: "test".into(),
            tenure: Default::default(),
        }
    }

    fn owner(v: &str) -> OwnerId {
        OwnerId::parse(v).unwrap()
    }

    fn bound(v: &str) -> Visibility {
        Visibility::owned_by(Arc::from("test-principal"), owner(v))
    }

    fn unbound() -> Visibility {
        Visibility::unrestricted(Arc::from("test-principal"))
    }

    fn cached_at(loaded_at: Instant) -> CachedView {
        CachedView {
            view: Arc::new(DirectoryView::build(Vec::new())),
            loaded_at,
        }
    }

    #[test]
    fn a_view_younger_than_the_interval_is_still_fresh() {
        let now = Instant::now();
        let cached = cached_at(now);
        assert!(cached.is_fresh(
            ReloadInterval::from_secs(5),
            now + std::time::Duration::from_secs(4),
        ));
    }

    #[test]
    fn a_view_older_than_the_interval_is_stale() {
        let now = Instant::now();
        let cached = cached_at(now);
        assert!(!cached.is_fresh(
            ReloadInterval::from_secs(5),
            now + std::time::Duration::from_secs(5),
        ));
        assert!(!cached.is_fresh(
            ReloadInterval::from_secs(5),
            now + std::time::Duration::from_mins(10),
        ));
    }

    /// `0` is the pre-expiry behaviour, and the only way to get it back.
    #[test]
    fn a_zero_interval_keeps_a_view_fresh_forever() {
        let now = Instant::now();
        let cached = cached_at(now);
        assert!(cached.is_fresh(
            ReloadInterval::from_secs(0),
            now + std::time::Duration::from_hours(24),
        ));
    }

    /// A clock that appears to run backwards (which `Instant` forbids, but
    /// arithmetic on one built by a caller does not) must not read as an
    /// enormous age and force a reload on every request.
    #[test]
    fn a_view_loaded_after_now_is_fresh_rather_than_infinitely_old() {
        let now = Instant::now();
        let cached = cached_at(now + std::time::Duration::from_secs(10));
        assert!(cached.is_fresh(ReloadInterval::from_secs(5), now));
    }

    #[test]
    fn the_default_directory_expires_its_view() {
        assert!(!NamespaceDirectory::new().freshness().is_disabled());
    }

    fn view() -> DirectoryView {
        DirectoryView::build(vec![
            record("orders", "orders", "acme"),
            record("ns_0199a", "orders", "beta"),
            record("billing", "billing", "acme"),
        ])
    }

    /// The requirement the whole registry exists for: two owners, same human
    /// name, different ids, neither able to see the other.
    #[test]
    fn same_name_under_two_owners_stays_separate() {
        let view = view();
        assert_eq!(
            view.resolve(&owner("acme"), &NamespaceName::parse("orders").unwrap())
                .unwrap()
                .as_str(),
            "orders"
        );
        assert_eq!(
            view.resolve(&owner("beta"), &NamespaceName::parse("orders").unwrap())
                .unwrap()
                .as_str(),
            "ns_0199a"
        );

        let acme = bound("acme");
        let beta = bound("beta");
        assert!(view.admits(&acme, "orders"));
        assert!(!view.admits(&beta, "orders"));
        assert!(view.admits(&beta, "ns_0199a"));
        assert!(!view.admits(&acme, "ns_0199a"));
    }

    #[test]
    fn unrestricted_visibility_admits_everything_including_unregistered() {
        let view = view();
        assert!(view.admits(&unbound(), "orders"));
        assert!(view.admits(&unbound(), "never-registered"));
    }

    /// A namespace with no registry row has no owner. Guessing one would hand
    /// a tenant data that is not theirs, so the answer is no.
    #[test]
    fn owner_scope_refuses_unregistered_namespaces() {
        let view = view();
        let acme = bound("acme");
        assert!(!view.admits(&acme, "never-registered"));
    }

    /// A namespace string that is not even a legal id cannot belong to
    /// anyone, and must not be admitted by a scoped caller on a technicality.
    #[test]
    fn owner_scope_refuses_malformed_namespaces() {
        let view = view();
        let acme = bound("acme");
        assert!(!view.admits(&acme, ""));
        assert!(!view.admits(&acme, "has space"));
    }

    #[test]
    fn owned_by_lists_only_that_owners_namespaces() {
        let view = view();
        let acme: Vec<String> = view
            .owned_by(&owner("acme"))
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(acme, vec!["billing".to_string(), "orders".to_string()]);
        assert!(view.owned_by(&owner("nobody")).is_empty());
    }

    #[test]
    fn resolve_misses_across_the_owner_boundary() {
        let view = view();
        assert!(view
            .resolve(&owner("beta"), &NamespaceName::parse("billing").unwrap())
            .is_none());
    }

    #[test]
    fn owned_branch_name_splits_owner_and_name() {
        let parsed = OwnedBranchName::try_parse("@acme:feature-x")
            .unwrap()
            .unwrap();
        assert_eq!(parsed.owner(), &owner("acme"));
        assert_eq!(parsed.name(), "feature-x");
        assert_eq!(parsed.to_string(), "@acme:feature-x");
    }

    /// No leading `@` is the signal this is an ordinary global branch, not
    /// an error: callers must be able to tell the two cases apart without
    /// matching on a `Status`.
    #[test]
    fn owned_branch_name_is_none_for_a_plain_branch() {
        assert!(OwnedBranchName::try_parse("feature-x").is_none());
        assert!(OwnedBranchName::try_parse("team/feature-x").is_none());
    }

    /// `@` with no `:` after it is also "not owned-shaped" rather than a
    /// malformed-owned-branch error, so a branch literally named `@weird`
    /// (if one ever existed) is not mistaken for a half-written owner tag.
    #[test]
    fn owned_branch_name_is_none_without_a_colon() {
        assert!(OwnedBranchName::try_parse("@acme").is_none());
    }

    #[test]
    fn owned_branch_name_rejects_an_invalid_owner() {
        let err = OwnedBranchName::try_parse("@has space:feature")
            .unwrap()
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn owned_branch_name_rejects_an_invalid_name() {
        let err = OwnedBranchName::try_parse("@acme:has space")
            .unwrap()
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn owned_branch_name_allows_a_slash_in_the_name_half() {
        let parsed = OwnedBranchName::try_parse("@acme:team/feature-x")
            .unwrap()
            .unwrap();
        assert_eq!(parsed.name(), "team/feature-x");
    }

    /// The same operator-chosen owner string two different tenants' owned
    /// branches both carry must parse to the same `OwnerId`, the way two
    /// namespaces under the same owner do.
    #[test]
    fn owned_branch_name_reuses_namespace_ownership_identity() {
        let branch = OwnedBranchName::try_parse("@acme:orders").unwrap().unwrap();
        let ns = view();
        assert!(ns
            .owned_by(branch.owner())
            .contains(&NamespaceId::parse("orders").unwrap()));
    }
}
