use std::collections::HashSet;

use prost::{
    bytes::{Buf as _, Bytes},
    encoding::{decode_key, decode_varint, skip_field, DecodeContext, WireType},
    Message,
};
use prost_reflect::{DynamicMessage, Kind, MessageDescriptor, ReflectMessage as _, Value};
use trogon_atlas_proto as pb;

use crate::{authored_date::AuthoredDate, transcode::TranscodePool};

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

const MAX_METADATA_DEPTH: usize = 32;
const MAX_METADATA_DECODE_BYTES: usize = 4_194_304;
const MAX_METADATA_MESSAGES: usize = 16_384;

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

fn validate_usage_period(period: &pb::GlossaryUsagePeriod) -> Result<()> {
    require(
        period.valid_from.is_some() || period.valid_until.is_some(),
        "usage_period.empty",
    )?;
    let from = calendar(period.valid_from.as_ref(), "usage_period.valid_from")?;
    let until = calendar(period.valid_until.as_ref(), "usage_period.valid_until")?;
    require(
        !matches!((from, until), (Some(from), Some(until)) if from >= until),
        "usage_period.interval",
    )
}

fn usage_period_reason(
    period: &pb::GlossaryUsagePeriod,
    at: &AuthoredDate,
) -> Result<Option<String>> {
    validate_usage_period(period)?;
    if let Some(from) = calendar(period.valid_from.as_ref(), "usage_period.valid_from")? {
        if at < &from {
            return Ok(Some(format!("Term is not valid for use before {from}.")));
        }
    }
    if let Some(until) = calendar(period.valid_until.as_ref(), "usage_period.valid_until")? {
        if at >= &until {
            return Ok(Some(format!(
                "Term is no longer valid for use on or after {until}."
            )));
        }
    }
    Ok(None)
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
    if let Some(period) = &glossary.usage_period {
        validate_usage_period(period)?;
    }
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
    let pool = TranscodePool::builtin()
        .map_err(|_| GlossaryError("candidate.canonical_content".into()))?;
    candidate_digest_with_pool(term, &pool)
}

pub fn candidate_digest_with_pool(
    term: &pb::Term,
    pool: &TranscodePool,
) -> Result<pb::ContentDigest> {
    require(term.encoded_len() <= 1_048_576, "term.encoded_bytes")?;
    let mut candidate = term.clone();
    let glossary = candidate
        .glossary
        .as_mut()
        .ok_or_else(|| GlossaryError("term.glossary".into()))?;
    glossary.review = None;
    glossary.next_review_on = None;
    validate_term(&candidate)?;
    verify_metadata_identity(&candidate, pool)?;
    let entity = pb::Entity {
        kind: Some(pb::entity::Kind::Term(candidate)),
        system: None,
    };
    let value = pool
        .entity_json_value(&entity)
        .map_err(|_| GlossaryError("candidate.canonical_content".into()))?;
    let hash = crate::content_hash::domain_digest(b"atlas.glossary.term.v1\0", &value)
        .map_err(|_| GlossaryError("candidate.canonical_content".into()))?;
    Ok(pb::ContentDigest {
        sha256: hash.to_hex(),
    })
}

struct ReviewMetadata<'a> {
    pool: &'a TranscodePool,
    pending: Vec<(MessageDescriptor, Bytes, usize)>,
    payloads: Vec<(String, Bytes, usize)>,
    decoded_bytes: usize,
    messages: usize,
}

