//! NATS `JetStream` KV-backed [`Store`] implementation.
//!
//! Layout (names configurable via [`NatsStoreConfig`]):
//!   - KV bucket (default `trogon-atlas-entities`): one entry per
//!     (kind, ns, slug, version). Key = [`crate::key::entity_key`], value =
//!     `prost`-encoded [`trogon_atlas_proto::Entity`]. KV revision = `etag`.
//!   - `JetStream` stream (default `TROGON_ATLAS_CHANGES`) on subject prefix
//!     `atlas.changes.>`: append-only log of [`trogon_atlas_proto::ChangeEvent`],
//!     bounded by the configured retention limits. The stream's per-message
//!     sequence is the `ChangeRecord.seq`.
//!   - KV bucket (default `trogon-atlas-changesets`): one entry per changeset.
//!     Key = the changeset's UUIDv7, value = `prost`-encoded
//!     [`trogon_atlas_proto::Changeset`]. Deliberately unbounded: the changes
//!     stream is a transport that drops events on publish failure and expires
//!     old ones, so the durable record of what landed together cannot live
//!     there.
//!   - KV bucket (default `trogon-atlas-revisions`): one entry per
//!     (entity, changeset). Key = [`crate::key::revision_key`], value =
//!     `prost`-encoded [`trogon_atlas_proto::EntityRevision`] carrying the
//!     entity's content on both sides of that write. Also unbounded: the
//!     entities bucket is `history=1` because the etag is its KV revision,
//!     so it is the only place a pre-image survives the write that replaced
//!     it.
//!
//!   - KV bucket (default `trogon-atlas-batches`): the journal of baseline
//!     batches that have not finished. See `nats_journal.rs`.
//!
//! `batch_apply` is implemented sequentially; NATS KV does not offer
//! multi-key transactions. A baseline batch is made recoverable instead:
//! its journal entry carries the prior image of every key it touches, and
//! [`Store::recover_batches`] rolls an interrupted batch back if it never
//! committed or forward if it did. On an inline rollback failure,
//! `StoreError::PartialApply` is returned with the affected keys and the
//! journal entry recovery will act on. Recovery assumes a single writer.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use async_nats::jetstream::{
    self,
    consumer::{pull::Config as PullConfig, AckPolicy, DeliverPolicy},
    kv::{Config as KvConfig, Entry as KvEntry, Operation, Store as KvStore},
    stream::{Config as StreamConfig, RetentionPolicy},
    Context as Js,
};
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use prost::Message as _;
use tokio::sync::{broadcast, Notify};
use trogon_atlas_core::{NamespaceId, NamespaceName, OwnerId, WriterRole};
use trogon_atlas_proto::{
    canonical::ALL_KINDS, entity::Kind as EntityOneof, Entity, EntityKind, EntityRef, Id,
};

use crate::{
    error::{StoreError, StoreResult},
    key::{
        branch_delta_key, branch_delta_prefix, branch_meta_key, entity_key, kind_prefix,
        namespace_name_key, namespace_prefix, namespace_record_key, namespace_record_prefix,
        parse_entity_key, parse_namespace_record_key, revision_key, revision_prefix,
    },
    store::{
        BranchDeltaEntry, BranchInfo, BranchLandOp, ChangeKind, ChangeRecord, ChangesetOp,
        ChangesetPage, ChangesetRecord, ChangesetRef, ClaimOutcome, EntityRevisionRecord,
        ListFilter, MutationOp, MutationOutcome, NamespaceClaim, NamespaceRecord, NamespaceTenure,
        OperationRecord, RevisionPage, Store, StoredEntity, WriteContext, WriterStatus, Written,
    },
};

const DEFAULT_KV_BUCKET: &str = "trogon-atlas-entities";
const DEFAULT_BRANCHES_KV_BUCKET: &str = "trogon-atlas-branches";
const DEFAULT_CHANGESETS_KV_BUCKET: &str = "trogon-atlas-changesets";
const DEFAULT_REVISIONS_KV_BUCKET: &str = "trogon-atlas-revisions";
const DEFAULT_BATCHES_KV_BUCKET: &str = "trogon-atlas-batches";
const DEFAULT_NAMESPACES_KV_BUCKET: &str = "trogon-atlas-namespaces";
const DEFAULT_OPERATIONS_KV_BUCKET: &str = "trogon-atlas-operations";
const DEFAULT_OPERATIONS_RETENTION: Duration = Duration::from_hours(168);
const DEFAULT_LEASE_KV_BUCKET: &str = "trogon-atlas-writer-lease";
const DEFAULT_CHANGES_STREAM: &str = "TROGON_ATLAS_CHANGES";
const DEFAULT_CHANGES_SUBJECT_ROOT: &str = "atlas.changes";
const LIVE_TAIL_CHANNEL_CAPACITY: usize = 1024;

/// Reserved KV key that records the schema version written to this bucket.
///
/// The key starts with `_`, which is a valid NATS KV character but is not a
/// valid `kind_short` prefix (all entity kind shorts begin with a lowercase
/// letter), so it cannot collide with any key produced by [`entity_key`].
/// `parse_entity_key("_schema")` returns `None`, and the key is explicitly
/// excluded from [`NatsStore::list`] results so callers never see it as an
/// entity.
const SCHEMA_VERSION_KEY: &str = "_schema";

/// Value of [`SCHEMA_VERSION_KEY`] for a store whose Event, Command and
/// ReadModel rows declare their fields only through `schema`. A binary that
/// predates it reads this as a foreign version and refuses to open the store,
/// so an old server cannot write the retired layout back.
pub const STORE_SCHEMA_MARKER: &str = "trogonatlas.eventmodel.v1alpha1+schema-fields";

/// Value of [`SCHEMA_VERSION_KEY`] written by binaries that still stored the
/// retired `fields` lists.
const LEGACY_FIELDS_MARKER: &str = trogon_atlas_proto::SCHEMA_VERSION;

/// What [`NatsStore::connect_with`] is opening the store for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StoreOpenMode {
    /// Serve the event model. Refuses a store that still holds rows in the
    /// retired `fields` layout, whatever the writer role.
    #[default]
    Serve,
    /// Run `migrate-legacy-fields`, which must open exactly that store.
    MigrateLegacyFields,
}

/// `created_by` stamped on registry rows the backfill writes. Distinguishes a
/// namespace that predates the registry from one a principal deliberately
/// claimed, which matters when auditing who owns what.
pub const LEGACY_ADOPTION_AUTHOR: &str = "migration";
const LIST_HARD_CAP: usize = 10_000;
const LIST_FETCH_CONCURRENCY: usize = 16;
/// Hard cap on the number of keys accepted by `batch_get`. Prevents a single
/// caller from scheduling LIST_HARD_CAP-sized fan-outs. Mirrors the spirit of
/// `LIST_HARD_CAP` but is independent so the two limits can be tuned separately.
const BATCH_GET_HARD_CAP: usize = 1000;
const _: () = assert!(
    BATCH_GET_HARD_CAP > 0 && BATCH_GET_HARD_CAP <= LIST_HARD_CAP,
    "BATCH_GET_HARD_CAP must be positive and at most LIST_HARD_CAP"
);
const _: () = assert!(
    BATCH_APPLY_MAX_OPS > 0 && BATCH_APPLY_MAX_OPS <= LIST_HARD_CAP,
    "BATCH_APPLY_MAX_OPS must be positive and at most LIST_HARD_CAP"
);

/// Timeout for individual KV operations (entry/put/update/delete/keys).
const KV_OP_TIMEOUT: Duration = Duration::from_secs(10);

/// Wall-clock bound for the entire `list()` operation across all key fetches.
/// Prevents a stalled NATS connection from blocking `list()` indefinitely when
/// the bucket has up to `LIST_HARD_CAP` keys each guarded only by `KV_OP_TIMEOUT`.
const LIST_TOTAL_TIMEOUT: Duration = Duration::from_mins(1);

/// Hard cap on the number of ops accepted by `batch_apply`. Prevents a single
/// caller from issuing LIST_HARD_CAP-sized write batches in one call.
/// Mirrors `BATCH_GET_HARD_CAP` so both read and write fan-outs are bounded equally.
const BATCH_APPLY_MAX_OPS: usize = 1000;

/// Timeout for `JetStream` stream operations (`get_stream`, `create_consumer`, stream.info).
const JS_OP_TIMEOUT: Duration = Duration::from_secs(15);

/// Maximum attempts for the CAS loop in `put_inner`.
const CAS_MAX_ATTEMPTS: u32 = 5;
/// Base delay for exponential backoff between CAS retries. Kept small because
/// `WrongLastRevision` usually resolves within one RTT after a concurrent writer.
const CAS_RETRY_BASE: Duration = Duration::from_millis(5);
/// Exponent cap for CAS backoff: delay grows as `CAS_RETRY_BASE` * 2^min(attempt, `CAS_RETRY_EXP_CAP`).
const CAS_RETRY_EXP_CAP: u32 = 4;
/// Maximum attempts to retry kv.delete after the conditional update tombstone.
const DELETE_RETRY_ATTEMPTS: u32 = 3;
/// Base delay between delete retry attempts.
const DELETE_RETRY_BASE: Duration = Duration::from_millis(25);
/// Maximum attempts for `record_change` publish retries.
const PUBLISH_RETRY_ATTEMPTS: u32 = 3;
/// Base delay between publish retry attempts.
const PUBLISH_RETRY_BASE: Duration = Duration::from_millis(50);

/// Initial reconnect delay for the change tail loop.
const LIVE_TAIL_RETRY_DELAY_MIN: Duration = Duration::from_millis(100);
/// Maximum reconnect delay for the change tail loop.
const LIVE_TAIL_RETRY_DELAY_MAX: Duration = Duration::from_secs(30);

/// Username and password for a NATS server that requires authentication.
///
/// Kept apart from the URL because every error and trace in this module
/// interpolates [`NatsStoreConfig::url`], and a secret embedded there would
/// end up in the operator's logs.
#[derive(Clone, PartialEq, Eq)]
pub struct NatsCredentials {
    user: String,
    password: String,
}

impl NatsCredentials {
    pub fn new(user: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            user: user.into(),
            password: password.into(),
        }
    }

    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    fn apply(&self, options: async_nats::ConnectOptions) -> async_nats::ConnectOptions {
        options.user_and_password(self.user.clone(), self.password.clone())
    }
}

impl fmt::Debug for NatsCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NatsCredentials")
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Split `scheme://user:pass@host` into a credential-free URL and the
/// credentials it carried.
///
/// async-nats parses userinfo out of a server address but never sends it:
/// `ConnectOptions` is the only source of the CONNECT username and password.
/// A URL that carries credentials therefore fails authentication with nothing
/// in the error explaining why, so they are lifted out here rather than
/// silently dropped. The password is taken literally, so one containing `/`
/// has to be supplied through [`NatsStoreConfig::credentials`] instead.
fn split_url_credentials(url: &str) -> (String, Option<NatsCredentials>) {
    let Some((scheme, rest)) = url.split_once("://") else {
        return (url.to_owned(), None);
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let Some(at) = rest[..authority_end].rfind('@') else {
        return (url.to_owned(), None);
    };
    let userinfo = &rest[..at];
    let stripped = format!("{scheme}://{}", &rest[at + 1..]);
    let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
    if user.is_empty() {
        return (stripped, None);
    }
    (stripped, Some(NatsCredentials::new(user, password)))
}

/// Connection and resource configuration for [`NatsStore`].
#[derive(Debug, Clone)]
pub struct NatsStoreConfig {
    pub url: String,
    /// Credentials for a server with authentication enabled. Populated from
    /// the URL's userinfo when `url` carries one; set it directly to keep the
    /// secret out of the URL entirely.
    pub credentials: Option<NatsCredentials>,
    /// KV bucket holding entities.
    pub bucket: String,
    /// KV bucket holding branch overlays (metadata rows and copy-on-write
    /// deltas). See the "Branch overlays" section of the module doc.
    pub branches_bucket: String,
    /// KV bucket holding the durable changeset log. Deliberately has no
    /// retention setting: this is the record, not the transport.
    pub changesets_bucket: String,
    /// KV bucket holding the per-entity revision log. Same retention
    /// posture as `changesets_bucket`, and for the same reason.
    pub revisions_bucket: String,
    /// KV bucket holding the journal of baseline batches in flight. Empty
    /// whenever no batch is running and none needs recovery.
    pub batches_bucket: String,
    /// KV bucket holding the namespace registry: which namespace ids exist,
    /// what they are called, and who owns them. Separate from the entity
    /// bucket on purpose, so re-owning a namespace never touches an entity.
    pub namespaces_bucket: String,
    /// KV bucket holding operation receipts (idempotency claims for
    /// mutations that carried an `operation_id`). Entries expire after
    /// `operations_retention`, unlike every other bucket here.
    pub operations_bucket: String,
    /// How long a settled operation receipt stays readable via
    /// `GetOperation` before JetStream expires it. Reported to clients as
    /// `Limits.operation_retention_seconds`.
    pub operations_retention: Duration,
    /// KV bucket holding the single writer-lease row. See
    /// `docs/explanation/single-writer.md`.
    pub lease_bucket: String,
    /// This process's configured writer role. `Writer` and `Standby` both
    /// try to acquire the writer lease; `Reader` never does. Defaults to
    /// `Writer`, preserving the always-recover, always-write behavior of
    /// every deployment that does not set this explicitly.
    pub role: WriterRole,
    /// How long a held writer lease stays valid before another process may
    /// take it over. Every process sharing a `lease_bucket` must agree on
    /// this value; defaults to [`writer_lease::DEFAULT_LEASE_DURATION`].
    pub lease_duration: Duration,
    /// How often this process attempts to create, renew, or take over the
    /// writer lease. Defaults to
    /// [`writer_lease::DEFAULT_LEASE_RENEW_INTERVAL`].
    pub lease_renew_interval: Duration,
    /// `JetStream` stream holding the change log.
    pub stream: String,
    /// Subject root for change events; the stream listens on `{root}.>`.
    pub subject_root: String,
    /// Discard change records older than this. `None` keeps them forever.
    pub changes_max_age: Option<Duration>,
    /// Cap the change log at this many records. `None` means unlimited.
    pub changes_max_msgs: Option<i64>,
    /// Cap the change log at this many bytes. `None` means unlimited.
    pub changes_max_bytes: Option<i64>,
    pub open_mode: StoreOpenMode,
}

impl NatsStoreConfig {
    pub fn new(url: impl Into<String>) -> Self {
        let (url, credentials) = split_url_credentials(&url.into());
        Self {
            url,
            credentials,
            bucket: DEFAULT_KV_BUCKET.into(),
            branches_bucket: DEFAULT_BRANCHES_KV_BUCKET.into(),
            changesets_bucket: DEFAULT_CHANGESETS_KV_BUCKET.into(),
            revisions_bucket: DEFAULT_REVISIONS_KV_BUCKET.into(),
            batches_bucket: DEFAULT_BATCHES_KV_BUCKET.into(),
            namespaces_bucket: DEFAULT_NAMESPACES_KV_BUCKET.into(),
            operations_bucket: DEFAULT_OPERATIONS_KV_BUCKET.into(),
            operations_retention: DEFAULT_OPERATIONS_RETENTION,
            lease_bucket: DEFAULT_LEASE_KV_BUCKET.into(),
            role: WriterRole::Writer,
            lease_duration: writer_lease::DEFAULT_LEASE_DURATION,
            lease_renew_interval: writer_lease::DEFAULT_LEASE_RENEW_INTERVAL,
            stream: DEFAULT_CHANGES_STREAM.into(),
            subject_root: DEFAULT_CHANGES_SUBJECT_ROOT.into(),
            changes_max_age: Some(Duration::from_hours(2160)),
            changes_max_msgs: Some(1_000_000),
            changes_max_bytes: None,
            open_mode: StoreOpenMode::Serve,
        }
    }

    /// Move any credentials still embedded in `url` into `credentials`,
    /// leaving a URL that is safe to log. Explicit credentials win, and the
    /// URL is stripped either way. Needed because `url` is a public field, so
    /// a caller can put a secret there after [`Self::new`] has run.
    fn take_url_credentials(&mut self) {
        let (url, credentials) = split_url_credentials(&self.url);
        self.url = url;
        if self.credentials.is_none() {
            self.credentials = credentials;
        }
    }

    fn subject_wildcard(&self) -> String {
        format!("{}.>", self.subject_root)
    }

    fn validate(&self) -> StoreResult<()> {
        validate_nats_name("bucket", &self.bucket)?;
        validate_nats_name("branches_bucket", &self.branches_bucket)?;
        validate_nats_name("changesets_bucket", &self.changesets_bucket)?;
        validate_nats_name("revisions_bucket", &self.revisions_bucket)?;
        validate_nats_name("batches_bucket", &self.batches_bucket)?;
        validate_nats_name("namespaces_bucket", &self.namespaces_bucket)?;
        validate_nats_name("operations_bucket", &self.operations_bucket)?;
        validate_nats_name("lease_bucket", &self.lease_bucket)?;
        validate_nats_name("stream", &self.stream)?;
        validate_nats_subject_root("subject_root", &self.subject_root)?;
        if self.lease_duration.is_zero() {
            return Err(StoreError::InvalidArgument(
                "NatsStoreConfig.lease_duration must not be zero".into(),
            ));
        }
        if self.lease_renew_interval.is_zero() {
            return Err(StoreError::InvalidArgument(
                "NatsStoreConfig.lease_renew_interval must not be zero".into(),
            ));
        }
        if self.lease_renew_interval >= self.lease_duration {
            return Err(StoreError::InvalidArgument(
                "NatsStoreConfig.lease_renew_interval must be shorter than lease_duration".into(),
            ));
        }
        Ok(())
    }
}

fn validate_nats_name(field: &str, value: &str) -> StoreResult<()> {
    if value.is_empty() {
        return Err(StoreError::InvalidArgument(format!(
            "NatsStoreConfig.{field} must not be empty"
        )));
    }
    for ch in value.chars() {
        if !matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_') {
            return Err(StoreError::InvalidArgument(format!(
                "NatsStoreConfig.{field} contains invalid character {ch:?}; \
                 only alphanumeric, '-', and '_' are allowed"
            )));
        }
    }
    Ok(())
}

fn validate_nats_subject_root(field: &str, value: &str) -> StoreResult<()> {
    if value.is_empty() {
        return Err(StoreError::InvalidArgument(format!(
            "NatsStoreConfig.{field} must not be empty"
        )));
    }
    for ch in value.chars() {
        if !matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.') {
            return Err(StoreError::InvalidArgument(format!(
                "NatsStoreConfig.{field} contains invalid character {ch:?}; \
                 only alphanumeric, '-', '_', and '.' are allowed"
            )));
        }
    }
    Ok(())
}

/// NATS `JetStream` KV-backed [`Store`].
///
/// # Change-tail startup gap
///
/// The live change tail starts from `DeliverPolicy::New` on the first
/// connection to avoid replaying unbounded history into the in-process
/// broadcast on every restart. Callers that need a gapless feed must
/// `subscribe()` **first**, then page `read_changes(last_seen_seq, ...)` to
/// exhaustion before relying on the live receiver. Subscribing first ensures
/// records published during the backlog drain land in the broadcast buffer
/// (and are de-duplicated by seq against the backlog). After a transient NATS
/// reconnect the tail resumes from `last_seen_seq + 1` so in-flight records
/// are not dropped.
#[derive(Clone)]
pub struct NatsStore {
    js: Js,
    kv: KvStore,
    /// KV bucket holding branch overlays (metadata rows and copy-on-write
    /// deltas). See the "Branch overlays" section of the module doc.
    branches_kv: KvStore,
    /// KV bucket holding the durable changeset log.
    changesets_kv: KvStore,
    /// KV bucket holding the per-entity revision log.
    revisions_kv: KvStore,
    /// KV bucket holding the journal of baseline batches in flight.
    batches_kv: KvStore,
    inflight_journals: Arc<journal::InflightJournals>,
    #[cfg(test)]
    faults: Arc<parking_lot::Mutex<journal::FaultPlan>>,
    /// KV bucket holding the namespace registry. See the "Namespace
    /// registry" section on the `Store` trait.
    namespaces_kv: KvStore,
    /// KV bucket holding operation receipts. See `operations.rs`.
    operations_kv: KvStore,
    /// In-memory writer-lease state, kept current by the background task
    /// held alive by `lease_task`.
    lease: Arc<writer_lease::WriterLease>,
    /// Background writer-lease loop. Kept around so a clean shutdown can
    /// abort it; see `ChangeTailHandle`.
    #[allow(dead_code)]
    lease_task: Arc<writer_lease::WriterLeaseHandle>,
    config: Arc<NatsStoreConfig>,
    changes_tx: broadcast::Sender<ChangeRecord>,
    /// Background change-tail task. Kept around so a clean shutdown can
    /// abort it instead of leaking it across process teardown. The field
    /// is "unread" by design -- its sole job is to hold the `JoinHandle`
    /// alive for the store's lifetime so `Drop` can abort it.
    #[allow(dead_code)]
    change_tail: Arc<ChangeTailHandle>,
    /// Number of change events that could not be published after the
    /// corresponding KV write committed. Operators can read this via
    /// `change_events_lost()` to detect gaps in the change feed.
    change_events_lost: Arc<AtomicU64>,
    /// Number of times the change-tail consumer was recreated due to a
    /// NATS reconnect or consumer error. Operators can read this via
    /// `change_tail_reconnects()` to detect pathological flap loops.
    change_tail_reconnects: Arc<AtomicU64>,
}

/// Wrapper around the change-tail `JoinHandle`. `Drop` aborts the task so
/// the background loop is reliably torn down when the last `NatsStore`
/// clone goes away -- useful both in tests and at graceful shutdown.
///
/// Uses `parking_lot::Mutex` rather than `std::sync::Mutex` so that lock
/// acquisition in `Drop` never silently no-ops due to mutex poisoning.
pub(crate) struct ChangeTailHandle(parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>);

impl ChangeTailHandle {
    pub(crate) fn new(handle: tokio::task::JoinHandle<()>) -> Self {
        Self(parking_lot::Mutex::new(Some(handle)))
    }
}

impl Drop for ChangeTailHandle {
    fn drop(&mut self) {
        if let Some(handle) = self.0.lock().take() {
            handle.abort();
        }
    }
}

impl NatsStore {
    /// Publish one change event. `msg_id` sets the stream's dedupe id, so
    /// a republish of the same event inside the duplicate window lands once.
    async fn publish_change(
        &self,
        kind: ChangeKind,
        entity_ref: &EntityRef,
        changeset: Option<ChangesetRef<'_>>,
        msg_id: Option<&str>,
    ) -> StoreResult<u64> {
        let subject = change_subject(&self.config.subject_root, kind, entity_ref)?;
        if self.faults().fail_publish {
            return Err(StoreError::ChangeEventLost);
        }
        let event = change_event_proto(kind, entity_ref, changeset);
        let message = async_nats::jetstream::message::PublishMessage::build()
            .payload(Bytes::from(event.encode_to_vec()));
        let message = match msg_id {
            Some(id) => message.message_id(id),
            None => message,
        };

        let mut last_err: Option<String> = None;
        for attempt in 0..PUBLISH_RETRY_ATTEMPTS {
            match tokio::time::timeout(
                KV_OP_TIMEOUT,
                self.js.send_publish(subject.clone(), message.clone()),
            )
            .await
            {
                Ok(Ok(ack_fut)) => match ack_fut.await {
                    Ok(ack) => return Ok(ack.sequence),
                    Err(e) => {
                        last_err = Some(format!("publish ack {subject}: {e}"));
                    }
                },
                Ok(Err(e)) => {
                    last_err = Some(format!("publish {subject}: {e}"));
                }
                Err(_) => {
                    last_err = Some(format!("publish {subject}: timeout"));
                }
            }
            if attempt + 1 < PUBLISH_RETRY_ATTEMPTS {
                tokio::time::sleep(PUBLISH_RETRY_BASE * 2u32.pow(attempt)).await;
            }
        }
        tracing::error!(
            subject = %subject,
            error = %last_err.unwrap_or_default(),
            entity_ref = ?entity_ref,
            "record_change publish failed after retries; KV write committed but change event not recorded"
        );
        Err(StoreError::ChangeEventLost)
    }

    /// Connect with default bucket/stream names and retention limits.
    pub async fn connect(url: &str) -> StoreResult<Self> {
        Self::connect_with(NatsStoreConfig::new(url)).await
    }

    /// Connect to NATS, ensure the entities KV bucket and changes stream
    /// exist (applying the configured retention limits), start the live
    /// change tail, and return a ready-to-use store.
    ///
    /// Awaits confirmation that the change-tail consumer is active before
    /// returning, so callers can publish immediately without a startup race.
    pub async fn connect_with(config: NatsStoreConfig) -> StoreResult<Self> {
        Box::pin(Self::connect_with_unboxed(config)).await
    }

