//! Process-wide metric instruments, generated from `otel/registry` into
//! `trogon-atlas-semconv`.
//!
//! Each instrument binds to the global `MeterProvider` the first time it is
//! touched, and OpenTelemetry never rebinds it, so
//! [`crate::telemetry::init_telemetry`] must run before anything records.

use std::sync::LazyLock;

use trogon_atlas_semconv::{
    semconv::{meter::Meter, metric, options::InstrumentationOptions},
    GlobalMeterProvider,
};

/// `error.type` when no lower-cardinality classification of the failure
/// exists, per the OpenTelemetry error semantic conventions.
pub const ERROR_TYPE_OTHER: &str = "_OTHER";

static METER: LazyLock<Meter> =
    LazyLock::new(|| Meter::new(&GlobalMeterProvider, InstrumentationOptions::default()));

pub static RPC_SERVER_CALL_DURATION: LazyLock<metric::RpcServerCallDuration> =
    LazyLock::new(|| metric::RpcServerCallDuration::new(&METER));
pub static RPC_SERVER_STREAM_TERMINATIONS: LazyLock<metric::RpcServerStreamTerminations> =
    LazyLock::new(|| metric::RpcServerStreamTerminations::new(&METER));

pub static MUTATION_LOCK_WAIT_DURATION: LazyLock<metric::MutationLockWaitDuration> =
    LazyLock::new(|| metric::MutationLockWaitDuration::new(&METER));

pub static CACHE_LOOKUPS: LazyLock<metric::CacheLookups> =
    LazyLock::new(|| metric::CacheLookups::new(&METER));

pub static CHANGE_STREAM_MESSAGE_COUNT: LazyLock<metric::ChangeStreamMessageCount> =
    LazyLock::new(|| metric::ChangeStreamMessageCount::new(&METER));
pub static CHANGE_STREAM_SIZE: LazyLock<metric::ChangeStreamSize> =
    LazyLock::new(|| metric::ChangeStreamSize::new(&METER));
pub static CHANGE_STREAM_OLDEST_MESSAGE_AGE: LazyLock<metric::ChangeStreamOldestMessageAge> =
    LazyLock::new(|| metric::ChangeStreamOldestMessageAge::new(&METER));

pub static AUTH_REGISTRY_MEMBERSHIP_PUBLISH_FAILURES: LazyLock<
    metric::AuthRegistryMembershipPublishFailures,
> = LazyLock::new(|| metric::AuthRegistryMembershipPublishFailures::new(&METER));
pub static AUTH_REGISTRY_RELOAD_FAILURES: LazyLock<metric::AuthRegistryReloadFailures> =
    LazyLock::new(|| metric::AuthRegistryReloadFailures::new(&METER));
pub static AUTH_REGISTRY_RELOADS: LazyLock<metric::AuthRegistryReloads> =
    LazyLock::new(|| metric::AuthRegistryReloads::new(&METER));

pub static BATCH_RECOVERY_FAILURES: LazyLock<metric::BatchRecoveryFailures> =
    LazyLock::new(|| metric::BatchRecoveryFailures::new(&METER));
pub static BATCH_RECOVERY_UNRESOLVED: LazyLock<metric::BatchRecoveryUnresolved> =
    LazyLock::new(|| metric::BatchRecoveryUnresolved::new(&METER));

pub static BROADCAST_LAG_EVENTS: LazyLock<metric::BroadcastLagEvents> =
    LazyLock::new(|| metric::BroadcastLagEvents::new(&METER));

pub static CHANGESETS_LOST: LazyLock<metric::ChangesetsLost> =
    LazyLock::new(|| metric::ChangesetsLost::new(&METER));

pub static GIT_MIRROR_FAILURES: LazyLock<metric::GitMirrorFailures> =
    LazyLock::new(|| metric::GitMirrorFailures::new(&METER));

pub static GRAPH_CLOSURE_TRUNCATIONS: LazyLock<metric::GraphClosureTruncations> =
    LazyLock::new(|| metric::GraphClosureTruncations::new(&METER));

pub static SEARCH_INDEX_ERRORS: LazyLock<metric::SearchIndexErrors> =
    LazyLock::new(|| metric::SearchIndexErrors::new(&METER));

pub static LLM_PROMPT_TRUNCATIONS: LazyLock<metric::LlmPromptTruncations> =
    LazyLock::new(|| metric::LlmPromptTruncations::new(&METER));
pub static LLM_REQUEST_DURATION: LazyLock<metric::LlmRequestDuration> =
    LazyLock::new(|| metric::LlmRequestDuration::new(&METER));
pub static LLM_TOKENS: LazyLock<metric::LlmTokens> =
    LazyLock::new(|| metric::LlmTokens::new(&METER));
