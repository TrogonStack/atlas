//! Process-wide metric instruments, generated from `otel/registry` into
//! `trogon-atlas-semconv`.
//!
//! Each instrument binds to the global `MeterProvider` the first time it is
//! touched, and OpenTelemetry never rebinds it, so
//! [`crate::telemetry::init_telemetry`] must run before anything records.

use std::sync::LazyLock;

use trogon_atlas_semconv::{
    semconv::{
        auth_registrymetric, batch_recoverymetric, broadcastmetric, cachemetric,
        change_streammetric, changesetsmetric, git_mirrormetric, graphmetric, llmmetric,
        meter::Meter, mutation_lockmetric, options::InstrumentationOptions, rpcmetric,
        search_indexmetric,
    },
    GlobalMeterProvider,
};

/// `error.type` when no lower-cardinality classification of the failure
/// exists, per the OpenTelemetry error semantic conventions.
pub const ERROR_TYPE_OTHER: &str = "_OTHER";

static METER: LazyLock<Meter> =
    LazyLock::new(|| Meter::new(&GlobalMeterProvider, InstrumentationOptions::default()));

pub static RPC_SERVER_CALL_DURATION: LazyLock<rpcmetric::ServerCallDurationHistogram> =
    LazyLock::new(|| rpcmetric::ServerCallDurationHistogram::new(&METER));
pub static RPC_SERVER_STREAM_TERMINATIONS: LazyLock<rpcmetric::ServerStreamTerminationsCounter> =
    LazyLock::new(|| rpcmetric::ServerStreamTerminationsCounter::new(&METER));

pub static MUTATION_LOCK_WAIT_DURATION: LazyLock<mutation_lockmetric::WaitDurationHistogram> =
    LazyLock::new(|| mutation_lockmetric::WaitDurationHistogram::new(&METER));

pub static CACHE_LOOKUPS: LazyLock<cachemetric::LookupsCounter> =
    LazyLock::new(|| cachemetric::LookupsCounter::new(&METER));

pub static CHANGE_STREAM_MESSAGE_COUNT: LazyLock<change_streammetric::MessageCountGauge> =
    LazyLock::new(|| change_streammetric::MessageCountGauge::new(&METER));
pub static CHANGE_STREAM_SIZE: LazyLock<change_streammetric::SizeGauge> =
    LazyLock::new(|| change_streammetric::SizeGauge::new(&METER));
pub static CHANGE_STREAM_OLDEST_MESSAGE_AGE: LazyLock<change_streammetric::OldestMessageAgeGauge> =
    LazyLock::new(|| change_streammetric::OldestMessageAgeGauge::new(&METER));

pub static AUTH_REGISTRY_MEMBERSHIP_PUBLISH_FAILURES: LazyLock<
    auth_registrymetric::MembershipPublishFailuresCounter,
> = LazyLock::new(|| auth_registrymetric::MembershipPublishFailuresCounter::new(&METER));
pub static AUTH_REGISTRY_RELOAD_FAILURES: LazyLock<auth_registrymetric::ReloadFailuresCounter> =
    LazyLock::new(|| auth_registrymetric::ReloadFailuresCounter::new(&METER));
pub static AUTH_REGISTRY_RELOADS: LazyLock<auth_registrymetric::ReloadsCounter> =
    LazyLock::new(|| auth_registrymetric::ReloadsCounter::new(&METER));

pub static BATCH_RECOVERY_FAILURES: LazyLock<batch_recoverymetric::FailuresCounter> =
    LazyLock::new(|| batch_recoverymetric::FailuresCounter::new(&METER));
pub static BATCH_RECOVERY_UNRESOLVED: LazyLock<batch_recoverymetric::UnresolvedCounter> =
    LazyLock::new(|| batch_recoverymetric::UnresolvedCounter::new(&METER));

pub static BROADCAST_LAG_EVENTS: LazyLock<broadcastmetric::LagEventsCounter> =
    LazyLock::new(|| broadcastmetric::LagEventsCounter::new(&METER));

pub static CHANGESETS_LOST: LazyLock<changesetsmetric::LostCounter> =
    LazyLock::new(|| changesetsmetric::LostCounter::new(&METER));

pub static GIT_MIRROR_FAILURES: LazyLock<git_mirrormetric::FailuresCounter> =
    LazyLock::new(|| git_mirrormetric::FailuresCounter::new(&METER));

pub static GRAPH_CLOSURE_TRUNCATIONS: LazyLock<graphmetric::ClosureTruncationsCounter> =
    LazyLock::new(|| graphmetric::ClosureTruncationsCounter::new(&METER));

pub static SEARCH_INDEX_ERRORS: LazyLock<search_indexmetric::ErrorsCounter> =
    LazyLock::new(|| search_indexmetric::ErrorsCounter::new(&METER));

pub static LLM_PROMPT_TRUNCATIONS: LazyLock<llmmetric::PromptTruncationsCounter> =
    LazyLock::new(|| llmmetric::PromptTruncationsCounter::new(&METER));
pub static LLM_REQUEST_DURATION: LazyLock<llmmetric::RequestDurationHistogram> =
    LazyLock::new(|| llmmetric::RequestDurationHistogram::new(&METER));
pub static LLM_TOKENS: LazyLock<llmmetric::TokensCounter> =
    LazyLock::new(|| llmmetric::TokensCounter::new(&METER));