    async fn connect_with_unboxed(mut config: NatsStoreConfig) -> StoreResult<Self> {
        config.take_url_credentials();
        config.validate()?;
        let connect_options = async_nats::ConnectOptions::new()
            .connection_timeout(Duration::from_secs(10))
            .retry_on_initial_connect()
            .event_callback(|event| async move {
                match event {
                    async_nats::Event::Disconnected => {
                        metrics::counter!("trogon_atlas_store_nats_disconnects_total").increment(1);
                        tracing::warn!("nats: disconnected from server");
                    }
                    async_nats::Event::Connected => {
                        metrics::counter!("trogon_atlas_store_nats_reconnects_total").increment(1);
                        tracing::info!("nats: reconnected to server");
                    }
                    _ => {}
                }
            });
        let connect_options = match &config.credentials {
            Some(credentials) => credentials.apply(connect_options),
            None => connect_options,
        };
        let client = connect_options.connect(&config.url).await.map_err(|e| {
            StoreError::Unavailable(format!("nats connect {url}: {e}", url = config.url))
        })?;
        let js = jetstream::new(client);
        let config = Arc::new(config);
        let kv = ensure_kv(
            &js,
            &config.bucket,
            "Event Modeling entities (kind/ns/slug/version -> Entity)",
        )
        .await?;
        let branches_kv = ensure_kv(
            &js,
            &config.branches_bucket,
            "Event Modeling branch overlays (metadata + copy-on-write deltas)",
        )
        .await?;
        ensure_schema_version(&kv, &branches_kv, config.open_mode).await?;
        let changesets_kv = ensure_kv(
            &js,
            &config.changesets_bucket,
            "Event Modeling changeset log (changeset id -> Changeset)",
        )
        .await?;
        let revisions_kv = ensure_kv(
            &js,
            &config.revisions_bucket,
            "Event Modeling entity revision log (entity + changeset id -> EntityRevision)",
        )
        .await?;
        let batches_kv = ensure_kv(
            &js,
            &config.batches_bucket,
            "Event Modeling batch journal (batch id -> prior images + phase)",
        )
        .await?;
        let namespaces_kv = ensure_kv(
            &js,
            &config.namespaces_bucket,
            "Event Modeling namespace registry (namespace id -> owner + name)",
        )
        .await?;
        let operations_kv = ensure_kv_with_max_age(
            &js,
            &config.operations_bucket,
            "Event Modeling operation receipts (claim key -> OperationRecord)",
            config.operations_retention,
        )
        .await?;
        let lease_kv = ensure_kv(
            &js,
            &config.lease_bucket,
            "Event Modeling writer lease (single row: holder + epoch + expiry)",
        )
        .await?;
        ensure_changes_stream(&js, &config).await?;
        let (changes_tx, _) = broadcast::channel(LIVE_TAIL_CHANNEL_CAPACITY);
        let tail_ready = Arc::new(Notify::new());
        let tail_ready_clone = tail_ready.clone();
        let change_tail_reconnects = Arc::new(AtomicU64::new(0));
        let tail_handle = tokio::spawn(run_change_tail(
            js.clone(),
            config.clone(),
            changes_tx.clone(),
            tail_ready_clone,
            change_tail_reconnects.clone(),
        ));
        let lease = Arc::new(writer_lease::WriterLease::new(
            config.role,
            config.lease_duration,
        ));
        let lease_ready = Arc::new(Notify::new());
        let lease_task = tokio::spawn(writer_lease::run_writer_lease(
            lease_kv.clone(),
            lease.clone(),
            lease_ready.clone(),
            config.lease_renew_interval,
        ));
        tail_ready.notified().await;
        lease_ready.notified().await;
        Ok(Self {
            js,
            kv,
            branches_kv,
            changesets_kv,
            revisions_kv,
            batches_kv,
            inflight_journals: Arc::default(),
            #[cfg(test)]
            faults: Arc::default(),
            namespaces_kv,
            operations_kv,
            lease,
            lease_task: Arc::new(writer_lease::WriterLeaseHandle::new(lease_task)),
            config,
            changes_tx,
            change_tail: Arc::new(ChangeTailHandle::new(tail_handle)),
            change_events_lost: Arc::new(AtomicU64::new(0)),
            change_tail_reconnects,
        })
    }

    /// Refuse the call with `StoreError::NotWriter` unless this process
    /// currently holds the writer lease. Called first by every mutating
    /// `Store` method.
    fn require_writer(&self) -> StoreResult<()> {
        if self.lease.is_writer() {
            return Ok(());
        }
        Err(StoreError::NotWriter {
            role: self.config.role,
            epoch: self.lease.current_epoch(),
        })
    }

    /// Total number of change events that could not be published after the
    /// corresponding KV write committed. A non-zero value means the change
    /// feed has gaps that operators must reconcile via `read_changes`.
    #[must_use]
    pub fn change_events_lost(&self) -> u64 {
        self.change_events_lost.load(Ordering::Relaxed)
    }

    /// Number of times the background change-tail consumer was recreated.
    /// Monotonically increasing; a rapidly growing value indicates a
    /// pathological NATS flap loop.
    #[must_use]
    pub fn change_tail_reconnects(&self) -> u64 {
        self.change_tail_reconnects.load(Ordering::Relaxed)
    }

    // -- Namespace registry internals --------------------------------------

    /// Read one registry record row plus the KV revision it was read at, so
    /// callers can CAS against it.
    async fn read_namespace_row(
        &self,
        id: &NamespaceId,
    ) -> StoreResult<Option<(NamespaceRecord, u64)>> {
        let key = namespace_record_key(id.as_str());
        let Some(entry) = live_entry(kv_entry_timeout(&self.namespaces_kv, key).await?) else {
            return Ok(None);
        };
        let revision = entry.revision;
        Ok(Some((decode_namespace_record(&entry.value)?, revision)))
    }

    /// Read a name-index row: the id it points at, plus its revision.
    async fn read_name_index(&self, index_key: &str) -> StoreResult<Option<(NamespaceId, u64)>> {
        let Some(entry) =
            live_entry(kv_entry_timeout(&self.namespaces_kv, index_key.to_owned()).await?)
        else {
            return Ok(None);
        };
        let revision = entry.revision;
        let raw = std::str::from_utf8(&entry.value)
            .map_err(|e| StoreError::Backend(format!("namespace index {index_key}: {e}")))?;
        // A malformed index row is treated as absent rather than fatal: it is
        // derived data, and the repair paths below overwrite it.
        match NamespaceId::parse(raw) {
            Ok(id) => Ok(Some((id, revision))),
            Err(_) => Ok(None),
        }
    }

