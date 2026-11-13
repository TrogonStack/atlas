#![allow(clippy::unwrap_used)]

use trogon_atlas_core::{
    authored_date::AuthoredDate,
    glossary::{self, GlossaryDifferenceKind, TextRange},
    outbound_refs, retarget_refs,
};
use trogon_atlas_proto::{self as pb, entity};

const DOCUMENT_UID: &str = "019a0000-0000-7000-8000-000000000001";
const TERM_UID: &str = "019a0000-0000-7000-8000-000000000002";
const SOURCE: &str = "A purchase is the customer request accepted by the store.";

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn document() -> pb::Entity {
    pb::Entity {
        kind: Some(entity::Kind::Document(pb::Document {
            id: Some(id("store", "source")),
            body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
                content: SOURCE.into(),
                ..Default::default()
            })),
            ..Default::default()
        })),
        system: Some(pb::SystemMeta {
            uid: DOCUMENT_UID.into(),
            ..Default::default()
        }),
    }
}

fn evidence() -> pb::GlossaryEvidence {
    pb::GlossaryEvidence {
        document: Some(pb::DocumentRef {
            id: Some(id("store", "source")),
        }),
        owner_uid: DOCUMENT_UID.into(),
        content_digest: Some(trogon_atlas_core::specification::content_digest(SOURCE)),
        locator: "paragraph:1".into(),
        quote: "the customer request accepted by the store".into(),
    }
}

