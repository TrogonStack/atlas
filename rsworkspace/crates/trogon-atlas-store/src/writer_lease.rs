//! The writer lease: a NATS KV record naming the one process allowed to
//! mutate, and the monotonically increasing epoch it was handed. See
//! `docs/explanation/single-writer.md` for the full design.
//!
//! A process configured [`WriterRole::Writer`] or [`WriterRole::Standby`]
//! runs a background loop (spawned in `connect_with_unboxed`) that tries,
//! once per `lease_renew_interval`, to create, renew, or (once expired)
//! take over the single lease row. [`WriterRole::Reader`] never writes to
//! it; it only ever reads the row to report status. Both intervals come
//! from [`crate::nats::NatsStoreConfig`] (defaulting to
//! [`DEFAULT_LEASE_DURATION`] and [`DEFAULT_LEASE_RENEW_INTERVAL`]), so an
//! integration test can shrink them instead of waiting out the production
//! lease window.
//!
//! The in-memory [`WriterLease`] is what [`crate::store::Store::is_writer`]
//! actually consults. It is believed current only while both are true: no
//! attempt since the last successful create/renew/take over proved the
//! lease lost, and that success happened within the lease duration of now.
//! A renewal that fails without losing a CAS race (a timeout, a dropped
//! connection) proves nothing, so it leaves the belief to the second check. The second check is a local
//! defense against a renewal loop that has stalled (e.g. a GC pause):
//! without it, a process that silently stopped renewing would otherwise
//! keep believing it is the writer indefinitely.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use async_nats::jetstream::{kv::Store as KvStore, response::Response, Context};
use bytes::Bytes;
use prost::Message as _;
use tokio::sync::Notify;
use trogon_atlas_core::{Epoch, WriterRole};

use super::{kv_entry_timeout, live_entry, KV_OP_TIMEOUT};
use crate::{
    error::{StoreError, StoreResult},
    store::WriterStatus,
};

/// How long a lease holds before it is eligible for takeover, absent an
/// explicit [`crate::nats::NatsStoreConfig::lease_duration`].
pub(crate) const DEFAULT_LEASE_DURATION: Duration = Duration::from_secs(15);

/// How often the lease loop tries to create, renew, or take over the
/// lease, absent an explicit
/// [`crate::nats::NatsStoreConfig::lease_renew_interval`]. A third of
/// [`DEFAULT_LEASE_DURATION`], so a holder gets several renewal attempts
/// before its lease is eligible for takeover.
pub(crate) const DEFAULT_LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(5);

/// Artifact publication receipts use separate immutable keys in this same stream.
const LEASE_KEY: &str = "writer-lease";

/// On-disk encoding of the lease row in the `trogon-atlas-writer-lease` KV
/// bucket. Hand-rolled and private, same rationale as `JournalHeader` in
/// `nats_journal.rs` and `OperationRow` in `operations.rs`: this is an
/// internal record, never part of the client-facing contract.
#[derive(Clone, PartialEq, ::prost::Message)]
struct LeaseRow {
    #[prost(string, tag = "1")]
    holder_id: String,
    #[prost(uint64, tag = "2")]
    epoch: u64,
    #[prost(int64, tag = "3")]
    expires_at_micros: i64,
}

/// In-memory writer-lease state for one `NatsStore`. Cheap to read from
/// every mutating method: no lock is held across an `await`.
pub(crate) struct WriterLease {
    /// Identifies this process's attempts to claim the lease. Stable for
    /// the process's lifetime; never reused across a restart.
    holder_id: String,
    configured_role: WriterRole,
    /// How long a lease holds before it is eligible for takeover. Must
    /// match every other process sharing this lease's bucket.
    lease_duration: Duration,
    holding: AtomicBool,
    epoch: AtomicU64,
    last_renewed_at: parking_lot::Mutex<Option<Instant>>,
    last_known_holder: parking_lot::Mutex<String>,
}

impl WriterLease {
    pub(super) fn publication_identity(&self) -> StoreResult<(String, Epoch)> {
        if !self.is_writer() {
            return Err(StoreError::NotWriter {
                role: self.status().role,
                epoch: self.current_epoch(),
            });
        }
        Ok((self.holder_id.clone(), self.current_epoch()))
    }

