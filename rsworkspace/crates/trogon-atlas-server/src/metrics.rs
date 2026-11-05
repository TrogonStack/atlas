//! Metric instruments the server records, generated from `otel/registry`
//! into `trogon-atlas-semconv`.

use std::{fmt, time::Duration};

use trogon_atlas_semconv::semconv::{attribute, meter::Meter, metric};
use trogon_atlas_store::ChangeStreamStats;

/// `error.type` when no lower-cardinality classification of the failure
/// exists, per the OpenTelemetry error semantic conventions.
const ERROR_TYPE_OTHER: &str = "_OTHER";

/// The server's instruments, bound to the [`Meter`] they were built from.
/// Cloning shares the instruments. [`Default`] records nothing.
#[derive(Clone)]
pub struct ServerMetrics {
    rpc_server_call_duration: metric::RpcServerCallDuration,
    rpc_server_stream_terminations: metric::RpcServerStreamTerminations,
    mutation_lock_wait_duration: metric::MutationLockWaitDuration,
    consequence_delta_duration: metric::ConsequenceDeltaDuration,
    cache_lookups: metric::CacheLookups,
    change_stream_message_count: metric::ChangeStreamMessageCount,
    change_stream_size: metric::ChangeStreamSize,
    change_stream_oldest_message_age: metric::ChangeStreamOldestMessageAge,
    auth_registry_membership_publish_failures: metric::AuthRegistryMembershipPublishFailures,
    auth_registry_reload_failures: metric::AuthRegistryReloadFailures,
    auth_registry_reloads: metric::AuthRegistryReloads,
    batch_recovery_failures: metric::BatchRecoveryFailures,
    batch_recovery_unresolved: metric::BatchRecoveryUnresolved,
    broadcast_lag_events: metric::BroadcastLagEvents,
    changesets_lost: metric::ChangesetsLost,
    git_mirror_failures: metric::GitMirrorFailures,
    graph_closure_truncations: metric::GraphClosureTruncations,
    search_index_errors: metric::SearchIndexErrors,
    llm_prompt_truncations: metric::LlmPromptTruncations,
    llm_request_duration: metric::LlmRequestDuration,
    llm_tokens: metric::LlmTokens,
}

impl ServerMetrics {
    pub fn new(meter: &Meter) -> Self {
        Self {
            rpc_server_call_duration: metric::RpcServerCallDuration::new(meter),
            rpc_server_stream_terminations: metric::RpcServerStreamTerminations::new(meter),
            mutation_lock_wait_duration: metric::MutationLockWaitDuration::new(meter),
            consequence_delta_duration: metric::ConsequenceDeltaDuration::new(meter),
            cache_lookups: metric::CacheLookups::new(meter),
            change_stream_message_count: metric::ChangeStreamMessageCount::new(meter),
            change_stream_size: metric::ChangeStreamSize::new(meter),
            change_stream_oldest_message_age: metric::ChangeStreamOldestMessageAge::new(meter),
            auth_registry_membership_publish_failures:
                metric::AuthRegistryMembershipPublishFailures::new(meter),
            auth_registry_reload_failures: metric::AuthRegistryReloadFailures::new(meter),
            auth_registry_reloads: metric::AuthRegistryReloads::new(meter),
            batch_recovery_failures: metric::BatchRecoveryFailures::new(meter),
            batch_recovery_unresolved: metric::BatchRecoveryUnresolved::new(meter),
            broadcast_lag_events: metric::BroadcastLagEvents::new(meter),
            changesets_lost: metric::ChangesetsLost::new(meter),
            git_mirror_failures: metric::GitMirrorFailures::new(meter),
            graph_closure_truncations: metric::GraphClosureTruncations::new(meter),
            search_index_errors: metric::SearchIndexErrors::new(meter),
            llm_prompt_truncations: metric::LlmPromptTruncations::new(meter),
            llm_request_duration: metric::LlmRequestDuration::new(meter),
            llm_tokens: metric::LlmTokens::new(meter),
        }
    }

    pub fn record_rpc_call(
        &self,
        elapsed: Duration,
        attributes: impl IntoIterator<Item = metric::RpcServerCallDurationAttr>,
    ) {
        self.rpc_server_call_duration.record(
            elapsed.as_secs_f64(),
            crate::telemetry::GRPC_SYSTEM.into(),
            attributes,
        );
    }

