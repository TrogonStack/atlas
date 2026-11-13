use std::collections::HashSet;

use prost::Message;
use sha2::{Digest, Sha256};
use trogon_atlas_proto as pb;

use crate::authored_date::AuthoredDate;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid glossary value at {0}")]
pub struct GlossaryError(pub String);

pub type Result<T> = std::result::Result<T, GlossaryError>;

pub const MAX_EVIDENCE: usize = 32;
pub const MAX_TERMS: usize = 1_000;
pub const MAX_TEXT_BYTES: usize = 262_144;
pub const MAX_FINDINGS: usize = 1_024;
pub const MAX_MATCH_WORK_BYTES: usize = 16_777_216;
pub const MAX_CANDIDATE_BYTES: usize = 8_388_608;

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

fn require(condition: bool, path: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(GlossaryError(path.into()))
    }
}

fn text(value: &str, limit: usize, path: &str) -> Result<()> {
    require(!value.trim().is_empty() && value.len() <= limit, path)
}

fn exact(id: Option<&pb::Id>, path: &str) -> Result<()> {
    let id = id.ok_or_else(|| GlossaryError(path.into()))?;
    require(
        id.version > 0
            && crate::is_safe_id_component(&id.namespace)
            && crate::is_safe_id_component(&id.slug),
        path,
    )
}

fn digest(value: Option<&pb::ContentDigest>, path: &str) -> Result<()> {
    value
        .ok_or_else(|| GlossaryError(path.into()))?
        .sha256
        .parse::<crate::ContentHash>()
        .map(|_| ())
        .map_err(|_| GlossaryError(path.into()))
}

fn calendar(value: Option<&pb::CalendarDate>, path: &str) -> Result<Option<AuthoredDate>> {
    value
        .map(|value| {
            value
                .value
                .parse::<AuthoredDate>()
                .map_err(|_| GlossaryError(path.into()))
        })
        .transpose()
}

fn validate_evidence(evidence: &pb::GlossaryEvidence) -> Result<()> {
    exact(
        evidence
            .document
            .as_ref()
            .and_then(|value| value.id.as_ref()),
        "evidence.document",
    )?;
    text(&evidence.owner_uid, 128, "evidence.owner_uid")?;
    digest(evidence.content_digest.as_ref(), "evidence.content_digest")?;
    text(&evidence.locator, 1_024, "evidence.locator")?;
    text(&evidence.quote, 16_384, "evidence.quote")
}

fn validate_policy(policy: &pb::GlossaryUsagePolicy) -> Result<()> {
    let usage = pb::GlossaryUsage::try_from(policy.usage)
        .map_err(|_| GlossaryError("policy.usage".into()))?;
    require(usage != pb::GlossaryUsage::Unspecified, "policy.usage")?;
    require(policy.rationale.len() <= 16_384, "policy.rationale")?;
    if usage != pb::GlossaryUsage::Allowed || policy.preferred_term.is_some() {
        text(&policy.rationale, 16_384, "policy.rationale")?;
    }
    if let Some(reference) = &policy.preferred_term {
        exact(reference.id.as_ref(), "policy.preferred_term")?;
        require(usage != pb::GlossaryUsage::Allowed, "policy.preferred_term")?;
    }
    let from = calendar(policy.effective_from.as_ref(), "policy.effective_from")?;
    let until = calendar(policy.effective_until.as_ref(), "policy.effective_until")?;
    require(
        !matches!((from, until), (Some(from), Some(until)) if from >= until),
        "policy.interval",
    )
}

fn validate_review(review: &pb::GlossaryReview) -> Result<()> {
    require(
        matches!(
            pb::GlossaryReviewDisposition::try_from(review.disposition),
            Ok(pb::GlossaryReviewDisposition::Accepted
                | pb::GlossaryReviewDisposition::Rejected
                | pb::GlossaryReviewDisposition::NeedsChanges)
        ),
        "review.disposition",
    )?;
    text(&review.actor, 256, "review.actor")?;
    text(&review.owner_uid, 128, "review.owner_uid")?;
    text(&review.reason, 16_384, "review.reason")?;
    require(
        review.reviewed_at.len() <= 64
            && chrono::DateTime::parse_from_rfc3339(&review.reviewed_at).is_ok(),
        "review.reviewed_at",
    )?;
    digest(review.content_digest.as_ref(), "review.content_digest")
}

pub fn validate_values(entity: &pb::Entity) -> Result<()> {
    if let Some(pb::entity::Kind::Term(term)) = &entity.kind {
        validate_term(term)?;
    }
    Ok(())
}