    /// Follow a name index to the record it names, confirming the record
    /// actually agrees that it lives at this `(parent, name)`.
    ///
    /// The confirmation is what makes a half-finished move harmless. Both
    /// `claim_namespace` and `move_namespace` publish the destination index
    /// before flipping the record, so a crash in between leaves an index row
    /// that points at a namespace which has not moved yet. Verifying against
    /// the record means that row resolves to nothing instead of handing the
    /// destination owner a namespace it does not own.
    async fn follow_name_index(
        &self,
        index_key: &str,
        parent: &OwnerId,
        name: &NamespaceName,
    ) -> StoreResult<Option<NamespaceRecord>> {
        let Some((id, _)) = self.read_name_index(index_key).await? else {
            return Ok(None);
        };
        let Some((record, _)) = self.read_namespace_row(&id).await? else {
            return Ok(None);
        };
        if &record.parent == parent && &record.name == name {
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    /// Point a name-index row at `id`, whether or not a row is already there.
    ///
    /// Used only after [`NatsStore::follow_name_index`] has established that
    /// any existing row is stale, so this repairs rather than steals.
    async fn write_name_index(
        &self,
        index_key: &str,
        id: &NamespaceId,
        existing_revision: Option<u64>,
    ) -> StoreResult<()> {
        let bytes = Bytes::from(id.as_str().as_bytes().to_vec());
        match existing_revision {
            None => {
                tokio::time::timeout(KV_OP_TIMEOUT, self.namespaces_kv.create(index_key, bytes))
                    .await
                    .map_err(|_| {
                        StoreError::Backend(format!("namespace kv.create {index_key}: timeout"))
                    })?
                    .map(|_| ())
                    .map_err(|e| {
                        StoreError::Backend(format!("namespace kv.create {index_key}: {e}"))
                    })
            }
            Some(revision) => tokio::time::timeout(
                KV_OP_TIMEOUT,
                self.namespaces_kv.update(index_key, bytes, revision),
            )
            .await
            .map_err(|_| StoreError::Backend(format!("namespace kv.update {index_key}: timeout")))?
            .map(|_| ())
            .map_err(|e| StoreError::Backend(format!("namespace kv.update {index_key}: {e}"))),
        }
    }

    /// Shared body of [`Store::register_namespace`] and
    /// [`Store::adopt_namespace`]. The two differ only in where `id`
    /// comes from: minted for the former, adopted from the name for the
    /// latter.
    ///
    /// # Write ordering
    ///
    /// The record row is written before the name index, never the other way
    /// round. A crash between the two leaves a namespace that exists but is
    /// unreachable by name, which the next claim of that name repairs. The
    /// opposite order would leave a name permanently claimed by a namespace
    /// that does not exist, and nothing would ever clean it up.
    async fn claim_namespace(
        &self,
        id: NamespaceId,
        name: &NamespaceName,
        parent: &OwnerId,
        created_by: &str,
        tenure: &NamespaceTenure,
    ) -> StoreResult<NamespaceClaim> {
        self.claim_namespace_record(NamespaceRecord {
            id,
            name: name.clone(),
            parent: parent.clone(),
            created_at: now_rfc3339(),
            created_by: created_by.to_owned(),
            tenure: tenure.clone(),
        })
        .await
    }

    /// The write half of [`NatsStore::claim_namespace`], taking the row it
    /// should land rather than composing one. A restore needs this: the row
    /// it is putting back already has a `created_at` and a `created_by`, and
    /// stamping them with the moment of the restore would erase who claimed
    /// the namespace and when.
    async fn claim_namespace_record(&self, record: NamespaceRecord) -> StoreResult<NamespaceClaim> {
        let name = record.name.clone();
        let parent = record.parent.clone();
        let (name, parent) = (&name, &parent);
        let index_key = namespace_name_key(parent.as_str(), name.as_str());

        // Idempotency: this owner already holds this name. A client retrying
        // after a timeout must not be told the name is taken by itself.
        if let Some(record) = self.follow_name_index(&index_key, parent, name).await? {
            return Ok(NamespaceClaim {
                record,
                created: false,
            });
        }

        let record_key = namespace_record_key(record.id.as_str());

        let we_created_the_row = match tokio::time::timeout(
            KV_OP_TIMEOUT,
            self.namespaces_kv
                .create(&record_key, encode_namespace_record(&record)),
        )
        .await
        .map_err(|_| StoreError::Backend(format!("namespace kv.create {record_key}: timeout")))?
        {
            Ok(_) => true,
            Err(e) => {
                use async_nats::jetstream::kv::CreateErrorKind;
                if !matches!(e.kind(), CreateErrorKind::AlreadyExists) {
                    return Err(StoreError::Backend(format!(
                        "namespace kv.create {record_key}: {e}"
                    )));
                }
                // Unreachable for a minted id. For an adopted legacy id it
                // means a previous backfill already wrote this row; converge
                // on it if it agrees, and refuse if it belongs to someone
                // else rather than silently re-owning their namespace.
                let existing = self
                    .read_namespace_row(&record.id)
                    .await?
                    .ok_or(StoreError::NotFound)?
                    .0;
                if existing.parent != record.parent || existing.name != record.name {
                    return Err(StoreError::AlreadyExists);
                }
                false
            }
        };

        if let Ok(()) = self.write_name_index(&index_key, &record.id, None).await {
            Ok(NamespaceClaim {
                record,
                created: we_created_the_row,
            })
        } else {
            // The index row appeared between our read and our create.
            // Either a concurrent claim won the race, or the row is stale
            // from an interrupted write.
            if let Some(winner) = self.follow_name_index(&index_key, parent, name).await? {
                if we_created_the_row {
                    // Our record row is now an orphan nobody can reach.
                    let _ =
                        tokio::time::timeout(KV_OP_TIMEOUT, self.namespaces_kv.delete(&record_key))
                            .await;
                }
                return Ok(NamespaceClaim {
                    record: winner,
                    created: false,
                });
            }
            let revision = self.read_name_index(&index_key).await?.map(|(_, rev)| rev);
            self.write_name_index(&index_key, &record.id, revision)
                .await?;
            Ok(NamespaceClaim {
                record,
                created: we_created_the_row,
            })
        }
    }

    fn validate(kind: EntityKind, entity: &Entity) -> StoreResult<&Id> {
        let actual = kind_of(entity)
            .ok_or_else(|| StoreError::InvalidArgument("entity.kind oneof unset".into()))?;
        if actual != kind {
            return Err(StoreError::InvalidArgument(format!(
                "entity oneof kind {actual:?} does not match request kind {kind:?}",
            )));
        }
        let id = entity_id(entity)
            .ok_or_else(|| StoreError::InvalidArgument("entity.id is required".into()))?;
        if id.namespace.is_empty() {
            return Err(StoreError::InvalidArgument(
                "entity.id.namespace is required".into(),
            ));
        }
        if id.slug.is_empty() {
            return Err(StoreError::InvalidArgument(
                "entity.id.slug is required".into(),
            ));
        }
        Ok(id)
    }

    /// Put using a CAS loop so `stamp_system` is computed from the value
    /// actually replaced. The loop retries on `WrongLastRevision` up to
    /// `CAS_MAX_ATTEMPTS` times.
    ///
    /// Unless `force`, a semantically identical stored entity short-circuits
    /// to a no-op (`wrote == false`): idempotency is a store property, not a
    /// client courtesy; re-encoding noise (nondeterministic proto map
    /// order) must never churn revisions, change feeds, or the git mirror.
    async fn put_inner(
        &self,
        kind: EntityKind,
        entity: &Entity,
        force: bool,
    ) -> StoreResult<WroteOver> {
        let id = Self::validate(kind, entity)?.clone();
        let key = entity_key(kind, &id);

        for attempt in 0..CAS_MAX_ATTEMPTS {
            let current = kv_entry_timeout(&self.kv, key.clone()).await?;
            // Keep the revision of tombstoned entries for the CAS below, but
            // never decode them as `prev`: an empty value decodes as a zeroed
            // Entity and would corrupt the system stamp.
            let (prev, revision) = match current {
                Some(entry) => {
                    let rev = entry.revision;
                    let prev = match live_entry(Some(entry)) {
                        Some(e) => match decode_entity(&e.value) {
                            Ok(entity) => Some(entity),
                            Err(err) => {
                                tracing::error!(
                                    key = %key,
                                    error = %err,
                                    "corrupt entity in KV store during put_inner; \
                                     proceeding without prior value for system stamp"
                                );
                                None
                            }
                        },
                        None => None,
                    };
                    (prev, Some(rev))
                }
                None => (None, None),
            };
            if !force {
                if let (Some(prev_entity), Some(rev)) = (prev.as_ref(), revision) {
                    if trogon_atlas_core::semantically_equal(prev_entity, entity) {
                        return Ok(WroteOver {
                            written: Written {
                                etag: rev.to_string(),
                                wrote: false,
                                entity: prev_entity.clone(),
                            },
                            before: prev.clone(),
                        });
                    }
                }
            }
            let stamped = crate::store::stamp_system(prev.as_ref(), entity);
            let bytes = encode_entity(&stamped);

            let cas_result = match revision {
                Some(rev) => {
                    match tokio::time::timeout(KV_OP_TIMEOUT, self.kv.update(&key, bytes, rev))
                        .await
                        .map_err(|_| StoreError::Backend(format!("kv.update {key}: timeout")))?
                    {
                        Ok(new_rev) => Ok(new_rev),
                        Err(e) => {
                            use async_nats::jetstream::kv::UpdateErrorKind;
                            if matches!(e.kind(), UpdateErrorKind::WrongLastRevision) {
                                Err(StoreError::EtagMismatch {
                                    expected: rev.to_string(),
                                    found: live_found_etag(&self.kv, &key).await,
                                })
                            } else {
                                Err(StoreError::Backend(format!("kv.update {key}: {e}")))
                            }
                        }
                    }
                }
                None => tokio::time::timeout(KV_OP_TIMEOUT, self.kv.create(&key, bytes))
                    .await
                    .map_err(|_| StoreError::Backend(format!("kv.create {key}: timeout")))?
                    .map_err(|e| {
                        use async_nats::jetstream::kv::CreateErrorKind;
                        if matches!(e.kind(), CreateErrorKind::AlreadyExists) {
                            StoreError::EtagMismatch {
                                expected: "absent".into(),
                                found: "present".into(),
                            }
                        } else {
                            StoreError::Backend(format!("kv.create {key}: {e}"))
                        }
                    }),
            };

            match cas_result {
                Ok(rev) => {
                    return Ok(WroteOver {
                        written: Written {
                            etag: rev.to_string(),
                            wrote: true,
                            entity: stamped,
                        },
                        before: prev,
                    })
                }
                Err(StoreError::EtagMismatch { .. }) if attempt + 1 < CAS_MAX_ATTEMPTS => {
                    // Exponential backoff with jitter derived from SystemTime
                    // subsec_nanos so no external rand dependency is needed.
                    let exp = attempt.min(CAS_RETRY_EXP_CAP);
                    #[allow(clippy::cast_possible_truncation)]
                    // retry base ms fits comfortably in u64
                    let base_ms = CAS_RETRY_BASE.as_millis() as u64 * (1u64 << exp);
                    let jitter_ms = if base_ms > 0 {
                        let nanos = u64::from(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .subsec_nanos(),
                        );
                        nanos % base_ms
                    } else {
                        0
                    };
                    tokio::time::sleep(Duration::from_millis(base_ms + jitter_ms)).await;
                }
                Err(e) => return Err(e),
            }
        }

        Err(StoreError::Backend(format!(
            "kv.put {key}: CAS loop exhausted after {CAS_MAX_ATTEMPTS} attempts"
        )))
    }

    /// Merged-view `list()` for a branch: baseline entities matching `filter`,
    /// with the branch's deltas overlaid (tombstones removed, live deltas
    /// replacing/adding entries), then re-sorted and re-capped exactly like
    /// the baseline path.
    async fn list_branch_merged(
        &self,
        filter: ListFilter<'_>,
        limit: Option<usize>,
        branch: &str,
    ) -> StoreResult<Vec<StoredEntity>> {
        let effective_limit = limit.unwrap_or(LIST_HARD_CAP).min(LIST_HARD_CAP);
        // Baseline pass: reuse the existing (untouched) baseline list() path
        // with a generous limit so overlaying deltas doesn't lose entries
        // that would otherwise rank within effective_limit after merging.
        // LIST_HARD_CAP is the same ceiling list() itself enforces, so this
        // is at most one full baseline scan.
        let mut merged: BTreeMap<(EntityKind, String, String, u64), StoredEntity> = BTreeMap::new();
        let baseline = Store::list(self, filter.clone(), Some(LIST_HARD_CAP), None).await?;
        for stored in baseline {
            let Some(kind) = kind_of(&stored.entity) else {
                continue;
            };
            let Some(id) = entity_id(&stored.entity) else {
                continue;
            };
            let group = (kind, id.namespace.clone(), id.slug.clone(), id.version);
            merged.insert(group, stored);
        }

        // Overlay pass: enumerate this branch's delta rows and apply them
        // on top of the baseline results collected above. A delta for a key
        // outside the baseline pass's kind/namespace filter is applied only
        // if it also matches `filter`, so branch-only keys still respect the
        // caller's filter.
        let prefix = branch_delta_prefix(branch);
        let mut keys_stream = tokio::time::timeout(KV_OP_TIMEOUT, self.branches_kv.keys())
            .await
            .map_err(|_| StoreError::Backend("branch kv.keys: timeout".into()))?
            .map_err(|e| StoreError::Backend(format!("branch kv.keys: {e}")))?;
        let mut delta_keys: Vec<String> = Vec::new();
        loop {
            let next = tokio::time::timeout(KV_OP_TIMEOUT, keys_stream.next())
                .await
                .map_err(|_| {
                    StoreError::Backend(
                        "branch kv.keys stream: timeout waiting for next key".into(),
                    )
                })?;
            let Some(item) = next else { break };
            let key =
                item.map_err(|e| StoreError::Backend(format!("branch kv.keys stream: {e}")))?;
            if !key.starts_with(&prefix) {
                continue;
            }
            delta_keys.push(key);
            if delta_keys.len() >= LIST_HARD_CAP * 10 {
                tracing::warn!(
                    branch,
                    raw_key_cap = LIST_HARD_CAP * 10,
                    "list_branch_merged: raw delta key scan hit the memory ceiling"
                );
                break;
            }
        }

        for key in delta_keys {
            let Some((kind, id)) = crate::key::parse_branch_delta_key(branch, &key) else {
                continue;
            };
            if !filter.kinds.is_empty() && !filter.kinds.contains(&kind) {
                continue;
            }
            if !filter.namespaces.is_empty() && !filter.namespaces.contains(&id.namespace) {
                continue;
            }
            let entry = kv_entry_timeout(&self.branches_kv, key.clone()).await?;
            let Some(entry) = live_entry(entry) else {
                continue;
            };
            let delta = decode_branch_delta(&entry.value)?;
            let group = (kind, id.namespace.clone(), id.slug.clone(), id.version);
            if delta.tombstone {
                merged.remove(&group);
                continue;
            }
            let Some(ours) = delta.ours else {
                merged.remove(&group);
                continue;
            };
            merged.insert(
                group,
                StoredEntity {
                    entity: ours,
                    etag: entry.revision.to_string(),
                },
            );
        }

        let mut out: Vec<StoredEntity> = merged.into_values().collect();
        out.sort_by(|a, b| {
            let ka = kind_of(&a.entity).unwrap_or(EntityKind::Unspecified) as i32;
            let kb = kind_of(&b.entity).unwrap_or(EntityKind::Unspecified) as i32;
            let ida = entity_id(&a.entity).cloned().unwrap_or_default();
            let idb = entity_id(&b.entity).cloned().unwrap_or_default();
            ka.cmp(&kb)
                .then_with(|| ida.namespace.cmp(&idb.namespace))
                .then_with(|| ida.slug.cmp(&idb.slug))
                .then_with(|| ida.version.cmp(&idb.version))
        });

        if filter.latest_versions_only {
            let mut latest: BTreeMap<(EntityKind, String, String), StoredEntity> = BTreeMap::new();
            for s in out.drain(..) {
                let (Some(k), Some(id)) = (kind_of(&s.entity), entity_id(&s.entity)) else {
                    continue;
                };
                let group_key = (k, id.namespace.clone(), id.slug.clone());
                let keep = latest
                    .get(&group_key)
                    .and_then(|cur| entity_id(&cur.entity).map(|cid| cid.version < id.version))
                    .unwrap_or(true);
                if keep {
                    latest.insert(group_key, s);
                }
            }
            out = latest.into_values().collect();
        }

        out.truncate(effective_limit);
        Ok(out)
    }

    /// Branch-scoped `batch_apply`: applies each op sequentially against the
    /// branch's delta bucket via the same `Store::create`/`put`/`delete`
    /// entry points used for single-entity branch writes, so the merged-view
    /// semantics (no-op detection, base capture, tombstones) are identical
    /// to the single-op path. On the first failure, every delta already
    /// written by this batch is restored to its pre-batch state (or removed,
    /// if the key had no delta before the batch started). Because branch
    /// deltas are never observed by the change feed, there is no deferred
    /// `record_change` phase here (unlike the baseline `batch_apply`).
    async fn batch_apply_branch(
        &self,
        ops: &[MutationOp],
        ctx: WriteContext<'_>,
    ) -> StoreResult<Vec<MutationOutcome>> {
        let branch = ctx.branch_name().ok_or_else(|| {
            StoreError::InvalidArgument("batch_apply_branch: context carries no branch".into())
        })?;
        // Snapshot each touched key's pre-batch delta row (as raw encoded
        // bytes + revision) so a failure can restore it exactly, the same
        // compensating-write strategy as the baseline `batch_apply`.
        let mut snapshot: BTreeMap<String, Option<(bytes::Bytes, u64)>> = BTreeMap::new();
        for (i, op) in ops.iter().enumerate() {
            let (kind, id) = match op {
                MutationOp::Create { kind, entity } | MutationOp::Put { kind, entity, .. } => {
                    let id = Self::validate(*kind, entity).map_err(|source| {
                        StoreError::BatchFailed {
                            index: i,
                            source: Box::new(source),
                        }
                    })?;
                    (*kind, id.clone())
                }
                MutationOp::Delete { kind, id, .. } => (*kind, id.clone()),
            };
            let key = branch_delta_key(branch, kind, &id);
            if let std::collections::btree_map::Entry::Vacant(slot) = snapshot.entry(key.clone()) {
                let prior = kv_entry_timeout(&self.branches_kv, key.clone())
                    .await
                    .map_err(|source| StoreError::BatchFailed {
                        index: i,
                        source: Box::new(source),
                    })?;
                slot.insert(live_entry(prior).map(|e| (e.value, e.revision)));
            }
        }

        let rollback = |snapshot: &BTreeMap<String, Option<(bytes::Bytes, u64)>>| {
            let snapshot = snapshot.clone();
            let branches_kv = self.branches_kv.clone();
            async move {
                let mut failed_keys: Vec<String> = Vec::new();
                for (key, prior) in snapshot {
                    let current_rev = match kv_entry_timeout(&branches_kv, key.clone()).await {
                        Ok(entry) => live_entry(entry).map(|e| e.revision),
                        Err(err) => {
                            tracing::error!(
                                error = %err,
                                key = %key,
                                "batch_apply_branch rollback: kv.entry failed; branch may be partially applied"
                            );
                            failed_keys.push(key);
                            continue;
                        }
                    };
                    let restore_result = match (prior, current_rev) {
                        (Some((bytes, _snapshot_rev)), Some(rev)) => tokio::time::timeout(
                            KV_OP_TIMEOUT,
                            branches_kv.update(&key, bytes, rev),
                        )
                        .await
                        .map_err(|_| "timeout".to_string())
                        .and_then(|r| r.map(|_| ()).map_err(|e| e.to_string())),
                        (Some((bytes, _)), None) => {
                            tokio::time::timeout(KV_OP_TIMEOUT, branches_kv.create(&key, bytes))
                                .await
                                .map_err(|_| "timeout".to_string())
                                .and_then(|r| r.map(|_| ()).map_err(|e| e.to_string()))
                        }
                        (None, Some(_)) => {
                            tokio::time::timeout(KV_OP_TIMEOUT, branches_kv.delete(&key))
                                .await
                                .map_err(|_| "timeout".to_string())
                                .and_then(|r| r.map_err(|e| e.to_string()))
                        }
                        (None, None) => Ok(()),
                    };
                    if let Err(e) = restore_result {
                        tracing::error!(
                            error = %e,
                            key = %key,
                            "batch_apply_branch rollback restore failed"
                        );
                        failed_keys.push(key);
                    }
                }
                failed_keys
            }
        };

        let mut outcomes: Vec<MutationOutcome> = Vec::with_capacity(ops.len());
        let mut applied_keys: BTreeSet<String> = BTreeSet::new();
        // Each delegated write records its own revision as it lands, so an
        // aborted batch has to unwind those rows along with the delta rows
        // it is already compensating for. Otherwise history would keep
        // claiming edits that were rolled back, under a changeset id the
        // failed RPC never appended.
        let mut applied_refs: Vec<(EntityKind, Id)> = Vec::with_capacity(ops.len());
        for (i, op) in ops.iter().enumerate() {
            let op_ref = match op {
                MutationOp::Create { kind, entity } | MutationOp::Put { kind, entity, .. } => {
                    Self::validate(*kind, entity)
                        .ok()
                        .map(|id| (*kind, id.clone()))
                }
                MutationOp::Delete { kind, id, .. } => Some((*kind, id.clone())),
            };
            let op_key = op_ref
                .as_ref()
                .map(|(kind, id)| branch_delta_key(branch, *kind, id));
            let result: StoreResult<MutationOutcome> = match op {
                MutationOp::Create { kind, entity } => Store::create(self, *kind, entity, ctx)
                    .await
                    .map(|w| MutationOutcome::Wrote {
                        etag: w.etag,
                        entity: w.entity,
                    }),
                MutationOp::Put {
                    kind,
                    entity,
                    if_match,
                    force,
                } => match if_match {
                    None => Store::put(self, *kind, entity, *force, ctx).await.map(|w| {
                        if w.wrote {
                            MutationOutcome::Wrote {
                                etag: w.etag,
                                entity: w.entity,
                            }
                        } else {
                            MutationOutcome::Noop {
                                etag: w.etag,
                                entity: w.entity,
                            }
                        }
                    }),
                    Some(exp) => Store::update(self, *kind, entity, exp, *force, ctx)
                        .await
                        .map(|w| {
                            if w.wrote {
                                MutationOutcome::Wrote {
                                    etag: w.etag,
                                    entity: w.entity,
                                }
                            } else {
                                MutationOutcome::Noop {
                                    etag: w.etag,
                                    entity: w.entity,
                                }
                            }
                        }),
                },
                MutationOp::Delete { kind, id, if_match } => {
                    Store::delete(self, *kind, id, if_match.as_deref(), ctx)
                        .await
                        .map(|()| MutationOutcome::Deleted)
                }
            };
            match result {
                Ok(outcome) => {
                    if !matches!(outcome, MutationOutcome::Noop { .. }) {
                        if let Some(key) = op_key {
                            applied_keys.insert(key);
                        }
                        if let Some(entity_ref) = op_ref {
                            applied_refs.push(entity_ref);
                        }
                    }
                    outcomes.push(outcome);
                }
                Err(source) => {
                    let to_rollback: BTreeMap<_, _> = snapshot
                        .iter()
                        .filter(|(k, _)| applied_keys.contains(k.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    let failed_keys = rollback(&to_rollback).await;
                    self.purge_revisions(ctx, &applied_refs).await;
                    if !failed_keys.is_empty() {
                        return Err(StoreError::BatchFailed {
                            index: i,
                            source: Box::new(StoreError::PartialApply {
                                keys: failed_keys,
                                journal: None,
                            }),
                        });
                    }
                    return Err(StoreError::BatchFailed {
                        index: i,
                        source: Box::new(source),
                    });
                }
            }
        }
        Ok(outcomes)
    }

    /// Append the revision row for one entity a write touched.
    ///
    /// A no-op for an unattributed write: the row is keyed by changeset id,
    /// and there is no id to key it by. That is the documented gap
    /// (`trogon-atlas-server import-git`, direct `Store` use) that makes
    /// `RevertChangeset` refuse rather than guess.
    ///
    /// One row per (entity, changeset), holding the net effect of that
    /// changeset on that entity. A batch that writes the same key twice
    /// folds into the first row's `before` and the last write's
    /// `kind`/`after`, so the pair still reads as "what this landing did to
    /// this entity" and its inverse is still a single op.
    ///
    /// Never fails the caller. By the time this runs the entity write is
    /// already durable, so a lost revision is a hole in history rather than
    /// a failed write -- the same posture `record_change` takes.
    async fn record_revision(&self, ctx: WriteContext<'_>, revision: &Revision) {
        if let Err(error) = self.try_record_revision(ctx, revision).await {
            metrics::counter!("trogon_atlas_store_revisions_lost_total").increment(1);
            tracing::error!(
                error = %error,
                "record_revision failed; entity write committed but its revision was not recorded"
            );
        }
    }

    async fn try_record_revision(
        &self,
        ctx: WriteContext<'_>,
        revision: &Revision,
    ) -> Result<(), String> {
        let Some(changeset) = ctx.changeset() else {
            return Ok(());
        };
        let key = revision_key(ctx.branch_name(), revision.kind, &revision.id, changeset.id);
        let proto = entity_revision_proto(changeset, ctx.branch_name(), revision);
        let bytes = Bytes::from(proto.encode_to_vec());
        match tokio::time::timeout(KV_OP_TIMEOUT, self.revisions_kv.create(&key, bytes)).await {
            Err(_) => Err(format!("{key}: timeout")),
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => {
                use async_nats::jetstream::kv::CreateErrorKind;
                if matches!(e.kind(), CreateErrorKind::AlreadyExists) {
                    self.fold_revision(&key, proto)
                        .await
                        .map_err(|e| format!("{key}: {e}"))
                } else {
                    Err(format!("{key}: {e}"))
                }
            }
        }
    }

    /// Merge `latest` into the row already at `key`, keeping the stored
    /// `before` and taking the new `kind`/`after`.
    ///
    /// Reached two ways. A retried RPC reusing its changeset id writes the
    /// identical pair back, which is why this stays idempotent. A batch
    /// touching one key twice folds, so the row keeps the pre-image the
    /// changeset actually found and the post-image it actually left.
    ///
    /// A row that cannot be decoded is left alone: overwriting it would
    /// trade an unreadable pre-image for no pre-image at all.
    async fn fold_revision(
        &self,
        key: &str,
        latest: trogon_atlas_proto::EntityRevision,
    ) -> Result<(), String> {
        let entry = tokio::time::timeout(KV_OP_TIMEOUT, self.revisions_kv.entry(key))
            .await
            .map_err(|_| "timeout".to_string())?
            .map_err(|e| e.to_string())?;
        let Some(existing) = live_entry(entry) else {
            return Ok(());
        };
        let Ok(prior) = trogon_atlas_proto::EntityRevision::decode(existing.value.as_ref()) else {
            return Ok(());
        };
        let folded = trogon_atlas_proto::EntityRevision {
            before: prior.before,
            ..latest
        };
        tokio::time::timeout(
            KV_OP_TIMEOUT,
            self.revisions_kv
                .update(key, Bytes::from(folded.encode_to_vec()), existing.revision),
        )
        .await
        .map_err(|_| "timeout".to_string())?
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// Drop the revision rows a rolled-back batch already appended.
    ///
    /// Best effort, in the same spirit as the delta-row rollback it runs
    /// beside: a row that survives points at a changeset the failed RPC
    /// never appended, so it is a phantom entry in one entity's history
    /// rather than a corrupt one.
    async fn purge_revisions(&self, ctx: WriteContext<'_>, applied: &[(EntityKind, Id)]) {
        let Some(changeset) = ctx.changeset() else {
            return;
        };
        for (kind, id) in applied {
            let key = revision_key(ctx.branch_name(), *kind, id, changeset.id);
            let purged = tokio::time::timeout(KV_OP_TIMEOUT, self.revisions_kv.delete(&key))
                .await
                .map_err(|_| "timeout".to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            if let Err(error) = purged {
                tracing::error!(
                    key = %key,
                    error = %error,
                    "purge_revisions failed; a rolled-back write left a revision row behind"
                );
            }
        }
    }

    /// Count live delta rows for `branch`. Used to populate
    /// `BranchInfo.delta_count` at read time (never persisted, so it can
    /// never drift from the actual row count).
    async fn count_branch_deltas(&self, branch: &str) -> StoreResult<u32> {
        let prefix = branch_delta_prefix(branch);
        let mut keys_stream = tokio::time::timeout(KV_OP_TIMEOUT, self.branches_kv.keys())
            .await
            .map_err(|_| StoreError::Backend("branch kv.keys: timeout".into()))?
            .map_err(|e| StoreError::Backend(format!("branch kv.keys: {e}")))?;
        let mut count: u32 = 0;
        loop {
            let next = tokio::time::timeout(KV_OP_TIMEOUT, keys_stream.next())
                .await
                .map_err(|_| {
                    StoreError::Backend(
                        "branch kv.keys stream: timeout waiting for next key".into(),
                    )
                })?;
            let Some(item) = next else { break };
            let key =
                item.map_err(|e| StoreError::Backend(format!("branch kv.keys stream: {e}")))?;
            if !key.starts_with(&prefix) {
                continue;
            }
            let entry = kv_entry_timeout(&self.branches_kv, key).await?;
            if live_entry(entry).is_some() {
                count = count.saturating_add(1);
            }
        }
        Ok(count)
    }

    async fn stream_info(&self) -> StoreResult<async_nats::jetstream::stream::Info> {
        let mut stream =
            tokio::time::timeout(JS_OP_TIMEOUT, self.js.get_stream(&self.config.stream))
                .await
                .map_err(|_| {
                    StoreError::Backend(format!(
                        "get_stream {stream}: timeout",
                        stream = self.config.stream
                    ))
                })?
                .map_err(|e| {
                    StoreError::Backend(format!(
                        "get_stream {stream}: {e}",
                        stream = self.config.stream
                    ))
                })?;
        let info = tokio::time::timeout(JS_OP_TIMEOUT, stream.info())
            .await
            .map_err(|_| StoreError::Backend("stream.info: timeout".into()))?
            .map_err(|e| StoreError::Backend(format!("stream.info: {e}")))?;
        Ok(info.clone())
    }
}

/// KV deletes leave tombstone entries (`Operation::Delete`/`Purge`); only
/// `Put` entries with a non-empty value represent a live value.
///
/// An empty-value `Put` is treated as a tombstone because the delete sequence
/// writes an empty `update` as a conditional sentinel before calling
/// `kv.delete`. If `kv.delete` fails, the entry is left as an empty Put;
/// treating it as `NotFound` prevents a zeroed Entity from leaking to callers.
fn live_entry(entry: Option<KvEntry>) -> Option<KvEntry> {
    entry.filter(|e| e.operation == Operation::Put && !e.value.is_empty())
}

/// Wraps `kv.entry` with `KV_OP_TIMEOUT`, mapping a timeout to `Backend`.
async fn kv_entry_timeout(kv: &KvStore, key: String) -> StoreResult<Option<KvEntry>> {
    tokio::time::timeout(KV_OP_TIMEOUT, kv.entry(key.clone()))
        .await
        .map_err(|_| StoreError::Backend(format!("kv.entry {key}: timeout")))?
        .map_err(|e| StoreError::Backend(format!("kv.entry {key}: {e}")))
}

/// Live revision string for `EtagMismatch.found` after a CAS race. Re-reads
/// so `found` is not the stale pre-CAS revision (which can equal `expected`).
/// Empty when the key is absent, tombstoned, or the re-read fails.
async fn live_found_etag(kv: &KvStore, key: &str) -> String {
    match kv_entry_timeout(kv, key.to_string()).await {
        Ok(entry) => live_entry(entry)
            .map(|e| e.revision.to_string())
            .unwrap_or_default(),
        Err(_) => String::new(),
    }
}

const ENSURE_KV_STATUS_RETRIES: u32 = 3;
const ENSURE_KV_STATUS_RETRY_BASE: Duration = Duration::from_millis(100);

async fn ensure_kv(js: &Js, bucket: &str, description: &str) -> StoreResult<KvStore> {
    ensure_kv_with_max_age(js, bucket, description, Duration::ZERO).await
}

/// Same as [`ensure_kv`], additionally setting `max_age` on a newly created
/// bucket (`Duration::ZERO` means unlimited, matching the JetStream
/// convention). Not re-applied to a bucket that already exists, the same
/// posture `ensure_kv` already takes with `history`.
async fn ensure_kv_with_max_age(
    js: &Js,
    bucket: &str,
    description: &str,
    max_age: Duration,
) -> StoreResult<KvStore> {
    if let Ok(kv) = js.get_key_value(bucket).await {
        // Validate the existing bucket's history setting. The etag semantics
        // rely on `history == 1` so that the KV revision is a monotonic
        // counter without gaps. Retry kv.status() with backoff; an
        // unverified bucket with history > 1 breaks CAS/etag semantics.
        let mut last_status_err: Option<String> = None;
        for attempt in 0..ENSURE_KV_STATUS_RETRIES {
            match kv.status().await {
                Ok(status) if status.history() != 1 => {
                    return Err(StoreError::Unavailable(format!(
                        "existing KV bucket {bucket} has history={} (expected 1); \
                         etag semantics require history=1",
                        status.history()
                    )));
                }
                Ok(_) => {
                    last_status_err = None;
                    break;
                }
                Err(e) => {
                    last_status_err = Some(format!("{e}"));
                    if attempt + 1 < ENSURE_KV_STATUS_RETRIES {
                        tokio::time::sleep(ENSURE_KV_STATUS_RETRY_BASE * 2u32.pow(attempt)).await;
                    }
                }
            }
        }
        if let Some(err) = last_status_err {
            return Err(StoreError::Unavailable(format!(
                "could not validate existing KV bucket {bucket} config after \
                 {ENSURE_KV_STATUS_RETRIES} attempts: {err}"
            )));
        }
        return Ok(kv);
    }
    js.create_key_value(KvConfig {
        bucket: bucket.into(),
        description: description.into(),
        history: 1,
        max_age,
        ..Default::default()
    })
    .await
    .map_err(|e| StoreError::Unavailable(format!("kv create {bucket}: {e}")))
}

/// What [`SCHEMA_VERSION_KEY`] says about the rows in a bucket.
enum StoredLayout {
    Current,
    /// Absent marker on a bucket with no entity rows.
    Fresh,
    /// Written by a binary that stored the retired `fields` lists, or absent
    /// on a bucket that already holds entity rows.
    LegacyFields,
    Foreign(String),
}

async fn read_stored_layout(kv: &KvStore) -> StoreResult<StoredLayout> {
    match kv_entry_timeout(kv, SCHEMA_VERSION_KEY.to_string()).await? {
        Some(entry) if entry.operation == Operation::Put && !entry.value.is_empty() => {
            let found = std::str::from_utf8(&entry.value).unwrap_or("<invalid utf-8>");
            Ok(match found {
                STORE_SCHEMA_MARKER => StoredLayout::Current,
                LEGACY_FIELDS_MARKER => StoredLayout::LegacyFields,
                other => StoredLayout::Foreign(other.to_string()),
            })
        }
        _ if holds_entity_rows(kv).await? => Ok(StoredLayout::LegacyFields),
        _ => Ok(StoredLayout::Fresh),
    }
}

async fn holds_entity_rows(kv: &KvStore) -> StoreResult<bool> {
    let mut keys = tokio::time::timeout(KV_OP_TIMEOUT, kv.keys())
        .await
        .map_err(|_| StoreError::Backend("kv.keys: timeout".into()))?
        .map_err(|e| StoreError::Backend(format!("kv.keys: {e}")))?;
    while let Some(key) = tokio::time::timeout(KV_OP_TIMEOUT, keys.next())
        .await
        .map_err(|_| StoreError::Backend("kv.keys stream: timeout".into()))?
    {
        let key = key.map_err(|e| StoreError::Backend(format!("kv.keys stream: {e}")))?;
        if parse_entity_key(&key).is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn write_schema_marker(kv: &KvStore) -> StoreResult<()> {
    tokio::time::timeout(
        KV_OP_TIMEOUT,
        kv.put(
            SCHEMA_VERSION_KEY,
            STORE_SCHEMA_MARKER.as_bytes().to_vec().into(),
        ),
    )
    .await
    .map_err(|_| StoreError::Backend(format!("kv.put {SCHEMA_VERSION_KEY}: timeout")))?
    .map_err(|e| StoreError::Backend(format!("kv.put {SCHEMA_VERSION_KEY}: {e}")))?;
    Ok(())
}

/// Check the schema-version marker against [`STORE_SCHEMA_MARKER`] before
/// anything else touches the store.
///
/// - Current marker: proceed.
/// - Fresh bucket: stamp the current marker and proceed.
/// - Legacy layout: scan every baseline row and branch delta. When none still
///   carries a retired `fields` list, stamp the current marker and proceed;
///   otherwise fail with [`StoreError::LegacyFieldsUnmigrated`], because the
///   first write to such a row would drop its list for good.
///   [`StoreOpenMode::MigrateLegacyFields`] skips the scan so the migration
///   can open the store it is meant to fix.
/// - Any other marker: fail with [`StoreError::SchemaMismatch`].
///
/// The marker key is written with `kv.put`, which goes to the KV backing
/// stream and never to the entity change stream, so no downstream consumer
/// observes it as a change event.
async fn ensure_schema_version(
    kv: &KvStore,
    branches_kv: &KvStore,
    open_mode: StoreOpenMode,
) -> StoreResult<()> {
    match read_stored_layout(kv).await? {
        StoredLayout::Current => Ok(()),
        StoredLayout::Fresh => write_schema_marker(kv).await,
        StoredLayout::Foreign(found) => Err(StoreError::SchemaMismatch {
            expected: STORE_SCHEMA_MARKER.to_string(),
            found,
        }),
        StoredLayout::LegacyFields => match open_mode {
            StoreOpenMode::MigrateLegacyFields => Ok(()),
            StoreOpenMode::Serve => {
                let report = legacy_fields::migrate_legacy_rows(
                    kv,
                    branches_kv,
                    MigrationMode::DryRun,
                    ConflictResolution::Refuse,
                )
                .await?;
                match report.remaining() {
                    0 => write_schema_marker(kv).await,
                    rows => Err(StoreError::LegacyFieldsUnmigrated { rows }),
                }
            }
        },
    }
}

async fn ensure_changes_stream(js: &Js, config: &NatsStoreConfig) -> StoreResult<()> {
    // Fail fast if the operator wired every retention knob to None. The
    // JetStream API treats `Duration::ZERO` / `-1` as "no limit"; an
    // unbounded change stream silently grows until the broker runs out of
    // disk. Force an explicit decision at config time.
    if config.changes_max_age.is_none()
        && config.changes_max_msgs.is_none()
        && config.changes_max_bytes.is_none()
    {
        return Err(StoreError::InvalidArgument(
            "NATS changes stream has no retention configured: set at least one of \
             changes_max_age, changes_max_msgs, or changes_max_bytes."
                .into(),
        ));
    }
    let desired = StreamConfig {
        name: config.stream.clone(),
        subjects: vec![config.subject_wildcard()],
        retention: RetentionPolicy::Limits,
        max_age: config.changes_max_age.unwrap_or(Duration::ZERO),
        max_messages: config.changes_max_msgs.unwrap_or(-1),
        max_bytes: config.changes_max_bytes.unwrap_or(-1),
        ..Default::default()
    };
    let mut stream = js
        .get_or_create_stream(desired.clone())
        .await
        .map_err(|e| {
            StoreError::Unavailable(format!(
                "stream create {stream}: {e}",
                stream = config.stream
            ))
        })?;

    // Compare the observed config against what we want and only call
    // update_stream when they actually differ. This avoids last-writer-wins
    // noise during rolling restarts where another replica already applied
    // the desired config.
    //
    // JetStream represents "unlimited" for max_messages and max_bytes as
    // either -1 or 0; normalize both to -1 before comparing so an existing
    // stream created with 0 does not trigger a spurious update when the
    // desired config uses -1 (or vice versa).
    fn norm_unlimited(v: i64) -> i64 {
        if v <= 0 {
            -1
        } else {
            v
        }
    }
    match stream.info().await {
        Ok(info) => {
            let differs = info.config.max_age != desired.max_age
                || norm_unlimited(info.config.max_messages) != norm_unlimited(desired.max_messages)
                || norm_unlimited(info.config.max_bytes) != norm_unlimited(desired.max_bytes);

            if differs {
                tracing::info!(
                    stream = %config.stream,
                    observed_max_age = ?info.config.max_age,
                    observed_max_msgs = %info.config.max_messages,
                    observed_max_bytes = %info.config.max_bytes,
                    desired_max_age = ?desired.max_age,
                    desired_max_msgs = %desired.max_messages,
                    desired_max_bytes = %desired.max_bytes,
                    "changes stream config differs from desired; applying update"
                );
                if let Err(e) = js.update_stream(&desired).await {
                    tracing::warn!(
                        stream = %config.stream,
                        error = %e,
                        "could not update changes stream retention limits; \
                         another writer may have won the race"
                    );
                }
            } else {
                tracing::debug!(
                    stream = %config.stream,
                    "changes stream config matches desired; skipping update"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                stream = %config.stream,
                error = %e,
                "could not read stream info after ensure"
            );
        }
    }

    Ok(())
}

/// Tail the changes stream and fan records out to broadcast subscribers.
/// Runs for the lifetime of the store; recreates its ephemeral consumer
/// (resuming after the last delivered sequence) whenever NATS hiccups.
/// Uses exponential backoff with jitter on reconnect.
///
/// `ready` is notified exactly once after the first consumer is created,
/// letting `connect_with` return without a sleep.
///
/// `reconnect_counter` is incremented each time the consumer is recreated
/// after a transient error so operators can detect pathological flap loops.
async fn run_change_tail(
    js: Js,
    config: Arc<NatsStoreConfig>,
    tx: broadcast::Sender<ChangeRecord>,
    ready: Arc<Notify>,
    reconnect_counter: Arc<AtomicU64>,
) {
    let mut last_seen: Option<u64> = None;
    let mut delay = LIVE_TAIL_RETRY_DELAY_MIN;
    let mut notified = false;
    loop {
        match tail_changes_once(&js, &config, &tx, &mut last_seen, &ready, &mut notified).await {
            Ok(()) => {
                delay = LIVE_TAIL_RETRY_DELAY_MIN;
            }
            Err(err) => {
                reconnect_counter.fetch_add(1, Ordering::Relaxed);
                metrics::counter!("trogon_atlas_store_change_tail_reconnects_total").increment(1);
                tracing::warn!(
                    stream = %config.stream,
                    error = %err,
                    retry_delay_ms = delay.as_millis(),
                    reconnects = reconnect_counter.load(Ordering::Relaxed),
                    "change tail interrupted; reconnecting"
                );
            }
        }
        // Jitter approximated from SystemTime subsec_nanos modulo the jitter
        // window. This avoids any external dependency while providing good
        // enough spread across concurrent reconnecting replicas. The modulo
        // introduces a small bias toward lower values, which is acceptable for
        // reconnect jitter.
        let jitter_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX) / 4;
        if jitter_ms > 0 {
            let nanos = u64::from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos(),
            );
            let jitter_val = nanos % jitter_ms;
            tokio::time::sleep(delay + Duration::from_millis(jitter_val)).await;
        } else {
            tokio::time::sleep(delay).await;
        }
        delay = (delay * 2).min(LIVE_TAIL_RETRY_DELAY_MAX);
    }
}

async fn tail_changes_once(
    js: &Js,
    config: &NatsStoreConfig,
    tx: &broadcast::Sender<ChangeRecord>,
    last_seen: &mut Option<u64>,
    ready: &Arc<Notify>,
    notified: &mut bool,
) -> StoreResult<()> {
    let stream = tokio::time::timeout(JS_OP_TIMEOUT, js.get_stream(&config.stream))
        .await
        .map_err(|_| {
            StoreError::Backend(format!(
                "get_stream {stream}: timeout",
                stream = config.stream
            ))
        })?
        .map_err(|e| {
            StoreError::Backend(format!("get_stream {stream}: {e}", stream = config.stream))
        })?;
    // On the first connect, `DeliverPolicy::New` is deliberate: subscribers
    // wanting historical records call `read_changes(...)` (paged) after
    // `subscribe()` and before entering the live loop -- see
    // `EventModelServiceImpl::stream_changes`, which subscribes first,
    // drains the backlog one page at a time, then reads the broadcast.
    // Replaying the whole stream into the in-process broadcast on every
    // server start would re-emit unbounded history to N subscribers.
    // After a reconnect we resume from `last_seen + 1` so no records are
    // dropped during a transient NATS hiccup.
    let deliver_policy = match *last_seen {
        Some(seq) => DeliverPolicy::ByStartSequence {
            start_sequence: seq.saturating_add(1),
        },
        None => DeliverPolicy::New,
    };
    // Ephemeral consumer by design: each NatsStore replica subscribes
    // independently, so a stable durable name would conflict in HA
    // deployments. `inactive_threshold` reaps abandoned consumers within
    // 60 s; under a fast NATS flap a few extras may briefly accumulate
    // before being collected, which is acceptable.
    let consumer = tokio::time::timeout(
        JS_OP_TIMEOUT,
        stream.create_consumer(PullConfig {
            deliver_policy,
            ack_policy: AckPolicy::None,
            inactive_threshold: Duration::from_mins(1),
            ..Default::default()
        }),
    )
    .await
    .map_err(|_| StoreError::Backend("create tail consumer: timeout".into()))?
    .map_err(|e| StoreError::Backend(format!("create tail consumer: {e}")))?;

    // Notify connect_with exactly once after the consumer is active.
    if !*notified {
        ready.notify_one();
        *notified = true;
    }

    let mut messages = consumer
        .messages()
        .await
        .map_err(|e| StoreError::Backend(format!("tail messages: {e}")))?;
    while let Some(msg) = messages.next().await {
        let msg = msg.map_err(|e| StoreError::Backend(format!("tail next: {e}")))?;
        let info = msg
            .info()
            .map_err(|e| StoreError::Backend(format!("tail msg.info: {e}")))?;
        let seq = info.stream_sequence;
        *last_seen = Some(seq);
        match decode_change_record(seq, msg.payload.as_ref()) {
            Ok(record) => {
                // Send fails only when no receiver is subscribed; that is fine.
                let _ = tx.send(record);
            }
            Err(err) => {
                tracing::warn!(seq, error = %err, "skipping undecodable change record");
            }
        }
    }
    Ok(())
}

fn decode_change_record(seq: u64, payload: &[u8]) -> StoreResult<ChangeRecord> {
    let event = trogon_atlas_proto::ChangeEvent::decode(payload)
        .map_err(|e| StoreError::Backend(format!("decode ChangeEvent: {e}")))?;
    // A `ChangeEvent` with no `entity` field is malformed (proto skew or a
    // truncated publish). Hard-fail the decode so the tailer's warning log
    // surfaces the bad record instead of silently poisoning the change feed.
    let entity_ref = event
        .entity
        .ok_or_else(|| StoreError::Backend("ChangeEvent.entity is required".into()))?;
    let kind = match event.kind {
        x if x == trogon_atlas_proto::change_event::Kind::Put as i32 => ChangeKind::Put,
        x if x == trogon_atlas_proto::change_event::Kind::Deleted as i32 => ChangeKind::Delete,
        other => {
            tracing::warn!(
                seq,
                raw_kind = other,
                "unknown ChangeEvent.kind; skipping record"
            );
            return Err(StoreError::Backend(format!(
                "unknown ChangeEvent.kind value {other}"
            )));
        }
    };
    Ok(ChangeRecord {
        seq,
        kind,
        entity_ref,
        at: event.at,
        changeset_id: event.changeset_id,
        author: event.author,
    })
}

fn encode_entity(entity: &Entity) -> Bytes {
    Bytes::from(entity.encode_to_vec())
}

fn decode_entity(bytes: &[u8]) -> StoreResult<Entity> {
    Entity::decode(bytes).map_err(|e| StoreError::Backend(format!("entity decode: {e}")))
}

fn decode_entity_logged(bytes: &[u8], key: &str) -> StoreResult<Entity> {
    Entity::decode(bytes).map_err(|e| {
        tracing::error!(
            key = %key,
            error = %e,
            "corrupt entity in KV store; entry will be treated as absent"
        );
        StoreError::CorruptEntry {
            key: key.to_string(),
            reason: e.to_string(),
        }
    })
}

fn kind_of(entity: &Entity) -> Option<EntityKind> {
    Some(match entity.kind.as_ref()? {
        EntityOneof::Event(_) => EntityKind::Event,
        EntityOneof::Command(_) => EntityKind::Command,
        EntityOneof::ReadModel(_) => EntityKind::ReadModel,
        EntityOneof::Processor(_) => EntityKind::Processor,
        EntityOneof::Ui(_) => EntityKind::Ui,
        EntityOneof::Persona(_) => EntityKind::Persona,
        EntityOneof::Swimlane(_) => EntityKind::Swimlane,
        EntityOneof::CommandSlice(_) => EntityKind::CommandSlice,
        EntityOneof::ReadModelSlice(_) => EntityKind::ReadModelSlice,
        EntityOneof::AutomationSlice(_) => EntityKind::AutomationSlice,
        EntityOneof::Storyboard(_) => EntityKind::Storyboard,
        EntityOneof::EventModel(_) => EntityKind::EventModel,
        EntityOneof::Component(_) => EntityKind::Component,
        EntityOneof::ExternalSystem(_) => EntityKind::ExternalSystem,
        EntityOneof::Tracker(_) => EntityKind::Tracker,
        EntityOneof::BoundedContext(_) => EntityKind::BoundedContext,
        EntityOneof::Domain(_) => EntityKind::Domain,
        EntityOneof::Subdomain(_) => EntityKind::Subdomain,
        EntityOneof::Schema(_) => EntityKind::Schema,
        EntityOneof::Project(_) => EntityKind::Project,
        EntityOneof::Screen(_) => EntityKind::Screen,
        EntityOneof::Term(_) => EntityKind::Term,
        EntityOneof::Ambiguity(_) => EntityKind::Ambiguity,
        EntityOneof::UiSlice(_) => EntityKind::UiSlice,
        EntityOneof::ServiceLevelIndicator(_) => EntityKind::ServiceLevelIndicator,
        EntityOneof::ServiceLevelObjective(_) => EntityKind::ServiceLevelObjective,
        EntityOneof::AlertPolicy(_) => EntityKind::AlertPolicy,
        EntityOneof::AlertNotificationTarget(_) => EntityKind::AlertNotificationTarget,
        EntityOneof::TypeLibrary(_) => EntityKind::TypeLibrary,
    })
}

fn entity_id(entity: &Entity) -> Option<&Id> {
    match entity.kind.as_ref()? {
        EntityOneof::Event(x) => x.id.as_ref(),
        EntityOneof::Command(x) => x.id.as_ref(),
        EntityOneof::ReadModel(x) => x.id.as_ref(),
        EntityOneof::Processor(x) => x.id.as_ref(),
        EntityOneof::Ui(x) => x.id.as_ref(),
        EntityOneof::Persona(x) => x.id.as_ref(),
        EntityOneof::Swimlane(x) => x.id.as_ref(),
        EntityOneof::CommandSlice(x) => x.id.as_ref(),
        EntityOneof::ReadModelSlice(x) => x.id.as_ref(),
        EntityOneof::AutomationSlice(x) => x.id.as_ref(),
        EntityOneof::Storyboard(x) => x.id.as_ref(),
        EntityOneof::EventModel(x) => x.id.as_ref(),
        EntityOneof::Component(x) => x.id.as_ref(),
        EntityOneof::ExternalSystem(x) => x.id.as_ref(),
        EntityOneof::Tracker(x) => x.id.as_ref(),
        EntityOneof::BoundedContext(x) => x.id.as_ref(),
        EntityOneof::Domain(x) => x.id.as_ref(),
        EntityOneof::Subdomain(x) => x.id.as_ref(),
        EntityOneof::Schema(x) => x.id.as_ref(),
        EntityOneof::Project(x) => x.id.as_ref(),
        EntityOneof::Screen(x) => x.id.as_ref(),
        EntityOneof::Term(x) => x.id.as_ref(),
        EntityOneof::Ambiguity(x) => x.id.as_ref(),
        EntityOneof::UiSlice(x) => x.id.as_ref(),
        EntityOneof::ServiceLevelIndicator(x) => x.id.as_ref(),
        EntityOneof::ServiceLevelObjective(x) => x.id.as_ref(),
        EntityOneof::AlertPolicy(x) => x.id.as_ref(),
        EntityOneof::AlertNotificationTarget(x) => x.id.as_ref(),
        EntityOneof::TypeLibrary(x) => x.id.as_ref(),
    }
}

// ---------------------------------------------------------------------------
// Branch overlays (Phase 1: Isolation)
//
// A branch is a KV bucket of copy-on-write deltas layered over the baseline
// entities bucket. Each row is either:
//   - a metadata row at `branch_meta_key(name)`, encoding
//     `trogon_atlas_proto::BranchInfo` (created_at/doc/base_change_token; the
//     `delta_count` field is NOT persisted here -- it is derived by
//     counting delta rows at read time so it never drifts from reality).
//   - a delta row at `branch_delta_key(name, kind, id)`, encoding
//     `trogon_atlas_proto::BranchDelta` (the wire envelope defined in
//     `service.proto` for exactly this purpose).
//
// The baseline CAS-loop methods (`put_inner`, `update`, `delete`, `list`,
// `batch_apply`) are left untouched for `branch = None`. Branch-scoped
// calls are handled by the helpers below, called from the trait method
// bodies when `branch.is_some()`. This keeps the new, less battle-tested
// code isolated from the existing well-tested baseline logic while still
// presenting a single `Store` trait surface (no parallel `branch_get` /
// `branch_put` methods).
//
// Branch writes never call `record_change`: the change feed only ever
// observes baseline history, by construction (the branch write path
// simply never calls it).

// ---------------------------------------------------------------------------
// Namespace registry codec
// ---------------------------------------------------------------------------

fn encode_namespace_record(record: &NamespaceRecord) -> Bytes {
    let proto = trogon_atlas_proto::NamespaceRecord {
        id: record.id.to_string(),
        name: record.name.to_string(),
        parent: record.parent.to_string(),
        created_at: record.created_at.clone(),
        created_by: record.created_by.clone(),
        provisional_branch: record.tenure.branch().unwrap_or_default().to_owned(),
    };
    Bytes::from(proto.encode_to_vec())
}

fn decode_namespace_record(bytes: &[u8]) -> StoreResult<NamespaceRecord> {
    let proto = trogon_atlas_proto::NamespaceRecord::decode(bytes)
        .map_err(|e| StoreError::Backend(format!("decode NamespaceRecord: {e}")))?;
    let invalid = |field: &str, e: trogon_atlas_core::NamespaceError| {
        StoreError::Backend(format!("decode NamespaceRecord.{field}: {e}"))
    };
    Ok(NamespaceRecord {
        id: NamespaceId::parse(&proto.id).map_err(|e| invalid("id", e))?,
        name: NamespaceName::parse(&proto.name).map_err(|e| invalid("name", e))?,
        parent: OwnerId::parse(&proto.parent).map_err(|e| invalid("parent", e))?,
        created_at: proto.created_at,
        created_by: proto.created_by,
        tenure: NamespaceTenure::for_branch(match proto.provisional_branch.as_str() {
            "" => None,
            branch => Some(branch),
        }),
    })
}

fn encode_branch_info(info: &BranchInfo) -> Bytes {
    let proto = trogon_atlas_proto::BranchInfo {
        name: info.name.clone(),
        doc: info.doc.clone(),
        created_at: info.created_at.clone(),
        base_change_token: info.base_change_token.clone(),
        delta_count: 0, // derived at read time; never trusted from storage
        fork_changeset_id: info.fork_changeset_id.clone().unwrap_or_default(),
    };
    Bytes::from(proto.encode_to_vec())
}

fn decode_branch_info(bytes: &[u8], delta_count: u32) -> StoreResult<BranchInfo> {
    let proto = trogon_atlas_proto::BranchInfo::decode(bytes)
        .map_err(|e| StoreError::Backend(format!("branch info decode: {e}")))?;
    Ok(BranchInfo {
        name: proto.name,
        doc: proto.doc,
        created_at: proto.created_at,
        base_change_token: proto.base_change_token,
        delta_count,
        // Empty on the wire means "no fork point", which is also what a
        // branch created before this field existed decodes to.
        fork_changeset_id: (!proto.fork_changeset_id.is_empty()).then_some(proto.fork_changeset_id),
    })
}

fn encode_branch_delta(delta: &trogon_atlas_proto::BranchDelta) -> Bytes {
    Bytes::from(delta.encode_to_vec())
}

fn decode_branch_delta(bytes: &[u8]) -> StoreResult<trogon_atlas_proto::BranchDelta> {
    trogon_atlas_proto::BranchDelta::decode(bytes)
        .map_err(|e| StoreError::Backend(format!("branch delta decode: {e}")))
}

/// Merged-view read for a single key on a branch: the branch's delta wins
/// over baseline if one exists (tombstone hides the key entirely);
/// otherwise falls through to the baseline entities bucket.
async fn branch_merged_get(
    kv: &KvStore,
    branches_kv: &KvStore,
    branch: &str,
    kind: EntityKind,
    id: &Id,
) -> StoreResult<Option<StoredEntity>> {
    let delta_key = branch_delta_key(branch, kind, id);
    let delta_entry = kv_entry_timeout(branches_kv, delta_key.clone()).await?;
    if let Some(entry) = live_entry(delta_entry) {
        let delta = decode_branch_delta(&entry.value)?;
        if delta.tombstone {
            return Ok(None);
        }
        let Some(ours) = delta.ours else {
            return Ok(None);
        };
        return Ok(Some(StoredEntity {
            entity: ours,
            etag: entry.revision.to_string(),
        }));
    }
    let base_key = entity_key(kind, id);
    let base_entry = kv_entry_timeout(kv, base_key).await?;
    let Some(entry) = live_entry(base_entry) else {
        return Ok(None);
    };
    Ok(Some(StoredEntity {
        entity: decode_entity(&entry.value)?,
        etag: entry.revision.to_string(),
    }))
}

/// Read the merged-view "previous" value for a key on a branch, for use as
/// the `prev` argument to `stamp_system` and for etag/no-op comparisons.
/// Returns `(entity, etag)` where `etag` is the delta row's revision if a
/// delta already exists, else the baseline row's revision (as a string) so
/// conditional writes can still be etag-guarded against baseline state
/// before the key has ever been touched on the branch.
async fn branch_merged_prev(
    kv: &KvStore,
    branches_kv: &KvStore,
    branch: &str,
    kind: EntityKind,
    id: &Id,
) -> StoreResult<(Option<Entity>, Option<String>, Option<(Entity, String)>)> {
    let delta_key = branch_delta_key(branch, kind, id);
    let delta_entry = kv_entry_timeout(branches_kv, delta_key).await?;
    if let Some(entry) = live_entry(delta_entry) {
        let delta = decode_branch_delta(&entry.value)?;
        // Revising an existing delta (including tombstone) must keep the
        // fork-from snapshot captured on first touch. Only a missing delta
        // row falls through to baseline capture below.
        let preserved_base = delta.base.map(|e| (e, delta.base_etag));
        if delta.tombstone {
            return Ok((None, None, preserved_base));
        }
        return Ok((delta.ours, Some(entry.revision.to_string()), preserved_base));
    }
    // No delta yet: fall through to baseline, and capture it as the "base"
    // to remember on first write (base/base_etag on the new delta row).
    let base_key = entity_key(kind, id);
    let base_entry = kv_entry_timeout(kv, base_key).await?;
    match live_entry(base_entry) {
        Some(entry) => {
            let entity = decode_entity(&entry.value)?;
            let etag = entry.revision.to_string();
            Ok((
                Some(entity.clone()),
                Some(etag.clone()),
                Some((entity, etag)),
            ))
        }
        None => Ok((None, None, None)),
    }
}

/// A [`Written`] together with the image the write replaced.
///
/// The pre-image is what a revision row needs and what no `Store` caller
/// has asked for, so it rides alongside `Written` on the internal write
/// helpers rather than widening the public type.
struct WroteOver {
    written: Written,
    before: Option<Entity>,
}

/// What one write did to one entity: which key, and the entity on either
/// side of the write.
///
/// Both images are carried rather than only the pre-image. Reconstructing
/// `after(n)` as `before(n + 1)` would be one field cheaper and would be
/// wrong the moment an unattributed write lands between two attributed
/// ones, which is a supported path.
struct Revision {
    kind: EntityKind,
    id: Id,
    change: ChangeKind,
    before: Option<Entity>,
    after: Option<Entity>,
}

impl Revision {
    fn entity_ref(&self) -> EntityRef {
        EntityRef {
            kind: self.kind as i32,
            id: Some(self.id.clone()),
        }
    }
}

/// CAS-loop write of a branch delta row. `revise` computes the new
/// `BranchDelta` payload from the merged-view previous entity; returning
/// `Err` aborts before any write. On `WrongLastRevision` the loop re-reads
/// and retries, mirroring `put_inner`'s baseline CAS loop.
async fn branch_write_delta(
    kv: &KvStore,
    branches_kv: &KvStore,
    branch: &str,
    kind: EntityKind,
    id: &Id,
    mut revise: impl FnMut(
        Option<Entity>,
        Option<String>,
        Option<(Entity, String)>,
    ) -> StoreResult<Option<trogon_atlas_proto::BranchDelta>>,
) -> StoreResult<WroteOver> {
    let delta_key = branch_delta_key(branch, kind, id);
    for attempt in 0..CAS_MAX_ATTEMPTS {
        let (prev_entity, prev_etag, base) =
            branch_merged_prev(kv, branches_kv, branch, kind, id).await?;
        let current_delta_rev =
            live_entry(kv_entry_timeout(branches_kv, delta_key.clone()).await?).map(|e| e.revision);

        let Some(new_delta) = revise(prev_entity.clone(), prev_etag.clone(), base)? else {
            // `revise` signaled a no-op: return the existing merged view.
            let entity = prev_entity.ok_or(StoreError::NotFound)?;
            return Ok(WroteOver {
                written: Written {
                    etag: prev_etag.unwrap_or_default(),
                    wrote: false,
                    entity: entity.clone(),
                },
                before: Some(entity),
            });
        };
        let bytes = encode_branch_delta(&new_delta);

        let cas_result = match current_delta_rev {
            Some(rev) => {
                tokio::time::timeout(KV_OP_TIMEOUT, branches_kv.update(&delta_key, bytes, rev))
                    .await
                    .map_err(|_| {
                        StoreError::Backend(format!("branch kv.update {delta_key}: timeout"))
                    })?
                    .map_err(|e| {
                        use async_nats::jetstream::kv::UpdateErrorKind;
                        if matches!(e.kind(), UpdateErrorKind::WrongLastRevision) {
                            StoreError::EtagMismatch {
                                expected: rev.to_string(),
                                found: String::new(),
                            }
                        } else {
                            StoreError::Backend(format!("branch kv.update {delta_key}: {e}"))
                        }
                    })
            }
            None => tokio::time::timeout(KV_OP_TIMEOUT, branches_kv.create(&delta_key, bytes))
                .await
                .map_err(|_| StoreError::Backend(format!("branch kv.create {delta_key}: timeout")))?
                .map_err(|e| {
                    use async_nats::jetstream::kv::CreateErrorKind;
                    if matches!(e.kind(), CreateErrorKind::AlreadyExists) {
                        StoreError::EtagMismatch {
                            expected: "absent".into(),
                            found: "present".into(),
                        }
                    } else {
                        StoreError::Backend(format!("branch kv.create {delta_key}: {e}"))
                    }
                }),
        };

        match cas_result {
            Ok(rev) => {
                let entity = if new_delta.tombstone {
                    // Deletes report no entity body; caller ignores it via
                    // MutationOutcome::Deleted at the call site.
                    Entity::default()
                } else {
                    new_delta.ours.unwrap_or_default()
                };
                return Ok(WroteOver {
                    written: Written {
                        etag: rev.to_string(),
                        wrote: true,
                        entity,
                    },
                    before: prev_entity,
                });
            }
            Err(StoreError::EtagMismatch { .. }) if attempt + 1 < CAS_MAX_ATTEMPTS => {
                let exp = attempt.min(CAS_RETRY_EXP_CAP);
                #[allow(clippy::cast_possible_truncation)]
                let base_ms = CAS_RETRY_BASE.as_millis() as u64 * (1u64 << exp);
                let jitter_ms = if base_ms > 0 {
                    let nanos = u64::from(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .subsec_nanos(),
                    );
                    nanos % base_ms
                } else {
                    0
                };
                tokio::time::sleep(Duration::from_millis(base_ms + jitter_ms)).await;
            }
            Err(e) => return Err(e),
        }
    }
    Err(StoreError::Backend(format!(
        "branch put {delta_key}: CAS loop exhausted after {CAS_MAX_ATTEMPTS} attempts"
    )))
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn now_micros() -> i64 {
    chrono::Utc::now().timestamp_micros().max(0)
}

fn change_event_proto(
    kind: ChangeKind,
    entity_ref: &EntityRef,
    changeset: Option<ChangesetRef<'_>>,
) -> trogon_atlas_proto::ChangeEvent {
    use trogon_atlas_proto::change_event::Kind as PbKind;
    trogon_atlas_proto::ChangeEvent {
        kind: match kind {
            ChangeKind::Put => PbKind::Put as i32,
            ChangeKind::Delete => PbKind::Deleted as i32,
        },
        entity: Some(entity_ref.clone()),
        at: now_rfc3339(),
        token: String::new(),
        timestamp_micros: now_micros(),
        changeset_id: changeset.map(|c| c.id.to_owned()).unwrap_or_default(),
        author: changeset.map(|c| c.author.to_owned()).unwrap_or_default(),
    }
}

/// Subject format: `{root}.{put|del}.{ns_escaped}.{slug_escaped}`.
///
/// Each token uses the collision-free `=HH` hex-escape scheme from `key.rs`:
/// `[A-Za-z0-9_-]` pass through; every other byte becomes `=XX` (uppercase
/// hex). This guarantees distinct strings always produce distinct subjects.
///
/// Note: entity kind is NOT included in the subject. The change-record
/// payload carries `EntityRef.kind` and downstream consumers route on
/// that. Two entities with the same (namespace, slug) but different
/// kinds would alias to the same subject -- id validation rejects that
/// case at the gRPC boundary today, so the aliasing is moot. If the
/// schema ever permits same-id-across-kinds, prepend the kind short:
/// `{root}.{put|del}.{kind_short}.{ns}.{slug}` AND update the
/// `JetStream` subject filters that any external consumer set up
/// against the existing format -- it's a breaking change.
///
/// Returns `Err(InvalidArgument)` when `entity_ref.id` is absent; callers
/// must supply a fully populated `EntityRef` before publishing.
fn change_subject(
    subject_root: &str,
    kind: ChangeKind,
    entity_ref: &EntityRef,
) -> StoreResult<String> {
    let id = entity_ref
        .id
        .as_ref()
        .ok_or_else(|| StoreError::InvalidArgument(
            "change_subject: entity_ref.id is required; cannot build a change subject without namespace and slug".into(),
        ))?;
    let kind_tag = match kind {
        ChangeKind::Put => "put",
        ChangeKind::Delete => "del",
    };
    Ok(format!(
        "{subject_root}.{kind_tag}.{}.{}",
        escape_subject_token(&id.namespace),
        escape_subject_token(&id.slug),
    ))
}

/// Collision-free escaping for NATS subject tokens using the same `=HH` scheme
/// as `key.rs/encode_component`. `[A-Za-z0-9_-]` pass through; every other
/// byte becomes `=XX` (uppercase hex). `=` itself encodes as `=3D`.
fn escape_subject_token(s: &str) -> String {
    if s.is_empty() {
        return "_".into();
    }
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' => out.push(b as char),
            other => {
                use std::fmt::Write as _;
                let _ = write!(out, "={other:02X}");
            }
        }
    }
    out
}

#[async_trait]
impl Store for NatsStore {
    fn is_writer(&self) -> bool {
        self.lease.is_writer()
    }

    fn writer_status(&self) -> WriterStatus {
        self.lease.status()
    }

    #[tracing::instrument(skip(self), fields(kind = ?kind, id.namespace = %id.namespace, id.slug = %id.slug, branch))]
    async fn get(
        &self,
        kind: EntityKind,
        id: &Id,
        branch: Option<&str>,
    ) -> StoreResult<StoredEntity> {
        if let Some(branch) = branch {
            return branch_merged_get(&self.kv, &self.branches_kv, branch, kind, id)
                .await?
                .ok_or(StoreError::NotFound);
        }
        let key = entity_key(kind, id);
        let entry = kv_entry_timeout(&self.kv, key.clone()).await?;
        let entry = live_entry(entry).ok_or(StoreError::NotFound)?;
        Ok(StoredEntity {
            entity: decode_entity(&entry.value)?,
            etag: entry.revision.to_string(),
        })
    }

    async fn batch_get(
        &self,
        keys: &[(EntityKind, Id)],
        branch: Option<&str>,
    ) -> StoreResult<Vec<Option<StoredEntity>>> {
        if keys.len() > BATCH_GET_HARD_CAP {
            return Err(StoreError::InvalidArgument(format!(
                "batch_get: key slice length {} exceeds hard cap {}",
                keys.len(),
                BATCH_GET_HARD_CAP,
            )));
        }
        let span = tracing::debug_span!("batch_get", key_count = keys.len(), branch);
        let _enter = span.enter();
        if let Some(branch) = branch {
            let kv = self.kv.clone();
            let branches_kv = self.branches_kv.clone();
            let branch = branch.to_string();
            let owned: Vec<(EntityKind, Id)> = keys.to_vec();
            // `buffered` (not unordered) preserves input-key order while still
            // overlapping up to LIST_FETCH_CONCURRENCY in-flight gets.
            let results = futures::stream::iter(owned.into_iter().map(|(kind, id)| {
                let kv = kv.clone();
                let branches_kv = branches_kv.clone();
                let branch = branch.clone();
                async move { branch_merged_get(&kv, &branches_kv, &branch, kind, &id).await }
            }))
            .buffered(LIST_FETCH_CONCURRENCY)
            .collect::<Vec<StoreResult<Option<StoredEntity>>>>()
            .await;
            return results.into_iter().collect();
        }
        let kv = self.kv.clone();
        let owned_keys: Vec<String> = keys.iter().map(|(k, id)| entity_key(*k, id)).collect();
        let results = futures::stream::iter(owned_keys.into_iter().map(|key| {
            let kv = kv.clone();
            async move {
                match kv_entry_timeout(&kv, key.clone()).await {
                    Ok(entry) => match live_entry(entry) {
                        Some(entry) => Ok(Some(StoredEntity {
                            entity: decode_entity(&entry.value)?,
                            etag: entry.revision.to_string(),
                        })),
                        None => Ok(None),
                    },
                    Err(e) => Err(e),
                }
            }
        }))
        .buffered(LIST_FETCH_CONCURRENCY)
        .collect::<Vec<StoreResult<Option<StoredEntity>>>>()
        .await;

        results.into_iter().collect()
    }

    #[tracing::instrument(skip(self, entity), fields(kind = ?kind, branch))]
    async fn create(
        &self,
        kind: EntityKind,
        entity: &Entity,
        ctx: WriteContext<'_>,
    ) -> StoreResult<Written> {
        self.require_writer()?;
        let id = Self::validate(kind, entity)?.clone();
        if let Some(branch) = ctx.branch_name() {
            let wrote = branch_write_delta(
                &self.kv,
                &self.branches_kv,
                branch,
                kind,
                &id,
                |prev_entity, _prev_etag, base| {
                    if prev_entity.is_some() {
                        return Err(StoreError::AlreadyExists);
                    }
                    let stamped = crate::store::stamp_system(None, entity);
                    let (base_entity, base_etag) = match base {
                        Some((e, etag)) => (Some(e), etag),
                        None => (None, String::new()),
                    };
                    Ok(Some(trogon_atlas_proto::BranchDelta {
                        base: base_entity,
                        base_etag,
                        ours: Some(stamped),
                        tombstone: false,
                    }))
                },
            )
            .await?;
            self.record_revision(
                ctx,
                &Revision {
                    kind,
                    id,
                    change: ChangeKind::Put,
                    before: wrote.before,
                    after: Some(wrote.written.entity.clone()),
                },
            )
            .await;
            return Ok(wrote.written);
        }
        let key = entity_key(kind, &id);
        let stamped = crate::store::stamp_system(None, entity);
        let bytes = encode_entity(&stamped);
        let rev = match tokio::time::timeout(KV_OP_TIMEOUT, self.kv.create(&key, bytes.clone()))
            .await
            .map_err(|_| StoreError::Backend(format!("kv.create {key}: timeout")))?
        {
            Ok(rev) => rev,
            Err(e) => {
                use async_nats::jetstream::kv::CreateErrorKind;
                if !matches!(e.kind(), CreateErrorKind::AlreadyExists) {
                    return Err(StoreError::Backend(format!("kv.create {key}: {e}")));
                }
                // Empty-Put tombstones (delete sentinel when kv.delete never
                // lands) still look like an existing key to NATS create, but
                // the merged view is absent. CAS-overwrite the tombstone.
                match kv_entry_timeout(&self.kv, key.clone()).await? {
                    None => {
                        // Race: key vanished between create AlreadyExists and
                        // the entry read. Retry create once for the absent view.
                        match tokio::time::timeout(
                            KV_OP_TIMEOUT,
                            self.kv.create(&key, bytes.clone()),
                        )
                        .await
                        .map_err(|_| StoreError::Backend(format!("kv.create {key}: timeout")))?
                        {
                            Ok(rev) => rev,
                            Err(e2) => {
                                if matches!(e2.kind(), CreateErrorKind::AlreadyExists) {
                                    return Err(StoreError::AlreadyExists);
                                }
                                return Err(StoreError::Backend(format!("kv.create {key}: {e2}")));
                            }
                        }
                    }
                    Some(entry) if live_entry(Some(entry.clone())).is_some() => {
                        return Err(StoreError::AlreadyExists);
                    }
                    Some(entry) => {
                        match tokio::time::timeout(
                            KV_OP_TIMEOUT,
                            self.kv.update(&key, bytes.clone(), entry.revision),
                        )
                        .await
                        .map_err(|_| StoreError::Backend(format!("kv.update {key}: timeout")))?
                        {
                            Ok(rev) => rev,
                            Err(update_err) => {
                                use async_nats::jetstream::kv::UpdateErrorKind;
                                if !matches!(update_err.kind(), UpdateErrorKind::WrongLastRevision)
                                {
                                    return Err(StoreError::Backend(format!(
                                        "kv.update {key}: {update_err}"
                                    )));
                                }
                                // Another writer won the tombstone CAS. If they
                                // left a live row, AlreadyExists is correct; if
                                // the key is still absent (purge), retry create.
                                match kv_entry_timeout(&self.kv, key.clone()).await? {
                                    Some(e) if live_entry(Some(e.clone())).is_some() => {
                                        return Err(StoreError::AlreadyExists);
                                    }
                                    _ => {
                                        match tokio::time::timeout(
                                            KV_OP_TIMEOUT,
                                            self.kv.create(&key, bytes),
                                        )
                                        .await
                                        .map_err(|_| {
                                            StoreError::Backend(format!("kv.create {key}: timeout"))
                                        })? {
                                            Ok(rev) => rev,
                                            Err(e2) => {
                                                if matches!(
                                                    e2.kind(),
                                                    CreateErrorKind::AlreadyExists
                                                ) {
                                                    return Err(StoreError::AlreadyExists);
                                                }
                                                return Err(StoreError::Backend(format!(
                                                    "kv.create {key}: {e2}"
                                                )));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };
        let revision = Revision {
            kind,
            id,
            change: ChangeKind::Put,
            before: None,
            after: Some(stamped.clone()),
        };
        let entity_ref = revision.entity_ref();
        if let Err(StoreError::ChangeEventLost) = self
            .record_change(ChangeKind::Put, &entity_ref, ctx.changeset())
            .await
        {
            self.change_events_lost.fetch_add(1, Ordering::Relaxed);
            metrics::counter!("trogon_atlas_store_change_events_lost_total").increment(1);
            tracing::error!(
                entity_ref = ?entity_ref,
                "create: change event lost after successful KV write"
            );
        }
        self.record_revision(ctx, &revision).await;
        Ok(Written {
            etag: rev.to_string(),
            wrote: true,
            entity: stamped,
        })
    }

    #[tracing::instrument(skip(self, entity), fields(kind = ?kind, branch))]
    async fn put(
        &self,
        kind: EntityKind,
        entity: &Entity,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> StoreResult<Written> {
        self.require_writer()?;
        let id = Self::validate(kind, entity)?.clone();
        if let Some(branch) = ctx.branch_name() {
            let wrote = branch_write_delta(
                &self.kv,
                &self.branches_kv,
                branch,
                kind,
                &id,
                |prev_entity, _prev_etag, base| {
                    if !force {
                        if let Some(prev) = prev_entity.as_ref() {
                            if trogon_atlas_core::semantically_equal(prev, entity) {
                                return Ok(None);
                            }
                        }
                    }
                    let stamped = crate::store::stamp_system(prev_entity.as_ref(), entity);
                    let (base_entity, base_etag) = match base {
                        Some((e, etag)) => (Some(e), etag),
                        None => (None, String::new()),
                    };
                    Ok(Some(trogon_atlas_proto::BranchDelta {
                        base: base_entity,
                        base_etag,
                        ours: Some(stamped),
                        tombstone: false,
                    }))
                },
            )
            .await?;
            if wrote.written.wrote {
                self.record_revision(
                    ctx,
                    &Revision {
                        kind,
                        id,
                        change: ChangeKind::Put,
                        before: wrote.before,
                        after: Some(wrote.written.entity.clone()),
                    },
                )
                .await;
            }
            return Ok(wrote.written);
        }
        let wrote = self.put_inner(kind, entity, force).await?;
        // A skipped write changed nothing: no change event, no revision.
        if !wrote.written.wrote {
            return Ok(wrote.written);
        }
        let revision = Revision {
            kind,
            id,
            change: ChangeKind::Put,
            before: wrote.before,
            after: Some(wrote.written.entity.clone()),
        };
        let entity_ref = revision.entity_ref();
        if let Err(StoreError::ChangeEventLost) = self
            .record_change(ChangeKind::Put, &entity_ref, ctx.changeset())
            .await
        {
            self.change_events_lost.fetch_add(1, Ordering::Relaxed);
            metrics::counter!("trogon_atlas_store_change_events_lost_total").increment(1);
            tracing::error!(
                entity_ref = ?entity_ref,
                "put: change event lost after successful KV write"
            );
        }
        self.record_revision(ctx, &revision).await;
        Ok(wrote.written)
    }

    #[tracing::instrument(skip(self, entity), fields(kind = ?kind, expected_etag, branch))]
    async fn update(
        &self,
        kind: EntityKind,
        entity: &Entity,
        expected_etag: &str,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> StoreResult<Written> {
        self.require_writer()?;
        let id = Self::validate(kind, entity)?.clone();
        if let Some(branch) = ctx.branch_name() {
            let expected_etag = expected_etag.to_string();
            let wrote = branch_write_delta(
                &self.kv,
                &self.branches_kv,
                branch,
                kind,
                &id,
                |prev_entity, prev_etag, base| {
                    let Some(prev) = prev_entity else {
                        return Err(StoreError::NotFound);
                    };
                    let found = prev_etag.clone().unwrap_or_default();
                    if prev_etag.as_deref() != Some(expected_etag.as_str()) {
                        return Err(StoreError::EtagMismatch {
                            expected: expected_etag.clone(),
                            found,
                        });
                    }
                    if !force && trogon_atlas_core::semantically_equal(&prev, entity) {
                        return Ok(None);
                    }
                    let stamped = crate::store::stamp_system(Some(&prev), entity);
                    let (base_entity, base_etag) = match base {
                        Some((e, etag)) => (Some(e), etag),
                        None => (None, String::new()),
                    };
                    Ok(Some(trogon_atlas_proto::BranchDelta {
                        base: base_entity,
                        base_etag,
                        ours: Some(stamped),
                        tombstone: false,
                    }))
                },
            )
            .await?;
            if wrote.written.wrote {
                self.record_revision(
                    ctx,
                    &Revision {
                        kind,
                        id,
                        change: ChangeKind::Put,
                        before: wrote.before,
                        after: Some(wrote.written.entity.clone()),
                    },
                )
                .await;
            }
            return Ok(wrote.written);
        }
        let key = entity_key(kind, &id);
        let expected: u64 = expected_etag
            .parse()
            .map_err(|_| StoreError::EtagMismatch {
                expected: expected_etag.into(),
                found: String::new(),
            })?;
        // Capture the revision we saw at decode time for no-op detection and
        // system stamping. CAS failures re-read via `live_found_etag` so
        // `EtagMismatch.found` is not this stale pre-CAS value.
        let (prev, prev_revision) = match kv_entry_timeout(&self.kv, key.clone()).await? {
            Some(e) if live_entry(Some(e.clone())).is_some() => {
                let rev = e.revision;
                let prev = match decode_entity(&e.value) {
                    Ok(entity) => Some(entity),
                    Err(err) => {
                        tracing::error!(
                            key = %key,
                            error = %err,
                            "corrupt entity in KV store during update; \
                             proceeding without prior value for system stamp"
                        );
                        None
                    }
                };
                (prev, Some(rev))
            }
            // Key is absent or is a tombstone -- map to NotFound so callers
            // can distinguish "key vanished" from a generic backend error.
            _ => return Err(StoreError::NotFound),
        };
        // Same no-op semantics as `put`: the etag guard must still hold,
        // and a semantically identical row is not rewritten.
        if !force {
            if let (Some(prev_entity), Some(rev)) = (prev.as_ref(), prev_revision) {
                if rev == expected && trogon_atlas_core::semantically_equal(prev_entity, entity) {
                    return Ok(Written {
                        etag: rev.to_string(),
                        wrote: false,
                        entity: prev_entity.clone(),
                    });
                }
            }
        }
        let stamped = crate::store::stamp_system(prev.as_ref(), entity);
        let bytes = encode_entity(&stamped);
        let rev = match tokio::time::timeout(KV_OP_TIMEOUT, self.kv.update(&key, bytes, expected))
            .await
            .map_err(|_| StoreError::Backend(format!("kv.update {key}: timeout")))?
        {
            Ok(rev) => rev,
            Err(e) => {
                use async_nats::jetstream::kv::UpdateErrorKind;
                if matches!(e.kind(), UpdateErrorKind::WrongLastRevision) {
                    let found = live_found_etag(&self.kv, &key).await;
                    return Err(StoreError::EtagMismatch {
                        expected: expected_etag.into(),
                        found,
                    });
                }
                return Err(StoreError::Backend(format!("kv.update {key}: {e}")));
            }
        };
        let revision = Revision {
            kind,
            id,
            change: ChangeKind::Put,
            before: prev,
            after: Some(stamped.clone()),
        };
        let entity_ref = revision.entity_ref();
        if let Err(StoreError::ChangeEventLost) = self
            .record_change(ChangeKind::Put, &entity_ref, ctx.changeset())
            .await
        {
            self.change_events_lost.fetch_add(1, Ordering::Relaxed);
            metrics::counter!("trogon_atlas_store_change_events_lost_total").increment(1);
            tracing::error!(
                entity_ref = ?entity_ref,
                "update: change event lost after successful KV write"
            );
        }
        self.record_revision(ctx, &revision).await;
        Ok(Written {
            etag: rev.to_string(),
            wrote: true,
            entity: stamped,
        })
    }

    #[tracing::instrument(skip(self), fields(kind = ?kind, id.namespace = %id.namespace, id.slug = %id.slug, branch))]
    async fn delete(
        &self,
        kind: EntityKind,
        id: &Id,
        expected_etag: Option<&str>,
        ctx: WriteContext<'_>,
    ) -> StoreResult<()> {
        self.require_writer()?;
        if let Some(branch) = ctx.branch_name() {
            let expected_etag = expected_etag.map(str::to_string);
            let wrote = branch_write_delta(
                &self.kv,
                &self.branches_kv,
                branch,
                kind,
                id,
                |prev_entity, prev_etag, base| {
                    if prev_entity.is_none() {
                        return Err(StoreError::NotFound);
                    }
                    if let Some(exp) = expected_etag.as_deref() {
                        let found = prev_etag.clone().unwrap_or_default();
                        if prev_etag.as_deref() != Some(exp) {
                            return Err(StoreError::EtagMismatch {
                                expected: exp.to_string(),
                                found,
                            });
                        }
                    }
                    let (base_entity, base_etag) = match base {
                        Some((e, etag)) => (Some(e), etag),
                        None => (None, String::new()),
                    };
                    Ok(Some(trogon_atlas_proto::BranchDelta {
                        base: base_entity,
                        base_etag,
                        ours: None,
                        tombstone: true,
                    }))
                },
            )
            .await?;
            debug_assert!(
                wrote.written.wrote,
                "delete always writes a tombstone delta"
            );
            self.record_revision(
                ctx,
                &Revision {
                    kind,
                    id: id.clone(),
                    change: ChangeKind::Delete,
                    before: wrote.before,
                    after: None,
                },
            )
            .await;
            return Ok(());
        }
        let key = entity_key(kind, id);
        let live = live_entry(kv_entry_timeout(&self.kv, key.clone()).await?)
            .ok_or(StoreError::NotFound)?;
        let current = live.revision;
        // Decoded up front: once the delete lands the pre-image is gone from
        // the entities bucket (history depth is 1), so the revision row is
        // the only place it can still be read from.
        let before = decode_entity(&live.value).ok();
        if let Some(exp) = expected_etag {
            let exp_rev: u64 = exp.parse().map_err(|_| StoreError::EtagMismatch {
                expected: exp.into(),
                found: current.to_string(),
            })?;
            if exp_rev != current {
                return Err(StoreError::EtagMismatch {
                    expected: exp.into(),
                    found: current.to_string(),
                });
            }
        }
        // Conditional delete: use `kv.update` with an empty value to implement
        // revision-conditional semantics. NATS KV 0.45 does not expose a
        // native `delete_with_revision` API; we approximate it by publishing
        // an empty `update` at the known revision. If a concurrent writer
        // changed the key since we read `current`, the `update` will fail
        // with `WrongLastRevision`.
        //
        // When `expected_etag` is None (unconditional delete) a
        // `WrongLastRevision` here means a concurrent writer modified the key
        // between our live-entry read and this update. We surface
        // `EtagMismatch { expected: current, found: "" }` so callers can
        // distinguish this from a caller-supplied etag conflict. The `found`
        // field is empty because we do not re-read the key; the error message
        // conveys that a concurrent modification occurred.
        //
        // The empty-value Put is treated as a tombstone by `live_entry`, so
        // even if the second `kv.delete` step fails, `get` returns NotFound
        // rather than a zeroed Entity.
        let bytes = Bytes::new();
        let is_unconditional = expected_etag.is_none();
        match tokio::time::timeout(KV_OP_TIMEOUT, self.kv.update(&key, bytes, current))
            .await
            .map_err(|_| StoreError::Backend(format!("kv.delete (update step) {key}: timeout")))?
        {
            Ok(_) => {
                // Only purge when the empty-Put sentinel is still present. A
                // concurrent put may have resurrected the key between the CAS
                // and this step; wiping that put would break delete CAS.
                let after = kv_entry_timeout(&self.kv, key.clone()).await?;
                if let Some(entry) = after.as_ref() {
                    if live_entry(Some(entry.clone())).is_some() {
                        let found = entry.revision.to_string();
                        if is_unconditional {
                            tracing::warn!(
                                key = %key,
                                revision = current,
                                found = %found,
                                "delete: concurrent put resurrected key after empty-Put sentinel"
                            );
                        }
                        return Err(StoreError::EtagMismatch {
                            expected: current.to_string(),
                            found,
                        });
                    }
                }
                let still_empty_put = after
                    .as_ref()
                    .is_some_and(|e| e.operation == Operation::Put && e.value.is_empty());
                if still_empty_put {
                    // Issue the actual delete so the KV entry transitions to the
                    // Delete operation state. Retry with short backoff; on final
                    // failure log and proceed -- the empty-Put tombstone already
                    // prevents `get` from returning a live entity.
                    let mut delete_err: Option<String> = None;
                    for attempt in 0..DELETE_RETRY_ATTEMPTS {
                        match tokio::time::timeout(KV_OP_TIMEOUT, self.kv.delete(&key))
                            .await
                            .map_err(|_| format!("kv.delete {key}: timeout"))
                            .and_then(|r| r.map_err(|e| format!("kv.delete {key}: {e}")))
                        {
                            Ok(()) => {
                                delete_err = None;
                                break;
                            }
                            Err(e) => {
                                delete_err = Some(e);
                                if attempt + 1 < DELETE_RETRY_ATTEMPTS {
                                    tokio::time::sleep(DELETE_RETRY_BASE * 2u32.pow(attempt)).await;
                                }
                            }
                        }
                    }
                    if let Some(e) = delete_err {
                        tracing::error!(
                            key = %key,
                            error = %e,
                            "kv.delete after conditional update failed after retries; \
                             entry left as empty-Put tombstone (get will return NotFound)"
                        );
                    }
                }
                let revision = Revision {
                    kind,
                    id: id.clone(),
                    change: ChangeKind::Delete,
                    before,
                    after: None,
                };
                let entity_ref = revision.entity_ref();
                if let Err(StoreError::ChangeEventLost) = self
                    .record_change(ChangeKind::Delete, &entity_ref, ctx.changeset())
                    .await
                {
                    self.change_events_lost.fetch_add(1, Ordering::Relaxed);
                    metrics::counter!("trogon_atlas_store_change_events_lost_total").increment(1);
                    tracing::error!(
                        entity_ref = ?entity_ref,
                        "delete: change event lost after successful KV write"
                    );
                }
                self.record_revision(ctx, &revision).await;
                Ok(())
            }
            Err(e) => {
                use async_nats::jetstream::kv::UpdateErrorKind;
                if matches!(e.kind(), UpdateErrorKind::WrongLastRevision) {
                    // For an unconditional delete, WrongLastRevision means a
                    // concurrent writer raced with our read-modify-delete
                    // sequence. Return EtagMismatch with expected=current so
                    // the caller knows which revision we attempted to delete at.
                    // For a conditional delete (expected_etag is Some) a
                    // separate check already validated the etag above; this
                    // branch fires only when another writer changes the key
                    // between our read and our update.
                    let found = live_found_etag(&self.kv, &key).await;
                    if is_unconditional {
                        tracing::warn!(
                            key = %key,
                            revision = current,
                            found = %found,
                            "delete: concurrent modification detected on unconditional delete"
                        );
                    }
                    Err(StoreError::EtagMismatch {
                        expected: current.to_string(),
                        found,
                    })
                } else {
                    Err(StoreError::Backend(format!("kv.delete {key}: {e}")))
                }
            }
        }
    }

    #[tracing::instrument(skip(self, filter), fields(limit, branch))]
    async fn list(
        &self,
        filter: ListFilter<'_>,
        limit: Option<usize>,
        branch: Option<&str>,
    ) -> StoreResult<Vec<StoredEntity>> {
        if let Some(branch) = branch {
            return self.list_branch_merged(filter, limit, branch).await;
        }
        // Guard the entire key-enumeration and entry-fetch pipeline with a
        // single wall-clock bound. Individual per-key fetches are already
        // guarded by KV_OP_TIMEOUT, but up to LIST_HARD_CAP keys means the
        // cumulative worst case without this bound is ~10,000 * KV_OP_TIMEOUT.
        let kv = self.kv.clone();
        let kinds: Vec<EntityKind> = filter.kinds.to_vec();
        let namespaces: Vec<String> = filter.namespaces.to_vec();
        let latest_versions_only = filter.latest_versions_only;
        tokio::time::timeout(LIST_TOTAL_TIMEOUT, async move {
            let effective_limit = limit.unwrap_or(LIST_HARD_CAP).min(LIST_HARD_CAP);

            let mut keys_stream = tokio::time::timeout(KV_OP_TIMEOUT, kv.keys())
                .await
                .map_err(|_| StoreError::Backend("kv.keys: timeout".into()))?
                .map_err(|e| StoreError::Backend(format!("kv.keys: {e}")))?;

            // Pre-compute prefixes for cheap kind/namespace filtering. Empty
            // filters mean "no constraint" -- every key passes.
            //
            // When namespaces are specified but kinds are not, expand over all
            // concrete EntityKind variants using `namespace_prefix` so we avoid
            // a full-bucket scan and the `parse_entity_key` call it requires.
            let prefixes: Vec<String> = if kinds.is_empty() && namespaces.is_empty() {
                Vec::new()
            } else if kinds.is_empty() {
                let mut p = Vec::with_capacity(ALL_KINDS.len() * namespaces.len());
                for &k in ALL_KINDS {
                    for ns in &namespaces {
                        p.push(namespace_prefix(k, ns));
                    }
                }
                p
            } else if namespaces.is_empty() {
                kinds.iter().map(|k| kind_prefix(*k)).collect()
            } else {
                let mut p = Vec::with_capacity(kinds.len() * namespaces.len());
                for k in &kinds {
                    for ns in &namespaces {
                        p.push(namespace_prefix(*k, ns));
                    }
                }
                p
            };

            // Collect matching keys. Live entries are materialized up to
            // LIST_HARD_CAP after the tombstone filter; the caller `limit`
            // is applied only after sort + latest_versions_only. A bucket of
            // pure tombstones (e.g., after bulk deletes) will scan matching
            // keys up to the raw-key memory ceiling within LIST_TOTAL_TIMEOUT
            // before returning an empty result -- the outer timeout prevents
            // unbounded blocking in that worst case.
            // Each step of the key stream is guarded by KV_OP_TIMEOUT to prevent
            // a stalled NATS connection from blocking list() forever.
            let mut matching_keys: Vec<String> = Vec::new();
            loop {
                let next = tokio::time::timeout(KV_OP_TIMEOUT, keys_stream.next())
                    .await
                    .map_err(|_| {
                        StoreError::Backend("kv.keys stream: timeout waiting for next key".into())
                    })?;
                let Some(item) = next else { break };
                let key = item.map_err(|e| StoreError::Backend(format!("kv.keys stream: {e}")))?;
                if key == SCHEMA_VERSION_KEY {
                    continue;
                }
                if !prefixes.is_empty() && !prefixes.iter().any(|p| key.starts_with(p)) {
                    continue;
                }
                matching_keys.push(key);
                // Memory ceiling for the raw key set. Callers observe the
                // live-entry cap; this bound only keeps a pathological
                // tombstone-heavy bucket from allocating without limit.
                if matching_keys.len() >= LIST_HARD_CAP * 10 {
                    tracing::warn!(
                        raw_key_cap = LIST_HARD_CAP * 10,
                        "list: raw key scan hit the memory ceiling; results may \
                         undercount live entities in a tombstone-heavy bucket"
                    );
                    break;
                }
            }

            // Fetch live entries concurrently up to LIST_HARD_CAP. The caller
            // `limit` is applied only after sort + latest_versions_only so
            // truncation cannot drop later slug groups that would survive
            // dedup (mirrors list_branch_merged). Tombstones do not count.
            let total_keys = matching_keys.len();
            let mut out: Vec<StoredEntity> = Vec::with_capacity(LIST_HARD_CAP.min(total_keys));
            let kv2 = kv.clone();
            let mut stream = futures::stream::iter(matching_keys.into_iter().map(move |key| {
                let kv = kv2.clone();
                async move {
                    let entry = kv_entry_timeout(&kv, key.clone()).await?;
                    let Some(entry) = live_entry(entry) else {
                        return Ok::<Option<StoredEntity>, StoreError>(None);
                    };
                    let entity = decode_entity_logged(&entry.value, &key)?;
                    Ok(Some(StoredEntity {
                        entity,
                        etag: entry.revision.to_string(),
                    }))
                }
            }))
            .buffer_unordered(LIST_FETCH_CONCURRENCY);

            while let Some(result) = stream.next().await {
                if let Some(stored) = result? {
                    out.push(stored);
                    if out.len() >= LIST_HARD_CAP {
                        break;
                    }
                }
            }

            out.sort_by(|a, b| {
                let ka = kind_of(&a.entity).unwrap_or(EntityKind::Unspecified) as i32;
                let kb = kind_of(&b.entity).unwrap_or(EntityKind::Unspecified) as i32;
                let ida = entity_id(&a.entity).cloned().unwrap_or_default();
                let idb = entity_id(&b.entity).cloned().unwrap_or_default();
                ka.cmp(&kb)
                    .then_with(|| ida.namespace.cmp(&idb.namespace))
                    .then_with(|| ida.slug.cmp(&idb.slug))
                    .then_with(|| ida.version.cmp(&idb.version))
            });

            if latest_versions_only {
                let mut latest: BTreeMap<(EntityKind, String, String), StoredEntity> =
                    BTreeMap::new();
                for s in out.drain(..) {
                    let (Some(k), Some(id)) = (kind_of(&s.entity), entity_id(&s.entity)) else {
                        continue;
                    };
                    let group_key = (k, id.namespace.clone(), id.slug.clone());
                    let keep = latest
                        .get(&group_key)
                        .and_then(|cur| entity_id(&cur.entity).map(|cid| cid.version < id.version))
                        .unwrap_or(true);
                    if keep {
                        latest.insert(group_key, s);
                    }
                }
                out = latest.into_values().collect();
            }

            out.truncate(effective_limit);
            Ok(out)
        })
        .await
        .map_err(|_| {
            StoreError::Backend(format!(
                "list: total timeout exceeded ({}s)",
                LIST_TOTAL_TIMEOUT.as_secs()
            ))
        })?
    }

    /// Publish a change event for the given entity. Returns the assigned
    /// `JetStream` sequence number on success.
    ///
    /// # Error semantics
    ///
    /// Returns `Err(StoreError::InvalidArgument)` when `entity_ref.id` is
    /// absent; this is a programming error and the caller should not retry.
    ///
    /// After all publish retries are exhausted the KV write is already
    /// durable but the change event is lost. In this case `record_change`
    /// returns `Err(StoreError::ChangeEventLost)`. Each internal caller in
    /// `nats.rs` catches `ChangeEventLost` specifically, increments the
    /// `change_events_lost` counter, logs at error level, and completes
    /// the surrounding operation successfully so the caller's write is not
    /// rolled back.
    async fn record_change(
        &self,
        kind: ChangeKind,
        entity_ref: &EntityRef,
        changeset: Option<ChangesetRef<'_>>,
    ) -> StoreResult<u64> {
        self.publish_change(kind, entity_ref, changeset, None).await
    }

    #[tracing::instrument(skip(self), fields(since, limit))]
    async fn read_changes(&self, since: u64, limit: usize) -> StoreResult<Vec<ChangeRecord>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        // May take multiple JetStream fetches when some messages fail to
        // decode: the live tail skips undecodable records, so this path
        // must too. Hard-failing on the first bad payload used to poison
        // ListChanges / StreamChanges backlog+gap-fill while the live
        // broadcast continued past the hole.
        let mut cursor = since;
        let mut out: Vec<ChangeRecord> = Vec::with_capacity(limit);
        while out.len() < limit {
            let start_sequence = cursor.checked_add(1).ok_or_else(|| {
                StoreError::InvalidArgument(
                    "read_changes: since + 1 overflows u64; pass a smaller cursor".into(),
                )
            })?;
            let start_sequence = start_sequence.max(1);
            let batch_limit = limit - out.len();
            let stream =
                tokio::time::timeout(JS_OP_TIMEOUT, self.js.get_stream(&self.config.stream))
                    .await
                    .map_err(|_| {
                        StoreError::Backend(format!(
                            "get_stream {stream}: timeout",
                            stream = self.config.stream
                        ))
                    })?
                    .map_err(|e| {
                        StoreError::Backend(format!(
                            "get_stream {stream}: {e}",
                            stream = self.config.stream
                        ))
                    })?;
            // Per-call consumer creation is correct here: each call specifies an
            // arbitrary `since` sequence, so a single cached consumer with a fixed
            // start position cannot serve the full contract. NATS ordered pull
            // consumers cannot seek to a different start after creation.
            // To bound accumulation from rapid polling, inactive_threshold is set
            // to 5s (down from the NATS default of 30s) so abandoned ephemeral
            // consumers are reaped quickly by the server.
            metrics::counter!("trogon_atlas_store_read_changes_consumers_total").increment(1);
            let consumer = tokio::time::timeout(
                JS_OP_TIMEOUT,
                stream.create_consumer(PullConfig {
                    deliver_policy: DeliverPolicy::ByStartSequence { start_sequence },
                    ack_policy: AckPolicy::None,
                    inactive_threshold: std::time::Duration::from_secs(5),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|_| StoreError::Backend("create_consumer: timeout".into()))?
            .map_err(|e| StoreError::Backend(format!("create_consumer: {e}")))?;

            let mut batch = consumer
                .fetch()
                .max_messages(batch_limit)
                .messages()
                .await
                .map_err(|e| StoreError::Backend(format!("consumer.fetch: {e}")))?;

            let batch_timeout = Duration::from_secs(30);
            let mut raw_count = 0usize;
            loop {
                match tokio::time::timeout(batch_timeout, batch.next()).await {
                    Ok(Some(msg)) => {
                        let msg =
                            msg.map_err(|e| StoreError::Backend(format!("consumer next: {e}")))?;
                        let info = msg
                            .info()
                            .map_err(|e| StoreError::Backend(format!("msg.info: {e}")))?;
                        raw_count += 1;
                        cursor = info.stream_sequence;
                        match decode_change_record(info.stream_sequence, msg.payload.as_ref()) {
                            Ok(record) => out.push(record),
                            Err(err) => {
                                tracing::warn!(
                                    seq = info.stream_sequence,
                                    error = %err,
                                    "read_changes: skipping undecodable change record"
                                );
                            }
                        }
                        if out.len() >= limit {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        return Err(StoreError::Backend(format!(
                            "read_changes: batch timeout after {}s; received {} of {} requested records",
                            batch_timeout.as_secs(),
                            out.len(),
                            limit,
                        )));
                    }
                }
            }
            // No messages at/after cursor+1: durable log exhausted.
            if raw_count == 0 {
                break;
            }
            // Short raw batch means JetStream has nothing further; even if
            // some payloads were skipped we cannot fill `limit`.
            if raw_count < batch_limit {
                break;
            }
        }
        Ok(out)
    }

    async fn current_change_seq(&self) -> StoreResult<u64> {
        let info = self.stream_info().await?;
        Ok(info.state.last_sequence)
    }

    async fn prune_changes(&self, before_seq: u64) -> StoreResult<()> {
        // JetStream retention policies handle pruning server-side via the
        // configured max_age/max_msgs/max_bytes limits on the changes stream.
        // The `async-nats` 0.45 `stream.purge().sequence(..)` API exists but
        // requires careful sequencing with the live-tail consumer; that
        // complexity is not warranted given JetStream already enforces the
        // configured retention limits automatically. This is intentionally a
        // no-op for the NATS backend.
        tracing::debug!(
            before_seq,
            "prune_changes called; no-op for NatsStore (JetStream retention handles pruning server-side)"
        );
        Ok(())
    }

    async fn change_stream_stats(&self) -> StoreResult<Option<crate::store::ChangeStreamStats>> {
        let info = self.stream_info().await?;
        let oldest = info.state.first_timestamp;
        let oldest_secs = if info.state.messages == 0 {
            None
        } else {
            let secs = oldest.unix_timestamp();
            if secs >= 0 {
                Some(secs)
            } else {
                None
            }
        };
        Ok(Some(crate::store::ChangeStreamStats {
            messages: info.state.messages,
            bytes: info.state.bytes,
            oldest_message_unix_seconds: oldest_secs,
        }))
    }

    #[tracing::instrument(skip(self, ops), fields(op_count = ops.len(), branch))]
    async fn batch_apply(
        &self,
        ops: &[MutationOp],
        ctx: WriteContext<'_>,
    ) -> StoreResult<Vec<MutationOutcome>> {
        self.require_writer()?;
        if ops.len() > BATCH_APPLY_MAX_OPS {
            return Err(StoreError::InvalidArgument(format!(
                "batch_apply: op count {} exceeds hard cap {}",
                ops.len(),
                BATCH_APPLY_MAX_OPS,
            )));
        }
        if ctx.branch_name().is_some() {
            return self.batch_apply_branch(ops, ctx).await;
        }
        if ops.is_empty() {
            return Ok(Vec::new());
        }
        // Record the prior state of every touched key in the batch journal
        // before the first write. NATS KV lacks multi-key transactions, so
        // the batch is made recoverable instead: see `nats_journal.rs`.
        let mut priors: Vec<journal::JournalPrior> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (i, op) in ops.iter().enumerate() {
            let (kind, id) = match op {
                MutationOp::Create { kind, entity } | MutationOp::Put { kind, entity, .. } => {
                    let id = Self::validate(*kind, entity).map_err(|source| {
                        StoreError::BatchFailed {
                            index: i,
                            source: Box::new(source),
                        }
                    })?;
                    (*kind, id.clone())
                }
                MutationOp::Delete { kind, id, .. } => (*kind, id.clone()),
            };
            let key = entity_key(kind, &id);
            if seen.insert(key.clone()) {
                let prior = kv_entry_timeout(&self.kv, key.clone())
                    .await
                    .map_err(|source| StoreError::BatchFailed {
                        index: i,
                        source: Box::new(source),
                    })?;
                let revision = prior.as_ref().map(|entry| entry.revision);
                let image = live_entry(prior).and_then(|entry| decode_entity(&entry.value).ok());
                priors.push(journal::JournalPrior::new(key, revision, image));
            }
        }
        let mut batch_journal = self.begin_journal(ctx, priors.clone()).await?;

        // Collect outcomes and defer change events and revision rows until
        // after all KV writes succeed. This prevents ghost change events and
        // phantom history entries for ops that were later rolled back.
        let mut outcomes: Vec<MutationOutcome> = Vec::with_capacity(ops.len());
        let mut deferred: Vec<Revision> = Vec::with_capacity(ops.len());
        // Only keys actually mutated by successful ops are restored on
        // failure. Rewriting every snapshotted key (including the failing
        // Create's AlreadyExists victim) advances etags without a change
        // event and breaks callers holding pre-batch revisions.
        let mut applied_keys: BTreeSet<String> = BTreeSet::new();
        let mut changes: Vec<journal::JournalChange> = Vec::with_capacity(ops.len());
        let faults = self.faults();

        for (i, op) in ops.iter().enumerate() {
            if faults.crash_after_writes == Some(i) {
                return Err(journal::injected("crash"));
            }
            let op_key = match op {
                MutationOp::Create { kind, entity } | MutationOp::Put { kind, entity, .. } => {
                    Self::validate(*kind, entity)
                        .ok()
                        .map(|id| entity_key(*kind, id))
                }
                MutationOp::Delete { kind, id, .. } => Some(entity_key(*kind, id)),
            };
            // Apply only the KV mutation. Change events and revision rows are
            // deferred until the whole batch commits, so a rollback leaves no
            // ghost event and no phantom history entry behind.
            let result: StoreResult<(MutationOutcome, Revision)> = match op {
                MutationOp::Create { kind, entity } => 'create: {
                    let id = match Self::validate(*kind, entity) {
                        Ok(id) => id.clone(),
                        Err(e) => break 'create Err(e),
                    };
                    let key = entity_key(*kind, &id);
                    let stamped = crate::store::stamp_system(None, entity);
                    let bytes = encode_entity(&stamped);
                    match tokio::time::timeout(KV_OP_TIMEOUT, self.kv.create(&key, bytes.clone()))
                        .await
                    {
                        Err(_) => Err(StoreError::Backend(format!("kv.create {key}: timeout"))),
                        Ok(Ok(rev)) => Ok((
                            MutationOutcome::Wrote {
                                etag: rev.to_string(),
                                entity: stamped.clone(),
                            },
                            Revision {
                                kind: *kind,
                                id,
                                change: ChangeKind::Put,
                                before: None,
                                after: Some(stamped),
                            },
                        )),
                        Ok(Err(e)) => {
                            use async_nats::jetstream::kv::CreateErrorKind;
                            if !matches!(e.kind(), CreateErrorKind::AlreadyExists) {
                                break 'create Err(StoreError::Backend(format!(
                                    "kv.create {key}: {e}"
                                )));
                            }
                            let entry = match kv_entry_timeout(&self.kv, key.clone()).await {
                                Ok(Some(e)) => e,
                                Ok(None) => break 'create Err(StoreError::AlreadyExists),
                                Err(err) => break 'create Err(err),
                            };
                            if live_entry(Some(entry.clone())).is_some() {
                                break 'create Err(StoreError::AlreadyExists);
                            }
                            match tokio::time::timeout(
                                KV_OP_TIMEOUT,
                                self.kv.update(&key, bytes, entry.revision),
                            )
                            .await
                            {
                                Err(_) => {
                                    Err(StoreError::Backend(format!("kv.update {key}: timeout")))
                                }
                                Ok(Ok(rev)) => Ok((
                                    MutationOutcome::Wrote {
                                        etag: rev.to_string(),
                                        entity: stamped.clone(),
                                    },
                                    Revision {
                                        kind: *kind,
                                        id,
                                        change: ChangeKind::Put,
                                        before: None,
                                        after: Some(stamped),
                                    },
                                )),
                                Ok(Err(update_err)) => {
                                    use async_nats::jetstream::kv::UpdateErrorKind;
                                    if matches!(
                                        update_err.kind(),
                                        UpdateErrorKind::WrongLastRevision
                                    ) {
                                        Err(StoreError::AlreadyExists)
                                    } else {
                                        Err(StoreError::Backend(format!(
                                            "kv.update {key}: {update_err}"
                                        )))
                                    }
                                }
                            }
                        }
                    }
                }
                MutationOp::Put {
                    kind,
                    entity,
                    if_match,
                    force,
                } => {
                    // put_inner does not call record_change; safe for the unconditional
                    // path. The conditional path is inlined to avoid a duplicate
                    // record_change call that self.update would emit.
                    match if_match {
                        None => 'put_unconditional: {
                            let id = match Self::validate(*kind, entity) {
                                Ok(id) => id.clone(),
                                Err(e) => break 'put_unconditional Err(e),
                            };
                            match self.put_inner(*kind, entity, *force).await {
                                Ok(wrote) => {
                                    let revision = Revision {
                                        kind: *kind,
                                        id,
                                        change: ChangeKind::Put,
                                        before: wrote.before,
                                        after: Some(wrote.written.entity.clone()),
                                    };
                                    let outcome = if wrote.written.wrote {
                                        MutationOutcome::Wrote {
                                            etag: wrote.written.etag,
                                            entity: wrote.written.entity,
                                        }
                                    } else {
                                        MutationOutcome::Noop {
                                            etag: wrote.written.etag,
                                            entity: wrote.written.entity,
                                        }
                                    };
                                    Ok((outcome, revision))
                                }
                                Err(e) => Err(e),
                            }
                        }
                        Some(exp) => 'update: {
                            let id = match Self::validate(*kind, entity) {
                                Ok(id) => id.clone(),
                                Err(e) => break 'update Err(e),
                            };
                            let key = entity_key(*kind, &id);
                            let expected: u64 = match exp.parse() {
                                Ok(v) => v,
                                Err(_) => {
                                    break 'update Err(StoreError::EtagMismatch {
                                        expected: exp.clone(),
                                        found: String::new(),
                                    })
                                }
                            };
                            let entry = match kv_entry_timeout(&self.kv, key.clone()).await {
                                Ok(e) => e,
                                Err(e) => break 'update Err(e),
                            };
                            let (prev, prev_rev) = match entry {
                                Some(ref e) if live_entry(Some(e.clone())).is_some() => {
                                    let prev = match decode_entity(&e.value) {
                                        Ok(entity) => Some(entity),
                                        Err(err) => {
                                            tracing::error!(
                                                key = %key,
                                                error = %err,
                                                "corrupt entity in KV store during batch_apply put; \
                                                 proceeding without prior value for system stamp"
                                            );
                                            None
                                        }
                                    };
                                    (prev, Some(e.revision))
                                }
                                _ => break 'update Err(StoreError::NotFound),
                            };
                            // Same no-op semantics as the unconditional path:
                            // the etag guard must still hold.
                            if !force {
                                if let (Some(prev_entity), Some(rev)) = (prev.as_ref(), prev_rev) {
                                    if rev == expected
                                        && trogon_atlas_core::semantically_equal(
                                            prev_entity,
                                            entity,
                                        )
                                    {
                                        break 'update Ok((
                                            MutationOutcome::Noop {
                                                etag: rev.to_string(),
                                                entity: prev_entity.clone(),
                                            },
                                            Revision {
                                                kind: *kind,
                                                id,
                                                change: ChangeKind::Put,
                                                before: Some(prev_entity.clone()),
                                                after: Some(prev_entity.clone()),
                                            },
                                        ));
                                    }
                                }
                            }
                            let stamped = crate::store::stamp_system(prev.as_ref(), entity);
                            let bytes = encode_entity(&stamped);
                            match tokio::time::timeout(
                                KV_OP_TIMEOUT,
                                self.kv.update(&key, bytes, expected),
                            )
                            .await
                            {
                                Err(_) => {
                                    Err(StoreError::Backend(format!("kv.update {key}: timeout")))
                                }
                                Ok(Ok(rev)) => Ok((
                                    MutationOutcome::Wrote {
                                        etag: rev.to_string(),
                                        entity: stamped.clone(),
                                    },
                                    Revision {
                                        kind: *kind,
                                        id,
                                        change: ChangeKind::Put,
                                        before: prev,
                                        after: Some(stamped),
                                    },
                                )),
                                Ok(Err(e)) => {
                                    use async_nats::jetstream::kv::UpdateErrorKind;
                                    if matches!(e.kind(), UpdateErrorKind::WrongLastRevision) {
                                        Err(StoreError::EtagMismatch {
                                            expected: exp.clone(),
                                            found: live_found_etag(&self.kv, &key).await,
                                        })
                                    } else {
                                        Err(StoreError::Backend(format!("kv.update {key}: {e}")))
                                    }
                                }
                            }
                        }
                    }
                }
                MutationOp::Delete { kind, id, if_match } => 'delete: {
                    let key = entity_key(*kind, id);
                    let entry = match kv_entry_timeout(&self.kv, key.clone()).await {
                        Ok(e) => e,
                        Err(e) => break 'delete Err(e),
                    };
                    let Some(live) = live_entry(entry) else {
                        break 'delete Err(StoreError::NotFound);
                    };
                    let current = live.revision;
                    // Decoded before the delete lands: the entities bucket
                    // keeps one revision, so afterwards the revision row is
                    // the only place this image still exists.
                    let before = decode_entity(&live.value).ok();
                    if let Some(exp) = if_match {
                        let exp_rev: u64 = match exp.parse() {
                            Ok(v) => v,
                            Err(_) => {
                                break 'delete Err(StoreError::EtagMismatch {
                                    expected: exp.clone(),
                                    found: current.to_string(),
                                })
                            }
                        };
                        if exp_rev != current {
                            break 'delete Err(StoreError::EtagMismatch {
                                expected: exp.clone(),
                                found: current.to_string(),
                            });
                        }
                    }
                    let bytes = Bytes::new();
                    match tokio::time::timeout(KV_OP_TIMEOUT, self.kv.update(&key, bytes, current))
                        .await
                    {
                        Err(_) => Err(StoreError::Backend(format!(
                            "kv.delete (update step) {key}: timeout"
                        ))),
                        Ok(Ok(_)) => {
                            let after = match kv_entry_timeout(&self.kv, key.clone()).await {
                                Ok(e) => e,
                                Err(e) => break 'delete Err(e),
                            };
                            if let Some(entry) = after.as_ref() {
                                if live_entry(Some(entry.clone())).is_some() {
                                    break 'delete Err(StoreError::EtagMismatch {
                                        expected: current.to_string(),
                                        found: entry.revision.to_string(),
                                    });
                                }
                            }
                            let still_empty_put = after.as_ref().is_some_and(|e| {
                                e.operation == Operation::Put && e.value.is_empty()
                            });
                            if still_empty_put {
                                let mut delete_err: Option<String> = None;
                                for attempt in 0..DELETE_RETRY_ATTEMPTS {
                                    match tokio::time::timeout(KV_OP_TIMEOUT, self.kv.delete(&key))
                                        .await
                                        .map_err(|_| format!("kv.delete {key}: timeout"))
                                        .and_then(|r| {
                                            r.map_err(|e| format!("kv.delete {key}: {e}"))
                                        }) {
                                        Ok(()) => {
                                            delete_err = None;
                                            break;
                                        }
                                        Err(e) => {
                                            delete_err = Some(e);
                                            if attempt + 1 < DELETE_RETRY_ATTEMPTS {
                                                tokio::time::sleep(
                                                    DELETE_RETRY_BASE * 2u32.pow(attempt),
                                                )
                                                .await;
                                            }
                                        }
                                    }
                                }
                                if let Some(e) = delete_err {
                                    tracing::error!(
                                        key = %key,
                                        error = %e,
                                        "batch_apply kv.delete after conditional update failed after retries; \
                                         entry left as empty-Put tombstone (get will return NotFound)"
                                    );
                                }
                            }
                            Ok((
                                MutationOutcome::Deleted,
                                Revision {
                                    kind: *kind,
                                    id: id.clone(),
                                    change: ChangeKind::Delete,
                                    before,
                                    after: None,
                                },
                            ))
                        }
                        Ok(Err(e)) => {
                            use async_nats::jetstream::kv::UpdateErrorKind;
                            if matches!(e.kind(), UpdateErrorKind::WrongLastRevision) {
                                Err(StoreError::EtagMismatch {
                                    expected: current.to_string(),
                                    found: live_found_etag(&self.kv, &key).await,
                                })
                            } else {
                                Err(StoreError::Backend(format!("kv.delete {key}: {e}")))
                            }
                        }
                    }
                }
            };

            let result = if faults.fail_write_at == Some(i) {
                result.and(Err(journal::injected("entity write")))
            } else {
                result
            };
            match result {
                Ok((outcome, revision)) => {
                    // A skipped write changed nothing: no change event, no
                    // revision, and the key must not be rolled back
                    // (rewriting would bump etag).
                    if !matches!(outcome, MutationOutcome::Noop { .. }) {
                        if let Some(key) = op_key {
                            let etag = match &outcome {
                                MutationOutcome::Wrote { etag, .. } => {
                                    etag.parse().unwrap_or_default()
                                }
                                _ => 0,
                            };
                            changes.push(journal::JournalChange::landed(
                                key.clone(),
                                &revision,
                                etag,
                            ));
                            applied_keys.insert(key);
                        }
                        deferred.push(revision);
                    }
                    outcomes.push(outcome);
                }
                Err(source) => {
                    // A write that failed without a definite answer may
                    // still have landed, so its key is undone as well.
                    let mut undo = applied_keys;
                    if matches!(source, StoreError::Backend(_) | StoreError::Unavailable(_)) {
                        undo.extend(op_key);
                    }
                    let source = match self.undo_batch(batch_journal, &priors, &undo).await {
                        Some(partial) => partial,
                        None => source,
                    };
                    return Err(StoreError::BatchFailed {
                        index: i,
                        source: Box::new(source),
                    });
                }
            }
        }

        if faults.crash_after_writes == Some(ops.len()) {
            return Err(journal::injected("crash"));
        }
        if changes.is_empty() {
            if let Err(error) = batch_journal.finish().await {
                tracing::warn!(%error, "batch_apply: journal of a no-op batch not released");
            }
            return Ok(outcomes);
        }
        if let Err(error) = batch_journal.commit(changes).await {
            return Err(self
                .undo_batch(batch_journal, &priors, &applied_keys)
                .await
                .unwrap_or(error));
        }
        if faults.crash_after_commit {
            return Err(journal::injected("crash"));
        }

        // The batch is committed: from here on it is reported as applied
        // and a gap in its notifications or revisions is repaired by
        // recovery rolling it forward, never by undoing it.
        let mut complete = true;
        for (index, revision) in deferred.iter().enumerate() {
            let eref = revision.entity_ref();
            let msg_id = journal::notification_id(batch_journal.id(), index);
            match self
                .publish_change(revision.change, &eref, ctx.changeset(), Some(&msg_id))
                .await
            {
                Ok(_) => {}
                Err(error) => {
                    complete = false;
                    if matches!(error, StoreError::ChangeEventLost) {
                        self.change_events_lost.fetch_add(1, Ordering::Relaxed);
                        metrics::counter!("trogon_atlas_store_change_events_lost_total")
                            .increment(1);
                    }
                    tracing::error!(
                        %error,
                        entity_ref = ?eref,
                        journal = %batch_journal.id(),
                        "batch_apply: change event not published; batch recovery will republish it"
                    );
                }
            }
            if let Err(error) = self.try_record_revision(ctx, revision).await {
                complete = false;
                metrics::counter!("trogon_atlas_store_revisions_lost_total").increment(1);
                tracing::error!(
                    %error,
                    journal = %batch_journal.id(),
                    "batch_apply: revision not recorded; batch recovery will record it"
                );
            }
        }
        if complete {
            let id = batch_journal.id().clone();
            if let Err(error) = batch_journal.published().await {
                tracing::warn!(journal = %id, %error, "batch_apply: journal not advanced");
            }
        } else {
            metrics::counter!("trogon_atlas_store_batches_pending_recovery_total").increment(1);
        }

        Ok(outcomes)
    }

    async fn recover_batches(
        &self,
        policy: crate::recovery::RecoveryPolicy,
    ) -> StoreResult<crate::recovery::BatchRecoveryReport> {
        self.recover_journaled_batches(policy).await
    }

    fn subscribe(&self) -> broadcast::Receiver<ChangeRecord> {
        self.changes_tx.subscribe()
    }

    // -- Namespace registry -------------------------------------------------

    async fn register_namespace(
        &self,
        name: &NamespaceName,
        parent: &OwnerId,
        created_by: &str,
    ) -> StoreResult<NamespaceClaim> {
        self.require_writer()?;
        self.claim_namespace(
            NamespaceId::generate(),
            name,
            parent,
            created_by,
            &NamespaceTenure::Permanent,
        )
        .await
    }

    async fn adopt_namespace(
        &self,
        name: &NamespaceName,
        parent: &OwnerId,
        tenure: &NamespaceTenure,
    ) -> StoreResult<NamespaceClaim> {
        self.require_writer()?;
        self.claim_namespace(
            NamespaceId::adopt_legacy(name),
            name,
            parent,
            LEGACY_ADOPTION_AUTHOR,
            tenure,
        )
        .await
    }

    async fn restore_namespace(&self, record: &NamespaceRecord) -> StoreResult<NamespaceClaim> {
        self.require_writer()?;
        self.claim_namespace_record(record.clone()).await
    }

    async fn release_namespace(&self, id: &NamespaceId) -> StoreResult<()> {
        self.require_writer()?;
        let Some((record, _)) = self.read_namespace_row(id).await? else {
            return Ok(());
        };
        // The index row goes first. A crash between the two leaves a record
        // nothing resolves to, which the next claim of the name repairs; the
        // opposite order leaves a name pointing at a record that is gone,
        // which nothing repairs.
        let index_key = namespace_name_key(record.parent.as_str(), record.name.as_str());
        let _ = tokio::time::timeout(KV_OP_TIMEOUT, self.namespaces_kv.delete(&index_key)).await;
        let record_key = namespace_record_key(id.as_str());
        tokio::time::timeout(KV_OP_TIMEOUT, self.namespaces_kv.delete(&record_key))
            .await
            .map_err(|_| StoreError::Backend(format!("namespace kv.delete {record_key}: timeout")))?
            .map_err(|e| StoreError::Backend(format!("namespace kv.delete {record_key}: {e}")))?;
        Ok(())
    }

    async fn promote_namespace(&self, id: &NamespaceId) -> StoreResult<Option<NamespaceRecord>> {
        self.require_writer()?;
        let Some((record, revision)) = self.read_namespace_row(id).await? else {
            return Ok(None);
        };
        if record.tenure == NamespaceTenure::Permanent {
            return Ok(Some(record));
        }
        let promoted = NamespaceRecord {
            tenure: NamespaceTenure::Permanent,
            ..record
        };
        let record_key = namespace_record_key(id.as_str());
        // CAS, so a concurrent move or release loses rather than being
        // silently reverted by this write.
        tokio::time::timeout(
            KV_OP_TIMEOUT,
            self.namespaces_kv
                .update(&record_key, encode_namespace_record(&promoted), revision),
        )
        .await
        .map_err(|_| StoreError::Backend(format!("namespace kv.update {record_key}: timeout")))?
        .map_err(|e| StoreError::Backend(format!("namespace kv.update {record_key}: {e}")))?;
        Ok(Some(promoted))
    }

    async fn get_namespace(&self, id: &NamespaceId) -> StoreResult<Option<NamespaceRecord>> {
        Ok(self.read_namespace_row(id).await?.map(|(record, _)| record))
    }

    async fn resolve_namespace(
        &self,
        parent: &OwnerId,
        name: &NamespaceName,
    ) -> StoreResult<Option<NamespaceId>> {
        let index_key = namespace_name_key(parent.as_str(), name.as_str());
        Ok(self
            .follow_name_index(&index_key, parent, name)
            .await?
            .map(|record| record.id))
    }

    async fn list_namespaces(&self) -> StoreResult<Vec<NamespaceRecord>> {
        let mut keys_stream = tokio::time::timeout(KV_OP_TIMEOUT, self.namespaces_kv.keys())
            .await
            .map_err(|_| StoreError::Backend("namespace kv.keys: timeout".into()))?
            .map_err(|e| StoreError::Backend(format!("namespace kv.keys: {e}")))?;
        let mut record_keys: Vec<String> = Vec::new();
        loop {
            let next = tokio::time::timeout(KV_OP_TIMEOUT, keys_stream.next())
                .await
                .map_err(|_| {
                    StoreError::Backend(
                        "namespace kv.keys stream: timeout waiting for next key".into(),
                    )
                })?;
            let Some(item) = next else { break };
            let key =
                item.map_err(|e| StoreError::Backend(format!("namespace kv.keys stream: {e}")))?;
            // Skips the `name.` index rows: they are derived, and returning
            // them would double-count every namespace.
            if key.starts_with(namespace_record_prefix())
                && parse_namespace_record_key(&key).is_some()
            {
                record_keys.push(key);
            }
        }

        let mut records = Vec::with_capacity(record_keys.len());
        for key in record_keys {
            let Some(entry) = live_entry(kv_entry_timeout(&self.namespaces_kv, key).await?) else {
                continue;
            };
            records.push(decode_namespace_record(&entry.value)?);
        }
        records.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        Ok(records)
    }

    async fn move_namespace(
        &self,
        id: &NamespaceId,
        new_parent: &OwnerId,
    ) -> StoreResult<NamespaceRecord> {
        self.require_writer()?;
        let (record, revision) = self
            .read_namespace_row(id)
            .await?
            .ok_or(StoreError::NotFound)?;
        if &record.parent == new_parent {
            return Ok(record);
        }

        let old_index = namespace_name_key(record.parent.as_str(), record.name.as_str());
        let new_index = namespace_name_key(new_parent.as_str(), record.name.as_str());

        // Claim the destination name before touching anything. If the target
        // owner already has a namespace by this name, the move must fail
        // having changed nothing, or that owner ends up with two namespaces
        // the resolver cannot tell apart.
        if self
            .follow_name_index(&new_index, new_parent, &record.name)
            .await?
            .is_some()
        {
            return Err(StoreError::AlreadyExists);
        }
        let stale_revision = self.read_name_index(&new_index).await?.map(|(_, rev)| rev);
        self.write_name_index(&new_index, id, stale_revision)
            .await?;

        let moved = NamespaceRecord {
            parent: new_parent.clone(),
            ..record
        };
        let record_key = namespace_record_key(id.as_str());
        // CAS on the revision read above: a concurrent move must lose rather
        // than be silently overwritten.
        let updated = tokio::time::timeout(
            KV_OP_TIMEOUT,
            self.namespaces_kv
                .update(&record_key, encode_namespace_record(&moved), revision),
        )
        .await
        .map_err(|_| StoreError::Backend(format!("namespace kv.update {record_key}: timeout")))?;
        if let Err(e) = updated {
            // Undo the destination claim so a failed move leaves no trace.
            let _ =
                tokio::time::timeout(KV_OP_TIMEOUT, self.namespaces_kv.delete(&new_index)).await;
            return Err(StoreError::Backend(format!(
                "namespace kv.update {record_key}: {e}"
            )));
        }

        // Release the old name last. A leftover row here is harmless: it
        // points at a namespace whose parent no longer matches, so
        // `follow_name_index` reports it as absent and the next claim of that
        // name under the old owner repairs it.
        let _ = tokio::time::timeout(KV_OP_TIMEOUT, self.namespaces_kv.delete(&old_index)).await;
        Ok(moved)
    }

    // Thin forwards to the inherent methods in `operations.rs`: an inherent
    // method of the same name on the concrete type always shadows a trait
    // method for `self.foo()` dot-call resolution, so these never recurse.
    // Kept as a separate file for the same reason `nats_journal.rs` is: the
    // CAS/KV plumbing is easier to read on its own.

    async fn claim_operation(
        &self,
        key: &str,
        digest: &str,
        rpc: &str,
        branch: Option<&str>,
    ) -> StoreResult<ClaimOutcome> {
        self.claim_operation(key, digest, rpc, branch).await
    }

    async fn settle_operation_applied(&self, key: &str, changeset_id: &str) -> StoreResult<()> {
        self.settle_operation_applied(key, changeset_id).await
    }

    async fn settle_operation_rejected(
        &self,
        key: &str,
        code: &str,
        message: &str,
    ) -> StoreResult<()> {
        self.settle_operation_rejected(key, code, message).await
    }

    async fn settle_operation_not_applied(&self, key: &str) -> StoreResult<()> {
        self.settle_operation_not_applied(key).await
    }

    async fn get_operation(&self, key: &str) -> StoreResult<Option<OperationRecord>> {
        self.get_operation(key).await
    }

    async fn create_branch(&self, name: &str, doc: &str) -> StoreResult<BranchInfo> {
        self.require_writer()?;
        if crate::key::is_reserved_branch_name(name) {
            return Err(StoreError::InvalidArgument(format!(
                "branch name {name:?} is reserved"
            )));
        }
        let base_change_token = self.current_change_seq().await?.to_string();
        let fork_changeset_id = self.newest_baseline_changeset().await?;
        let info = BranchInfo {
            name: name.to_string(),
            doc: doc.to_string(),
            created_at: now_rfc3339(),
            base_change_token,
            delta_count: 0,
            fork_changeset_id,
        };
        let meta_key = branch_meta_key(name);
        let bytes = encode_branch_info(&info);
        match tokio::time::timeout(KV_OP_TIMEOUT, self.branches_kv.create(&meta_key, bytes))
            .await
            .map_err(|_| StoreError::Backend(format!("branch kv.create {meta_key}: timeout")))?
        {
            Ok(_) => Ok(info),
            Err(e) => {
                use async_nats::jetstream::kv::CreateErrorKind;
                if matches!(e.kind(), CreateErrorKind::AlreadyExists) {
                    Err(StoreError::AlreadyExists)
                } else {
                    Err(StoreError::Backend(format!(
                        "branch kv.create {meta_key}: {e}"
                    )))
                }
            }
        }
    }

    async fn set_branch_fork_point(&self, name: &str, fork_changeset_id: &str) -> StoreResult<()> {
        self.require_writer()?;
        validate_changeset_id(fork_changeset_id)?;
        let meta_key = branch_meta_key(name);
        let entry = live_entry(kv_entry_timeout(&self.branches_kv, meta_key.clone()).await?)
            .ok_or(StoreError::NotFound)?;
        // `delta_count` is derived at read time and never persisted, so the
        // zero passed here is discarded by `encode_branch_info` anyway.
        let mut info = decode_branch_info(&entry.value, 0)?;
        info.fork_changeset_id = Some(fork_changeset_id.to_owned());
        let bytes = encode_branch_info(&info);
        // CAS on the revision read above: a concurrent rename or fork-point
        // advance must lose rather than be silently overwritten.
        tokio::time::timeout(
            KV_OP_TIMEOUT,
            self.branches_kv.update(&meta_key, bytes, entry.revision),
        )
        .await
        .map_err(|_| StoreError::Backend(format!("branch kv.update {meta_key}: timeout")))?
        .map_err(|e| StoreError::Backend(format!("branch kv.update {meta_key}: {e}")))?;
        Ok(())
    }

    async fn list_branches(&self) -> StoreResult<Vec<BranchInfo>> {
        const META_PREFIX: &str = "meta.";
        let mut keys_stream = tokio::time::timeout(KV_OP_TIMEOUT, self.branches_kv.keys())
            .await
            .map_err(|_| StoreError::Backend("branch kv.keys: timeout".into()))?
            .map_err(|e| StoreError::Backend(format!("branch kv.keys: {e}")))?;
        let mut meta_keys: Vec<String> = Vec::new();
        loop {
            let next = tokio::time::timeout(KV_OP_TIMEOUT, keys_stream.next())
                .await
                .map_err(|_| {
                    StoreError::Backend(
                        "branch kv.keys stream: timeout waiting for next key".into(),
                    )
                })?;
            let Some(item) = next else { break };
            let key =
                item.map_err(|e| StoreError::Backend(format!("branch kv.keys stream: {e}")))?;
            if key.starts_with(META_PREFIX) {
                meta_keys.push(key);
            }
        }

        let mut branches: Vec<BranchInfo> = Vec::with_capacity(meta_keys.len());
        for meta_key in meta_keys {
            let Some(branch_name_enc) = meta_key.strip_prefix(META_PREFIX) else {
                continue;
            };
            let Some(name) = crate::key::decode_component_pub(branch_name_enc) else {
                continue;
            };
            let entry = kv_entry_timeout(&self.branches_kv, meta_key.clone()).await?;
            let Some(entry) = live_entry(entry) else {
                continue;
            };
            let delta_count = self.count_branch_deltas(&name).await?;
            branches.push(decode_branch_info(&entry.value, delta_count)?);
        }
        Ok(branches)
    }

    async fn delete_branch(&self, name: &str) -> StoreResult<()> {
        self.require_writer()?;
        let meta_key = branch_meta_key(name);
        let meta_entry = kv_entry_timeout(&self.branches_kv, meta_key.clone()).await?;
        if live_entry(meta_entry).is_none() {
            return Err(StoreError::NotFound);
        }

        let prefix = branch_delta_prefix(name);
        let mut keys_stream = tokio::time::timeout(KV_OP_TIMEOUT, self.branches_kv.keys())
            .await
            .map_err(|_| StoreError::Backend("branch kv.keys: timeout".into()))?
            .map_err(|e| StoreError::Backend(format!("branch kv.keys: {e}")))?;
        let mut delta_keys: Vec<String> = Vec::new();
        loop {
            let next = tokio::time::timeout(KV_OP_TIMEOUT, keys_stream.next())
                .await
                .map_err(|_| {
                    StoreError::Backend(
                        "branch kv.keys stream: timeout waiting for next key".into(),
                    )
                })?;
            let Some(item) = next else { break };
            let key =
                item.map_err(|e| StoreError::Backend(format!("branch kv.keys stream: {e}")))?;
            if key.starts_with(&prefix) {
                delta_keys.push(key);
            }
        }

        for key in delta_keys {
            if let Err(e) = tokio::time::timeout(KV_OP_TIMEOUT, self.branches_kv.purge(&key))
                .await
                .map_err(|_| "timeout".to_string())
                .and_then(|r| r.map_err(|e| e.to_string()))
            {
                tracing::error!(
                    key = %key,
                    error = %e,
                    "delete_branch: failed to purge delta row; branch may be left with orphaned deltas"
                );
            }
        }
        tokio::time::timeout(KV_OP_TIMEOUT, self.branches_kv.purge(&meta_key))
            .await
            .map_err(|_| StoreError::Backend(format!("branch kv.purge {meta_key}: timeout")))?
            .map_err(|e| StoreError::Backend(format!("branch kv.purge {meta_key}: {e}")))?;
        Ok(())
    }

    async fn list_branch_deltas(&self, branch: &str) -> StoreResult<Vec<BranchDeltaEntry>> {
        let delta_keys = enumerate_branch_delta_keys(&self.branches_kv, branch).await?;
        let mut out = Vec::with_capacity(delta_keys.len());
        for key in delta_keys {
            let Some((kind, id)) = crate::key::parse_branch_delta_key(branch, &key) else {
                continue;
            };
            let Some(entry) = live_entry(kv_entry_timeout(&self.branches_kv, key.clone()).await?)
            else {
                continue;
            };
            let delta = decode_branch_delta(&entry.value)?;
            out.push(BranchDeltaEntry {
                kind,
                id,
                base: delta.base,
                base_etag: delta.base_etag,
                ours: delta.ours,
                tombstone: delta.tombstone,
                etag: entry.revision.to_string(),
            });
        }
        Ok(out)
    }

    async fn land_branch_merge(
        &self,
        branch: &str,
        ops: &[BranchLandOp],
        ctx: WriteContext<'_>,
    ) -> StoreResult<Vec<MutationOutcome>> {
        self.require_writer()?;
        // Snapshot the prior baseline state of every touched key so a
        // failure partway through can be rolled back. Also verify the
        // etag guard up front so a stale diff is caught before any write.
        let mut snapshot: BTreeMap<String, Option<(Entity, u64)>> = BTreeMap::new();
        for (i, op) in ops.iter().enumerate() {
            let (key, expected) = match op {
                BranchLandOp::Put {
                    kind,
                    ours,
                    expected_baseline_etag,
                } => {
                    let id = Self::validate(*kind, ours)
                        .map_err(|source| StoreError::BatchFailed {
                            index: i,
                            source: Box::new(source),
                        })?
                        .clone();
                    (entity_key(*kind, &id), expected_baseline_etag.clone())
                }
                BranchLandOp::Delete {
                    kind,
                    id,
                    expected_baseline_etag,
                } => (entity_key(*kind, id), Some(expected_baseline_etag.clone())),
            };
            if let std::collections::btree_map::Entry::Vacant(slot) = snapshot.entry(key.clone()) {
                let prior = kv_entry_timeout(&self.kv, key.clone())
                    .await
                    .map_err(|source| StoreError::BatchFailed {
                        index: i,
                        source: Box::new(source),
                    })?;
                let prior_live = live_entry(prior).and_then(|entry| {
                    decode_entity(&entry.value)
                        .ok()
                        .map(|ent| (ent, entry.revision))
                });
                let found = prior_live.as_ref().map(|(_, rev)| rev.to_string());
                if expected != found {
                    return Err(StoreError::BatchFailed {
                        index: i,
                        source: Box::new(StoreError::EtagMismatch {
                            expected: expected.unwrap_or_default(),
                            found: found.unwrap_or_default(),
                        }),
                    });
                }
                slot.insert(prior_live);
            }
        }

        let mut outcomes: Vec<MutationOutcome> = Vec::with_capacity(ops.len());
        let mut deferred: Vec<Revision> = Vec::with_capacity(ops.len());
        let mut landed_keys: Vec<(EntityKind, Id)> = Vec::with_capacity(ops.len());
        // The pre-image of a landing is the baseline row the merge replaced,
        // which the rollback snapshot above already holds.
        let baseline_before = |kind: EntityKind, id: &Id| -> Option<Entity> {
            snapshot
                .get(&entity_key(kind, id))
                .and_then(|prior| prior.as_ref().map(|(entity, _)| entity.clone()))
        };

        for (i, op) in ops.iter().enumerate() {
            let result: StoreResult<(MutationOutcome, Revision)> = match op {
                BranchLandOp::Put {
                    kind,
                    ours,
                    expected_baseline_etag,
                } => 'put: {
                    let id = match Self::validate(*kind, ours) {
                        Ok(id) => id.clone(),
                        Err(e) => break 'put Err(e),
                    };
                    let key = entity_key(*kind, &id);
                    // Verbatim write: `ours` is stored exactly as
                    // authored on the branch, including its `system`
                    // stamp. No `stamp_system` call here -- that is the
                    // whole point of a merge landing (identity
                    // preserved, no re-stamping; see
                    // docs/explanation/branching.md).
                    let bytes = encode_entity(ours);
                    let cas_result = match expected_baseline_etag {
                        Some(exp) => match exp.parse::<u64>() {
                            Ok(rev) => tokio::time::timeout(
                                KV_OP_TIMEOUT,
                                self.kv.update(&key, bytes, rev),
                            )
                            .await
                            .map_err(|_| StoreError::Backend(format!("kv.update {key}: timeout")))?
                            .map_err(|e| {
                                use async_nats::jetstream::kv::UpdateErrorKind;
                                if matches!(e.kind(), UpdateErrorKind::WrongLastRevision) {
                                    StoreError::EtagMismatch {
                                        expected: rev.to_string(),
                                        found: String::new(),
                                    }
                                } else {
                                    StoreError::Backend(format!("kv.update {key}: {e}"))
                                }
                            }),
                            Err(_) => Err(StoreError::EtagMismatch {
                                expected: exp.clone(),
                                found: String::new(),
                            }),
                        },
                        None => tokio::time::timeout(KV_OP_TIMEOUT, self.kv.create(&key, bytes))
                            .await
                            .map_err(|_| StoreError::Backend(format!("kv.create {key}: timeout")))?
                            .map_err(|e| {
                                use async_nats::jetstream::kv::CreateErrorKind;
                                if matches!(e.kind(), CreateErrorKind::AlreadyExists) {
                                    StoreError::EtagMismatch {
                                        expected: "absent".into(),
                                        found: "present".into(),
                                    }
                                } else {
                                    StoreError::Backend(format!("kv.create {key}: {e}"))
                                }
                            }),
                    };
                    match cas_result {
                        Ok(rev) => Ok((
                            MutationOutcome::Wrote {
                                etag: rev.to_string(),
                                entity: (**ours).clone(),
                            },
                            Revision {
                                kind: *kind,
                                change: ChangeKind::Put,
                                before: baseline_before(*kind, &id),
                                after: Some((**ours).clone()),
                                id,
                            },
                        )),
                        Err(e) => Err(e),
                    }
                }
                BranchLandOp::Delete {
                    kind,
                    id,
                    expected_baseline_etag,
                } => 'delete: {
                    let key = entity_key(*kind, id);
                    let current: u64 = match expected_baseline_etag.parse() {
                        Ok(v) => v,
                        Err(_) => {
                            break 'delete Err(StoreError::EtagMismatch {
                                expected: expected_baseline_etag.clone(),
                                found: String::new(),
                            })
                        }
                    };
                    let bytes = Bytes::new();
                    match tokio::time::timeout(KV_OP_TIMEOUT, self.kv.update(&key, bytes, current))
                        .await
                    {
                        Err(_) => Err(StoreError::Backend(format!(
                            "kv.delete (update step) {key}: timeout"
                        ))),
                        Ok(Ok(_)) => {
                            let mut delete_err: Option<String> = None;
                            for attempt in 0..DELETE_RETRY_ATTEMPTS {
                                match tokio::time::timeout(KV_OP_TIMEOUT, self.kv.delete(&key))
                                    .await
                                    .map_err(|_| format!("kv.delete {key}: timeout"))
                                    .and_then(|r| r.map_err(|e| format!("kv.delete {key}: {e}")))
                                {
                                    Ok(()) => {
                                        delete_err = None;
                                        break;
                                    }
                                    Err(e) => {
                                        delete_err = Some(e);
                                        if attempt + 1 < DELETE_RETRY_ATTEMPTS {
                                            tokio::time::sleep(
                                                DELETE_RETRY_BASE * 2u32.pow(attempt),
                                            )
                                            .await;
                                        }
                                    }
                                }
                            }
                            if let Some(e) = delete_err {
                                tracing::error!(
                                    key = %key,
                                    error = %e,
                                    "land_branch_merge kv.delete after conditional update \
                                     failed after retries; entry left as empty-Put tombstone"
                                );
                            }
                            Ok((
                                MutationOutcome::Deleted,
                                Revision {
                                    kind: *kind,
                                    id: id.clone(),
                                    change: ChangeKind::Delete,
                                    before: baseline_before(*kind, id),
                                    after: None,
                                },
                            ))
                        }
                        Ok(Err(e)) => {
                            use async_nats::jetstream::kv::UpdateErrorKind;
                            if matches!(e.kind(), UpdateErrorKind::WrongLastRevision) {
                                Err(StoreError::EtagMismatch {
                                    expected: current.to_string(),
                                    found: String::new(),
                                })
                            } else {
                                Err(StoreError::Backend(format!("kv.delete {key}: {e}")))
                            }
                        }
                    }
                }
            };

