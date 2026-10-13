//! Metric instruments the store records, generated from `otel/registry` into
//! `trogon-atlas-semconv`.

use std::{fmt, sync::Arc};

use trogon_atlas_semconv::semconv::{meter::Meter, metric};

/// The store's instruments, bound to the [`Meter`] they were built from.
/// Cloning shares the instruments. [`Default`] records nothing.
#[derive(Clone)]
pub struct StoreMetrics(Arc<Instruments>);

struct Instruments {
    batches_pending_recovery: metric::StoreBatchesPendingRecovery,
    change_events_lost: metric::StoreChangeEventsLost,
    change_tail_reconnects: metric::StoreChangeTailReconnects,
    nats_disconnects: metric::StoreNatsDisconnects,
    nats_reconnects: metric::StoreNatsReconnects,
    read_changes_consumers: metric::StoreReadChangesConsumers,
    revisions_lost: metric::StoreRevisionsLost,
}

impl StoreMetrics {
    pub fn new(meter: &Meter) -> Self {
        Self(Arc::new(Instruments {
            batches_pending_recovery: metric::StoreBatchesPendingRecovery::new(meter),
            change_events_lost: metric::StoreChangeEventsLost::new(meter),
            change_tail_reconnects: metric::StoreChangeTailReconnects::new(meter),
            nats_disconnects: metric::StoreNatsDisconnects::new(meter),
            nats_reconnects: metric::StoreNatsReconnects::new(meter),
            read_changes_consumers: metric::StoreReadChangesConsumers::new(meter),
            revisions_lost: metric::StoreRevisionsLost::new(meter),
        }))
    }

    pub fn record_batch_pending_recovery(&self) {
        self.0.batches_pending_recovery.add(1);
    }

    pub fn record_change_event_lost(&self) {
        self.0.change_events_lost.add(1);
    }

    pub fn record_change_tail_reconnect(&self) {
        self.0.change_tail_reconnects.add(1);
    }

    pub fn record_nats_disconnect(&self) {
        self.0.nats_disconnects.add(1);
    }

    pub fn record_nats_reconnect(&self) {
        self.0.nats_reconnects.add(1);
    }

    pub fn record_read_changes_consumer(&self) {
        self.0.read_changes_consumers.add(1);
    }

    pub fn record_revision_lost(&self) {
        self.0.revisions_lost.add(1);
    }
}

impl Default for StoreMetrics {
    fn default() -> Self {
        Self::new(&Meter::default())
    }
}

impl fmt::Debug for StoreMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreMetrics").finish_non_exhaustive()
    }
}