pub fn validate_term(term: &pb::Term) -> Result<()> {
    let Some(glossary) = &term.glossary else {
        return Ok(());
    };
    exact(term.id.as_ref(), "term.id")?;
    text(&term.title, 256, "term.title")?;
    text(&term.doc, 16_384, "term.doc")?;
    text(&glossary.generation_revision, 256, "generation_revision")?;
    text(&glossary.audience, 128, "audience")?;
    require(
        !glossary.evidence.is_empty() && glossary.evidence.len() <= MAX_EVIDENCE,
        "evidence.count",
    )?;
    let mut seen = HashSet::new();
    for evidence in &glossary.evidence {
        validate_evidence(evidence)?;
        require(
            evidence
                .document
                .as_ref()
                .and_then(|value| value.id.as_ref())
                .map(|value| value.namespace.as_str())
                == term.id.as_ref().map(|value| value.namespace.as_str()),
            "evidence.namespace",
        )?;
        require(seen.insert(evidence.encode_to_vec()), "evidence.duplicate")?;
    }
    calendar(glossary.next_review_on.as_ref(), "next_review_on")?;
    if let Some(policy) = &glossary.policy {
        validate_policy(policy)?;
        require(
            policy
                .preferred_term
                .as_ref()
                .and_then(|value| value.id.as_ref())
                != term.id.as_ref(),
            "policy.self_replacement",
        )?;
    }
    if let Some(review) = &glossary.review {
        validate_review(review)?;
    }
    require(term.encoded_len() <= 1_048_576, "term.encoded_bytes")?;
    Ok(())
}

pub fn candidate_digest(term: &pb::Term) -> Result<pb::ContentDigest> {
    require(term.encoded_len() <= 1_048_576, "term.encoded_bytes")?;
    let mut candidate = term.clone();
    let glossary = candidate
        .glossary
        .as_mut()
        .ok_or_else(|| GlossaryError("term.glossary".into()))?;
    glossary.review = None;
    glossary.next_review_on = None;
    validate_term(&candidate)?;
    let mut hash = Sha256::new();
    hash.update(b"atlas.glossary.term.v1\0");
    hash.update(candidate.encode_to_vec());
    Ok(pb::ContentDigest {
        sha256: crate::ContentHash::from_bytes(hash.finalize().into()).to_hex(),
    })
}

#[must_use]
pub fn review_is_current(term: &pb::Term, owner_uid: &str) -> bool {
    let Some(review) = term
        .glossary
        .as_ref()
        .and_then(|value| value.review.as_ref())
    else {
        return false;
    };
    !owner_uid.is_empty()
        && review.owner_uid == owner_uid
        && validate_term(term).is_ok()
        && candidate_digest(term).ok().as_ref() == review.content_digest.as_ref()
}

#[must_use]
pub fn review_is_accepted(term: &pb::Term, owner_uid: &str) -> bool {
    term.glossary
        .as_ref()
        .and_then(|value| value.review.as_ref())
        .is_some_and(|review| {
            review.disposition == pb::GlossaryReviewDisposition::Accepted as i32
                && review_is_current(term, owner_uid)
        })
}

pub fn retained_document_bytes(entity: &pb::Entity) -> Result<&[u8]> {
    let Some(pb::entity::Kind::Document(document)) = &entity.kind else {
        return Err(GlossaryError("document.kind".into()));
    };
    let bytes = match &document.body {
        Some(pb::document::Body::Retained(body)) => body.content.as_bytes(),
        Some(pb::document::Body::RetainedContent(body)) => body.content.as_slice(),
        _ => return Err(GlossaryError("document.retained_content".into())),
    };
    require(
        !bytes.is_empty() && bytes.len() <= MAX_TEXT_BYTES,
        "document.bytes",
    )?;
    Ok(bytes)
}

pub fn verify_evidence(evidence: &pb::GlossaryEvidence, entity: &pb::Entity) -> Result<()> {
    validate_evidence(evidence)?;
    require(
        evidence
            .document
            .as_ref()
            .and_then(|value| value.id.as_ref())
            == crate::entity_id(entity),
        "evidence.document",
    )?;
    require(
        entity
            .system
            .as_ref()
            .is_some_and(|system| system.uid == evidence.owner_uid),
        "evidence.owner_uid",
    )?;
    let bytes = retained_document_bytes(entity)?;
    require(
        evidence.content_digest.as_ref()
            == Some(&crate::document_catalog_context::bytes_content_digest(
                bytes,
            )),
        "evidence.content_digest",
    )?;
    let content = std::str::from_utf8(bytes).map_err(|_| GlossaryError("evidence.utf8".into()))?;
    require(content.contains(&evidence.quote), "evidence.quote")
}

