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
fn draft_evidence_retarget_repins_exact_target_without_touching_other_citations() {
    let mut target = document();
    let Some(entity::Kind::Document(value)) = &mut target.kind else {
        panic!("document expected");
    };
    value.id = Some(id("store", "replacement-source"));
    let content = format!("Updated context: {SOURCE}");
    value.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
        content: content.clone(),
        ..Default::default()
    }));
    target.system.as_mut().unwrap().uid = TERM_UID.into();
    let mut candidate = term();
    let mut second = evidence();
    second.locator = "paragraph:2".into();
    second.quote = "purchase".into();
    let mut existing_target = evidence();
    existing_target.document.as_mut().unwrap().id = Some(id("store", "replacement-source"));
    existing_target.owner_uid = TERM_UID.into();
    existing_target.content_digest =
        Some(trogon_atlas_core::specification::content_digest(&content));
    existing_target.locator = "paragraph:3".into();
    candidate.glossary.as_mut().unwrap().evidence =
        vec![evidence(), second, existing_target.clone()];
    let original = candidate.clone();
    assert_eq!(
        glossary::retarget_evidence(&mut candidate, &id("store", "source"), &target,).unwrap(),
        vec![
            "glossary.evidence[0].document",
            "glossary.evidence[1].document"
        ]
    );
    for item in &candidate.glossary.as_ref().unwrap().evidence {
        glossary::verify_evidence(item, &target).unwrap();
    }
    assert_eq!(
        candidate.glossary.as_ref().unwrap().evidence[2],
        existing_target
    );
    for index in 0..2 {
        assert_eq!(
            candidate.glossary.as_ref().unwrap().evidence[index].quote,
            original.glossary.as_ref().unwrap().evidence[index].quote
        );
        assert_eq!(
            candidate.glossary.as_ref().unwrap().evidence[index].locator,
            original.glossary.as_ref().unwrap().evidence[index].locator
        );
    }
    assert_ne!(
        glossary::candidate_digest(&candidate).unwrap(),
        glossary::candidate_digest(&original).unwrap()
    );
}

#[test]
fn evidence_retarget_refuses_missing_quote_without_partial_mutation() {
    let mut candidate = term();
    let mut second = evidence();
    second.locator = "paragraph:2".into();
    second.quote = "store.".into();
    glossary::verify_evidence(&second, &document()).unwrap();
    candidate.glossary.as_mut().unwrap().evidence.push(second);
    let original = candidate.clone();
    let mut target = document();
    let Some(entity::Kind::Document(value)) = &mut target.kind else {
        panic!("document expected");
    };
    value.id = Some(id("store", "replacement-source"));
    let Some(pb::document::Body::Retained(body)) = &mut value.body else {
        panic!("retained text expected");
    };
    body.content = SOURCE.trim_end_matches('.').into();
    target.system.as_mut().unwrap().uid = TERM_UID.into();
    assert_eq!(
        glossary::retarget_evidence(&mut candidate, &id("store", "source"), &target),
        Err(glossary::GlossaryError("evidence.quote".into()))
    );
    assert_eq!(candidate, original);
}

#[test]
fn evidence_retarget_corrects_negative_review_without_changing_receipt() {
    for disposition in [
        pb::GlossaryReviewDisposition::Rejected,
        pb::GlossaryReviewDisposition::NeedsChanges,
    ] {
        let mut candidate = term();
        candidate.glossary.as_mut().unwrap().review = Some(pb::GlossaryReview {
            disposition: disposition as i32,
            actor: "expert".into(),
            reviewed_at: "2026-11-12T12:00:00Z".into(),
            content_digest: Some(glossary::candidate_digest(&candidate).unwrap()),
            owner_uid: TERM_UID.into(),
            reason: "The definition and source were reviewed.".into(),
        });
        assert!(glossary::review_is_current(&candidate, TERM_UID));
        let review = candidate.glossary.as_ref().unwrap().review.clone();
        let mut target = document();
        let Some(entity::Kind::Document(value)) = &mut target.kind else {
            panic!("document expected");
        };
        value.id = Some(id("store", "replacement-source"));
        assert_eq!(
            glossary::retarget_evidence(&mut candidate, &id("store", "source"), &target).unwrap(),
            vec!["glossary.evidence[0].document"]
        );
        glossary::verify_evidence(&candidate.glossary.as_ref().unwrap().evidence[0], &target)
            .unwrap();
        assert_eq!(candidate.glossary.as_ref().unwrap().review, review);
        assert!(!glossary::review_is_current(&candidate, TERM_UID));
    }
}

