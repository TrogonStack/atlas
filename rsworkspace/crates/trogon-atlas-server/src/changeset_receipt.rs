//! The validation receipt a changeset carries: what the write path checked
//! before it landed, under which rule catalogue, and what it found.
//!
//! Derived from the same `ValidationCoverage` the write RPC returns, so the
//! durable record never claims more than the caller was told.

use std::sync::OnceLock;

use prost::Message as _;
use sha2::{Digest, Sha256};
use trogon_atlas_core::ContentHash;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    ChangesetValidation, IssueCounts, ReceiptCoverage, RuleCatalogDigest, ValidationReceipt,
};

use crate::validation;

const RULE_CATALOG_DOMAIN: &[u8] = b"trogonatlas.validation.rule_catalog.v1\0";

/// Skip reasons that mean a suite is outside the write path by design, so
/// skipping it leaves nothing the path promised unexamined. Any other skip
/// counts as incomplete.
const BY_DESIGN_SKIP_REASONS: &[&str] = &[
    validation::REASON_WRITE_PATH,
    validation::REASON_MERGE_PATH,
    validation::REASON_KIND_NOT_APPLICABLE,
];

/// Identity of the rule catalogue this server validates with.
///
/// Covers every declared rule field, prose included, so any catalogue edit
/// yields a new identity and counts from before and after are never
/// presented as comparable. It names the declared catalogue, not the code
/// that evaluates it.
#[must_use]
pub fn rule_catalog_digest() -> RuleCatalogDigest {
    static DIGEST: OnceLock<String> = OnceLock::new();
    let hex = DIGEST.get_or_init(|| {
        let mut rules = validation::rule_catalog();
        rules.sort_by(|a, b| a.code.cmp(&b.code));
        let mut hasher = Sha256::new();
        hasher.update(RULE_CATALOG_DOMAIN);
        for rule in &rules {
            let bytes = rule.encode_to_vec();
            hasher.update((bytes.len() as u64).to_be_bytes());
            hasher.update(bytes);
        }
        ContentHash::from_bytes(hasher.finalize().into()).to_hex()
    });
    RuleCatalogDigest::new(hex.clone())
}

#[must_use]
pub fn receipt<'a>(
    coverage: &pb::ValidationCoverage,
    issues: impl IntoIterator<Item = &'a pb::ValidationIssue>,
) -> ChangesetValidation {
    ChangesetValidation::Validated(ValidationReceipt {
        coverage: receipt_coverage(coverage),
        issues: count_issues(issues),
        rule_catalog: rule_catalog_digest(),
    })
}

fn receipt_coverage(coverage: &pb::ValidationCoverage) -> ReceiptCoverage {
    use pb::validation_check_result::State;
    let mut incomplete: Vec<String> = coverage
        .checks
        .iter()
        .filter(|check| match State::try_from(check.state) {
            Ok(State::Completed) => false,
            Ok(State::Skipped) => !BY_DESIGN_SKIP_REASONS.contains(&check.reason.as_str()),
            _ => true,
        })
        .map(|check| check.check_id.clone())
        .collect();
    if coverage.checks.is_empty() {
        incomplete.push("no_checks_reported".to_owned());
    }
    incomplete.sort();
    incomplete.dedup();
    if incomplete.is_empty() {
        ReceiptCoverage::Complete
    } else {
        ReceiptCoverage::Incomplete(incomplete)
    }
}

fn count_issues<'a>(issues: impl IntoIterator<Item = &'a pb::ValidationIssue>) -> IssueCounts {
    use pb::validation_issue::Severity;
    issues
        .into_iter()
        .fold(IssueCounts::default(), |mut counts, issue| {
            match Severity::try_from(issue.severity) {
                Ok(Severity::Error) => counts.errors += 1,
                Ok(Severity::Warning) => counts.warnings += 1,
                _ => counts.infos += 1,
            }
            counts
        })
}

#[cfg(test)]
mod tests {
    use pb::validation_check_result::State;

    use super::*;

    fn check(id: &str, state: State, reason: &str) -> pb::ValidationCheckResult {
        validation::coverage_check(id, state, reason, 1)
    }

    fn coverage(checks: Vec<pb::ValidationCheckResult>) -> pb::ValidationCoverage {
        pb::ValidationCoverage {
            checks,
            ..Default::default()
        }
    }

    fn issue(severity: pb::validation_issue::Severity) -> pb::ValidationIssue {
        pb::ValidationIssue {
            severity: severity as i32,
            ..Default::default()
        }
    }

    #[test]
    fn by_design_skips_leave_coverage_complete() {
        let cov = coverage(vec![
            check("scenarios", State::Completed, ""),
            check("model", State::Skipped, validation::REASON_WRITE_PATH),
        ]);
        assert_eq!(receipt_coverage(&cov), ReceiptCoverage::Complete);
    }

    #[test]
    fn partial_errored_and_blocked_checks_are_incomplete() {
        let cov = coverage(vec![
            check("strategic", State::Partial, "snapshot_truncated"),
            check("architecture", State::Error, "boom"),
            check(
                "scenarios",
                State::Skipped,
                validation::REASON_BLOCKED_BY_HYGIENE,
            ),
            check("model", State::Unspecified, ""),
        ]);
        assert_eq!(
            receipt_coverage(&cov),
            ReceiptCoverage::Incomplete(vec![
                "architecture".into(),
                "model".into(),
                "scenarios".into(),
                "strategic".into(),
            ])
        );
    }

    #[test]
    fn coverage_without_checks_is_never_complete() {
        assert!(matches!(
            receipt_coverage(&coverage(Vec::new())),
            ReceiptCoverage::Incomplete(_)
        ));
    }

    #[test]
    fn issues_are_counted_by_severity() {
        use pb::validation_issue::Severity;
        let issues = [
            issue(Severity::Error),
            issue(Severity::Warning),
            issue(Severity::Warning),
            issue(Severity::Info),
        ];
        assert_eq!(
            count_issues(&issues),
            IssueCounts {
                errors: 1,
                warnings: 2,
                infos: 1
            }
        );
    }

    #[test]
    fn rule_catalog_digest_is_stable_hex() {
        let digest = rule_catalog_digest();
        assert_eq!(digest, rule_catalog_digest());
        assert_eq!(digest.as_str().len(), 64);
        assert!(digest.as_str().bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