impl ReviewMetadata<'_> {
    fn push(&mut self, descriptor: MessageDescriptor, bytes: Bytes, depth: usize) -> Result<()> {
        require(
            depth <= MAX_METADATA_DEPTH && self.messages < MAX_METADATA_MESSAGES,
            "candidate.metadata_limit",
        )?;
        self.messages += 1;
        self.pending.push((descriptor, bytes, depth));
        Ok(())
    }

    fn decode(&mut self, type_url: &str, bytes: Bytes, depth: usize) -> Result<()> {
        let decoded_bytes = self.decoded_bytes.saturating_add(bytes.len());
        require(
            depth <= MAX_METADATA_DEPTH && decoded_bytes <= MAX_METADATA_DECODE_BYTES,
            "candidate.metadata_limit",
        )?;
        self.decoded_bytes = decoded_bytes;
        let descriptor = self
            .pool
            .message_descriptor(type_url.rsplit('/').next().unwrap_or_default())
            .map_err(|_| GlossaryError("candidate.canonical_content".into()))?;
        self.push(descriptor.clone(), bytes.clone(), depth)?;
        while let Some((descriptor, bytes, depth)) = self.pending.pop() {
            self.visit(&descriptor, bytes, depth)?;
        }
        let message = DynamicMessage::decode(descriptor, bytes)
            .map_err(|_| GlossaryError("candidate.canonical_content".into()))?;
        self.queue_payloads(message, depth)
    }

    fn visit(
        &mut self,
        descriptor: &MessageDescriptor,
        mut bytes: Bytes,
        depth: usize,
    ) -> Result<()> {
        let invalid = || GlossaryError("candidate.canonical_content".into());
        // Inspect occurrences before decoding can discard map-entry fields or overwritten values.
        while bytes.has_remaining() {
            let (tag, wire) = decode_key(&mut bytes).map_err(|_| invalid())?;
            let kind = descriptor
                .get_field(tag)
                .map(|field| field.kind())
                .or_else(|| descriptor.get_extension(tag).map(|field| field.kind()))
                .ok_or_else(invalid)?;
            if let Kind::Message(child) = kind {
                let value = match wire {
                    WireType::LengthDelimited => {
                        let len = decode_varint(&mut bytes).map_err(|_| invalid())?;
                        let len = usize::try_from(len).map_err(|_| invalid())?;
                        require(len <= bytes.len(), "candidate.canonical_content")?;
                        bytes.split_to(len)
                    }
                    WireType::StartGroup => metadata_group(tag, &mut bytes, depth + 1)?,
                    _ => return Err(invalid()),
                };
                self.push(child, value, depth + 1)?;
            } else {
                require(wire != WireType::StartGroup, "candidate.canonical_content")?;
                skip_field(wire, tag, &mut bytes, DecodeContext::default())
                    .map_err(|_| invalid())?;
            }
        }
        Ok(())
    }

    fn queue_payloads(&mut self, message: DynamicMessage, depth: usize) -> Result<()> {
        let invalid = || GlossaryError("candidate.canonical_content".into());
        let mut pending = vec![(message, depth)];
        // Singular message fragments must merge before their Any payload gets an interpretation.
        while let Some((mut message, depth)) = pending.pop() {
            require(depth <= MAX_METADATA_DEPTH, "candidate.metadata_limit")?;
            if message.descriptor().full_name() == "google.protobuf.Any" {
                let Some(Value::String(type_url)) = message.take_field_by_name("type_url") else {
                    return Err(invalid());
                };
                let value = match message.take_field_by_name("value") {
                    Some(Value::Bytes(value)) => value,
                    None => Bytes::new(),
                    _ => return Err(invalid()),
                };
                self.payloads.push((type_url, value, depth + 1));
            } else {
                for (_, value) in message.take_fields() {
                    metadata_children(&mut pending, value, depth + 1);
                }
                for (_, value) in message.take_extensions() {
                    metadata_children(&mut pending, value, depth + 1);
                }
            }
        }
        Ok(())
    }
}

fn metadata_children(pending: &mut Vec<(DynamicMessage, usize)>, value: Value, depth: usize) {
    match value {
        Value::Message(message) => pending.push((message, depth)),
        Value::List(values) => {
            for value in values {
                if let Value::Message(message) = value {
                    pending.push((message, depth));
                }
            }
        }
        Value::Map(values) => {
            for value in values.into_values() {
                if let Value::Message(message) = value {
                    pending.push((message, depth));
                }
            }
        }
        _ => {}
    }
}