fn term() -> pb::Term {
    pb::Term {
        id: Some(id("store", "purchase")),
        title: "Purchase".into(),
        doc: "A customer request accepted by the store.".into(),
        glossary: Some(pb::GlossaryEntry {
            evidence: vec![evidence()],
            generation_revision: "prompt-v1/config-sha256-01".into(),
            audience: "customer".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn policy() -> pb::GlossaryUsagePolicy {
    pb::GlossaryUsagePolicy {
        usage: pb::GlossaryUsage::Deprecated as i32,
        preferred_term: Some(pb::TermRef {
            id: Some(id("store", "order")),
        }),
        rationale: "Use the customer-facing term Order.".into(),
        effective_from: Some(pb::CalendarDate {
            value: "2026-11-01".into(),
        }),
        effective_until: Some(pb::CalendarDate {
            value: "2026-12-01".into(),
        }),
    }
}

fn date(value: &str) -> AuthoredDate {
    value.parse().unwrap()
}

#[test]
fn generated_entries_require_bounded_evidence_and_a_contextual_definition() {
    glossary::validate_term(&pb::Term::default()).unwrap();
    let mut candidate = term();
    glossary::validate_term(&candidate).unwrap();
    candidate.doc = " \u{a0}".into();
    assert!(glossary::validate_term(&candidate).is_err());
    candidate = term();
    candidate.glossary.as_mut().unwrap().evidence.clear();
    assert!(glossary::validate_term(&candidate).is_err());
    candidate = term();
    candidate.glossary.as_mut().unwrap().evidence[0]
        .document
        .as_mut()
        .unwrap()
        .id
        .as_mut()
        .unwrap()
        .namespace = "hidden".into();
    assert!(glossary::validate_term(&candidate).is_err());
}

#[test]
fn evidence_resolves_exact_document_content_and_incarnation() {
    let source = document();
    assert_eq!(
        glossary::retained_document_bytes(&source).unwrap(),
        SOURCE.as_bytes()
    );
    glossary::verify_evidence(&evidence(), &source).unwrap();
    let mut changed = source.clone();
    changed.system.as_mut().unwrap().uid = TERM_UID.into();
    assert!(glossary::verify_evidence(&evidence(), &changed).is_err());
    let mut wrong_quote = evidence();
    wrong_quote.quote = "an invented definition".into();
    assert!(glossary::verify_evidence(&wrong_quote, &source).is_err());
    let mut wrong_digest = evidence();
    wrong_digest.content_digest.as_mut().unwrap().sha256 = "0".repeat(64);
    assert!(glossary::verify_evidence(&wrong_digest, &source).is_err());
    let mut wrong_key = evidence();
    wrong_key
        .document
        .as_mut()
        .unwrap()
        .id
        .as_mut()
        .unwrap()
        .version = 2;
    assert!(glossary::verify_evidence(&wrong_key, &source).is_err());
}

#[test]
fn candidate_digest_binds_meaning_but_not_schedule_or_server_review() {
    let original = term();
    let digest = glossary::candidate_digest(&original).unwrap();
    let mut candidate = original.clone();
    candidate.glossary.as_mut().unwrap().next_review_on = Some(pb::CalendarDate {
        value: "2027-01-01".into(),
    });
    candidate.glossary.as_mut().unwrap().review = Some(pb::GlossaryReview {
        disposition: pb::GlossaryReviewDisposition::Accepted as i32,
        actor: "expert".into(),
        reviewed_at: "2026-11-12T12:00:00Z".into(),
        content_digest: Some(digest.clone()),
        owner_uid: TERM_UID.into(),
        reason: "Reviewed against the supplied source.".into(),
    });
    assert_eq!(glossary::candidate_digest(&candidate).unwrap(), digest);
    candidate.doc.push_str(" Updated meaning.");
    assert_ne!(glossary::candidate_digest(&candidate).unwrap(), digest);
    candidate = original.clone();
    candidate.glossary.as_mut().unwrap().policy = Some(policy());
    assert_ne!(glossary::candidate_digest(&candidate).unwrap(), digest);
    candidate = original;
    candidate.glossary.as_mut().unwrap().evidence[0].locator = "paragraph:2".into();
    assert_ne!(glossary::candidate_digest(&candidate).unwrap(), digest);
}

#[test]
fn accepted_review_is_current_only_for_exact_meaning_and_owner() {
    let mut candidate = term();
    let digest = glossary::candidate_digest(&candidate).unwrap();
    candidate.glossary.as_mut().unwrap().review = Some(pb::GlossaryReview {
        disposition: pb::GlossaryReviewDisposition::Accepted as i32,
        actor: "expert".into(),
        reviewed_at: "2026-11-12T12:00:00Z".into(),
        content_digest: Some(digest),
        owner_uid: TERM_UID.into(),
        reason: "This source matches the intended definition.".into(),
    });
    assert!(glossary::review_is_current(&candidate, TERM_UID));
    assert!(glossary::review_is_accepted(&candidate, TERM_UID));
    assert!(!glossary::review_is_current(&candidate, DOCUMENT_UID));
    candidate.doc.push_str(" Meaning changed.");
    assert!(!glossary::review_is_current(&candidate, TERM_UID));
}

#[test]
fn policy_uses_explicit_calendar_interval_and_rejects_unknown_states() {
    let valid = policy();
    assert!(!glossary::policy_active(&valid, &date("2026-10-31")).unwrap());
    assert!(glossary::policy_active(&valid, &date("2026-11-01")).unwrap());
    assert!(!glossary::policy_active(&valid, &date("2026-12-01")).unwrap());
    let mut invalid = valid.clone();
    invalid.usage = 999;
    assert!(glossary::policy_active(&invalid, &date("2026-11-12")).is_err());
    invalid = valid;
    invalid.effective_until.as_mut().unwrap().value = "2026-10-01".into();
    assert!(glossary::policy_active(&invalid, &date("2026-11-12")).is_err());
}

#[test]
fn source_and_preferred_term_participate_in_reference_discovery_and_retargeting() {
    let mut candidate = term();
    candidate.glossary.as_mut().unwrap().policy = Some(policy());
    let mut owner = pb::Entity {
        kind: Some(entity::Kind::Term(candidate)),
        ..Default::default()
    };
    let references = outbound_refs(&owner);
    assert!(references.iter().any(|reference| reference.from_field
        == "glossary.evidence[0].document"
        && reference.to_kind == Some(pb::EntityKind::Document)));
    assert!(references.iter().any(|reference| reference.from_field
        == "glossary.policy.preferred_term"
        && reference.to_kind == Some(pb::EntityKind::Term)));
    assert_eq!(
        retarget_refs(
            &mut owner,
            pb::EntityKind::Document,
            &id("store", "source"),
            &id("store", "replacement-source")
        ),
        vec!["glossary.evidence[0].document"]
    );
    assert_eq!(
        retarget_refs(
            &mut owner,
            pb::EntityKind::Term,
            &id("store", "order"),
            &id("store", "preferred-order")
        ),
        vec!["glossary.policy.preferred_term"]
    );
}

#[test]
fn usage_checks_preserve_utf8_spans_audience_boundaries_and_exclusions() {
    let mut candidate = term();
    candidate.title = "Café event".into();
    candidate.glossary.as_mut().unwrap().policy = Some(policy());
    let source = "A CAFÉ EVENT happened, not preCafé event. `Café event` is historical.";
    let findings = glossary::check_usage(
        &[candidate.clone()],
        "customer",
        &date("2026-11-12"),
        source,
        &[],
        true,
    )
    .unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(
        &source[findings[0].range.start()..findings[0].range.end()],
        "CAFÉ EVENT"
    );
    assert_eq!(findings[0].matched, "CAFÉ EVENT");
    assert_eq!(findings[0].preferred_term, policy().preferred_term);
    assert_eq!(
        glossary::check_usage(
            &[candidate.clone()],
            "internal",
            &date("2026-11-12"),
            source,
            &[],
            true
        )
        .unwrap(),
        Vec::new()
    );
    assert_eq!(
        glossary::check_usage(
            &[candidate.clone()],
            "customer",
            &date("2026-12-01"),
            source,
            &[],
            true
        )
        .unwrap(),
        Vec::new()
    );
    let excluded = TextRange::new(findings[0].range.start(), findings[0].range.end()).unwrap();
    assert_eq!(
        glossary::check_usage(
            &[candidate],
            "customer",
            &date("2026-11-12"),
            source,
            &[excluded],
            true
        )
        .unwrap(),
        Vec::new()
    );
}

#[test]
fn comparison_reports_candidates_without_merging_contexts() {
    let mut left = term();
    let mut right = term();
    right.id.as_mut().unwrap().namespace = "billing".into();
    right.glossary.as_mut().unwrap().evidence[0]
        .document
        .as_mut()
        .unwrap()
        .id
        .as_mut()
        .unwrap()
        .namespace = "billing".into();
    right.doc = "A captured payment in the billing context.".into();
    let differences = glossary::compare_glossaries(&[left.clone()], &[right.clone()]).unwrap();
    assert_eq!(differences.len(), 1);
    assert_eq!(
        differences[0].kind,
        GlossaryDifferenceKind::DifferentMeaning
    );
    assert_eq!(
        differences[0]
            .left
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .namespace,
        "store"
    );
    assert_eq!(
        differences[0]
            .right
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .namespace,
        "billing"
    );
    left.doc = right.doc.clone();
    left.title = "Settlement".into();
    let differences = glossary::compare_glossaries(&[left], &[right]).unwrap();
    assert_eq!(differences[0].kind, GlossaryDifferenceKind::SameMeaning);
    assert_eq!(
        glossary::compare_glossaries(&[term()], &[]).unwrap()[0].kind,
        GlossaryDifferenceKind::MissingCounterpart
    );
}

#[test]
fn source_ownership_cannot_supply_or_clear_native_glossary_fields() {
    use trogon_atlas_core::source_integration::{
        extract_unit, replace_unit, validate_create_ownership, OwnershipUnit,
    };
    let mut owner = pb::Entity {
        kind: Some(entity::Kind::Term(term())),
        ..Default::default()
    };
    assert!(extract_unit(&owner, OwnershipUnit::WholeOwner).is_err());
    let blank = pb::Entity {
        kind: Some(entity::Kind::Term(pb::Term {
            id: Some(id("store", "purchase")),
            title: "Source title".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let whole = extract_unit(&blank, OwnershipUnit::WholeOwner).unwrap();
    assert!(replace_unit(&mut owner, OwnershipUnit::WholeOwner, &whole).is_err());
    let title = OwnershipUnit::Field(pb::SourceOwnedField::Title);
    let field = extract_unit(&blank, title).unwrap();
    replace_unit(&mut owner, title, &field).unwrap();
    let Some(entity::Kind::Term(updated)) = owner.kind.as_ref() else {
        panic!("term expected");
    };
    assert_eq!(updated.glossary, term().glossary);
    for policy in [
        pb::source_field_ownership::Policy::None(pb::SourceNoOwnership {}),
        pb::source_field_ownership::Policy::WholeOwner(pb::SourceWholeOwnerOwnership {}),
        pb::source_field_ownership::Policy::Fields(pb::SourceOwnedFields {
            fields: vec![pb::SourceOwnedField::Title as i32],
        }),
    ] {
        assert!(validate_create_ownership(
            &owner,
            &pb::SourceFieldOwnership {
                policy: Some(policy)
            }
        )
        .is_err());
    }
}

#[test]
fn malformed_quoted_exclusion_does_not_hide_unchecked_policy_usage() {
    let mut candidate = term();
    candidate.glossary.as_mut().unwrap().policy = Some(policy());
    let text = "An unmatched quote starts here: \" Purchase must still be inspected.";
    assert!(glossary::check_usage(
        &[candidate],
        "customer",
        &date("2026-11-12"),
        text,
        &[],
        true,
    )
    .is_err());
}

#[test]
fn comparison_and_usage_bound_the_total_retained_candidate_bytes() {
    let terms = (0..600)
        .map(|index| {
            let mut candidate = term();
            candidate.id.as_mut().unwrap().slug = format!("purchase-{index}");
            candidate.doc = "a".repeat(16_384);
            candidate
        })
        .collect::<Vec<_>>();
    assert!(glossary::compare_glossaries(&terms, &[]).is_err());
    assert!(
        glossary::check_usage(&terms, "customer", &date("2026-11-12"), "", &[], false,).is_err()
    );
}