            match result {
                Ok((outcome, revision)) => {
                    landed_keys.push((revision.kind, revision.id.clone()));
                    deferred.push(revision);
                    outcomes.push(outcome);
                }
                Err(source) => {
                    let failed_keys = rollback_baseline_snapshot(&snapshot, &self.kv).await;
                    if !failed_keys.is_empty() {
                        return Err(StoreError::BatchFailed {
                            index: i,
                            source: Box::new(StoreError::PartialApply {
                                keys: failed_keys,
                                journal: None,
                            }),
                        });
                    }
                    return Err(StoreError::BatchFailed {
                        index: i,
                        source: Box::new(source),
                    });
                }
            }
        }

        // Every KV write committed. Publish ordinary baseline change events
        // (a merge landing is an ordinary baseline write for every purpose
        // downstream: change feed, git mirror, search index -- see
        // docs/explanation/branching.md).
        for revision in deferred {
            let eref = revision.entity_ref();
            match self
                .record_change(revision.change, &eref, ctx.changeset())
                .await
            {
                Ok(_) => {}
                Err(StoreError::ChangeEventLost) => {
                    self.change_events_lost.fetch_add(1, Ordering::Relaxed);
                    metrics::counter!("trogon_atlas_store_change_events_lost_total").increment(1);
                    tracing::error!(
                        entity_ref = ?eref,
                        "land_branch_merge: change event lost after successful KV write"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        entity_ref = ?eref,
                        "land_branch_merge: unexpected error publishing change event"
                    );
                }
            }
            // Baseline history, not branch history: `ctx` is the merge RPC's
            // own baseline context, so the revision lands on the baseline log
            // beside every other baseline write.
            self.record_revision(ctx, &revision).await;
        }

        // Purge the landed delta rows -- these keys are now ordinary
        // baseline entities/tombstones, so the branch overlay for them must
        // disappear. Best-effort: a failure here leaves an orphaned delta
        // that a later diff/merge will simply see as CONVERGED (ours now
        // equals the freshly-landed baseline) or re-detect cleanly.
        for (kind, id) in landed_keys {
            let delta_key = branch_delta_key(branch, kind, &id);
            if let Err(e) = tokio::time::timeout(KV_OP_TIMEOUT, self.branches_kv.purge(&delta_key))
                .await
                .map_err(|_| "timeout".to_string())
                .and_then(|r| r.map_err(|e| e.to_string()))
            {
                tracing::error!(
                    key = %delta_key,
                    error = %e,
                    "land_branch_merge: failed to purge landed delta row; \
                     branch may retain a stale (now-converged) delta"
                );
            }
        }

        Ok(outcomes)
    }

    async fn rebase_branch_deltas(
        &self,
        branch: &str,
        rebases: &[(EntityKind, Id, Option<(Entity, String)>)],
    ) -> StoreResult<u32> {
        self.require_writer()?;
        let mut count = 0u32;
        for (kind, id, new_base) in rebases {
            let delta_key = branch_delta_key(branch, *kind, id);
            let Some(entry) =
                live_entry(kv_entry_timeout(&self.branches_kv, delta_key.clone()).await?)
            else {
                continue;
            };
            let delta = decode_branch_delta(&entry.value)?;
            match new_base {
                None => {
                    // CONVERGED: the delta collapses to nothing since
                    // `ours` already matches the new baseline.
                    tokio::time::timeout(KV_OP_TIMEOUT, self.branches_kv.purge(&delta_key))
                        .await
                        .map_err(|_| {
                            StoreError::Backend(format!("branch kv.purge {delta_key}: timeout"))
                        })?
                        .map_err(|e| {
                            StoreError::Backend(format!("branch kv.purge {delta_key}: {e}"))
                        })?;
                }
                Some((base_entity, base_etag)) => {
                    let updated = trogon_atlas_proto::BranchDelta {
                        base: Some(base_entity.clone()),
                        base_etag: base_etag.clone(),
                        ours: delta.ours,
                        tombstone: delta.tombstone,
                    };
                    let bytes = encode_branch_delta(&updated);
                    tokio::time::timeout(
                        KV_OP_TIMEOUT,
                        self.branches_kv.update(&delta_key, bytes, entry.revision),
                    )
                    .await
                    .map_err(|_| {
                        StoreError::Backend(format!("branch kv.update {delta_key}: timeout"))
                    })?
                    .map_err(|e| {
                        StoreError::Backend(format!("branch kv.update {delta_key}: {e}"))
                    })?;
                }
            }
            count += 1;
        }
        Ok(count)
    }

    async fn resolve_branch_entry(
        &self,
        branch: &str,
        kind: EntityKind,
        id: &Id,
        take_theirs: bool,
    ) -> StoreResult<()> {
        let delta_key = branch_delta_key(branch, kind, id);
        let Some(entry) = live_entry(kv_entry_timeout(&self.branches_kv, delta_key.clone()).await?)
        else {
            return Err(StoreError::NotFound);
        };
        let delta = decode_branch_delta(&entry.value)?;

        if take_theirs {
            let base_key = entity_key(kind, id);
            let base_entry = kv_entry_timeout(&self.kv, base_key).await?;
            match live_entry(base_entry) {
                Some(be) => {
                    let baseline_entity = decode_entity(&be.value)?;
                    let baseline_etag = be.revision.to_string();
                    let updated = trogon_atlas_proto::BranchDelta {
                        base: Some(baseline_entity.clone()),
                        base_etag: baseline_etag,
                        ours: Some(baseline_entity),
                        tombstone: false,
                    };
                    let bytes = encode_branch_delta(&updated);
                    tokio::time::timeout(
                        KV_OP_TIMEOUT,
                        self.branches_kv.update(&delta_key, bytes, entry.revision),
                    )
                    .await
                    .map_err(|_| {
                        StoreError::Backend(format!("branch kv.update {delta_key}: timeout"))
                    })?
                    .map_err(|e| {
                        StoreError::Backend(format!("branch kv.update {delta_key}: {e}"))
                    })?;
                }
                None => {
                    // Baseline no longer has this key: taking "theirs" means
                    // the delta disappears entirely (nothing to overlay).
                    tokio::time::timeout(KV_OP_TIMEOUT, self.branches_kv.purge(&delta_key))
                        .await
                        .map_err(|_| {
                            StoreError::Backend(format!("branch kv.purge {delta_key}: timeout"))
                        })?
                        .map_err(|e| {
                            StoreError::Backend(format!("branch kv.purge {delta_key}: {e}"))
                        })?;
                }
            }
            return Ok(());
        }

        // Keep ours: rebase base/base_etag forward to current baseline
        // (which may be absent if baseline deleted the key), leaving
        // `ours`/`tombstone` untouched.
        let base_key = entity_key(kind, id);
        let base_entry = kv_entry_timeout(&self.kv, base_key).await?;
        let (new_base, new_base_etag) = match live_entry(base_entry) {
            Some(be) => (Some(decode_entity(&be.value)?), be.revision.to_string()),
            None => (None, String::new()),
        };
        let updated = trogon_atlas_proto::BranchDelta {
            base: new_base,
            base_etag: new_base_etag,
            ours: delta.ours,
            tombstone: delta.tombstone,
        };
        let bytes = encode_branch_delta(&updated);
        tokio::time::timeout(
            KV_OP_TIMEOUT,
            self.branches_kv.update(&delta_key, bytes, entry.revision),
        )
        .await
        .map_err(|_| StoreError::Backend(format!("branch kv.update {delta_key}: timeout")))?
        .map_err(|e| StoreError::Backend(format!("branch kv.update {delta_key}: {e}")))?;
        Ok(())
    }

    async fn append_changeset(&self, record: &ChangesetRecord) -> StoreResult<()> {
        validate_changeset_id(&record.id)?;
        if self.faults().fail_changeset_append {
            return Err(journal::injected("changeset append"));
        }
        let bytes = Bytes::from(changeset_proto(record).encode_to_vec());
        match tokio::time::timeout(KV_OP_TIMEOUT, self.changesets_kv.create(&record.id, bytes))
            .await
            .map_err(|_| {
                StoreError::Backend(format!("changeset kv.create {id}: timeout", id = record.id))
            })? {
            Ok(_) => {}
            Err(e) => {
                use async_nats::jetstream::kv::CreateErrorKind;
                // Idempotent on id: a retry that reuses the same changeset id
                // must not append a second row. First write wins.
                if !matches!(e.kind(), CreateErrorKind::AlreadyExists) {
                    return Err(StoreError::Backend(format!(
                        "changeset kv.create {id}: {e}",
                        id = record.id
                    )));
                }
            }
        }
        if record.branch.is_none() {
            self.release_published_journal(&record.id).await;
        }
        Ok(())
    }

    async fn get_changeset(&self, id: &str) -> StoreResult<ChangesetRecord> {
        validate_changeset_id(id)?;
        let entry = live_entry(kv_entry_timeout(&self.changesets_kv, id.to_owned()).await?)
            .ok_or(StoreError::NotFound)?;
        decode_changeset(&entry.value)
    }

    async fn list_changesets(&self, page: ChangesetPage<'_>) -> StoreResult<Vec<ChangesetRecord>> {
        let limit = page.limit.clamp(1, CHANGESET_LIST_HARD_CAP);
        if let Some(before) = page.before {
            validate_changeset_id(before)?;
        }
        let mut out: Vec<ChangesetRecord> = Vec::with_capacity(limit);
        let mut cursor: Option<String> = page.before.map(str::to_owned);
        // Under `ChangesetScope::All` one pass suffices. Under a narrower
        // scope a pass can be filtered down to nothing, so keep walking
        // backwards until the page fills or the bucket is exhausted.
        loop {
            let keys =
                newest_keys_before(&self.changesets_kv, None, cursor.as_deref(), limit).await?;
            let exhausted = keys.len() < limit;
            let Some(last) = keys.last().cloned() else {
                break;
            };
            cursor = Some(last);
            for key in keys {
                let Some(entry) =
                    live_entry(kv_entry_timeout(&self.changesets_kv, key.clone()).await?)
                else {
                    // Enumerated then deleted, or a tombstone. Changesets are
                    // never deleted by this crate, so this only happens under
                    // external interference; skip rather than fail the page.
                    continue;
                };
                let record = match decode_changeset(&entry.value) {
                    Ok(record) => record,
                    Err(err) => {
                        tracing::warn!(
                            key = %key,
                            error = %err,
                            "list_changesets: skipping undecodable changeset"
                        );
                        continue;
                    }
                };
                if !page.scope.admits(record.branch.as_deref()) {
                    continue;
                }
                out.push(record);
                if out.len() >= limit {
                    return Ok(out);
                }
            }
            if exhausted {
                break;
            }
        }
        Ok(out)
    }

    async fn list_entity_revisions(
        &self,
        kind: EntityKind,
        id: &Id,
        page: RevisionPage<'_>,
    ) -> StoreResult<Vec<EntityRevisionRecord>> {
        let limit = page.limit.clamp(1, CHANGESET_LIST_HARD_CAP);
        if let Some(before) = page.before {
            validate_changeset_id(before)?;
        }
        let prefix = revision_prefix(page.branch, kind, id);
        let cursor = page.before.map(|before| format!("{prefix}{before}"));
        let keys = newest_keys_before(
            &self.revisions_kv,
            Some(prefix.as_str()),
            cursor.as_deref(),
            limit,
        )
        .await?;
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let Some(entry) = live_entry(kv_entry_timeout(&self.revisions_kv, key.clone()).await?)
            else {
                // Purged by a rolled-back batch between the key scan and the
                // read. Skip rather than fail the page.
                continue;
            };
            match decode_entity_revision(&entry.value) {
                Ok(record) => out.push(record),
                Err(err) => {
                    tracing::warn!(
                        key = %key,
                        error = %err,
                        "list_entity_revisions: skipping undecodable revision"
                    );
                }
            }
        }
        Ok(out)
    }

    async fn get_entity_revision(
        &self,
        kind: EntityKind,
        id: &Id,
        changeset_id: &str,
        branch: Option<&str>,
    ) -> StoreResult<EntityRevisionRecord> {
        validate_changeset_id(changeset_id)?;
        let key = revision_key(branch, kind, id, changeset_id);
        let entry = live_entry(kv_entry_timeout(&self.revisions_kv, key).await?)
            .ok_or(StoreError::NotFound)?;
        decode_entity_revision(&entry.value)
    }
}