    pub(super) async fn publication_snapshot(
        &self,
        context: &Context,
        stream: &async_nats::jetstream::stream::Stream,
        subject: &str,
        holder: &str,
        epoch: Epoch,
    ) -> StoreResult<PublicationLease> {
        // Direct KV reads may lag takeover on a follower, even after fresh stream info.
        let message = tokio::time::timeout(
            KV_OP_TIMEOUT,
            stream.get_last_raw_message_by_subject(subject),
        )
        .await
        .map_err(|_| StoreError::Unavailable("artifact writer lease read timed out".into()))?
        .map_err(|_| StoreError::Unavailable("artifact writer lease unavailable".into()))?;
        let row = LeaseRow::decode(message.payload)
            .map_err(|_| StoreError::Unavailable("invalid artifact writer lease".into()))?;
        let now = server_now_micros(context, &stream.cached_info().config.name).await?;
        if message.subject.as_str() != subject
            || holder != self.holder_id
            || row.holder_id != holder
            || row.epoch != epoch.get()
            || row.expires_at_micros <= now
        {
            return Err(StoreError::NotWriter {
                role: self.status().role,
                epoch,
            });
        }
        Ok(PublicationLease {
            revision: message.sequence,
            expires_at_micros: row.expires_at_micros,
        })
    }

    pub(crate) fn new(configured_role: WriterRole, lease_duration: Duration) -> Self {
        Self {
            holder_id: uuid::Uuid::now_v7().to_string(),
            configured_role,
            lease_duration,
            holding: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            last_renewed_at: parking_lot::Mutex::new(None),
            last_known_holder: parking_lot::Mutex::new(String::new()),
        }
    }

    fn expires_after(&self, now: i64) -> StoreResult<i64> {
        let duration = i64::try_from(self.lease_duration.as_micros()).map_err(|_| {
            StoreError::Unavailable("writer lease duration exceeds clock range".into())
        })?;
        now.checked_add(duration)
            .filter(|expiry| *expiry > now)
            .ok_or_else(|| {
                StoreError::Unavailable("writer lease expiry exceeds clock range".into())
            })
    }

    /// Whether this process is believed to hold the lease right now. See
    /// the module doc for why both conditions are required.
    pub(crate) fn is_writer(&self) -> bool {
        if !self.configured_role.participates_in_lease() {
            return false;
        }
        if !self.holding.load(Ordering::Acquire) {
            return false;
        }
        matches!(*self.last_renewed_at.lock(), Some(renewed_at) if renewed_at.elapsed() < self.lease_duration)
    }

    pub(super) async fn verify_epoch(
        &self,
        context: &Context,
        kv: &KvStore,
        expected: Epoch,
    ) -> StoreResult<()> {
        let entry = live_entry(kv_entry_timeout(kv, LEASE_KEY.into()).await?)
            .ok_or_else(|| StoreError::Unavailable("writer lease unavailable".into()))?;
        let row = LeaseRow::decode(entry.value.as_ref())
            .map_err(|_| StoreError::Unavailable("writer lease unavailable".into()))?;
        let now = server_now_micros(context, &format!("KV_{}", kv.name)).await?;
        if row.epoch != expected.get()
            || row.holder_id != self.holder_id
            || row.expires_at_micros <= now
            || !self.is_writer()
        {
            return Err(StoreError::NotWriter {
                role: self.status().role,
                epoch: expected,
            });
        }
        Ok(())
    }

    pub(crate) fn current_epoch(&self) -> Epoch {
        Epoch::new(self.epoch.load(Ordering::Acquire))
    }

    pub(crate) fn status(&self) -> WriterStatus {
        let role = if self.is_writer() {
            WriterRole::Writer
        } else if self.configured_role.participates_in_lease() {
            WriterRole::Standby
        } else {
            WriterRole::Reader
        };
        WriterStatus {
            role,
            epoch: self.current_epoch(),
            lease_holder: self.last_known_holder.lock().clone(),
        }
    }

    fn record_acquired(&self, epoch: u64, holder_id: String, acquired_at: Instant) {
        self.epoch.store(epoch, Ordering::Release);
        self.holding.store(true, Ordering::Release);
        *self.last_renewed_at.lock() = Some(acquired_at);
        *self.last_known_holder.lock() = holder_id;
    }

    /// We are not (or no longer) the holder. Called both when another
    /// process holds an unexpired lease and when our own create/renew/
    /// takeover attempt lost a CAS race.
    fn record_not_holding(&self) {
        self.holding.store(false, Ordering::Release);
    }

    /// Our renewal of a row we hold failed. Only a lost CAS race proves
    /// another process rewrote the row. Any other failure (a timeout, a
    /// dropped connection) leaves the outcome unknown, so the belief stays
    /// bounded by the staleness check in [`Self::is_writer`] rather than
    /// refusing every mutation until the next tick.
    fn record_renewal_failed(&self, lost_race: bool) {
        if lost_race {
            self.record_not_holding();
        }
    }