#[test]
fn evidence_retarget_refuses_accepted_meaning_even_with_stale_review_atomically() {
    for stale in [false, true] {
        let mut candidate = term();
        candidate.glossary.as_mut().unwrap().review = Some(pb::GlossaryReview {
            disposition: pb::GlossaryReviewDisposition::Accepted as i32,
            actor: "expert".into(),
            reviewed_at: "2026-11-12T12:00:00Z".into(),
            content_digest: Some(glossary::candidate_digest(&candidate).unwrap()),
            owner_uid: TERM_UID.into(),
            reason: "The definition and source were reviewed.".into(),
        });
        if stale {
            candidate.doc.push_str(" A changed draft.");
        }
        assert_eq!(glossary::review_is_current(&candidate, TERM_UID), !stale);
        let original = candidate.clone();
        let mut target = document();
        let Some(entity::Kind::Document(value)) = &mut target.kind else {
            panic!("document expected");
        };
        value.id = Some(id("store", "replacement-source"));
        assert_eq!(
            glossary::retarget_evidence(&mut candidate, &id("store", "source"), &target),
            Err(glossary::GlossaryError("evidence.accepted_retarget".into()))
        );
        assert_eq!(candidate, original);
    }
}

#[test]
fn evidence_retarget_refuses_invalid_target_identity_content_or_namespace_atomically() {
    let mut replacement = document();
    let Some(entity::Kind::Document(value)) = &mut replacement.kind else {
        panic!("document expected");
    };
    value.id = Some(id("store", "replacement-source"));
    let mut wrong_namespace = replacement.clone();
    let Some(entity::Kind::Document(value)) = &mut wrong_namespace.kind else {
        panic!("document expected");
    };
    value.id.as_mut().unwrap().namespace = "other".into();
    let mut missing_owner = replacement.clone();
    missing_owner.system = None;
    let mut empty_owner = replacement.clone();
    empty_owner.system.as_mut().unwrap().uid.clear();
    let mut invalid_id = replacement.clone();
    let Some(entity::Kind::Document(value)) = &mut invalid_id.kind else {
        panic!("document expected");
    };
    value.id.as_mut().unwrap().version = 0;
    let mut non_utf8 = replacement;
    let Some(entity::Kind::Document(value)) = &mut non_utf8.kind else {
        panic!("document expected");
    };
    value.body = Some(pb::document::Body::RetainedContent(pb::RetainedContent {
        content: vec![0xff],
        ..Default::default()
    }));
    for target in [
        wrong_namespace,
        missing_owner,
        empty_owner,
        invalid_id,
        non_utf8,
        pb::Entity {
            kind: Some(entity::Kind::Term(term())),
            system: document().system,
        },
    ] {
        let mut candidate = term();
        let original = candidate.clone();
        assert!(
            glossary::retarget_evidence(&mut candidate, &id("store", "source"), &target).is_err()
        );
        assert_eq!(candidate, original);
    }
}