fn metadata_group(tag: u32, bytes: &mut Bytes, depth: usize) -> Result<Bytes> {
    let invalid = || GlossaryError("candidate.canonical_content".into());
    let original = bytes.clone();
    let mut groups = vec![tag];
    require(depth <= MAX_METADATA_DEPTH, "candidate.metadata_limit")?;
    while bytes.has_remaining() {
        let end = original.len() - bytes.len();
        let (tag, wire) = decode_key(bytes).map_err(|_| invalid())?;
        match wire {
            WireType::EndGroup => {
                require(groups.pop() == Some(tag), "candidate.canonical_content")?;
                if groups.is_empty() {
                    return Ok(original.slice(..end));
                }
            }
            WireType::StartGroup => {
                require(
                    depth + groups.len() <= MAX_METADATA_DEPTH,
                    "candidate.metadata_limit",
                )?;
                groups.push(tag);
            }
            _ => skip_field(wire, tag, bytes, DecodeContext::default()).map_err(|_| invalid())?,
        }
    }
    Err(invalid())
}

fn verify_metadata_identity(term: &pb::Term, pool: &TranscodePool) -> Result<()> {
    let mut work = ReviewMetadata {
        pool,
        pending: Vec::new(),
        payloads: Vec::new(),
        decoded_bytes: 0,
        messages: 0,
    };
    for metadata in &term.metadata {
        work.decode(
            &metadata.type_url,
            Bytes::copy_from_slice(&metadata.value),
            0,
        )?;
    }
    while let Some((type_url, bytes, depth)) = work.payloads.pop() {
        work.decode(&type_url, bytes, depth)?;
    }
    Ok(())
}

#[must_use]
pub fn review_is_current(term: &pb::Term, owner_uid: &str) -> bool {
    TranscodePool::builtin().is_ok_and(|pool| review_is_current_with_pool(term, owner_uid, &pool))
}

#[must_use]
pub fn review_is_current_with_pool(term: &pb::Term, owner_uid: &str, pool: &TranscodePool) -> bool {
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
        && candidate_digest_with_pool(term, pool).ok().as_ref() == review.content_digest.as_ref()
}

#[must_use]
pub fn review_is_accepted(term: &pb::Term, owner_uid: &str) -> bool {
    TranscodePool::builtin().is_ok_and(|pool| review_is_accepted_with_pool(term, owner_uid, &pool))
}

