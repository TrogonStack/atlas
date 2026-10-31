//! The server's advisory verdict on landing a branch. Pure: the merge dry
//! run gathers the evidence and the binding, this module only judges them.
//! See docs/explanation/change-assessment.md.

use std::{collections::BTreeSet, fmt::Write as _, sync::OnceLock};

use pb::{
    change_assessment::{Reason, Verdict},
    finding_change::Kind,
};
use prost::Message as _;
use sha2::{Digest as _, Sha256};
use trogon_atlas_proto as pb;

use crate::{ownership::Visibility, validation};

/// Bumped whenever the rules in [`assess`] change meaning, so a verdict
/// taken under older rules never matches a newer catalogue revision.
const ASSESSMENT_RULES: &str = "change-assessment/1";

/// What the merge dry run learned about the landing.
pub enum Evidence<'a> {
    /// The branch conflicts with baseline; nothing past detection ran.
    Conflicts,
    /// The landed world fails a blocking check.
    Blocked {
        delta: Option<&'a pb::ConsequenceDelta>,
    },
    /// The landing would be admitted.
    Admissible {
        delta: Option<&'a pb::ConsequenceDelta>,
        invariants: Option<&'a pb::InvariantCheck>,
    },
}

#[must_use]
pub fn assess(evidence: &Evidence<'_>, binding: pb::AssessmentBinding) -> pb::ChangeAssessment {
    let mut reasons = BTreeSet::new();
    match evidence {
        Evidence::Conflicts => {
            reasons.insert(Reason::Conflicts);
        }
        Evidence::Blocked { delta } => {
            reasons.insert(Reason::BlockingFindings);
            delta_reasons(*delta, &binding, &mut reasons);
        }
        Evidence::Admissible { delta, invariants } => {
            delta_reasons(*delta, &binding, &mut reasons);
            if invariants.is_some_and(|check| !check.complete) {
                reasons.insert(Reason::InvariantsIncomplete);
            }
        }
    }
    let verdict = if reasons.iter().any(|reason| rejects(*reason)) {
        Verdict::Reject
    } else if reasons.is_empty() {
        Verdict::Land
    } else {
        Verdict::NeedsReview
    };
    pb::ChangeAssessment {
        verdict: verdict as i32,
        reasons: reasons.into_iter().map(|reason| reason as i32).collect(),
        binding: Some(binding),
    }
}

fn rejects(reason: Reason) -> bool {
    matches!(reason, Reason::Conflicts | Reason::BlockingFindings)
}

fn delta_reasons(
    delta: Option<&pb::ConsequenceDelta>,
    binding: &pb::AssessmentBinding,
    reasons: &mut BTreeSet<Reason>,
) {
    if binding.baseline_snapshot_id.is_empty() || binding.landed_snapshot_id.is_empty() {
        reasons.insert(Reason::Unbound);
    }
    let Some(delta) = delta else {
        reasons.insert(Reason::DeltaMissing);
        return;
    };
    if !delta.complete {
        reasons.insert(Reason::DeltaIncomplete);
    }
    for change in &delta.changes {
        match change.kind() {
            Kind::Introduced => {
                reasons.insert(Reason::FindingsIntroduced);
            }
            Kind::SeverityChanged | Kind::CarriedToSuccessor if worsens(change) => {
                reasons.insert(Reason::FindingsWorsened);
            }
            Kind::Unknown | Kind::Unspecified => {
                reasons.insert(Reason::FindingsUnknown);
            }
            Kind::SeverityChanged
            | Kind::CarriedToSuccessor
            | Kind::Resolved
            | Kind::SubjectRemoved => {}
        }
    }
}

fn worsens(change: &pb::FindingChange) -> bool {
    match (&change.before, &change.after) {
        (Some(before), Some(after)) => after.severity > before.severity,
        _ => false,
    }
}

/// Binds a verdict to the branch entries it judged, the worlds the delta
/// compared, the caller's read scope and the rules in force.
#[must_use]
pub fn binding<'a>(
    branch: &str,
    entries: impl IntoIterator<Item = &'a pb::BranchDiffEntry>,
    delta: Option<&pb::ConsequenceDelta>,
    vis: &Visibility,
) -> pb::AssessmentBinding {
    let mut entries: Vec<pb::AssessmentPin> = entries
        .into_iter()
        .map(|entry| pb::AssessmentPin {
            r#ref: entry.r#ref.clone(),
            baseline_etag: entry.base_etag.clone(),
        })
        .collect();
    entries.sort_by_key(|pin| {
        let r = pin.r#ref.as_ref();
        let id = r.and_then(|r| r.id.as_ref());
        (
            r.map(|r| r.kind),
            id.map(|id| (id.namespace.clone(), id.slug.clone(), id.version)),
        )
    });
    pb::AssessmentBinding {
        branch: branch.to_owned(),
        baseline_snapshot_id: delta
            .map(|d| d.before_snapshot_id.clone())
            .unwrap_or_default(),
        landed_snapshot_id: delta
            .map(|d| d.after_snapshot_id.clone())
            .unwrap_or_default(),
        entries,
        scope: vis.to_string(),
        catalogue_revision: catalogue_revision().to_owned(),
    }
}