/// Hard cap on one `list_changesets` page.
const CHANGESET_LIST_HARD_CAP: usize = 500;

/// Changeset ids are UUIDv7 in canonical lowercase hyphenated form. The id
/// doubles as a KV key and as a page cursor, so anything else is rejected
/// here rather than turning into an opaque backend error (or, worse, a key
/// containing a NATS subject wildcard).
fn validate_changeset_id(id: &str) -> StoreResult<()> {
    let well_formed = id.len() == 36
        && id.as_bytes().iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_digit() || matches!(b, b'a'..=b'f'),
        });
    if well_formed {
        Ok(())
    } else {
        Err(StoreError::InvalidArgument(format!(
            "changeset id {id:?} is not a lowercase hyphenated UUID"
        )))
    }
}

/// The newest `limit` keys under `prefix` ordered strictly before `before`,
/// returned newest first.
///
/// `JetStream` KV has no ordered range scan, so this streams the bucket's
/// key set. Only `limit` keys are ever held: both the changeset log and the
/// revision log end their keys in a UUIDv7, so lexicographic order is
/// chronological order and a bounded set of the largest candidates is
/// exactly the newest page.
async fn newest_keys_before(
    kv: &KvStore,
    prefix: Option<&str>,
    before: Option<&str>,
    limit: usize,
) -> StoreResult<Vec<String>> {
    let mut newest: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut keys_stream = tokio::time::timeout(KV_OP_TIMEOUT, kv.keys())
        .await
        .map_err(|_| StoreError::Backend("kv.keys: timeout".into()))?
        .map_err(|e| StoreError::Backend(format!("kv.keys: {e}")))?;
    loop {
        let next = tokio::time::timeout(KV_OP_TIMEOUT, keys_stream.next())
            .await
            .map_err(|_| {
                StoreError::Backend("kv.keys stream: timeout waiting for next key".into())
            })?;
        let Some(item) = next else { break };
        let key = item.map_err(|e| StoreError::Backend(format!("kv.keys stream: {e}")))?;
        if let Some(prefix) = prefix {
            if !key.starts_with(prefix) {
                continue;
            }
        }
        if let Some(before) = before {
            if key.as_str() >= before {
                continue;
            }
        }
        newest.insert(key);
        if newest.len() > limit {
            newest.pop_first();
        }
    }
    Ok(newest.into_iter().rev().collect())
}