    pub fn record_stream_termination(&self, rpc_method: String, outcome: attribute::StreamOutcome) {
        self.rpc_server_stream_terminations
            .add(1, rpc_method.into(), outcome);
    }

    /// Time a caller spent blocked acquiring the global mutation lock.
    pub fn record_mutation_lock_wait(&self, waited: Duration) {
        self.mutation_lock_wait_duration
            .record(waited.as_secs_f64());
    }

    /// Time spent computing one consequence delta.
    pub fn record_consequence_delta(&self, elapsed: Duration) {
        self.consequence_delta_duration
            .record(elapsed.as_secs_f64());
    }

    pub fn record_snapshot_cache_hit(&self) {
        self.record_cache_lookup(attribute::CacheName::Snapshot, attribute::CacheResult::Hit);
    }

    pub fn record_snapshot_cache_miss(&self) {
        self.record_cache_lookup(attribute::CacheName::Snapshot, attribute::CacheResult::Miss);
    }

    /// A miss means one branch query rebuilt a whole index, which is the
    /// cost the branch search cache exists to avoid; a sustained miss rate
    /// means branches are being written between every search, or the cache
    /// is too small for the number of branches under review.
    pub fn record_branch_search_index_hit(&self) {
        self.record_cache_lookup(
            attribute::CacheName::BranchSearchIndex,
            attribute::CacheResult::Hit,
        );
    }

    pub fn record_branch_search_index_miss(&self) {
        self.record_cache_lookup(
            attribute::CacheName::BranchSearchIndex,
            attribute::CacheResult::Miss,
        );
    }

    fn record_cache_lookup(&self, name: attribute::CacheName, result: attribute::CacheResult) {
        self.cache_lookups.add(1, name, result);
    }

    pub fn record_change_stream_stats(&self, stats: &ChangeStreamStats) {
        self.change_stream_message_count
            .record(i64::try_from(stats.messages).unwrap_or(i64::MAX));
        self.change_stream_size
            .record(i64::try_from(stats.bytes).unwrap_or(i64::MAX));
        let age = match stats.oldest_message_unix_seconds {
            Some(secs) => (chrono::Utc::now().timestamp() - secs).max(0),
            None => 0,
        };
        self.change_stream_oldest_message_age.record(age);
    }

    pub fn record_auth_registry_reload(&self) {
        self.auth_registry_reloads.add(1);
    }

    pub fn record_auth_registry_reload_failure(&self) {
        self.auth_registry_reload_failures.add(1);
    }

    pub fn record_auth_registry_membership_publish_failure(&self) {
        self.auth_registry_membership_publish_failures.add(1);
    }

    pub fn record_batch_recovery_failure(&self) {
        self.batch_recovery_failures.add(1);
    }

    pub fn record_batch_recovery_unresolved(&self, unresolved: usize) {
        self.batch_recovery_unresolved
            .add(u64::try_from(unresolved).unwrap_or(u64::MAX));
    }

    pub fn record_broadcast_lag(&self, missed: u64) {
        self.broadcast_lag_events.add(missed);
    }

    pub fn record_changeset_lost(&self) {
        self.changesets_lost.add(1);
    }

    pub fn record_git_mirror_failure(&self) {
        self.git_mirror_failures.add(1);
    }

    pub fn record_graph_closure_truncation(&self) {
        self.graph_closure_truncations.add(1);
    }

    pub fn record_search_index_error(&self) {
        self.search_index_errors.add(1);
    }

    pub fn record_llm_prompt_truncation(&self) {
        self.llm_prompt_truncations.add(1);
    }

    pub fn record_llm_request<T, E>(
        &self,
        elapsed: Duration,
        model: attribute::LlmModel,
        provider: attribute::LlmProvider,
        result: &Result<T, E>,
    ) {
        self.llm_request_duration.record(
            elapsed.as_secs_f64(),
            model,
            provider,
            result
                .is_err()
                .then(|| metric::LlmRequestDurationAttr::ErrorType(ERROR_TYPE_OTHER.into())),
        );
    }

    pub fn record_llm_tokens(
        &self,
        model: attribute::LlmModel,
        token_type: attribute::LlmTokenType,
        tokens: u64,
    ) {
        self.llm_tokens.add(tokens, model, token_type);
    }
}

impl Default for ServerMetrics {
    fn default() -> Self {
        Self::new(&Meter::default())
    }
}

impl fmt::Debug for ServerMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerMetrics").finish_non_exhaustive()
    }
}
