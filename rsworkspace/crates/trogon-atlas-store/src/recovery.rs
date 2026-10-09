//! Recovery of baseline batches that did not finish.
//!
//! A batch is never transactional on a backend without multi-key
//! transactions. It is recoverable instead: before its first entity write
//! the store records a journal entry holding the prior image of every key
//! the batch touches and the changeset the batch belongs to, and that entry
//! is removed only once the batch's revisions, change notifications, and
//! changeset have all been written. A journal entry that outlives its batch
//! is the evidence a recovery pass acts on.

use std::{fmt, time::Duration};

/// Identity of one batch journal entry. Equal to the changeset id when the
/// batch was attributed, otherwise minted by the store.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BatchJournalId(String);

impl BatchJournalId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BatchJournalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How far a journaled batch got before it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalPhase {
    /// Prior images recorded; entity writes may have started. Nobody was
    /// told the batch succeeded.
    Planned,
    /// An entity write failed and the batch is being undone.
    Aborted,
    /// Every entity write landed and the caller is told the batch
    /// succeeded. Revisions, notifications, or the changeset may still be
    /// missing.
    Committed,
    /// Revisions and notifications are written; only the changeset append
    /// is outstanding.
    Published,
}

impl JournalPhase {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Aborted => "aborted",
            Self::Committed => "committed",
            Self::Published => "published",
        }
    }
}

impl fmt::Display for JournalPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What recovery does with one journaled batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Restore every key the batch changed to its prior image and forget
    /// it. The batch was never reported as applied, so it wrote no
    /// revision rows, change events, or changeset.
    RollBack,
    /// Write the missing revisions, notifications, and changeset. The batch
    /// was reported as applied.
    RollForward,
    /// Only the changeset append is outstanding.
    CompleteChangeset,
}

impl RecoveryAction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RollBack => "roll_back",
            Self::RollForward => "roll_forward",
            Self::CompleteChangeset => "complete_changeset",
        }
    }
}

/// Whether recovery only reports or also repairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryMode {
    Report,
    Repair,
}

/// Which journal entries a recovery pass may act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryPolicy {
    pub mode: RecoveryMode,
    /// Entries touched more recently than this are left alone, so a pass
    /// run beside a live writer does not act on a batch still in flight.
    pub min_age: Duration,
}

impl RecoveryPolicy {
    /// How long an entry must sit untouched before the server repairs it
    /// on its own. Another process may still be writing a younger entry,
    /// for example the old process during a rolling deploy.
    pub const AUTOMATIC_MIN_AGE: Duration = Duration::from_secs(60);

    /// What the server runs on its own, at startup and in the background
    /// sweep.
    #[must_use]
    pub fn automatic() -> Self {
        Self::repair_all().older_than(Self::AUTOMATIC_MIN_AGE)
    }

    /// Repair every entry not in flight in this process, whatever its age.
    #[must_use]
    pub fn repair_all() -> Self {
        Self {
            mode: RecoveryMode::Repair,
            min_age: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn report_all() -> Self {
        Self {
            mode: RecoveryMode::Report,
            min_age: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn older_than(self, min_age: Duration) -> Self {
        Self { min_age, ..self }
    }
}

/// Where one batch ended up after a recovery pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryResult {
    /// The batch converged: fully applied with its revisions, notifications,
    /// and changeset, or fully absent.
    Converged,
    /// Report mode: the action that a repair pass would take.
    Pending,
    /// Repair was attempted and did not converge. The journal entry stays,
    /// so the next pass retries.
    Unresolved { reason: String },
}

/// One journaled batch a recovery pass found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRecovery {
    pub journal: BatchJournalId,
    pub phase: JournalPhase,
    pub action: RecoveryAction,
    pub result: RecoveryResult,
    /// Entity keys whose state the action changes.
    pub keys: Vec<String>,
    /// Keys a later write already replaced. Their notification and
    /// changeset attribution are still repaired, but the image the batch
    /// wrote no longer exists, so no revision row can be rebuilt for them.
    pub superseded: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchRecoveryReport {
    pub batches: Vec<BatchRecovery>,
    /// Entries left alone because a batch in this process still owns them.
    pub skipped_in_flight: usize,
    /// Entries left alone because they are younger than the policy allows.
    pub skipped_recent: usize,
}

impl BatchRecoveryReport {
    /// True when nothing is left for a later pass to do.
    #[must_use]
    pub fn is_converged(&self) -> bool {
        self.skipped_in_flight == 0
            && self.skipped_recent == 0
            && self
                .batches
                .iter()
                .all(|batch| batch.result == RecoveryResult::Converged)
    }

    pub fn unresolved(&self) -> impl Iterator<Item = &BatchRecovery> {
        self.batches
            .iter()
            .filter(|batch| matches!(batch.result, RecoveryResult::Unresolved { .. }))
    }
}
