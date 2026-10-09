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
//! actually consults. It is believed current only while both are true: the
//! last attempt to create/renew/take over succeeded, and that success
//! happened within the lease duration of now. The second check is a local
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

use async_nats::jetstream::kv::Store as KvStore;
use bytes::Bytes;
use prost::Message as _;
use tokio::sync::Notify;
use trogon_atlas_core::{Epoch, WriterRole};

use super::{kv_entry_timeout, live_entry, now_micros, KV_OP_TIMEOUT};
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

/// The single key every lease row lives under. One bucket, one lease: no
/// second key is ever written here.
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

    fn lease_duration_micros(&self) -> i64 {
        i64::try_from(self.lease_duration.as_micros()).unwrap_or(i64::MAX)
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

    fn record_acquired(&self, epoch: u64, holder_id: String) {
        self.epoch.store(epoch, Ordering::Release);
        self.holding.store(true, Ordering::Release);
        *self.last_renewed_at.lock() = Some(Instant::now());
        *self.last_known_holder.lock() = holder_id;
    }

    /// We are not (or no longer) the holder. Called both when another
    /// process holds an unexpired lease and when our own create/renew/
    /// takeover attempt lost a CAS race.
    fn record_not_holding(&self) {
        self.holding.store(false, Ordering::Release);
    }

    fn observe(&self, holder_id: String, epoch: u64) {
        self.epoch.store(epoch, Ordering::Release);
        *self.last_known_holder.lock() = holder_id;
    }
}

/// Background loop driving one [`WriterLease`]. Runs for the lifetime of
/// the `NatsStore` (aborted by [`WriterLeaseHandle`]'s `Drop`); never
/// returns on its own.
pub(crate) async fn run_writer_lease(
    lease_kv: KvStore,
    lease: Arc<WriterLease>,
    ready: Arc<Notify>,
    renew_interval: Duration,
) {
    let mut notified = false;
    loop {
        if let Err(err) = tick(&lease_kv, &lease).await {
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

async fn tick(lease_kv: &KvStore, lease: &WriterLease) -> StoreResult<()> {
    let entry = live_entry(kv_entry_timeout(lease_kv, LEASE_KEY.to_owned()).await?);

    if !lease.configured_role.participates_in_lease() {
        if let Some(entry) = entry {
            let row = LeaseRow::decode(entry.value.as_ref())
                .map_err(|e| StoreError::Backend(format!("decode writer lease: {e}")))?;
            lease.observe(row.holder_id, row.epoch);
        }
        return Ok(());
    }

    let now = now_micros();
    match entry {
        None => {
            let row = LeaseRow {
                holder_id: lease.holder_id.clone(),
                epoch: 1,
                expires_at_micros: now + lease.lease_duration_micros(),
            };
            let bytes = Bytes::from(row.encode_to_vec());
            match tokio::time::timeout(KV_OP_TIMEOUT, lease_kv.create(LEASE_KEY, bytes))
                .await
                .map_err(|_| StoreError::Backend("writer lease create: timeout".into()))?
            {
                Ok(_) => lease.record_acquired(1, lease.holder_id.clone()),
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
        Some(entry) => {
            let row = LeaseRow::decode(entry.value.as_ref())
                .map_err(|e| StoreError::Backend(format!("decode writer lease: {e}")))?;
            lease.observe(row.holder_id.clone(), row.epoch);

            if row.holder_id == lease.holder_id {
                let renewed = LeaseRow {
                    expires_at_micros: now + lease.lease_duration_micros(),
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
                    Ok(_) => lease.record_acquired(renewed.epoch, renewed.holder_id),
                    Err(e) => {
                        lease.record_not_holding();
                        use async_nats::jetstream::kv::UpdateErrorKind;
                        if !matches!(e.kind(), UpdateErrorKind::WrongLastRevision) {
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
                expires_at_micros: now + lease.lease_duration_micros(),
            };
            let bytes = Bytes::from(taken.encode_to_vec());
            match tokio::time::timeout(
                KV_OP_TIMEOUT,
                lease_kv.update(LEASE_KEY, bytes, entry.revision),
            )
            .await
            .map_err(|_| StoreError::Backend("writer lease takeover: timeout".into()))?
            {
                Ok(_) => lease.record_acquired(next_epoch, lease.holder_id.clone()),
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
        lease.record_acquired(1, lease.holder_id.clone());
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
        lease.record_acquired(3, "holder-a".to_owned());
        assert!(lease.is_writer());
        let status = lease.status();
        assert_eq!(status.role, WriterRole::Writer);
        assert_eq!(status.epoch, Epoch::new(3));
        assert_eq!(status.lease_holder, "holder-a");
    }

    #[test]
    fn losing_the_lease_stops_reporting_as_writer() {
        let lease = WriterLease::new(WriterRole::Writer, DEFAULT_LEASE_DURATION);
        lease.record_acquired(1, lease.holder_id.clone());
        assert!(lease.is_writer());
        lease.record_not_holding();
        assert!(!lease.is_writer());
    }
}