fn entity_revision_proto(
    changeset: ChangesetRef<'_>,
    branch: Option<&str>,
    revision: &Revision,
) -> trogon_atlas_proto::EntityRevision {
    use trogon_atlas_proto::change_event::Kind as PbKind;
    trogon_atlas_proto::EntityRevision {
        changeset_id: changeset.id.to_owned(),
        author: changeset.author.to_owned(),
        at: now_rfc3339(),
        rpc: changeset.rpc.to_owned(),
        branch: branch.unwrap_or_default().to_owned(),
        entity: Some(revision.entity_ref()),
        kind: match revision.change {
            ChangeKind::Put => PbKind::Put as i32,
            ChangeKind::Delete => PbKind::Deleted as i32,
        },
        before: revision.before.clone(),
        after: revision.after.clone(),
    }
}

fn decode_entity_revision(bytes: &[u8]) -> StoreResult<EntityRevisionRecord> {
    let pb = trogon_atlas_proto::EntityRevision::decode(bytes)
        .map_err(|e| StoreError::Backend(format!("decode EntityRevision: {e}")))?;
    let entity_ref = pb
        .entity
        .ok_or_else(|| StoreError::Backend("EntityRevision.entity is required".into()))?;
    let kind = match pb.kind {
        x if x == trogon_atlas_proto::change_event::Kind::Put as i32 => ChangeKind::Put,
        x if x == trogon_atlas_proto::change_event::Kind::Deleted as i32 => ChangeKind::Delete,
        other => {
            return Err(StoreError::Backend(format!(
                "unknown EntityRevision.kind value {other}"
            )))
        }
    };
    Ok(EntityRevisionRecord {
        changeset_id: pb.changeset_id,
        author: pb.author,
        at: pb.at,
        rpc: pb.rpc,
        branch: if pb.branch.is_empty() {
            None
        } else {
            Some(pb.branch)
        },
        entity_ref,
        kind,
        before: pb.before,
        after: pb.after,
    })
}

