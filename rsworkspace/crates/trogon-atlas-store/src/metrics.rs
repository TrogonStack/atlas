//! Process-wide metric instruments, generated from `otel/registry` into
//! `trogon-atlas-semconv`.
//!
//! Each instrument binds to the global `MeterProvider` the first time it is
//! touched, and OpenTelemetry never rebinds it, so the binary must install
//! its provider before using a store.

use std::sync::LazyLock;

use trogon_atlas_semconv::{
    semconv::{meter::Meter, options::InstrumentationOptions, storemetric},
    GlobalMeterProvider,
};

static METER: LazyLock<Meter> =
    LazyLock::new(|| Meter::new(&GlobalMeterProvider, InstrumentationOptions::default()));

pub static BATCHES_PENDING_RECOVERY: LazyLock<storemetric::BatchesPendingRecoveryCounter> =
    LazyLock::new(|| storemetric::BatchesPendingRecoveryCounter::new(&METER));
pub static CHANGE_EVENTS_LOST: LazyLock<storemetric::ChangeEventsLostCounter> =
    LazyLock::new(|| storemetric::ChangeEventsLostCounter::new(&METER));
pub static CHANGE_TAIL_RECONNECTS: LazyLock<storemetric::ChangeTailReconnectsCounter> =
    LazyLock::new(|| storemetric::ChangeTailReconnectsCounter::new(&METER));
pub static NATS_DISCONNECTS: LazyLock<storemetric::NatsDisconnectsCounter> =
    LazyLock::new(|| storemetric::NatsDisconnectsCounter::new(&METER));
pub static NATS_RECONNECTS: LazyLock<storemetric::NatsReconnectsCounter> =
    LazyLock::new(|| storemetric::NatsReconnectsCounter::new(&METER));
pub static READ_CHANGES_CONSUMERS: LazyLock<storemetric::ReadChangesConsumersCounter> =
    LazyLock::new(|| storemetric::ReadChangesConsumersCounter::new(&METER));
pub static REVISIONS_LOST: LazyLock<storemetric::RevisionsLostCounter> =
    LazyLock::new(|| storemetric::RevisionsLostCounter::new(&METER));