    fn observe(&self, holder_id: String, epoch: u64) {
        self.epoch.store(epoch, Ordering::Release);
        *self.last_known_holder.lock() = holder_id;
    }
}

pub(super) struct PublicationLease {
    pub(super) revision: u64,
    pub(super) expires_at_micros: i64,
}

async fn server_now_micros(context: &Context, stream: &str) -> StoreResult<i64> {
    // Lease deadlines and receipt timestamps share the NATS leader clock.
    // NATS node clocks must remain synchronized across leader changes.
    let response: Response<serde_json::Value> = tokio::time::timeout(
        KV_OP_TIMEOUT,
        context.request(format!("STREAM.INFO.{stream}"), &()),
    )
    .await
    .map_err(|_| clock_unavailable())?
    .map_err(|_| clock_unavailable())?;
    let Response::Ok(info) = response else {
        return Err(clock_unavailable());
    };
    let timestamp = info
        .get("ts")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(clock_unavailable)?;
    let now = chrono::DateTime::parse_from_rfc3339(timestamp)
        .map_err(|_| clock_unavailable())?
        .timestamp_micros();
    if now <= 0 {
        return Err(clock_unavailable());
    }
    Ok(now)
}

fn clock_unavailable() -> StoreError {
    StoreError::Unavailable("writer lease server clock unavailable".into())
}

/// Background loop driving one [`WriterLease`]. Runs for the lifetime of
/// the `NatsStore` (aborted by [`WriterLeaseHandle`]'s `Drop`); never
/// returns on its own.
pub(crate) async fn run_writer_lease(
    context: Context,
    lease_kv: KvStore,
    lease: Arc<WriterLease>,
    ready: Arc<Notify>,
    renew_interval: Duration,
) {
    let mut notified = false;
    loop {
        if let Err(err) = tick(&context, &lease_kv, &lease).await {
            tracing::warn!(
                error = %err,
                "writer lease tick failed; will retry next interval"
            );
        }
        if !notified {
            ready.notify_one();
            notified = true;
        }
        tokio::time::sleep(renew_interval).await;
    }
}

