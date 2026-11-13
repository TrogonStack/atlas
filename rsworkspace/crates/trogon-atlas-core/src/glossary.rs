use trogon_atlas_proto as pb;

use crate::authored_date::AuthoredDate;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid glossary value at {0}")]
pub struct GlossaryError(pub String);

pub type Result<T> = std::result::Result<T, GlossaryError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRange {
    start: usize,
    end: usize,
}

impl TextRange {
    pub fn new(start: usize, end: usize) -> Result<Self> {
        if start >= end {
            return Err(GlossaryError("range".into()));
        }
        Ok(Self { start, end })
    }

    #[must_use]
    pub fn start(self) -> usize {
        self.start
    }

    #[must_use]
    pub fn end(self) -> usize {
        self.end
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageFinding {
    pub term: pb::TermRef,
    pub range: TextRange,
    pub matched: String,
    pub usage: pb::GlossaryUsage,
    pub preferred_term: Option<pb::TermRef>,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlossaryDifferenceKind {
    DifferentMeaning,
    SameMeaning,
    MissingCounterpart,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlossaryDifference {
    pub kind: GlossaryDifferenceKind,
    pub left: Option<pb::TermRef>,
    pub right: Option<pb::TermRef>,
    pub title: String,
    pub explanation: String,
}

pub fn validate_values(_entity: &pb::Entity) -> Result<()> {
    Ok(())
}

pub fn validate_term(_term: &pb::Term) -> Result<()> {
    Ok(())
}

pub fn candidate_digest(_term: &pb::Term) -> Result<pb::ContentDigest> {
    Err(GlossaryError("candidate_digest".into()))
}

#[must_use]
pub fn review_is_current(_term: &pb::Term, _owner_uid: &str) -> bool {
    false
}

#[must_use]
pub fn review_is_accepted(_term: &pb::Term, _owner_uid: &str) -> bool {
    false
}

pub fn retained_document_bytes(_entity: &pb::Entity) -> Result<&[u8]> {
    Err(GlossaryError("document".into()))
}

pub fn verify_evidence(_evidence: &pb::GlossaryEvidence, _document: &pb::Entity) -> Result<()> {
    Err(GlossaryError("evidence".into()))
}

pub fn policy_active(_policy: &pb::GlossaryUsagePolicy, _at: &AuthoredDate) -> Result<bool> {
    Ok(false)
}

pub fn check_usage(
    _terms: &[pb::Term],
    _audience: &str,
    _at: &AuthoredDate,
    _text: &str,
    _exclusions: &[TextRange],
    _skip_quoted: bool,
) -> Result<Vec<UsageFinding>> {
    Ok(Vec::new())
}

pub fn compare_glossaries(
    _left: &[pb::Term],
    _right: &[pb::Term],
) -> Result<Vec<GlossaryDifference>> {
    Ok(Vec::new())
}
