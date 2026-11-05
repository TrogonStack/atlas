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
//! render every bundle. A digest that matches proves the bundle is
//! unchanged since it was sealed under its format. It does not prove who
//! sealed it. The signed format adds a detached signature over the digest
//! (see [`attested_signing`](crate::attested_signing)), which proves the
//! sealer held a key only to a verifier that trusts that key.

use std::{collections::BTreeSet, str::FromStr};

use trogon_atlas_proto as pb;

use crate::{
    attested_signing::{KeyId, SigningKey, TrustedKeys, SIGNATURE_LEN},
    content_hash::{domain_digest, ContentHash, ContentHashError},
    transcode::{self, TranscodeError},
    SnapshotHasher,
};

/// The value of `AttestedModelBundle.format` for a bundle that carries no
/// signature.
pub const BUNDLE_FORMAT: &str = "trogonatlas.attested-model.v1";
/// The value of `AttestedModelBundle.format` for a bundle that carries a
/// signature.
pub const SIGNED_BUNDLE_FORMAT: &str = "trogonatlas.attested-model.v2";

/// The formats this build seals and verifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleFormat {
    Unsigned,
    Signed,
}

impl BundleFormat {
    fn parse(format: &str) -> Result<Self, BundleError> {
        match format {
            BUNDLE_FORMAT => Ok(Self::Unsigned),
            SIGNED_BUNDLE_FORMAT => Ok(Self::Signed),
            other => Err(BundleError::UnknownFormat(other.to_string())),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unsigned => BUNDLE_FORMAT,
            Self::Signed => SIGNED_BUNDLE_FORMAT,
        }
    }

    const fn digest_domain(self) -> &'static [u8] {
        match self {
            Self::Unsigned => b"trogonatlas.attested-model.bundle.v1\0",
            Self::Signed => b"trogonatlas.attested-model.bundle.v2\0",
        }
    }
}
const RULE_CATALOG_DOMAIN: &[u8] = b"trogonatlas.validation.rule-catalog.v1\0";
const BUNDLE_MESSAGE: &str = "trogonatlas.api.eventmodel.v1alpha1.AttestedModelBundle";
const RULE_CATALOG_MESSAGE: &str =
    "trogonatlas.api.eventmodel.v1alpha1.ListValidationRulesResponse";

pub type IncompleteReason = pb::attested_completeness::Reason;
pub type UnresolvedCause = pb::attested_unresolved::Cause;

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("reading bundle: {0}")]
    Parse(#[source] TranscodeError),
    #[error("computing digest: {0}")]
    Digest(#[from] ContentHashError),
    #[error(
        "unknown bundle format {0:?}; this build verifies {BUNDLE_FORMAT:?} and \
         {SIGNED_BUNDLE_FORMAT:?}"
    )]
    UnknownFormat(String),
    #[error("format {BUNDLE_FORMAT:?} carries no signature, but this bundle has one")]
    SignatureNotAllowed,
    #[error("signature is malformed: {0}")]
    MalformedSignature(&'static str),
    #[error("bundle is unsigned, but trusted keys were given, so a signature is required")]
    SignatureRequired,
    #[error("bundle is signed by key {0}, which is not a trusted key")]
    UntrustedKey(KeyId),
    #[error("signature by key {0} does not verify against the bundle digest")]
    BadSignature(KeyId),
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
    #[error("unresolved entity {key:?} is out of key order, repeated, or has no known cause")]
    MalformedUnresolved { key: String },
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

/// What a verified bundle proves about who sealed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authenticity {
    /// No signature: integrity only, issuer unauthenticated.
    Unsigned,
    /// A well-formed signature whose key was not checked, because no
    /// trusted keys were given. Not authenticated.
    SignatureUnchecked(KeyId),
    /// Signed by a trusted key, and the signature verifies.
    Authenticated(KeyId),
}

impl Authenticity {
    /// Stable machine-readable name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unsigned => "unsigned",
            Self::SignatureUnchecked(_) => "signature_unchecked",
            Self::Authenticated(_) => "authenticated",
        }
    }

    /// The signing key, when the bundle names one.
    #[must_use]
    pub const fn key_id(self) -> Option<KeyId> {
        match self {
            Self::Unsigned => None,
            Self::SignatureUnchecked(id) | Self::Authenticated(id) => Some(id),
        }
    }
}