pub fn policy_active(policy: &pb::GlossaryUsagePolicy, at: &AuthoredDate) -> Result<bool> {
    validate_policy(policy)?;
    let from = calendar(policy.effective_from.as_ref(), "policy.effective_from")?;
    let until = calendar(policy.effective_until.as_ref(), "policy.effective_until")?;
    Ok(
        from.as_ref().is_none_or(|from| at >= from)
            && until.as_ref().is_none_or(|until| at < until),
    )
}

pub fn check_usage(
    terms: &[pb::Term],
    audience: &str,
    at: &AuthoredDate,
    text: &str,
    exclusions: &[TextRange],
    skip_quoted: bool,
) -> Result<Vec<UsageFinding>> {
    require(
        terms.len() <= MAX_TERMS
            && text.len() <= MAX_TEXT_BYTES
            && exclusions.len() <= MAX_FINDINGS
            && terms.len().saturating_mul(text.len()) <= MAX_MATCH_WORK_BYTES,
        "usage.limit",
    )?;
    candidate_bytes(terms.iter())?;
    let mut ignored = exclusions.to_vec();
    for range in &ignored {
        require(
            range.start < range.end
                && range.end <= text.len()
                && text.is_char_boundary(range.start)
                && text.is_char_boundary(range.end),
            "usage.exclusions",
        )?;
    }
    if skip_quoted {
        ignored.extend(quoted_ranges(text)?);
    }
    ignored.sort_by_key(|range| (range.start, range.end));
    let mut excluded: Vec<TextRange> = Vec::new();
    for range in ignored {
        if let Some(previous) = excluded
            .last_mut()
            .filter(|previous| range.start <= previous.end)
        {
            previous.end = previous.end.max(range.end);
        } else {
            excluded.push(range);
        }
    }
    let (folded, mapping) = folded_with_offsets(text);
    let mut findings = Vec::new();
    for term in terms {
        validate_term(term)?;
        let Some(entry) = &term.glossary else {
            continue;
        };
        let Some(policy) = &entry.policy else {
            continue;
        };
        if entry.audience != audience
            || policy.usage == pb::GlossaryUsage::Allowed as i32
            || !policy_active(policy, at)?
        {
            continue;
        }
        let pattern = term.title.trim().to_lowercase();
        for (start, matched) in folded.match_indices(&pattern) {
            let end = start + matched.len();
            if start == end
                || (start > 0 && mapping[start - 1].start == mapping[start].start)
                || (end < mapping.len() && mapping[end - 1].start == mapping[end].start)
            {
                continue;
            }
            let range = TextRange {
                start: mapping[start].start,
                end: mapping[end - 1].end,
            };
            if !word_boundary(text[..range.start].chars().next_back())
                || !word_boundary(text[range.end..].chars().next())
                || excluded
                    .get(excluded.partition_point(|ignored| ignored.end <= range.start))
                    .is_some_and(|ignored| ignored.start < range.end)
            {
                continue;
            }
            require(findings.len() < MAX_FINDINGS, "usage.findings_limit")?;
            findings.push(UsageFinding {
                term: pb::TermRef {
                    id: term.id.clone(),
                },
                range,
                matched: text[range.start..range.end].to_owned(),
                usage: pb::GlossaryUsage::try_from(policy.usage)
                    .map_err(|_| GlossaryError("policy.usage".into()))?,
                preferred_term: policy.preferred_term.clone(),
                rationale: policy.rationale.clone(),
            });
        }
    }
    findings.sort_by(|a, b| {
        (a.range.start, a.range.end)
            .cmp(&(b.range.start, b.range.end))
            .then_with(|| reference_key(&a.term).cmp(&reference_key(&b.term)))
    });
    Ok(findings)
}

