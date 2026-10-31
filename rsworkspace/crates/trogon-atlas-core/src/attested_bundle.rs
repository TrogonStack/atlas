//! Digest and offline verification for an attested model bundle.
//!
//! The bundle's digest is taken the same way an entity's content hash is:
//! proto3 JSON through the embedded descriptor pool, re-serialized by
//! [`canonical_json`](crate::canonical_json), SHA-256 behind a domain tag.
//! `bundle_digest` is cleared before rendering, so the digest covers every
//! other field and never itself.
//!
//! Verification needs nothing but the bundle: manifests are carried as text
//! and no field holds a `google.protobuf.Any`, so the built-in pool can
//! render every bundle. A digest that matches proves the bundle is the one
//! the server sealed under this format. It does not prove which server
//! sealed it; that needs a signature this format does not carry yet.

use std::{collections::BTreeSet, str::FromStr};

use trogon_atlas_proto as pb;

use crate::{
    content_hash::{domain_digest, ContentHash, ContentHashError},
    transcode::{self, TranscodeError},
    SnapshotHasher,
};

/// The value of `AttestedModelBundle.format` this module seals and verifies.
pub const BUNDLE_FORMAT: &str = "trogonatlas.attested-model.v1";

const BUNDLE_DOMAIN: &[u8] = b"trogonatlas.attested-model.bundle.v1\0";
const RULE_CATALOG_DOMAIN: &[u8] = b"trogonatlas.validation.rule-catalog.v1\0";
const BUNDLE_MESSAGE: &str = "trogonatlas.api.eventmodel.v1alpha1.AttestedModelBundle";
const RULE_CATALOG_MESSAGE: &str =
    "trogonatlas.api.eventmodel.v1alpha1.ListValidationRulesResponse";