impl std::fmt::Display for Authenticity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsigned => f.write_str("integrity only, issuer unauthenticated"),
            Self::SignatureUnchecked(id) => {
                write!(f, "signature present, not checked (key {id})")
            }
            Self::Authenticated(id) => write!(f, "signed by trusted key {id}"),
        }
    }
}

/// A bundle whose digest and internal consistency checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBundle {
    pub digest: ContentHash,
    pub completeness: Completeness,
    pub unresolved: Vec<Unresolved>,
    pub authenticity: Authenticity,
}

/// An entity the export could not attest from its snapshot, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    pub key: String,
    pub cause: UnresolvedCause,
}

fn unresolved_rows(rows: &[pb::AttestedUnresolved]) -> Result<Vec<Unresolved>, BundleError> {
    let mut previous: Option<&str> = None;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let malformed = || BundleError::MalformedUnresolved {
            key: row.key.clone(),
        };
        if previous.is_some_and(|p| p >= row.key.as_str()) {
            return Err(malformed());
        }
        previous = Some(&row.key);
        let cause = match UnresolvedCause::try_from(row.cause) {
            Ok(UnresolvedCause::Unspecified) | Err(_) => return Err(malformed()),
            Ok(cause) => cause,
        };
        out.push(Unresolved {
            key: row.key.clone(),
            cause,
        });
    }
    Ok(out)
}

/// The digest of `bundle` under its format, ignoring whatever
/// `bundle_digest` and `signature` currently hold.
pub fn bundle_digest(bundle: &pb::AttestedModelBundle) -> Result<ContentHash, BundleError> {
    let format = BundleFormat::parse(&bundle.format)?;
    let mut unsealed = bundle.clone();
    unsealed.bundle_digest.clear();
    unsealed.signature = None;
    let value =
        transcode::message_json_value(BUNDLE_MESSAGE, &unsealed).map_err(BundleError::Parse)?;
    Ok(domain_digest(format.digest_domain(), &value)?)
}

/// Seal `bundle` unsigned: set the unsigned format, drop any signature and
/// stamp `bundle_digest`. The last step of producing a bundle: any field
/// changed afterwards fails verification.
pub fn seal(bundle: &mut pb::AttestedModelBundle) -> Result<ContentHash, BundleError> {
    bundle.format = BundleFormat::Unsigned.as_str().to_string();
    bundle.signature = None;
    let digest = bundle_digest(bundle)?;
    bundle.bundle_digest = digest.to_hex();
    Ok(digest)
}

/// Seal `bundle` under the signed format and sign its digest with `key`.
/// The signature covers the digest, and the digest covers completeness,
/// so signing never changes what an incomplete bundle says about itself.
pub fn seal_signed(
    bundle: &mut pb::AttestedModelBundle,
    key: &SigningKey,
) -> Result<ContentHash, BundleError> {
    bundle.format = BundleFormat::Signed.as_str().to_string();
    bundle.signature = None;
    let digest = bundle_digest(bundle)?;
    bundle.bundle_digest = digest.to_hex();
    bundle.signature = Some(key.sign(&digest));
    Ok(digest)
}

/// A signature whose shape checked out, not yet checked against a key.
struct ParsedSignature {
    key_id: KeyId,
    value: [u8; SIGNATURE_LEN],
}

fn parse_signature(
    format: BundleFormat,
    signature: Option<&pb::BundleSignature>,
) -> Result<Option<ParsedSignature>, BundleError> {
    let signature = match (format, signature) {
        (BundleFormat::Unsigned, None) => return Ok(None),
        (BundleFormat::Unsigned, Some(_)) => return Err(BundleError::SignatureNotAllowed),
        (BundleFormat::Signed, None) => return Err(BundleError::Missing("signature")),
        (BundleFormat::Signed, Some(signature)) => signature,
    };
    if signature.algorithm() != pb::bundle_signature::Algorithm::Ed25519 {
        return Err(BundleError::MalformedSignature(
            "algorithm is not ALGORITHM_ED25519",
        ));
    }
    let key_id = KeyId::from_str(&signature.key_id)
        .map_err(|_| BundleError::MalformedSignature("key_id is not a lowercase hex SHA-256"))?;
    let value = <[u8; SIGNATURE_LEN]>::try_from(signature.value.as_slice())
        .map_err(|_| BundleError::MalformedSignature("value is not 64 bytes"))?;
    Ok(Some(ParsedSignature { key_id, value }))
}

