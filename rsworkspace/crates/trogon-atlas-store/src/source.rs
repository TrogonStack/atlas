use crate::{StoreError, StoreResult};

pub const SOURCE_DOCUMENT_MAX_BYTES: usize = 128 * 1024 * 1024;
pub const SOURCE_REGISTRATION_MAX_COUNT: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDocumentKey(String);

impl SourceDocumentKey {
    pub fn new(value: impl Into<String>) -> StoreResult<Self> {
        let value = value.into();
        if value.trim().is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
            return Err(StoreError::InvalidArgument(
                "invalid source registration identity".into(),
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct SourceDocument {
    pub key: SourceDocumentKey,
    pub bytes: Vec<u8>,
    pub revision: u64,
    pub created_at_micros: i64,
}

#[derive(Clone, Debug)]
pub struct SourceDocumentPage {
    pub keys: Vec<SourceDocumentKey>,
    pub next_cursor: Option<String>,
}

#[derive(Clone)]
pub struct SourcePublication {
    pub registration: SourceDocumentKey,
    pub control_revision: u64,
    pub run_id: String,
    pub plan_digest: String,
    pub plan: trogon_atlas_proto::SourceRunPlan,
    pub values: Vec<trogon_atlas_proto::SourceCommittedValue>,
    pub storage_limit: u64,
    pub finalization_reserve_bytes: u64,
}

#[derive(Clone, Debug)]
pub enum SourcePublicationOutcome {
    Committed(trogon_atlas_proto::SourceCommitMarker),
    NotCommitted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourcePublicationPoint {
    IntentDurable,
    PriorChunkDurable(usize),
    MarkerChunkDurable(usize),
    FenceAcquired,
    BeforeNativeWrite(usize),
    AfterNativeWrite(usize),
    BeforeCommitDecision,
    CommitDecisionDurable,
    FenceReleased,
}

pub trait SourcePublicationObserver: Send + Sync {
    fn native_access_admitted(&self) -> StoreResult<()> {
        Ok(())
    }
    fn reached(&self, point: SourcePublicationPoint) -> StoreResult<()>;
}