pub fn compare_glossaries(
    left: &[pb::Term],
    right: &[pb::Term],
) -> Result<Vec<GlossaryDifference>> {
    require(
        left.len() <= MAX_TERMS && right.len() <= MAX_TERMS,
        "comparison.limit",
    )?;
    candidate_bytes(left.iter().chain(right))?;
    for term in left.iter().chain(right) {
        validate_term(term)?;
        exact(term.id.as_ref(), "term.id")?;
        text(&term.title, 256, "term.title")?;
        text(&term.doc, 16_384, "term.doc")?;
    }
    let mut left_matched = vec![false; left.len()];
    let mut right_matched = vec![false; right.len()];
    let normalized_left = normalized_terms(left);
    let normalized_right = normalized_terms(right);
    let mut differences = Vec::new();
    for (left_index, a) in left.iter().enumerate() {
        for (right_index, b) in right.iter().enumerate() {
            let same_title = normalized_left[left_index].0 == normalized_right[right_index].0;
            let same_meaning = normalized_left[left_index].1 == normalized_right[right_index].1;
            if !same_title && !same_meaning {
                continue;
            }
            left_matched[left_index] = true;
            right_matched[right_index] = true;
            let (kind, explanation) = if same_title && !same_meaning {
                (
                    GlossaryDifferenceKind::DifferentMeaning,
                    "Equal term labels have different definition text; review a possible homonym.",
                )
            } else if !same_title && same_meaning {
                (GlossaryDifferenceKind::SameMeaning,
                    "Different term labels have equal normalized definition text; review a possible synonym.")
            } else {
                continue;
            };
            require(
                differences.len() < MAX_FINDINGS,
                "comparison.findings_limit",
            )?;
            differences.push(GlossaryDifference {
                kind,
                left: Some(pb::TermRef { id: a.id.clone() }),
                right: Some(pb::TermRef { id: b.id.clone() }),
                title: a.title.clone(),
                explanation: explanation.into(),
            });
        }
    }
    for (terms, matched, is_left) in [(left, &left_matched, true), (right, &right_matched, false)] {
        for (term, matched) in terms.iter().zip(matched) {
            if !matched {
                require(
                    differences.len() < MAX_FINDINGS,
                    "comparison.findings_limit",
                )?;
                let reference = Some(pb::TermRef {
                    id: term.id.clone(),
                });
                differences.push(GlossaryDifference {
                    kind: GlossaryDifferenceKind::MissingCounterpart,
                    left: if is_left { reference.clone() } else { None },
                    right: if is_left { None } else { reference },
                    title: term.title.clone(),
                    explanation: "No equal label or normalized definition in the selected counterpart glossary; this is not proof of semantic absence.".into(),
                });
            }
        }
    }
    differences.sort_by(|a, b| {
        (
            &a.title,
            a.left.as_ref().map(reference_key),
            a.right.as_ref().map(reference_key),
        )
            .cmp(&(
                &b.title,
                b.left.as_ref().map(reference_key),
                b.right.as_ref().map(reference_key),
            ))
    });
    Ok(differences)
}

fn candidate_bytes<'a>(mut terms: impl Iterator<Item = &'a pb::Term>) -> Result<()> {
    let bytes = terms.try_fold(0usize, |total, term| {
        let total = total.saturating_add(term.encoded_len());
        require(total <= MAX_CANDIDATE_BYTES, "candidates.bytes_limit")?;
        Ok(total)
    });
    bytes.map(|_| ())
}

fn normalized_terms(terms: &[pb::Term]) -> Vec<(String, String)> {
    terms
        .iter()
        .map(|term| (normalized(&term.title), normalized(&term.doc)))
        .collect()
}

fn normalized(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn reference_key(reference: &pb::TermRef) -> (String, String, u64) {
    reference.id.as_ref().map_or_else(
        || (String::new(), String::new(), 0),
        |id| (id.namespace.clone(), id.slug.clone(), id.version),
    )
}

fn word_boundary(value: Option<char>) -> bool {
    !value.is_some_and(|value| value.is_alphanumeric() || value == '_')
}

fn folded_with_offsets(value: &str) -> (String, Vec<TextRange>) {
    let mut folded = String::new();
    let mut mapping = Vec::new();
    for (start, value) in value.char_indices() {
        let range = TextRange {
            start,
            end: start + value.len_utf8(),
        };
        for value in value.to_lowercase() {
            folded.push(value);
            mapping.extend(std::iter::repeat_n(range, value.len_utf8()));
        }
    }
    (folded, mapping)
}

fn quoted_ranges(value: &str) -> Result<Vec<TextRange>> {
    let mut ranges = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut previous = None;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            previous = Some(character);
            continue;
        }
        if character == '\\' {
            escaped = true;
            previous = Some(character);
            continue;
        }
        match quote {
            Some((delimiter, start)) if character == delimiter => {
                ranges.push(TextRange {
                    start,
                    end: index + character.len_utf8(),
                });
                quote = None;
            }
            None if matches!(character, '"' | '`')
                || (character == '\'' && word_boundary(previous)) =>
            {
                quote = Some((character, index));
            }
            _ => {}
        }
        previous = Some(character);
    }
    require(quote.is_none(), "usage.unclosed_quote")?;
    Ok(ranges)
}