fn authenticity(
    digest: &ContentHash,
    signature: Option<ParsedSignature>,
    trusted: Option<&TrustedKeys>,
) -> Result<Authenticity, BundleError> {
    match (signature, trusted) {
        (None, None) => Ok(Authenticity::Unsigned),
        (None, Some(_)) => Err(BundleError::SignatureRequired),
        (Some(signature), None) => Ok(Authenticity::SignatureUnchecked(signature.key_id)),
        (Some(signature), Some(trusted)) => {
            let key = trusted
                .get(&signature.key_id)
                .ok_or(BundleError::UntrustedKey(signature.key_id))?;
            if key.verifies(digest, &signature.value) {
                Ok(Authenticity::Authenticated(signature.key_id))
            } else {
                Err(BundleError::BadSignature(signature.key_id))
            }
        }
    }
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
/// a clean attestation of integrity; an incomplete one passes only as what
/// it is. A signature is checked for shape but not against any key, so a
/// signed bundle reports [`Authenticity::SignatureUnchecked`].
pub fn verify(bundle: &pb::AttestedModelBundle) -> Result<VerifiedBundle, BundleError> {
    verify_with(bundle, None)
}

/// [`verify`], and also require a signature by one of `trusted`. An
/// unsigned bundle, a key outside `trusted` or a signature that does not
/// verify fails.
pub fn verify_trusted(
    bundle: &pb::AttestedModelBundle,
    trusted: &TrustedKeys,
) -> Result<VerifiedBundle, BundleError> {
    verify_with(bundle, Some(trusted))
}

fn verify_with(
    bundle: &pb::AttestedModelBundle,
    trusted: Option<&TrustedKeys>,
) -> Result<VerifiedBundle, BundleError> {
    let format = BundleFormat::parse(&bundle.format)?;
    let signature = parse_signature(format, bundle.signature.as_ref())?;
    let claimed = ContentHash::from_str(&bundle.bundle_digest)
        .map_err(|_| BundleError::MalformedDigest(bundle.bundle_digest.clone()))?;
    let computed = bundle_digest(bundle)?;
    if claimed != computed {
        return Err(BundleError::DigestMismatch { claimed, computed });
    }
    let authenticity = authenticity(&computed, signature, trusted)?;

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

    let unresolved = unresolved_rows(&bundle.unresolved)?;
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
            if state.closure_truncated {
                return Err(BundleError::FalseComplete("its closure was truncated"));
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
            if !unresolved.is_empty() {
                return Err(BundleError::FalseComplete("it lists unresolved entities"));
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
            if reasons.contains(&IncompleteReason::UnresolvedEntities) == unresolved.is_empty() {
                return Err(BundleError::MalformedIncomplete(
                    "its unresolved entities and the unresolved-entities reason disagree",
                ));
            }
            if reasons.contains(&IncompleteReason::ClosureTruncated) != state.closure_truncated {
                return Err(BundleError::MalformedIncomplete(
                    "its closure truncation and the closure-truncated reason disagree",
                ));
            }
            Completeness::Incomplete(reasons)
        }
    };

    Ok(VerifiedBundle {
        digest: computed,
        completeness,
        unresolved,
        authenticity,
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
                closure_truncated: false,
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
            unresolved: Vec::new(),
            signature: None,
        };
        seal(&mut bundle).unwrap();
        bundle
    }

    fn unresolved(key: &str, cause: UnresolvedCause) -> pb::AttestedUnresolved {
        pb::AttestedUnresolved {
            key: key.into(),
            entity: None,
            cause: cause as i32,
        }
    }

    fn incomplete(reasons: &[IncompleteReason]) -> pb::AttestedCompleteness {
        Completeness::from_reasons(reasons.iter().copied().collect()).to_proto()
    }

    #[test]
    fn unresolved_entities_verify_as_incomplete_and_are_reported() {
        let mut bundle = sealed();
        bundle.unresolved = vec![
            unresolved("a", UnresolvedCause::Changed),
            unresolved("c", UnresolvedCause::Missing),
        ];
        bundle.completeness = Some(incomplete(&[
            IncompleteReason::UnresolvedEntities,
            IncompleteReason::ConcurrentWrite,
        ]));
        seal(&mut bundle).unwrap();
        let verified = verify(&bundle).unwrap();
        assert_eq!(
            verified.unresolved,
            vec![
                Unresolved {
                    key: "a".into(),
                    cause: UnresolvedCause::Changed
                },
                Unresolved {
                    key: "c".into(),
                    cause: UnresolvedCause::Missing
                },
            ]
        );
    }

    #[test]
    fn complete_bundle_listing_unresolved_entities_is_refused_even_when_resealed() {
        let mut bundle = sealed();
        bundle.unresolved = vec![unresolved("c", UnresolvedCause::Hidden)];
        seal(&mut bundle).unwrap();
        assert!(matches!(
            verify(&bundle),
            Err(BundleError::FalseComplete(_))
        ));
    }

    #[test]
    fn unresolved_entities_and_their_reason_must_agree() {
        let mut listed_without_reason = sealed();
        listed_without_reason.unresolved = vec![unresolved("c", UnresolvedCause::Missing)];
        listed_without_reason.completeness =
            Some(incomplete(&[IncompleteReason::SnapshotTruncated]));
        seal(&mut listed_without_reason).unwrap();
        assert!(matches!(
            verify(&listed_without_reason),
            Err(BundleError::MalformedIncomplete(_))
        ));

        let mut reason_without_listing = sealed();
        reason_without_listing.completeness =
            Some(incomplete(&[IncompleteReason::UnresolvedEntities]));
        seal(&mut reason_without_listing).unwrap();
        assert!(matches!(
            verify(&reason_without_listing),
            Err(BundleError::MalformedIncomplete(_))
        ));
    }

    #[test]
    fn unresolved_entities_must_be_sorted_unique_and_have_a_cause() {
        for rows in [
            vec![
                unresolved("c", UnresolvedCause::Missing),
                unresolved("a", UnresolvedCause::Missing),
            ],
            vec![
                unresolved("a", UnresolvedCause::Missing),
                unresolved("a", UnresolvedCause::Changed),
            ],
            vec![unresolved("a", UnresolvedCause::Unspecified)],
        ] {
            let mut bundle = sealed();
            bundle.unresolved = rows;
            bundle.completeness = Some(incomplete(&[IncompleteReason::UnresolvedEntities]));
            seal(&mut bundle).unwrap();
            assert!(
                matches!(
                    verify(&bundle),
                    Err(BundleError::MalformedUnresolved { .. })
                ),
                "{:?}",
                bundle.unresolved
            );
        }
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
        let empty = pb::AttestedModelBundle {
            format: BUNDLE_FORMAT.into(),
            ..Default::default()
        };
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
    fn truncated_closure_relabelled_complete_is_refused_even_when_resealed() {
        let mut bundle = sealed();
        bundle.state.as_mut().unwrap().closure_truncated = true;
        bundle.completeness = Some(incomplete(&[IncompleteReason::ClosureTruncated]));
        seal(&mut bundle).unwrap();
        assert_eq!(
            verify(&bundle).unwrap().completeness,
            Completeness::Incomplete(vec![IncompleteReason::ClosureTruncated])
        );

        bundle.completeness = Some(Completeness::Complete.to_proto());
        seal(&mut bundle).unwrap();
        assert!(matches!(
            verify(&bundle),
            Err(BundleError::FalseComplete(_))
        ));
    }

    #[test]
    fn closure_truncation_and_its_reason_must_agree() {
        let mut flagged_without_reason = sealed();
        flagged_without_reason
            .state
            .as_mut()
            .unwrap()
            .closure_truncated = true;
        flagged_without_reason.completeness =
            Some(incomplete(&[IncompleteReason::SnapshotTruncated]));
        seal(&mut flagged_without_reason).unwrap();
        assert!(matches!(
            verify(&flagged_without_reason),
            Err(BundleError::MalformedIncomplete(_))
        ));

        let mut reason_without_flag = sealed();
        reason_without_flag.completeness = Some(incomplete(&[IncompleteReason::ClosureTruncated]));
        seal(&mut reason_without_flag).unwrap();
        assert!(matches!(
            verify(&reason_without_flag),
            Err(BundleError::MalformedIncomplete(_))
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
        bundle.format = "trogonatlas.attested-model.v3".into();
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

    fn key(seed: u8) -> SigningKey {
        use ed25519_dalek::pkcs8::{spki::der::pem::LineEnding, EncodePrivateKey};
        let pem = ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
            .to_pkcs8_pem(LineEnding::LF)
            .unwrap();
        SigningKey::from_pkcs8_pem(&pem).unwrap()
    }

    fn trusting(keys: &[&SigningKey]) -> TrustedKeys {
        TrustedKeys::new(keys.iter().map(|k| k.trusted_key())).unwrap()
    }

    fn signed(key: &SigningKey) -> pb::AttestedModelBundle {
        let mut bundle = sealed();
        seal_signed(&mut bundle, key).unwrap();
        bundle
    }

    #[test]
    fn unsigned_bundle_verifies_as_integrity_only() {
        let verified = verify(&sealed()).unwrap();
        assert_eq!(verified.authenticity, Authenticity::Unsigned);
        assert_eq!(
            verified.authenticity.to_string(),
            "integrity only, issuer unauthenticated"
        );
    }

    #[test]
    fn unsigned_bundle_digest_is_unchanged_by_the_signed_format() {
        let bundle = sealed();
        let mut resealed = bundle.clone();
        seal(&mut resealed).unwrap();
        assert_eq!(resealed, bundle);
        let mut signed = bundle.clone();
        seal_signed(&mut signed, &key(1)).unwrap();
        assert_ne!(signed.bundle_digest, bundle.bundle_digest);
    }

    #[test]
    fn unsigned_bundle_is_refused_when_trusted_keys_are_given() {
        assert!(matches!(
            verify_trusted(&sealed(), &trusting(&[&key(1)])),
            Err(BundleError::SignatureRequired)
        ));
    }

    #[test]
    fn signed_bundle_authenticates_against_its_trusted_key() {
        let signer = key(1);
        let bundle = signed(&signer);
        assert_eq!(bundle.format, SIGNED_BUNDLE_FORMAT);
        let verified = verify_trusted(&bundle, &trusting(&[&key(2), &signer])).unwrap();
        assert_eq!(
            verified.authenticity,
            Authenticity::Authenticated(signer.key_id())
        );
        assert!(verified.completeness.is_complete());
        let back = bundle_from_json(&bundle_to_json(&bundle).unwrap()).unwrap();
        assert_eq!(
            verify_trusted(&back, &trusting(&[&signer])).unwrap(),
            verified
        );
    }

    #[test]
    fn signed_bundle_without_trusted_keys_is_not_authenticated() {
        let signer = key(1);
        let verified = verify(&signed(&signer)).unwrap();
        assert_eq!(
            verified.authenticity,
            Authenticity::SignatureUnchecked(signer.key_id())
        );
        assert!(verified
            .authenticity
            .to_string()
            .starts_with("signature present, not checked"));
    }

    #[test]
    fn signature_by_an_untrusted_key_is_refused() {
        let signer = key(1);
        assert!(matches!(
            verify_trusted(&signed(&signer), &trusting(&[&key(2)])),
            Err(BundleError::UntrustedKey(id)) if id == signer.key_id()
        ));
    }

    #[test]
    fn signature_relabelled_with_a_trusted_key_id_is_refused() {
        let trusted = key(2);
        let mut bundle = signed(&key(1));
        bundle.signature.as_mut().unwrap().key_id = trusted.key_id().to_hex();
        assert!(matches!(
            verify_trusted(&bundle, &trusting(&[&trusted])),
            Err(BundleError::BadSignature(_))
        ));
    }

    #[test]
    fn tampered_signature_is_refused() {
        let signer = key(1);
        let mut bundle = signed(&signer);
        bundle.signature.as_mut().unwrap().value[0] ^= 1;
        assert!(matches!(
            verify_trusted(&bundle, &trusting(&[&signer])),
            Err(BundleError::BadSignature(_))
        ));
    }

    #[test]
    fn resealed_content_breaks_the_signature() {
        let signer = key(1);
        let mut bundle = signed(&signer);
        let signature = bundle.signature.clone();
        bundle.manifests.push_str("title: edited\n");
        seal_signed(&mut bundle, &key(2)).unwrap();
        bundle.signature = signature;
        assert!(matches!(
            verify_trusted(&bundle, &trusting(&[&signer])),
            Err(BundleError::BadSignature(_))
        ));
    }

    #[test]
    fn signature_under_the_unsigned_format_is_refused() {
        let mut bundle = sealed();
        bundle.signature = signed(&key(1)).signature;
        assert!(matches!(
            verify(&bundle),
            Err(BundleError::SignatureNotAllowed)
        ));
    }

    #[test]
    fn signed_format_without_a_signature_is_refused() {
        let mut bundle = signed(&key(1));
        bundle.signature = None;
        assert!(matches!(
            verify(&bundle),
            Err(BundleError::Missing("signature"))
        ));
    }

    #[test]
    fn stripping_a_signature_and_downgrading_does_not_authenticate() {
        let signer = key(1);
        let mut bundle = signed(&signer);
        seal(&mut bundle).unwrap();
        assert_eq!(
            verify(&bundle).unwrap().authenticity,
            Authenticity::Unsigned
        );
        assert!(matches!(
            verify_trusted(&bundle, &trusting(&[&signer])),
            Err(BundleError::SignatureRequired)
        ));
    }

    #[test]
    fn malformed_signatures_are_refused() {
        let mut wrong_algorithm = signed(&key(1));
        wrong_algorithm.signature.as_mut().unwrap().algorithm =
            pb::bundle_signature::Algorithm::Unspecified as i32;
        let mut short_value = signed(&key(1));
        short_value.signature.as_mut().unwrap().value.pop();
        let mut bad_key_id = signed(&key(1));
        bad_key_id.signature.as_mut().unwrap().key_id = "ABC".into();
        for bundle in [wrong_algorithm, short_value, bad_key_id] {
            assert!(matches!(
                verify(&bundle),
                Err(BundleError::MalformedSignature(_))
            ));
        }
    }

    #[test]
    fn signed_incomplete_bundle_stays_incomplete() {
        let signer = key(1);
        let mut bundle = sealed();
        bundle.completeness = Some(incomplete(&[IncompleteReason::SnapshotTruncated]));
        seal_signed(&mut bundle, &signer).unwrap();
        let verified = verify_trusted(&bundle, &trusting(&[&signer])).unwrap();
        assert_eq!(
            verified.completeness,
            Completeness::Incomplete(vec![IncompleteReason::SnapshotTruncated])
        );
        assert_eq!(
            verified.authenticity,
            Authenticity::Authenticated(signer.key_id())
        );

        let mut relabelled = bundle.clone();
        relabelled.completeness = Some(Completeness::Complete.to_proto());
        assert!(matches!(
            verify_trusted(&relabelled, &trusting(&[&signer])),
            Err(BundleError::DigestMismatch { .. })
        ));
        let signature = relabelled.signature.clone();
        seal_signed(&mut relabelled, &key(2)).unwrap();
        relabelled.signature = signature;
        assert!(matches!(
            verify_trusted(&relabelled, &trusting(&[&signer])),
            Err(BundleError::BadSignature(_))
        ));
    }

    #[test]
    fn trusted_key_reads_the_public_pem_with_the_signers_key_id() {
        use ed25519_dalek::pkcs8::{spki::der::pem::LineEnding, EncodePublicKey};
        let pem = ed25519_dalek::SigningKey::from_bytes(&[1; 32])
            .verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        let trusted = crate::attested_signing::TrustedKey::from_public_key_pem(&pem).unwrap();
        assert_eq!(trusted.key_id(), key(1).key_id());
        assert!(crate::attested_signing::TrustedKey::from_public_key_pem("nope").is_err());
        assert!(SigningKey::from_pkcs8_pem(&pem).is_err());
        assert!(TrustedKeys::new([]).is_err());
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