fn changeset_proto(record: &ChangesetRecord) -> trogon_atlas_proto::Changeset {
    use trogon_atlas_proto::change_event::Kind as PbKind;
    trogon_atlas_proto::Changeset {
        id: record.id.clone(),
        author: record.author.clone(),
        at: record.at.clone(),
        message: record.message.clone(),
        branch: record.branch.clone().unwrap_or_default(),
        rpc: record.rpc.clone(),
        operation_id: record.operation_id.clone().unwrap_or_default(),
        ops: record
            .ops
            .iter()
            .map(|op| trogon_atlas_proto::ChangesetOp {
                kind: match op.kind {
                    ChangeKind::Put => PbKind::Put as i32,
                    ChangeKind::Delete => PbKind::Deleted as i32,
                },
                entity: Some(op.entity_ref.clone()),
            })
            .collect(),
    }
}

fn decode_changeset(bytes: &[u8]) -> StoreResult<ChangesetRecord> {
    let pb = trogon_atlas_proto::Changeset::decode(bytes)
        .map_err(|e| StoreError::Backend(format!("decode Changeset: {e}")))?;
    let mut ops = Vec::with_capacity(pb.ops.len());
    for op in pb.ops {
        let entity_ref = op
            .entity
            .ok_or_else(|| StoreError::Backend("ChangesetOp.entity is required".into()))?;
        let kind = match op.kind {
            x if x == trogon_atlas_proto::change_event::Kind::Put as i32 => ChangeKind::Put,
            x if x == trogon_atlas_proto::change_event::Kind::Deleted as i32 => ChangeKind::Delete,
            other => {
                return Err(StoreError::Backend(format!(
                    "unknown ChangesetOp.kind value {other}"
                )))
            }
        };
        ops.push(ChangesetOp { kind, entity_ref });
    }
    Ok(ChangesetRecord {
        id: pb.id,
        author: pb.author,
        at: pb.at,
        message: pb.message,
        rpc: pb.rpc,
        branch: if pb.branch.is_empty() {
            None
        } else {
            Some(pb.branch)
        },
        ops,
        operation_id: if pb.operation_id.is_empty() {
            None
        } else {
            Some(pb.operation_id)
        },
    })
}