pub type IncompleteReason = pb::attested_completeness::Reason;

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("reading bundle: {0}")]
    Parse(#[source] TranscodeError),
    #[error("computing digest: {0}")]
    Digest(#[from] ContentHashError),
    #[error("unknown bundle format {0:?}; this build verifies {BUNDLE_FORMAT:?}")]
    UnknownFormat(String),
    #[error("bundle_digest is not a lowercase hex SHA-256: {0:?}")]
    MalformedDigest(String),
    #[error("bundle_digest {claimed} does not match the content, which digests to {computed}")]
    DigestMismatch {
        claimed: ContentHash,
        computed: ContentHash,
    },
    #[error("bundle is missing {0}")]
    Missing(&'static str),
    #[error("entry {key:?} is out of key order or repeated")]
    EntryOrder { key: String },
    #[error("entry {key:?} has a malformed content hash")]
    MalformedEntryHash { key: String },
    #[error("closure_id {claimed:?} does not match the entries, which hash to {computed}")]
    ClosureMismatch {
        claimed: String,
        computed: ContentHash,
    },
    #[error("bundle claims to be complete but {0}")]
    FalseComplete(&'static str),
    #[error("bundle is marked incomplete but {0}")]
    MalformedIncomplete(&'static str),
    #[error("bundle completeness state is unspecified")]
    UnspecifiedCompleteness,
}

/// What a bundle that passed verification says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    Incomplete(Vec<IncompleteReason>),
}

impl Completeness {
    /// The completeness the server records for `reasons`.
    #[must_use]
    pub fn from_reasons(reasons: BTreeSet<IncompleteReason>) -> Self {
        if reasons.is_empty() {
            Self::Complete
        } else {
            Self::Incomplete(reasons.into_iter().collect())
        }
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }

    #[must_use]
    pub fn to_proto(&self) -> pb::AttestedCompleteness {
        use pb::attested_completeness::State;
        match self {
            Self::Complete => pb::AttestedCompleteness {
                state: State::Complete as i32,
                reasons: Vec::new(),
            },
            Self::Incomplete(reasons) => pb::AttestedCompleteness {
                state: State::Incomplete as i32,
                reasons: reasons.iter().map(|r| *r as i32).collect(),
            },
        }
    }
}

/// A bundle whose digest and internal consistency checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBundle {
    pub digest: ContentHash,
    pub completeness: Completeness,
}

/// The digest of `bundle`, ignoring whatever `bundle_digest` currently holds.
pub fn bundle_digest(bundle: &pb::AttestedModelBundle) -> Result<ContentHash, BundleError> {
    let mut unsealed = bundle.clone();
    unsealed.bundle_digest.clear();
    let value =
        transcode::message_json_value(BUNDLE_MESSAGE, &unsealed).map_err(BundleError::Parse)?;
    Ok(domain_digest(BUNDLE_DOMAIN, &value)?)
}

/// Stamp `bundle_digest`. The last step of producing a bundle: any field
/// changed afterwards fails verification.
pub fn seal(bundle: &mut pb::AttestedModelBundle) -> Result<ContentHash, BundleError> {
    let digest = bundle_digest(bundle)?;
    bundle.bundle_digest = digest.to_hex();
    Ok(digest)
}

/// Identity of a rule catalogue, as `ListValidationRules` returns it.
pub fn rule_catalog_digest(rules: &[pb::ValidationRule]) -> Result<ContentHash, BundleError> {
    let catalog = pb::ListValidationRulesResponse {
        rules: rules.to_vec(),
    };
    let value = transcode::message_json_value(RULE_CATALOG_MESSAGE, &catalog)
        .map_err(BundleError::Parse)?;
    Ok(domain_digest(RULE_CATALOG_DOMAIN, &value)?)
}

/// Snapshot id over `entries`, by the rules `GetSnapshotId` uses.
pub fn closure_id(entries: &[pb::SnapshotEntry]) -> Result<ContentHash, BundleError> {
    let mut hasher = SnapshotHasher::new();
    let mut previous: Option<&str> = None;
    for entry in entries {
        if previous.is_some_and(|p| p >= entry.key.as_str()) {
            return Err(BundleError::EntryOrder {
                key: entry.key.clone(),
            });
        }
        previous = Some(&entry.key);
        let hash = ContentHash::from_str(&entry.content_hash).map_err(|_| {
            BundleError::MalformedEntryHash {
                key: entry.key.clone(),
            }
        })?;
        hasher.insert(entry.key.clone(), hash)?;
    }
    Ok(hasher.finish())
}

/// Parse a bundle from the proto3 JSON a client saved. Unknown fields are
/// refused, since the digest cannot cover what this build cannot read.
pub fn bundle_from_json(json: &str) -> Result<pb::AttestedModelBundle, BundleError> {
    transcode::message_from_json(BUNDLE_MESSAGE, json).map_err(BundleError::Parse)
}

/// Render a bundle as pretty proto3 JSON, the form clients save.
pub fn bundle_to_json(bundle: &pb::AttestedModelBundle) -> Result<String, BundleError> {
    transcode::message_to_json(BUNDLE_MESSAGE, bundle).map_err(BundleError::Parse)
}

/// Check the digest, then that the bundle is consistent with what it says
/// about itself. A bundle that passes and is [`Completeness::Complete`] is
/// a clean attestation; an incomplete one passes only as what it is.
pub fn verify(bundle: &pb::AttestedModelBundle) -> Result<VerifiedBundle, BundleError> {
    if bundle.format != BUNDLE_FORMAT {
        return Err(BundleError::UnknownFormat(bundle.format.clone()));
    }
    let claimed = ContentHash::from_str(&bundle.bundle_digest)
        .map_err(|_| BundleError::MalformedDigest(bundle.bundle_digest.clone()))?;
    let computed = bundle_digest(bundle)?;
    if claimed != computed {
        return Err(BundleError::DigestMismatch { claimed, computed });
    }

    let scope = bundle.scope.as_ref().ok_or(BundleError::Missing("scope"))?;
    if scope.event_model_id.is_none() {
        return Err(BundleError::Missing("scope.event_model_id"));
    }
    let state = bundle.state.as_ref().ok_or(BundleError::Missing("state"))?;
    if state.mode() == pb::attested_state::Mode::Unspecified {
        return Err(BundleError::Missing("state.mode"));
    }
    let closure = closure_id(&bundle.entries)?;
    if closure.to_hex() != state.closure_id {
        return Err(BundleError::ClosureMismatch {
            claimed: state.closure_id.clone(),
            computed: closure,
        });
    }
    let validation = bundle
        .validation
        .as_ref()
        .ok_or(BundleError::Missing("validation"))?;
    let coverage = validation
        .coverage
        .as_ref()
        .ok_or(BundleError::Missing("validation.coverage"))?;
    let catalog = validation
        .rule_catalog
        .as_ref()
        .ok_or(BundleError::Missing("validation.rule_catalog"))?;
    ContentHash::from_str(&catalog.digest)
        .map_err(|_| BundleError::Missing("validation.rule_catalog.digest"))?;

    let completeness = bundle
        .completeness
        .as_ref()
        .ok_or(BundleError::Missing("completeness"))?;
    let completeness = match completeness.state() {
        pb::attested_completeness::State::Unspecified => {
            return Err(BundleError::UnspecifiedCompleteness)
        }
        pb::attested_completeness::State::Complete => {
            if !completeness.reasons.is_empty() {
                return Err(BundleError::FalseComplete("it lists incomplete reasons"));
            }
            if coverage.snapshot_truncated {
                return Err(BundleError::FalseComplete(
                    "its validation coverage reports a truncated snapshot",
                ));
            }
            if coverage.checks.is_empty()
                || coverage
                    .checks
                    .iter()
                    .any(|c| c.state() != pb::validation_check_result::State::Completed)
            {
                return Err(BundleError::FalseComplete(
                    "a validation suite did not complete",
                ));
            }
            if !bundle.skipped.is_empty() {
                return Err(BundleError::FalseComplete("it skipped entities"));
            }
            Completeness::Complete
        }
        pb::attested_completeness::State::Incomplete => {
            let mut reasons = Vec::with_capacity(completeness.reasons.len());
            for raw in &completeness.reasons {
                match IncompleteReason::try_from(*raw) {
                    Ok(IncompleteReason::Unspecified) | Err(_) => {
                        return Err(BundleError::MalformedIncomplete(
                            "a reason is unspecified or unknown",
                        ))
                    }
                    Ok(reason) => reasons.push(reason),
                }
            }
            if reasons.is_empty() {
                return Err(BundleError::MalformedIncomplete("it lists no reason"));
            }
            if reasons.windows(2).any(|w| w[0] >= w[1]) {
                return Err(BundleError::MalformedIncomplete(
                    "its reasons are not sorted and unique",
                ));
            }
            Completeness::Incomplete(reasons)
        }
    };

    Ok(VerifiedBundle {
        digest: computed,
        completeness,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn entry(key: &str, hash: char) -> pb::SnapshotEntry {
        pb::SnapshotEntry {
            key: key.into(),
            entity: None,
            content_hash: hash.to_string().repeat(64),
        }
    }

    fn sealed() -> pb::AttestedModelBundle {
        let entries = vec![entry("a", '1'), entry("b", '2')];
        let mut bundle = pb::AttestedModelBundle {
            format: BUNDLE_FORMAT.into(),
            scope: Some(pb::AttestedScope {
                event_model_id: Some(pb::Id {
                    namespace: "ns".into(),
                    slug: "m".into(),
                    version: 1,
                }),
                branch: String::new(),
                lens: Some(pb::AttestedLens {
                    unrestricted: true,
                    namespaces: Vec::new(),
                }),
            }),
            state: Some(pb::AttestedState {
                mode: pb::attested_state::Mode::Current as i32,
                snapshot_id: "0".repeat(64),
                snapshot_entity_count: 2,
                closure_id: closure_id(&entries).unwrap().to_hex(),
                baseline_changeset_id: String::new(),
                branch_changeset_id: String::new(),
            }),
            manifests: "kind: Event\n".into(),
            external_manifests: String::new(),
            entries,
            skipped: Vec::new(),
            validation: Some(pb::AttestedValidation {
                issues: Vec::new(),
                coverage: Some(pb::ValidationCoverage {
                    checks: vec![pb::ValidationCheckResult {
                        check_id: "model".into(),
                        state: pb::validation_check_result::State::Completed as i32,
                        reason: String::new(),
                        subjects_examined: 1,
                    }],
                    effective_scope: None,
                    snapshot_truncated: false,
                    requested_scope: None,
                }),
                rule_catalog: Some(pb::RuleCatalogIdentity {
                    digest: rule_catalog_digest(&[]).unwrap().to_hex(),
                    server_version: "0.1.0".into(),
                }),
            }),
            completeness: Some(Completeness::Complete.to_proto()),
            bundle_digest: String::new(),
        };
        seal(&mut bundle).unwrap();
        bundle
    }

    #[test]
    fn sealed_bundle_verifies_and_survives_a_json_round_trip() {
        let bundle = sealed();
        let verified = verify(&bundle).unwrap();
        assert!(verified.completeness.is_complete());
        let back = bundle_from_json(&bundle_to_json(&bundle).unwrap()).unwrap();
        assert_eq!(verify(&back).unwrap(), verified);
    }

    #[test]
    fn digest_ignores_its_own_field() {
        let mut bundle = sealed();
        let digest = bundle_digest(&bundle).unwrap();
        bundle.bundle_digest = "f".repeat(64);
        assert_eq!(bundle_digest(&bundle).unwrap(), digest);
    }

    #[test]
    fn digest_is_domain_separated_from_the_catalog_digest() {
        let empty = pb::AttestedModelBundle::default();
        let empty_catalog = rule_catalog_digest(&[]).unwrap();
        assert_ne!(bundle_digest(&empty).unwrap(), empty_catalog);
    }

    #[test]
    fn edited_manifests_fail_the_digest() {
        let mut bundle = sealed();
        bundle.manifests.push_str("title: edited\n");
        assert!(matches!(
            verify(&bundle),
            Err(BundleError::DigestMismatch { .. })
        ));
    }

    #[test]
    fn resealed_entries_still_fail_the_closure_check() {
        let mut bundle = sealed();
        bundle.entries[0].content_hash = "3".repeat(64);
        seal(&mut bundle).unwrap();
        assert!(matches!(
            verify(&bundle),
            Err(BundleError::ClosureMismatch { .. })
        ));
    }

    #[test]
    fn complete_bundle_with_partial_coverage_is_refused_even_when_resealed() {
        let mut bundle = sealed();
        let coverage = bundle
            .validation
            .as_mut()
            .unwrap()
            .coverage
            .as_mut()
            .unwrap();
        coverage.checks[0].state = pb::validation_check_result::State::Partial as i32;
        seal(&mut bundle).unwrap();
        assert!(matches!(
            verify(&bundle),
            Err(BundleError::FalseComplete(_))
        ));
    }

    #[test]
    fn incomplete_bundle_verifies_as_incomplete() {
        let mut bundle = sealed();
        bundle.completeness = Some(
            Completeness::from_reasons(BTreeSet::from([
                IncompleteReason::EntitiesSkipped,
                IncompleteReason::SnapshotTruncated,
            ]))
            .to_proto(),
        );
        seal(&mut bundle).unwrap();
        let verified = verify(&bundle).unwrap();
        assert_eq!(
            verified.completeness,
            Completeness::Incomplete(vec![
                IncompleteReason::SnapshotTruncated,
                IncompleteReason::EntitiesSkipped,
            ])
        );
    }

    #[test]
    fn incomplete_without_reasons_is_refused() {
        let mut bundle = sealed();
        bundle.completeness = Some(pb::AttestedCompleteness {
            state: pb::attested_completeness::State::Incomplete as i32,
            reasons: Vec::new(),
        });
        seal(&mut bundle).unwrap();
        assert!(matches!(
            verify(&bundle),
            Err(BundleError::MalformedIncomplete(_))
        ));
    }

    #[test]
    fn unknown_format_is_refused() {
        let mut bundle = sealed();
        bundle.format = "trogonatlas.attested-model.v2".into();
        seal(&mut bundle).unwrap();
        assert!(matches!(
            verify(&bundle),
            Err(BundleError::UnknownFormat(_))
        ));
    }

    #[test]
    fn unknown_json_fields_are_refused() {
        let json = bundle_to_json(&sealed()).unwrap();
        let tampered = json.replacen('{', "{\"signedBy\": \"me\",", 1);
        assert!(matches!(
            bundle_from_json(&tampered),
            Err(BundleError::Parse(_))
        ));
    }

    #[test]
    fn unsorted_entries_are_refused() {
        let entries = vec![entry("b", '1'), entry("a", '2')];
        assert!(matches!(
            closure_id(&entries),
            Err(BundleError::EntryOrder { .. })
        ));
    }
}