#[test]
fn evidence_retarget_same_source_preserves_pins_and_accepted_receipt() {
    let mut candidate = term();
    candidate.glossary.as_mut().unwrap().review = Some(pb::GlossaryReview {
        disposition: pb::GlossaryReviewDisposition::Accepted as i32,
        actor: "expert".into(),
        reviewed_at: "2026-11-12T12:00:00Z".into(),
        content_digest: Some(glossary::candidate_digest(&candidate).unwrap()),
        owner_uid: TERM_UID.into(),
        reason: "The definition and source were reviewed.".into(),
    });
    let original = candidate.clone();
    let mut changed_source = document();
    changed_source.system.as_mut().unwrap().uid = TERM_UID.into();
    assert_eq!(
        glossary::retarget_evidence(&mut candidate, &id("store", "source"), &changed_source,)
            .unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(candidate, original);
}

#[test]
fn evidence_retarget_ignores_unrelated_references() {
    let mut candidate = term();
    let original = candidate.clone();
    assert_eq!(
        glossary::retarget_evidence(
            &mut candidate,
            &id("store", "unrelated-source"),
            &document(),
        )
        .unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(candidate, original);
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
fn quoted_exclusions_keep_word_internal_apostrophes_inside_the_delimiters() {
    let mut candidate = term();
    candidate.title = "Consignment".into();
    candidate.glossary.as_mut().unwrap().policy = Some(policy());
    for check in [glossary::check_usage, glossary::check_code_usage] {
        for text in [
            "'Don't use consignment' consignment",
            "'Café's consignment' consignment",
            r"'Don\'t use consignment' consignment",
            "'consignment' consignment",
            "\"Don't use consignment\" consignment",
            "`Don't use consignment` consignment",
            "Don't use consignment",
        ] {
            let findings = check(
                std::slice::from_ref(&candidate),
                "customer",
                &date("2026-11-12"),
                text,
                &[],
                true,
            )
            .unwrap();
            assert_eq!(findings.len(), 1, "{text}");
            assert_eq!(
                findings[0].range.start(),
                text.rfind("consignment").unwrap()
            );
            assert_eq!(
                &text[findings[0].range.start()..findings[0].range.end()],
                "consignment"
            );
        }
    }
}

#[test]
fn rejected_usage_matches_do_not_skip_overlapping_valid_occurrences() {
    for check in [glossary::check_usage, glossary::check_code_usage] {
        for (title, text, exclusions) in [
            ("end to end", "backend to end to end", vec![]),
            ("é é", "pré é é", vec![]),
            (
                "end to end",
                "end to end to end",
                vec![glossary::TextRange::new(0, 1).unwrap()],
            ),
        ] {
            let mut candidate = term();
            candidate.title = title.into();
            candidate.glossary.as_mut().unwrap().policy = Some(policy());
            let findings = check(
                &[candidate],
                "customer",
                &date("2026-11-12"),
                text,
                &exclusions,
                false,
            )
            .unwrap();
            assert_eq!(findings.len(), 1, "{text}");
            assert_eq!(findings[0].range.start(), text.rfind(title).unwrap());
            assert_eq!(findings[0].matched, title);
        }
    }
}

#[test]
fn code_quote_exclusions_preserve_rust_lifetimes_and_character_literals() {
    let mut candidate = term();
    candidate.glossary.as_mut().unwrap().policy = Some(policy());
    for text in [
        "fn f<'a>(purchase: &'a str) {}",
        "fn f<'a: 'b, 'b>(purchase: &'a str) {}",
        "fn f<'a\u{301}>(purchase: &'a\u{301} str) {}",
        "fn f<'r#scope>(purchase: &'r#scope str) {}",
        "fn f(purchase: &'static str) {}",
        "fn f(purchase: &'_ str) {}",
        "'outer: loop { purchase(); break 'outer; }",
        "let marker = 'p'; let purchase = \"purchase\";",
        "let marker = 'é'; purchase();",
        "const label = 'purchase copy'; purchase();",
        "const label = { title: 'purchase copy' }; purchase();",
        "const label = prefix + 'purchase copy'; purchase();",
        "const label = 'purchase: copy'; purchase();",
        "const label = 'purchase: loop {}'; purchase();",
        "const label = prefix + 'purchase; copy'; purchase();",
        "const label = { title: 'purchase, copy' }; purchase();",
        "const label = 'purchase: loop { it\\'s valid }'; purchase();",
        "let result = 'outer: loop { purchase(); break 'outer; };",
        "let result = 'outer: loop { let ch = 'x'; purchase(); break 'outer; };",
        "let result = 'outer: loop { let ch = ';'; purchase(); break 'outer; };",
        "let result = 'outer: loop { let note = \"an ' aside\"; purchase(); break 'outer; };",
        "type Handler = dyn Fn(purchase: &str) + 'static;",
    ] {
        let findings = glossary::check_code_usage(
            std::slice::from_ref(&candidate),
            "customer",
            &date("2026-11-12"),
            text,
            &[],
            true,
        )
        .unwrap();
        assert_eq!(findings.len(), 1, "{text}");
        let expected = if text.starts_with("const label") {
            text.rfind("purchase").unwrap()
        } else {
            text.find("purchase").unwrap()
        };
        assert_eq!(findings[0].range.start(), expected);
        assert_eq!(findings[0].matched, "purchase");
    }
    assert_eq!(
        glossary::check_code_usage(
            std::slice::from_ref(&candidate),
            "customer",
            &date("2026-11-12"),
            "'static",
            &[],
            true,
        )
        .unwrap(),
        Vec::new()
    );
    assert!(glossary::check_usage(
        &[candidate],
        "customer",
        &date("2026-11-12"),
        "An unmatched 'static Purchase",
        &[],
        true,
    )
    .is_err());
}

#[test]
fn apostrophes_do_not_make_an_unclosed_single_quoted_exclusion_valid() {
    let mut candidate = term();
    candidate.glossary.as_mut().unwrap().policy = Some(policy());
    for check in [glossary::check_usage, glossary::check_code_usage] {
        assert!(check(
            std::slice::from_ref(&candidate),
            "customer",
            &date("2026-11-12"),
            "'Don't use Purchase",
            &[],
            true,
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

#[test]
fn semantically_equal_metadata_encodings_preserve_review_identity() {
    use prost::Message;

    let struct_field = |key: &str, value: &str| {
        let mut message = prost_types::Struct::default();
        message.fields.insert(
            key.into(),
            prost_types::Value {
                kind: Some(prost_types::value::Kind::StringValue(value.into())),
            },
        );
        message.encode_to_vec()
    };
    let mut forward_struct = struct_field("a", "1");
    forward_struct.extend(struct_field("b", "2"));
    let mut reversed_struct = struct_field("b", "2");
    reversed_struct.extend(struct_field("a", "1"));

    let url = pb::LinkAnnotation {
        url: "https://example.com/vocabulary".into(),
        ..Default::default()
    }
    .encode_to_vec();
    let title = pb::LinkAnnotation {
        title: "Vocabulary source".into(),
        ..Default::default()
    }
    .encode_to_vec();
    let mut forward_link = url.clone();
    forward_link.extend(&title);
    let mut reversed_link = title;
    reversed_link.extend(url);

    for (type_name, forward, reversed) in [
        ("google.protobuf.Struct", forward_struct, reversed_struct),
        (
            "trogonatlas.annotation.v1alpha1.LinkAnnotation",
            forward_link,
            reversed_link,
        ),
    ] {
        assert_ne!(forward, reversed);
        let with_payload = |value| {
            let mut candidate = term();
            candidate.metadata.push(prost_types::Any {
                type_url: format!("type.googleapis.com/{type_name}"),
                value,
            });
            candidate
        };
        let original = with_payload(forward);
        let mut equivalent = with_payload(reversed);
        let wrap = |value| pb::Entity {
            kind: Some(entity::Kind::Term(value)),
            ..Default::default()
        };
        assert!(trogon_atlas_core::semantically_equal(
            &wrap(original.clone()),
            &wrap(equivalent.clone())
        ));
        assert_eq!(
            trogon_atlas_core::entity_content_hash(&wrap(original.clone())).unwrap(),
            trogon_atlas_core::entity_content_hash(&wrap(equivalent.clone())).unwrap()
        );
        let digest = glossary::candidate_digest(&original).unwrap();
        assert_eq!(glossary::candidate_digest(&equivalent).unwrap(), digest);
        equivalent.glossary.as_mut().unwrap().review = Some(pb::GlossaryReview {
            disposition: pb::GlossaryReviewDisposition::Accepted as i32,
            actor: "expert".into(),
            reviewed_at: "2026-11-12T12:00:00Z".into(),
            content_digest: Some(digest),
            owner_uid: TERM_UID.into(),
            reason: "Exact definition and evidence reviewed.".into(),
        });
        assert!(glossary::review_is_accepted(&equivalent, TERM_UID));
    }
}

#[test]
fn unrenderable_metadata_has_no_glossary_review_identity() {
    let mut candidate = term();
    candidate.metadata.push(prost_types::Any {
        type_url: "type.googleapis.com/retired.package.Gone".into(),
        value: vec![1],
    });
    assert!(glossary::candidate_digest(&candidate).is_err());
}

#[test]
fn prose_phrase_matching_normalizes_whitespace_without_matching_identifiers() {
    let mut term = term();
    term.title = "Ready to ship".into();
    term.glossary.as_mut().unwrap().policy = Some(pb::GlossaryUsagePolicy {
        usage: pb::GlossaryUsage::Prohibited as i32,
        rationale: "Use awaiting carrier handoff in this audience.".into(),
        ..Default::default()
    });
    let text = "é Ready  to\nship; ready_to_ship readyToShip already to ship";
    let findings =
        glossary::check_usage(&[term], "customer", &date("2026-11-13"), text, &[], false).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].matched, "Ready  to\nship");
    assert_eq!(
        &text[findings[0].range.start()..findings[0].range.end()],
        "Ready  to\nship"
    );
}

#[test]
fn usage_period_enforces_own_half_open_window_with_allowed_or_absent_policy() {
    for policy in [
        None,
        Some(pb::GlossaryUsagePolicy {
            usage: pb::GlossaryUsage::Allowed as i32,
            ..Default::default()
        }),
    ] {
        let mut term = term();
        let entry = term.glossary.as_mut().unwrap();
        entry.policy = policy;
        entry.usage_period = Some(pb::GlossaryUsagePeriod {
            valid_from: Some(pb::CalendarDate {
                value: "2026-11-01".into(),
            }),
            valid_until: Some(pb::CalendarDate {
                value: "2026-12-01".into(),
            }),
        });
        for (on, count) in [
            ("2026-10-31", 1),
            ("2026-11-01", 0),
            ("2026-11-30", 0),
            ("2026-12-01", 1),
        ] {
            let findings = glossary::check_usage(
                std::slice::from_ref(&term),
                "customer",
                &date(on),
                "Purchase",
                &[],
                false,
            )
            .unwrap();
            assert_eq!(findings.len(), count, "{on}");
            if count > 0 {
                assert_eq!(findings[0].usage, pb::GlossaryUsage::Prohibited);
                assert_ne!(findings[0].rationale, "");
            }
        }
    }
}

#[test]
fn usage_period_refuses_empty_invalid_and_reversed_intervals_and_binds_review() {
    let mut candidate = term();
    let digest = glossary::candidate_digest(&candidate).unwrap();
    for period in [
        pb::GlossaryUsagePeriod::default(),
        pb::GlossaryUsagePeriod {
            valid_from: Some(pb::CalendarDate {
                value: "bad-date".into(),
            }),
            valid_until: None,
        },
        pb::GlossaryUsagePeriod {
            valid_from: Some(pb::CalendarDate {
                value: "2026-12-01".into(),
            }),
            valid_until: Some(pb::CalendarDate {
                value: "2026-11-01".into(),
            }),
        },
    ] {
        candidate.glossary.as_mut().unwrap().usage_period = Some(period);
        assert!(glossary::validate_term(&candidate).is_err());
    }
    candidate.glossary.as_mut().unwrap().usage_period = Some(pb::GlossaryUsagePeriod {
        valid_from: Some(pb::CalendarDate {
            value: "2026-11-01".into(),
        }),
        valid_until: None,
    });
    assert!(glossary::validate_term(&candidate).is_ok());
    assert_ne!(glossary::candidate_digest(&candidate).unwrap(), digest);
}

#[test]
fn code_matching_segments_identifiers_acronyms_and_retains_original_utf8_ranges() {
    let mut term = term();
    term.title = "HTTP booking".into();
    term.glossary.as_mut().unwrap().policy = Some(pb::GlossaryUsagePolicy {
        usage: pb::GlossaryUsage::Prohibited as i32,
        rationale: "Use Parcel.".into(),
        ..Default::default()
    });
    let text = "é HTTPBooking httpBooking HttpBooking HTTP_BOOKING http_booking xhttp_booking \"HTTPBooking\"";
    let excluded = text.find("HttpBooking").unwrap();
    let findings = glossary::check_code_usage(
        &[term.clone()],
        "customer",
        &date("2026-11-13"),
        text,
        &[TextRange::new(excluded, excluded + "HttpBooking".len()).unwrap()],
        true,
    )
    .unwrap();
    assert_eq!(
        findings
            .iter()
            .map(|finding| finding.matched.as_str())
            .collect::<Vec<_>>(),
        vec!["HTTPBooking", "httpBooking", "HTTP_BOOKING", "http_booking"]
    );
    for finding in &findings {
        assert_eq!(
            &text[finding.range.start()..finding.range.end()],
            finding.matched
        );
    }
    assert_eq!(
        glossary::check_usage(&[term], "customer", &date("2026-11-13"), text, &[], true).unwrap(),
        Vec::new()
    );
}

#[test]
fn code_one_word_policy_matches_whole_segments_but_prose_keeps_ordinary_boundaries() {
    let mut term = term();
    term.glossary.as_mut().unwrap().policy = Some(pb::GlossaryUsagePolicy {
        usage: pb::GlossaryUsage::Prohibited as i32,
        rationale: "Use Order.".into(),
        ..Default::default()
    });
    let text = "repurchase purchase_id createPurchase Purchased purchase";
    let code = glossary::check_code_usage(
        &[term.clone()],
        "customer",
        &date("2026-11-13"),
        text,
        &[],
        false,
    )
    .unwrap();
    assert_eq!(
        code.iter()
            .map(|finding| finding.matched.as_str())
            .collect::<Vec<_>>(),
        vec!["purchase", "Purchase", "purchase"]
    );
    let prose =
        glossary::check_usage(&[term], "customer", &date("2026-11-13"), text, &[], false).unwrap();
    assert_eq!(prose.len(), 1);
    assert_eq!(prose[0].range.end(), text.len());
}
