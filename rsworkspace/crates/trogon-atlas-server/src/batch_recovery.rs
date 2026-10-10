//! Server-side entry points for recovering baseline batches that stopped
//! part way. The store owns the journal and the repair; this module decides
//! when a pass runs and how its result is reported.

use std::{sync::Arc, time::Duration};

use trogon_atlas_store::{
    store::Store, BatchRecovery, BatchRecoveryReport, RecoveryPolicy, RecoveryResult,
};

pub const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// Machine-readable report for `trogon-atlas-server recover-batches`.
pub fn report_json(report: &BatchRecoveryReport) -> serde_json::Value {
    serde_json::json!({
        "converged": report.is_converged(),
        "skipped_in_flight": report.skipped_in_flight,
        "skipped_recent": report.skipped_recent,
        "batches": report.batches.iter().map(batch_json).collect::<Vec<_>>(),
    })
}

fn batch_json(batch: &BatchRecovery) -> serde_json::Value {
    let (result, reason) = match &batch.result {
        RecoveryResult::Converged => ("converged", None),
        RecoveryResult::Pending => ("pending", None),
        RecoveryResult::Unresolved { reason } => ("unresolved", Some(reason.as_str())),
    };
    serde_json::json!({
        "journal": batch.journal.as_str(),
        "phase": batch.phase.as_str(),
        "action": batch.action.as_str(),
        "result": result,
        "reason": reason,
        "keys": batch.keys,
        "superseded": batch.superseded,
    })
}

/// Repair the batches an earlier process left behind before this one
/// serves a request. Entries younger than
/// [`RecoveryPolicy::AUTOMATIC_MIN_AGE`] are left to the sweep, because
/// during a rolling deploy the old process may still be writing them.
///
/// Skipped when this process does not hold the writer lease: a rolling
/// deploy briefly overlaps an old writer and a new standby on the same
/// journal, and only the lease holder may repair a batch it might still be
/// mid-write on. Manual recovery (`trogon-atlas-server recover-batches`) is
/// not gated this way; see its own doc comment.
pub async fn recover_at_startup(store: &dyn Store) -> BatchRecoveryReport {
    if !store.is_writer() {
        tracing::info!("skipping startup batch recovery; this process is not the writer");
        return BatchRecoveryReport::default();
    }
    match store.recover_batches(RecoveryPolicy::automatic()).await {
        Ok(report) => {
            log_report("startup", &report);
            report
        }
        Err(error) => {
            metrics::counter!("trogon_atlas_batch_recovery_failures_total").increment(1);
            tracing::error!(%error, "startup batch recovery could not list the journal");
            BatchRecoveryReport::default()
        }
    }
}

/// Retry, in the background, batches whose notifications, revisions, or
/// changeset could not be written when they ran. Each tick skips the pass
/// when this process does not currently hold the writer lease; see
/// [`recover_at_startup`] for why.
pub fn spawn_sweep(store: Arc<dyn Store>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if !store.is_writer() {
                tracing::debug!("skipping batch recovery sweep; this process is not the writer");
                continue;
            }
            match store.recover_batches(RecoveryPolicy::automatic()).await {
                Ok(report) => log_report("sweep", &report),
                Err(error) => {
                    metrics::counter!("trogon_atlas_batch_recovery_failures_total").increment(1);
                    tracing::warn!(%error, "batch recovery sweep could not list the journal");
                }
            }
        }
    })
}

fn log_report(pass: &'static str, report: &BatchRecoveryReport) {
    if report.batches.is_empty() {
        return;
    }
    let unresolved = report.unresolved().count();
    metrics::counter!("trogon_atlas_batch_recovery_unresolved_total")
        .increment(u64::try_from(unresolved).unwrap_or(u64::MAX));
    if unresolved == 0 {
        tracing::warn!(
            pass,
            batches = report.batches.len(),
            "batch recovery repaired interrupted batches"
        );
    } else {
        tracing::error!(
            pass,
            batches = report.batches.len(),
            unresolved,
            "batch recovery left batches unresolved; run `trogon-atlas-server recover-batches`"
        );
    }
}