#[must_use]
pub fn review_is_accepted_with_pool(
    term: &pb::Term,
    owner_uid: &str,
    pool: &TranscodePool,
) -> bool {
    term.glossary
        .as_ref()
        .and_then(|value| value.review.as_ref())
        .is_some_and(|review| {
            review.disposition == pb::GlossaryReviewDisposition::Accepted as i32
                && review_is_current_with_pool(term, owner_uid, pool)
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

/// Owns the immutable source so its computed digest stays bound to every citation check.
#[derive(Debug)]
pub struct EvidenceDocument {
    entity: pb::Entity,
    content_digest: Result<pb::ContentDigest>,
}

impl EvidenceDocument {
    #[must_use]
    pub fn new(entity: pb::Entity) -> Self {
        let content_digest = retained_document_bytes(&entity)
            .map(crate::document_catalog_context::bytes_content_digest);
        Self {
            entity,
            content_digest,
        }
    }

    #[must_use]
    pub fn entity(&self) -> &pb::Entity {
        &self.entity
    }

    pub fn content_digest(&self) -> Result<&pb::ContentDigest> {
        self.content_digest.as_ref().map_err(Clone::clone)
    }

    pub fn verify(&self, evidence: &pb::GlossaryEvidence) -> Result<()> {
        verify_evidence_identity(evidence, &self.entity)?;
        let content_digest = self.content_digest()?;
        verify_evidence_content(
            evidence,
            retained_document_bytes(&self.entity)?,
            content_digest,
        )
    }
}

fn verify_evidence_identity(evidence: &pb::GlossaryEvidence, entity: &pb::Entity) -> Result<()> {
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
    Ok(())
}

fn verify_evidence_content(
    evidence: &pb::GlossaryEvidence,
    bytes: &[u8],
    content_digest: &pb::ContentDigest,
) -> Result<()> {
    require(
        evidence.content_digest.as_ref() == Some(content_digest),
        "evidence.content_digest",
    )?;
    let content = std::str::from_utf8(bytes).map_err(|_| GlossaryError("evidence.utf8".into()))?;
    require(content.contains(&evidence.quote), "evidence.quote")
}

pub fn verify_evidence(evidence: &pb::GlossaryEvidence, entity: &pb::Entity) -> Result<()> {
    verify_evidence_identity(evidence, entity)?;
    let bytes = retained_document_bytes(entity)?;
    let digest = crate::document_catalog_context::bytes_content_digest(bytes);
    verify_evidence_content(evidence, bytes, &digest)
}

pub fn retarget_evidence(
    term: &mut pb::Term,
    from_id: &pb::Id,
    target: &pb::Entity,
) -> Result<Vec<String>> {
    validate_term(term)?;
    let Some(glossary) = &term.glossary else {
        return Ok(Vec::new());
    };
    if crate::entity_id(target) == Some(from_id)
        || !glossary.evidence.iter().any(|evidence| {
            evidence
                .document
                .as_ref()
                .and_then(|reference| reference.id.as_ref())
                == Some(from_id)
        })
    {
        return Ok(Vec::new());
    }
    require(
        !glossary.review.as_ref().is_some_and(|review| {
            review.disposition == pb::GlossaryReviewDisposition::Accepted as i32
        }),
        "evidence.accepted_retarget",
    )?;
    let to_id =
        crate::entity_id(target).ok_or_else(|| GlossaryError("evidence.document".into()))?;
    exact(Some(to_id), "evidence.document")?;
    let owner_uid = target
        .system
        .as_ref()
        .map(|system| system.uid.as_str())
        .unwrap_or_default();
    text(owner_uid, 128, "evidence.owner_uid")?;
    let content_digest =
        crate::document_catalog_context::bytes_content_digest(retained_document_bytes(target)?);
    let mut updated = term.clone();
    let mut rewritten = Vec::new();
    if let Some(glossary) = &mut updated.glossary {
        for (index, evidence) in glossary.evidence.iter_mut().enumerate() {
            if evidence
                .document
                .as_ref()
                .and_then(|reference| reference.id.as_ref())
                != Some(from_id)
            {
                continue;
            }
            evidence.document = Some(pb::DocumentRef {
                id: Some(to_id.clone()),
            });
            evidence.owner_uid = owner_uid.into();
            evidence.content_digest = Some(content_digest.clone());
            verify_evidence(evidence, target)?;
            rewritten.push(format!("glossary.evidence[{index}].document"));
        }
    }
    validate_term(&updated)?;
    *term = updated;
    Ok(rewritten)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageTextKind {
    Prose,
    Code,
}

pub fn check_code_usage(
    terms: &[pb::Term],
    audience: &str,
    at: &AuthoredDate,
    text: &str,
    exclusions: &[TextRange],
    skip_quoted: bool,
) -> Result<Vec<UsageFinding>> {
    check_usage_in(
        terms,
        audience,
        at,
        text,
        exclusions,
        skip_quoted,
        UsageTextKind::Code,
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
    check_usage_in(
        terms,
        audience,
        at,
        text,
        exclusions,
        skip_quoted,
        UsageTextKind::Prose,
    )
}

fn check_usage_in(
    terms: &[pb::Term],
    audience: &str,
    at: &AuthoredDate,
    text: &str,
    exclusions: &[TextRange],
    skip_quoted: bool,
    kind: UsageTextKind,
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
        ignored.extend(quoted_ranges(text, kind)?);
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
    let (folded, mapping) = folded_with_offsets(text, kind);
    let mut findings = Vec::new();
    for term in terms {
        validate_term(term)?;
        let Some(entry) = &term.glossary else {
            continue;
        };
        if entry.audience != audience {
            continue;
        }
        let period_reason = entry
            .usage_period
            .as_ref()
            .map(|period| usage_period_reason(period, at))
            .transpose()?
            .flatten();
        let (usage, preferred_term, rationale) = if let Some(reason) = period_reason {
            (
                pb::GlossaryUsage::Prohibited,
                entry
                    .policy
                    .as_ref()
                    .and_then(|policy| policy.preferred_term.clone()),
                reason,
            )
        } else {
            let Some(policy) = &entry.policy else {
                continue;
            };
            if policy.usage == pb::GlossaryUsage::Allowed as i32 || !policy_active(policy, at)? {
                continue;
            }
            (
                pb::GlossaryUsage::try_from(policy.usage)
                    .map_err(|_| GlossaryError("policy.usage".into()))?,
                policy.preferred_term.clone(),
                policy.rationale.clone(),
            )
        };
        let (pattern, _) = folded_with_offsets(term.title.trim(), kind);
        let pattern = pattern.trim();
        if pattern.is_empty() {
            continue;
        }
        let mut search_from = 0;
        while let Some(relative_start) = folded[search_from..].find(pattern) {
            let start = search_from + relative_start;
            let end = start + pattern.len();
            search_from = folded[start..]
                .chars()
                .next()
                .map_or(folded.len(), |character| start + character.len_utf8());
            if start == end
                || (start > 0 && mapping[start - 1] == mapping[start])
                || (end < mapping.len() && mapping[end - 1] == mapping[end])
            {
                continue;
            }
            let range = TextRange {
                start: mapping[start].start,
                end: mapping[end - 1].end,
            };
            if !word_boundary(folded[..start].chars().next_back())
                || !word_boundary(folded[end..].chars().next())
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
                usage,
                preferred_term: preferred_term.clone(),
                rationale: rationale.clone(),
            });
            search_from = end;
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
                (
                    GlossaryDifferenceKind::SameMeaning,
                    "Different term labels have equal normalized definition text; review a possible synonym.",
                )
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

fn folded_with_offsets(value: &str, kind: UsageTextKind) -> (String, Vec<TextRange>) {
    let characters: Vec<_> = value.char_indices().collect();
    let mut folded = String::new();
    let mut mapping: Vec<TextRange> = Vec::new();
    for (index, &(start, character)) in characters.iter().enumerate() {
        let range = TextRange {
            start,
            end: start + character.len_utf8(),
        };
        if character.is_whitespace() || (kind == UsageTextKind::Code && character == '_') {
            separator(&mut folded, &mut mapping, range);
            continue;
        }
        if kind == UsageTextKind::Code && character.is_uppercase() {
            let previous = index
                .checked_sub(1)
                .and_then(|index| characters.get(index))
                .map(|(_, character)| *character);
            let next = characters.get(index + 1).map(|(_, character)| *character);
            if previous.is_some_and(|previous| {
                previous.is_lowercase()
                    || previous.is_numeric()
                    || (previous.is_uppercase() && next.is_some_and(char::is_lowercase))
            }) {
                separator(&mut folded, &mut mapping, TextRange { start, end: start });
            }
        }
        for character in character.to_lowercase() {
            folded.push(character);
            mapping.extend(std::iter::repeat_n(range, character.len_utf8()));
        }
    }
    (folded, mapping)
}

fn separator(folded: &mut String, mapping: &mut Vec<TextRange>, range: TextRange) {
    if folded.ends_with(' ') {
        if let Some(previous) = mapping.last_mut() {
            previous.end = range.end;
        }
    } else {
        folded.push(' ');
        mapping.push(range);
    }
}

fn closed_code_quote_ahead(value: &str, index: usize) -> bool {
    let mut characters = value[index + 1..].chars().peekable();
    let mut previous = None;
    let mut escaped = false;
    while let Some(character) = characters.next() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if matches!(character, '\n' | '\r' | '"' | '`') {
            return false;
        } else if character == '\''
            && (word_boundary(previous) || word_boundary(characters.peek().copied()))
        {
            let next = characters.peek().copied();
            return !next.is_some_and(|character| {
                unicode_ident::is_xid_start(character) || matches!(character, '_' | '\\')
            }) && characters.clone().nth(1) != Some('\'');
        }
        previous = Some(character);
    }
    false
}

fn rust_lifetime_at(value: &str, index: usize) -> bool {
    let tail = &value[index + 1..];
    let tail = tail.strip_prefix("r#").unwrap_or(tail);
    if !tail
        .chars()
        .next()
        .is_some_and(|character| unicode_ident::is_xid_start(character) || character == '_')
    {
        return false;
    }
    let identifier_end = tail
        .char_indices()
        .find(|(_, character)| !unicode_ident::is_xid_continue(*character))
        .map_or(tail.len(), |(index, _)| index);
    let suffix = &tail[identifier_end..];
    if suffix.starts_with('\'') || closed_code_quote_ahead(value, index) {
        return false;
    }
    let prefix = value[..index].trim_end();
    let suffix = suffix.trim_start();
    let parameter_end =
        suffix.is_empty() || suffix.starts_with([',', '>', '+', '{', ';', ')', ']']);
    let label = suffix.strip_prefix(':').is_some_and(|body| {
        let body = body.trim_start();
        ["loop ", "loop{", "while ", "for ", "{"]
            .iter()
            .any(|prefix| body.starts_with(prefix))
    });
    prefix.ends_with('&')
        || (prefix.ends_with(['<', '+', ':', ',']) && parameter_end)
        || ((prefix.ends_with('<') || prefix.split_whitespace().next_back() == Some("where"))
            && suffix.starts_with(':'))
        || label
        || matches!(
            prefix.split_whitespace().next_back(),
            Some("break" | "continue")
        )
        || (prefix.is_empty() && suffix.is_empty())
}

#[derive(Clone, Copy)]
enum Quote {
    Escaped { delimiter: char, start: usize },
    RustRaw { hashes: usize, start: usize },
}

fn rust_raw_quote_at(value: &str, index: usize) -> Option<Quote> {
    let hashes = value[..index]
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'#')
        .count();
    let marker = index.checked_sub(hashes + 1)?;
    if value.as_bytes()[marker] != b'r' {
        return None;
    }
    let start = if marker > 0 && matches!(value.as_bytes()[marker - 1], b'b' | b'c') {
        marker - 1
    } else {
        marker
    };
    if value[..start]
        .chars()
        .next_back()
        .is_some_and(unicode_ident::is_xid_continue)
    {
        return None;
    }
    Some(Quote::RustRaw { hashes, start })
}

fn quoted_ranges(value: &str, kind: UsageTextKind) -> Result<Vec<TextRange>> {
    let mut ranges = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut previous = None;
    let mut characters = value.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if let Some(Quote::RustRaw { hashes, start }) = quote {
            let end = index + 1 + hashes;
            if character == '"'
                && value
                    .as_bytes()
                    .get(index + 1..end)
                    .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
            {
                ranges.push(TextRange { start, end });
                quote = None;
                for _ in 0..hashes {
                    characters.next();
                }
            }
            previous = Some(character);
            continue;
        }
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
        if quote.is_none()
            && character == '\''
            && kind == UsageTextKind::Code
            && rust_lifetime_at(value, index)
        {
            previous = Some(character);
            continue;
        }
        match quote {
            Some(Quote::Escaped { delimiter, start })
                if character == delimiter
                    && (delimiter != '\''
                        || word_boundary(previous)
                        || word_boundary(characters.peek().map(|(_, character)| *character))) =>
            {
                ranges.push(TextRange {
                    start,
                    end: index + character.len_utf8(),
                });
                quote = None;
            }
            None if matches!(character, '"' | '`')
                || (character == '\'' && word_boundary(previous)) =>
            {
                let raw_quote = if kind == UsageTextKind::Code && character == '"' {
                    rust_raw_quote_at(value, index)
                } else {
                    None
                };
                quote = Some(raw_quote.unwrap_or(Quote::Escaped {
                    delimiter: character,
                    start: index,
                }));
            }
            _ => {}
        }
        previous = Some(character);
    }
    require(quote.is_none(), "usage.unclosed_quote")?;
    Ok(ranges)
}