/// Identity of the rules a verdict was judged by: the assessment rules
/// themselves and every validation rule the delta can report.
#[must_use]
pub fn catalogue_revision() -> &'static str {
    static REVISION: OnceLock<String> = OnceLock::new();
    REVISION.get_or_init(|| {
        let mut hasher = Sha256::new();
        hasher.update(ASSESSMENT_RULES.as_bytes());
        for rule in validation::rule_catalog() {
            hasher.update(rule.encode_length_delimited_to_vec());
        }
        hasher
            .finalize()
            .iter()
            .fold(String::from("sha256:"), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound() -> pb::AssessmentBinding {
        pb::AssessmentBinding {
            branch: "feature".into(),
            baseline_snapshot_id: "before".into(),
            landed_snapshot_id: "after".into(),
            ..Default::default()
        }
    }

    fn issue(severity: pb::validation_issue::Severity) -> pb::ValidationIssue {
        pb::ValidationIssue {
            severity: severity as i32,
            code: "MEMBER_NOT_FOUND".into(),
            ..Default::default()
        }
    }

    fn change(
        kind: Kind,
        before: Option<pb::validation_issue::Severity>,
        after: Option<pb::validation_issue::Severity>,
    ) -> pb::FindingChange {
        pb::FindingChange {
            kind: kind as i32,
            before: before.map(issue),
            after: after.map(issue),
            ..Default::default()
        }
    }

    fn delta(changes: Vec<pb::FindingChange>) -> pb::ConsequenceDelta {
        pb::ConsequenceDelta {
            changes,
            complete: true,
            before_snapshot_id: "before".into(),
            after_snapshot_id: "after".into(),
            ..Default::default()
        }
    }

    fn judged(evidence: &Evidence<'_>) -> (Verdict, Vec<Reason>) {
        let assessment = assess(evidence, bound());
        (
            assessment.verdict(),
            assessment.reasons().collect::<Vec<_>>(),
        )
    }

    use pb::validation_issue::Severity::{Error, Warning};

    #[test]
    fn a_clean_complete_landing_lands() {
        let delta = delta(vec![change(Kind::Resolved, Some(Error), None)]);
        assert_eq!(
            judged(&Evidence::Admissible {
                delta: Some(&delta),
                invariants: None,
            }),
            (Verdict::Land, vec![])
        );
    }

    #[test]
    fn introduced_and_worsened_findings_need_review() {
        let delta = delta(vec![
            change(Kind::Introduced, None, Some(Warning)),
            change(Kind::SeverityChanged, Some(Warning), Some(Error)),
            change(Kind::SeverityChanged, Some(Error), Some(Warning)),
        ]);
        assert_eq!(
            judged(&Evidence::Admissible {
                delta: Some(&delta),
                invariants: None,
            }),
            (
                Verdict::NeedsReview,
                vec![Reason::FindingsIntroduced, Reason::FindingsWorsened]
            )
        );
    }

    #[test]
    fn unknown_or_incomplete_coverage_never_lands() {
        let mut incomplete = delta(vec![change(Kind::Unknown, Some(Error), None)]);
        incomplete.complete = false;
        assert_eq!(
            judged(&Evidence::Admissible {
                delta: Some(&incomplete),
                invariants: None,
            }),
            (
                Verdict::NeedsReview,
                vec![Reason::FindingsUnknown, Reason::DeltaIncomplete]
            )
        );
        assert_eq!(
            judged(&Evidence::Admissible {
                delta: None,
                invariants: None,
            })
            .0,
            Verdict::NeedsReview
        );
        let unevaluated = pb::InvariantCheck::default();
        assert_eq!(
            judged(&Evidence::Admissible {
                delta: Some(&delta(vec![])),
                invariants: Some(&unevaluated),
            }),
            (Verdict::NeedsReview, vec![Reason::InvariantsIncomplete])
        );
    }

    #[test]
    fn an_unbound_world_never_lands() {
        let delta = delta(vec![]);
        let assessment = assess(
            &Evidence::Admissible {
                delta: Some(&delta),
                invariants: None,
            },
            pb::AssessmentBinding::default(),
        );
        assert_eq!(assessment.verdict(), Verdict::NeedsReview);
        assert_eq!(
            assessment.reasons().collect::<Vec<_>>(),
            vec![Reason::Unbound]
        );
    }

    #[test]
    fn conflicts_and_blocking_findings_reject() {
        assert_eq!(
            judged(&Evidence::Conflicts),
            (Verdict::Reject, vec![Reason::Conflicts])
        );
        let delta = delta(vec![change(Kind::Introduced, None, Some(Error))]);
        assert_eq!(
            judged(&Evidence::Blocked {
                delta: Some(&delta),
            }),
            (
                Verdict::Reject,
                vec![Reason::BlockingFindings, Reason::FindingsIntroduced]
            )
        );
    }

    #[test]
    fn the_catalogue_revision_is_stable() {
        assert_eq!(catalogue_revision(), catalogue_revision());
        assert!(catalogue_revision().starts_with("sha256:"));
    }
}