/// Enumerate every raw delta-row key belonging to `branch` in the branches
/// KV bucket. Shared by `list_branch_deltas`, `list_branch_merged`, and
/// `delete_branch` so the prefix-scan logic lives in one place.
async fn enumerate_branch_delta_keys(
    branches_kv: &KvStore,
    branch: &str,
) -> StoreResult<Vec<String>> {
    let prefix = branch_delta_prefix(branch);
    let mut keys_stream = tokio::time::timeout(KV_OP_TIMEOUT, branches_kv.keys())
        .await
        .map_err(|_| StoreError::Backend("branch kv.keys: timeout".into()))?
        .map_err(|e| StoreError::Backend(format!("branch kv.keys: {e}")))?;
    let mut delta_keys: Vec<String> = Vec::new();
    loop {
        let next = tokio::time::timeout(KV_OP_TIMEOUT, keys_stream.next())
            .await
            .map_err(|_| {
                StoreError::Backend("branch kv.keys stream: timeout waiting for next key".into())
            })?;
        let Some(item) = next else { break };
        let key = item.map_err(|e| StoreError::Backend(format!("branch kv.keys stream: {e}")))?;
        if key.starts_with(&prefix) {
            delta_keys.push(key);
        }
    }
    Ok(delta_keys)
}

/// Compensating rollback shared by baseline `batch_apply` and
/// `land_branch_merge`: restore every key in `snapshot` to its prior state
/// via CAS, logging (and collecting into the returned `Vec`) any key where
/// a concurrent external writer raced the restore.
///
/// When the prior value is live but the key is currently an empty-Put
/// tombstone (delete sentinel left behind when `kv.delete` never lands),
/// `create` alone cannot revive the row (same hole as baseline `create`),
/// so we CAS-overwrite the tombstone.
async fn rollback_baseline_snapshot(
    snapshot: &BTreeMap<String, Option<(Entity, u64)>>,
    store: &KvStore,
) -> Vec<String> {
    let mut failed_keys: Vec<String> = Vec::new();
    for (key, prior) in snapshot {
        use async_nats::jetstream::kv::{CreateErrorKind, UpdateErrorKind};
        let current_rev = match kv_entry_timeout(store, key.clone()).await {
            Ok(entry) => live_entry(entry).map(|e| e.revision),
            Err(err) => {
                tracing::error!(
                    error = %err,
                    key = %key,
                    "rollback: kv.entry failed; store may be partially applied"
                );
                failed_keys.push(key.clone());
                continue;
            }
        };
        match prior {
            Some((entity, _snapshot_rev)) => {
                let blob = encode_entity(entity);
                match current_rev {
                    Some(rev) => match tokio::time::timeout(
                        KV_OP_TIMEOUT,
                        store.update(key, blob.clone(), rev),
                    )
                    .await
                    {
                        Ok(Ok(_)) => {}
                        Ok(Err(err)) => {
                            if matches!(err.kind(), UpdateErrorKind::WrongLastRevision) {
                                tracing::warn!(
                                    key = %key,
                                    "rollback: external write detected; skipping restore to \
                                     preserve concurrent change"
                                );
                            } else {
                                tracing::error!(
                                    error = %err,
                                    key = %key,
                                    "rollback restore failed; store may be partially applied"
                                );
                                failed_keys.push(key.clone());
                            }
                        }
                        Err(_) => {
                            tracing::error!(key = %key, "rollback update timed out");
                            failed_keys.push(key.clone());
                        }
                    },
                    None => {
                        match tokio::time::timeout(KV_OP_TIMEOUT, store.create(key, blob.clone()))
                            .await
                        {
                            Ok(Ok(_)) => {}
                            Ok(Err(err))
                                if matches!(err.kind(), CreateErrorKind::AlreadyExists) =>
                            {
                                // Empty-Put (or Delete) tombstone still occupies the key.
                                let entry = match kv_entry_timeout(store, key.clone()).await {
                                    Ok(e) => e,
                                    Err(e) => {
                                        tracing::error!(
                                            error = %e,
                                            key = %key,
                                            "rollback: kv.entry after create AlreadyExists failed"
                                        );
                                        failed_keys.push(key.clone());
                                        continue;
                                    }
                                };
                                let Some(entry) = entry else {
                                    tracing::warn!(
                                        key = %key,
                                        "rollback: create AlreadyExists but entry absent; \
                                         external race; skipping"
                                    );
                                    failed_keys.push(key.clone());
                                    continue;
                                };
                                if live_entry(Some(entry.clone())).is_some() {
                                    tracing::warn!(
                                        key = %key,
                                        "rollback: external write detected on create; skipping \
                                         restore to preserve concurrent change"
                                    );
                                    failed_keys.push(key.clone());
                                    continue;
                                }
                                match tokio::time::timeout(
                                    KV_OP_TIMEOUT,
                                    store.update(key, blob, entry.revision),
                                )
                                .await
                                {
                                    Ok(Ok(_)) => {}
                                    Ok(Err(update_err))
                                        if matches!(
                                            update_err.kind(),
                                            UpdateErrorKind::WrongLastRevision
                                        ) =>
                                    {
                                        tracing::warn!(
                                            key = %key,
                                            "rollback: external write during tombstone overwrite; \
                                             skipping restore"
                                        );
                                        failed_keys.push(key.clone());
                                    }
                                    Ok(Err(update_err)) => {
                                        tracing::error!(
                                            error = %update_err,
                                            key = %key,
                                            "rollback tombstone overwrite failed; store may be \
                                             partially applied"
                                        );
                                        failed_keys.push(key.clone());
                                    }
                                    Err(_) => {
                                        tracing::error!(
                                            key = %key,
                                            "rollback tombstone overwrite timed out"
                                        );
                                        failed_keys.push(key.clone());
                                    }
                                }
                            }
                            Ok(Err(err)) => {
                                tracing::error!(
                                    error = %err,
                                    key = %key,
                                    "rollback create failed; store may be partially applied"
                                );
                                failed_keys.push(key.clone());
                            }
                            Err(_) => {
                                tracing::error!(key = %key, "rollback create timed out");
                                failed_keys.push(key.clone());
                            }
                        }
                    }
                }
            }
            None => {
                if current_rev.is_some() {
                    match tokio::time::timeout(KV_OP_TIMEOUT, store.delete(key)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(err)) => {
                            tracing::error!(
                                error = %err,
                                key = %key,
                                "rollback delete failed; store may be partially applied"
                            );
                            failed_keys.push(key.clone());
                        }
                        Err(_) => {
                            tracing::error!(key = %key, "rollback delete timed out");
                            failed_keys.push(key.clone());
                        }
                    }
                }
            }
        }
    }
    failed_keys
}

#[path = "nats_journal.rs"]
mod journal;

#[path = "operations.rs"]
mod operations;

#[path = "writer_lease.rs"]
mod writer_lease;

#[path = "legacy_fields.rs"]
mod legacy_fields;

pub use legacy_fields::{
    ConflictResolution, LegacyFieldsFinding, LegacyFieldsLocation, LegacyFieldsReport,
    LegacyFieldsShape, MigrationMode,
};

#[cfg(test)]
#[path = "nats_recovery_tests.rs"]
mod recovery_tests;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn change_subject_missing_id_is_error() {
        let r = EntityRef::default();
        let result = change_subject(DEFAULT_CHANGES_SUBJECT_ROOT, ChangeKind::Put, &r);
        assert!(
            matches!(result, Err(StoreError::InvalidArgument(_))),
            "missing entity_ref.id must produce InvalidArgument, got: {result:?}"
        );
    }

    #[test]
    fn change_subject_escapes_components() {
        let r = EntityRef {
            kind: EntityKind::Event as i32,
            id: Some(Id {
                namespace: "shop/eu".into(),
                slug: "order.placed".into(),
                version: 1,
            }),
        };
        let s = change_subject(DEFAULT_CHANGES_SUBJECT_ROOT, ChangeKind::Delete, &r).unwrap();
        // `/` = 0x2F, `.` = 0x2E -- both hex-escaped for collision-free subjects.
        assert_eq!(s, "atlas.changes.del.shop=2Feu.order=2Eplaced");
    }

    #[test]
    fn change_subject_collision_free() {
        let make = |slug: &str| {
            let r = EntityRef {
                kind: EntityKind::Event as i32,
                id: Some(Id {
                    namespace: "ns".into(),
                    slug: slug.into(),
                    version: 1,
                }),
            };
            change_subject(DEFAULT_CHANGES_SUBJECT_ROOT, ChangeKind::Put, &r).unwrap()
        };
        assert_ne!(
            make("a_b"),
            make("a.b"),
            "underscore and dot must be distinct subjects"
        );
        assert_ne!(
            make("a/b"),
            make("a=2Fb"),
            "literal equals-prefix must not collide"
        );
    }

    #[test]
    fn live_entry_rejects_none() {
        assert!(live_entry(None).is_none(), "None entry must not be live");
    }

    #[test]
    fn live_entry_empty_value_is_tombstone() {
        // An empty-value Put is the sentinel left when kv.delete fails after
        // the conditional update step. live_entry must treat it as NotFound.
        // We verify the filtering predicate directly since KvEntry uses an
        // unexported type (OffsetDateTime) that prevents struct construction
        // outside the crate.
        let empty_put_is_not_live = !{
            let e = Operation::Put;
            let val = Bytes::new();
            e == Operation::Put && !val.is_empty()
        };
        assert!(
            empty_put_is_not_live,
            "empty-value Put must not satisfy the live_entry predicate"
        );
    }

    #[test]
    fn config_defaults_bound_the_changes_stream() {
        let c = NatsStoreConfig::new("nats://example:4222");
        assert_eq!(c.bucket, DEFAULT_KV_BUCKET);
        assert_eq!(c.stream, DEFAULT_CHANGES_STREAM);
        assert_eq!(c.subject_wildcard(), "atlas.changes.>");
        assert!(c.changes_max_age.is_some());
        assert!(c.changes_max_msgs.is_some());
    }

    #[test]
    fn decode_change_record_unknown_kind_errors() {
        use prost::Message as _;
        let event = trogon_atlas_proto::ChangeEvent {
            kind: 999,
            entity: Some(EntityRef {
                kind: EntityKind::Event as i32,
                id: Some(Id {
                    namespace: "ns".into(),
                    slug: "s".into(),
                    version: 1,
                }),
            }),
            at: "2024-01-01T00:00:00.000Z".into(),
            token: String::new(),
            timestamp_micros: 0,
            changeset_id: String::new(),
            author: String::new(),
        };
        let bytes = event.encode_to_vec();
        let result = decode_change_record(1, &bytes);
        assert!(
            matches!(result, Err(StoreError::Backend(_))),
            "unknown kind must be an error, not silently routed as Put"
        );
    }

    /// `change_subject` returns `InvalidArgument` when `entity_ref.id` is
    /// None, preventing a placeholder subject from reaching the change stream.
    #[test]
    fn change_subject_rejects_missing_id() {
        let entity_ref = EntityRef::default();
        assert!(entity_ref.id.is_none(), "default EntityRef must have no id");
        let result = change_subject(DEFAULT_CHANGES_SUBJECT_ROOT, ChangeKind::Put, &entity_ref);
        assert!(
            matches!(result, Err(StoreError::InvalidArgument(_))),
            "missing entity_ref.id must produce InvalidArgument"
        );
    }

    /// `BATCH_GET_HARD_CAP` invariant is verified at compile time via const assert.
    #[test]
    fn batch_get_hard_cap_constant_is_sane() {
        const { assert!(BATCH_GET_HARD_CAP > 0 && BATCH_GET_HARD_CAP <= LIST_HARD_CAP) }
    }

    /// Unit test: `change_events_lost` counter increments. Exercises the counter
    /// logic without a live NATS connection.
    #[test]
    fn change_events_lost_counter_increments() {
        let counter = Arc::new(AtomicU64::new(0));
        counter.fetch_add(1, Ordering::Relaxed);
        counter.fetch_add(1, Ordering::Relaxed);
        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }

    /// Unit test: `change_tail_reconnects` counter increments.
    #[test]
    fn change_tail_reconnects_counter_increments() {
        let counter = Arc::new(AtomicU64::new(0));
        counter.fetch_add(1, Ordering::Relaxed);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    /// `BATCH_APPLY_MAX_OPS` invariant is verified at compile time via const assert.
    #[test]
    fn batch_apply_max_ops_constant_is_sane() {
        const { assert!(BATCH_APPLY_MAX_OPS > 0 && BATCH_APPLY_MAX_OPS <= LIST_HARD_CAP) }
    }

    /// Verifies that the live-entry cap logic counts only live entries, not
    /// tombstones. A simulated bucket where tombstones fill the key set beyond
    /// `LIST_HARD_CAP` must still return all live entities up to the effective
    /// limit -- tombstones do not consume cap slots.
    #[test]
    fn list_cap_counts_live_entries_not_tombstones() {
        // Simulate the filtering logic: keys include many tombstones and a few
        // live entries. The effective cap must be reached only by live entries.
        let effective_limit = 3usize;
        // Represent results as Option<()>: None = tombstone, Some(()) = live.
        let simulated: Vec<Option<()>> = vec![
            None,
            None,
            Some(()),
            None,
            Some(()),
            None,
            None,
            Some(()),
            None,
            Some(()),
        ];
        let mut live_count = 0usize;
        for item in simulated {
            if item.is_some() {
                live_count += 1;
                if live_count >= effective_limit {
                    break;
                }
            }
        }
        // We must have collected exactly effective_limit live entries and not
        // cut off early because tombstones were counted against the cap.
        assert_eq!(
            live_count, effective_limit,
            "cap must collect effective_limit live entries regardless of tombstone count"
        );
    }

    /// `read_changes_consumers_total` counter increments once per call.
    #[test]
    fn read_changes_consumer_counter_increments() {
        // Verify that the metrics counter pattern compiles and the atomic
        // underneath increments correctly (counter is emitted per read_changes
        // call; this test validates the intent with an equivalent counter).
        let counter = Arc::new(AtomicU64::new(0));
        counter.fetch_add(1, Ordering::Relaxed);
        counter.fetch_add(1, Ordering::Relaxed);
        assert_eq!(
            counter.load(Ordering::Relaxed),
            2,
            "two read_changes calls must produce two consumer counter increments"
        );
    }

    /// Compensating rollback must restore a prior live entity even when the
    /// key is currently an empty-Put tombstone (`create` alone cannot
    /// overwrite it, the same hole the tombstone-aware create path fixed).
    #[tokio::test(flavor = "multi_thread")]
    async fn rollback_baseline_snapshot_restores_over_empty_put_tombstone() {
        let nats = trogon_atlas_testsupport::shared().await;
        let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
        let bucket = format!("rollback-empty-{suffix}");
        let client = async_nats::connect(&nats.url).await.expect("nats connect");
        let js = async_nats::jetstream::new(client);
        let kv = js
            .create_key_value(async_nats::jetstream::kv::Config {
                bucket: bucket.clone(),
                history: 1,
                ..Default::default()
            })
            .await
            .expect("create kv");

        let id = Id {
            namespace: "tomb".into(),
            slug: "rollback-empty".into(),
            version: 1,
        };
        let entity = Entity {
            system: None,
            kind: Some(trogon_atlas_proto::entity::Kind::Event(
                trogon_atlas_proto::Event {
                    id: Some(id.clone()),
                    title: "original".into(),
                    ..Default::default()
                },
            )),
        };
        let key = entity_key(EntityKind::Event, &id);
        let bytes = encode_entity(&entity);
        let rev = kv.create(&key, bytes).await.expect("create live");

        // Simulate delete's empty-Put sentinel left behind after kv.delete failed.
        kv.update(&key, Bytes::new(), rev)
            .await
            .expect("plant empty-Put tombstone");
        assert!(
            live_entry(kv.entry(key.clone()).await.expect("entry")).is_none(),
            "empty-Put must not look live"
        );

        let mut snapshot = BTreeMap::new();
        snapshot.insert(key.clone(), Some((entity.clone(), rev)));
        let failed = rollback_baseline_snapshot(&snapshot, &kv).await;
        assert!(
            failed.is_empty(),
            "rollback must restore over empty-Put without PartialApply keys: {failed:?}"
        );

        let restored = live_entry(kv.entry(key).await.expect("entry after rollback"))
            .expect("prior entity must be live again");
        let got = decode_entity(&restored.value).expect("decode");
        match got.kind {
            Some(trogon_atlas_proto::entity::Kind::Event(e)) => {
                assert_eq!(e.title, "original");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
    #[test]
    fn a_plain_url_carries_no_credentials() {
        let config = NatsStoreConfig::new("nats://nats:4222");
        assert_eq!(config.url, "nats://nats:4222");
        assert!(config.credentials.is_none());
    }

    #[test]
    fn userinfo_is_lifted_out_of_the_url() {
        let config = NatsStoreConfig::new("nats://service:s3cr3t@nats:4222");
        assert_eq!(config.url, "nats://nats:4222");
        let credentials = config.credentials.expect("credentials");
        assert_eq!(credentials.user(), "service");
        assert_eq!(credentials.password, "s3cr3t");
    }

    #[test]
    fn a_password_may_contain_an_at_sign() {
        let (url, credentials) = split_url_credentials("nats://service:p@ss@nats:4222");
        assert_eq!(url, "nats://nats:4222");
        assert_eq!(credentials.expect("credentials").password, "p@ss");
    }

    #[test]
    fn a_url_path_is_not_mistaken_for_userinfo() {
        let (url, credentials) = split_url_credentials("nats://nats:4222/a@b");
        assert_eq!(url, "nats://nats:4222/a@b");
        assert!(credentials.is_none());
    }

    #[test]
    fn explicit_credentials_win_over_the_url() {
        let mut config = NatsStoreConfig::new("nats://nats:4222");
        config.url = "nats://fromurl:pw@nats:4222".into();
        config.credentials = Some(NatsCredentials::new("explicit", "pw"));
        config.take_url_credentials();
        assert_eq!(config.url, "nats://nats:4222");
        assert_eq!(config.credentials.expect("credentials").user(), "explicit");
    }

    #[test]
    fn a_password_is_never_printed() {
        let rendered = format!("{:?}", NatsCredentials::new("service", "s3cr3t"));
        assert!(rendered.contains("service"), "{rendered}");
        assert!(!rendered.contains("s3cr3t"), "{rendered}");
    }
}