async fn tick(context: &Context, lease_kv: &KvStore, lease: &WriterLease) -> StoreResult<()> {
    let entry = live_entry(kv_entry_timeout(lease_kv, LEASE_KEY.to_owned()).await?);
    let entry = match entry {
        Some(entry) => {
            let row = LeaseRow::decode(entry.value.as_ref())
                .map_err(|e| StoreError::Backend(format!("decode writer lease: {e}")))?;
            lease.observe(row.holder_id.clone(), row.epoch);
            if row.holder_id != lease.holder_id {
                lease.record_not_holding();
            }
            Some((entry, row))
        }
        None => None,
    };

    if !lease.configured_role.participates_in_lease() {
        return Ok(());
    }

    let acquired_at = Instant::now();
    let now = server_now_micros(context, &format!("KV_{}", lease_kv.name)).await?;
    let expires_at_micros = lease.expires_after(now)?;
    match entry {
        None => {
            let row = LeaseRow {
                holder_id: lease.holder_id.clone(),
                epoch: 1,
                expires_at_micros,
            };
            let bytes = Bytes::from(row.encode_to_vec());
            match tokio::time::timeout(KV_OP_TIMEOUT, lease_kv.create(LEASE_KEY, bytes))
                .await
                .map_err(|_| StoreError::Backend("writer lease create: timeout".into()))?
            {
                Ok(_) => lease.record_acquired(1, lease.holder_id.clone(), acquired_at),
                Err(e) => {
                    use async_nats::jetstream::kv::CreateErrorKind;
                    if !matches!(e.kind(), CreateErrorKind::AlreadyExists) {
                        return Err(StoreError::Backend(format!("writer lease create: {e}")));
                    }
                    // Lost the race to create it; the next tick reads
                    // whoever won.
                }
            }
        }
        Some((entry, row)) => {
            if row.holder_id == lease.holder_id {
                let renewed = LeaseRow {
                    expires_at_micros,
                    ..row
                };
                let bytes = Bytes::from(renewed.encode_to_vec());
                match tokio::time::timeout(
                    KV_OP_TIMEOUT,
                    lease_kv.update(LEASE_KEY, bytes, entry.revision),
                )
                .await
                .map_err(|_| StoreError::Backend("writer lease renew: timeout".into()))?
                {
                    Ok(_) => lease.record_acquired(renewed.epoch, renewed.holder_id, acquired_at),
                    Err(e) => {
                        use async_nats::jetstream::kv::UpdateErrorKind;
                        let lost_race = matches!(e.kind(), UpdateErrorKind::WrongLastRevision);
                        lease.record_renewal_failed(lost_race);
                        if !lost_race {
                            return Err(StoreError::Backend(format!("writer lease renew: {e}")));
                        }
                    }
                }
                return Ok(());
            }

            if row.expires_at_micros > now {
                lease.record_not_holding();
                return Ok(());
            }

            let next_epoch = row.epoch + 1;
            let taken = LeaseRow {
                holder_id: lease.holder_id.clone(),
                epoch: next_epoch,
                expires_at_micros,
            };
            let bytes = Bytes::from(taken.encode_to_vec());
            match tokio::time::timeout(
                KV_OP_TIMEOUT,
                lease_kv.update(LEASE_KEY, bytes, entry.revision),
            )
            .await
            .map_err(|_| StoreError::Backend("writer lease takeover: timeout".into()))?
            {
                Ok(_) => lease.record_acquired(next_epoch, lease.holder_id.clone(), acquired_at),
                Err(e) => {
                    lease.record_not_holding();
                    use async_nats::jetstream::kv::UpdateErrorKind;
                    if !matches!(e.kind(), UpdateErrorKind::WrongLastRevision) {
                        return Err(StoreError::Backend(format!("writer lease takeover: {e}")));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Wrapper around the writer-lease loop's `JoinHandle`. `Drop` aborts it,
/// mirroring `ChangeTailHandle` in `nats.rs`.
pub(crate) struct WriterLeaseHandle(parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>);

impl WriterLeaseHandle {
    pub(crate) fn new(handle: tokio::task::JoinHandle<()>) -> Self {
        Self(parking_lot::Mutex::new(Some(handle)))
    }
}

impl Drop for WriterLeaseHandle {
    fn drop(&mut self) {
        if let Some(handle) = self.0.lock().take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn fresh_lease_is_not_the_writer() {
        let lease = WriterLease::new(WriterRole::Writer, DEFAULT_LEASE_DURATION);
        assert!(!lease.is_writer());
        assert_eq!(lease.status().role, WriterRole::Standby);
    }

    #[test]
    fn reader_never_reports_as_writer_even_once_marked_acquired() {
        let lease = WriterLease::new(WriterRole::Reader, DEFAULT_LEASE_DURATION);
        lease.record_acquired(1, lease.holder_id.clone(), Instant::now());
        assert!(!lease.is_writer());
        assert_eq!(lease.status().role, WriterRole::Reader);
    }

    #[test]
    fn holding_without_a_recent_renewal_is_not_the_writer() {
        let lease = WriterLease::new(WriterRole::Writer, DEFAULT_LEASE_DURATION);
        lease.holding.store(true, Ordering::Release);
        *lease.last_renewed_at.lock() = Some(
            Instant::now()
                .checked_sub(DEFAULT_LEASE_DURATION * 2)
                .unwrap(),
        );
        assert!(!lease.is_writer());
    }

    #[test]
    fn acquired_lease_reports_writer_with_its_epoch() {
        let lease = WriterLease::new(WriterRole::Standby, DEFAULT_LEASE_DURATION);
        lease.record_acquired(3, "holder-a".to_owned(), Instant::now());
        assert!(lease.is_writer());
        let status = lease.status();
        assert_eq!(status.role, WriterRole::Writer);
        assert_eq!(status.epoch, Epoch::new(3));
        assert_eq!(status.lease_holder, "holder-a");
    }

    #[test]
    fn a_renewal_failure_that_lost_no_race_keeps_the_writer_role() {
        let lease = WriterLease::new(WriterRole::Writer, DEFAULT_LEASE_DURATION);
        lease.record_acquired(1, lease.holder_id.clone(), Instant::now());
        lease.record_renewal_failed(false);
        assert!(lease.is_writer());
    }

    #[test]
    fn a_renewal_that_lost_the_race_stops_reporting_as_writer() {
        let lease = WriterLease::new(WriterRole::Writer, DEFAULT_LEASE_DURATION);
        lease.record_acquired(1, lease.holder_id.clone(), Instant::now());
        lease.record_renewal_failed(true);
        assert!(!lease.is_writer());
    }

    #[test]
    fn losing_the_lease_stops_reporting_as_writer() {
        let lease = WriterLease::new(WriterRole::Writer, DEFAULT_LEASE_DURATION);
        lease.record_acquired(1, lease.holder_id.clone(), Instant::now());
        assert!(lease.is_writer());
        lease.record_not_holding();
        assert!(!lease.is_writer());
    }
}
