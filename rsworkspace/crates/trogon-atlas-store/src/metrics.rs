//! Process-wide metric instruments, generated from `otel/registry` into
//! `trogon-atlas-semconv`.
//!
//! Each instrument binds to the global `MeterProvider` the first time it is
//! touched, and OpenTelemetry never rebinds it, so the binary must install
//! its provider before using a store.

use std::sync::LazyLock;

use trogon_atlas_semconv::{
    semconv::{meter::Meter, metric, options::InstrumentationOptions},
    GlobalMeterProvider,
};

static METER: LazyLock<Meter> =
    LazyLock::new(|| Meter::new(&GlobalMeterProvider, InstrumentationOptions::default()));

pub static BATCHES_PENDING_RECOVERY: LazyLock<metric::StoreBatchesPendingRecovery> =
    LazyLock::new(|| metric::StoreBatchesPendingRecovery::new(&METER));
pub static CHANGE_EVENTS_LOST: LazyLock<metric::StoreChangeEventsLost> =
    LazyLock::new(|| metric::StoreChangeEventsLost::new(&METER));
pub static CHANGE_TAIL_RECONNECTS: LazyLock<metric::StoreChangeTailReconnects> =
    LazyLock::new(|| metric::StoreChangeTailReconnects::new(&METER));
pub static NATS_DISCONNECTS: LazyLock<metric::StoreNatsDisconnects> =
    LazyLock::new(|| metric::StoreNatsDisconnects::new(&METER));
pub static NATS_RECONNECTS: LazyLock<metric::StoreNatsReconnects> =
    LazyLock::new(|| metric::StoreNatsReconnects::new(&METER));
pub static READ_CHANGES_CONSUMERS: LazyLock<metric::StoreReadChangesConsumers> =
    LazyLock::new(|| metric::StoreReadChangesConsumers::new(&METER));
pub static REVISIONS_LOST: LazyLock<metric::StoreRevisionsLost> =
    LazyLock::new(|| metric::StoreRevisionsLost::new(&METER));
