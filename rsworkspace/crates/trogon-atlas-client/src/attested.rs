//! Attested model export (`ExportAttestedModel`) and its offline check.
//! Verification reads only the saved bundle, so it needs no server. It
//! proves integrity, and proves who sealed a signed bundle only against
//! public keys the verifier supplies.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::{json, Value};
use trogon_atlas_core::{
    attested_bundle::{self, BundleError, Completeness, VerifiedBundle},
    attested_signing::{TrustedKey, TrustedKeys},
};
use trogon_atlas_proto as pb;

use crate::{client::Client, view::EntityAddress};

/// What to export: the model, and optionally the baseline changeset whose
/// state the bundle should describe instead of the current one.
#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub address: EntityAddress,
    pub as_of_changeset: Option<String>,
    pub allow_incomplete: bool,
}

/// Export the addressed EventModel as a sealed bundle, checked here before
/// it is returned so a caller never saves one that would fail `verify`.
pub async fn export_attested_model(
    client: &mut Client,
    request: &ExportRequest,
) -> Result<(pb::AttestedModelBundle, VerifiedBundle)> {
    let address = &request.address;
    let id = pb::Id {
        namespace: address.namespace.clone(),
        slug: address.slug.clone(),
        // ExportAttestedModel resolves zero against the snapshot it exports.
        version: address.version.unwrap_or_default(),
    };
    let bundle = client
        .export_attested_model(pb::ExportAttestedModelRequest {
            id: Some(id),
            as_of_changeset: request.as_of_changeset.clone().unwrap_or_default(),
            allow_incomplete: request.allow_incomplete,
        })
        .await
        .context("ExportAttestedModel failed")?
        .into_inner()
        .bundle
        .ok_or_else(|| anyhow::anyhow!("server returned no bundle"))?;
    let verified = attested_bundle::verify(&bundle)
        .context("the server returned a bundle that does not verify")?;
    Ok((bundle, verified))
}

/// Read SPKI PEM Ed25519 public keys a verifier trusts. `None` when no
/// path is given, so verification checks integrity only.
pub fn read_trusted_keys(paths: &[impl AsRef<Path>]) -> Result<Option<TrustedKeys>> {
    if paths.is_empty() {
        return Ok(None);
    }
    let keys = paths
        .iter()
        .map(|path| {
            let path = path.as_ref();
            let pem = std::fs::read_to_string(path).map_err(|e| {
                crate::failure::invalid_input(format!("reading {}: {e}", path.display()))
            })?;
            TrustedKey::from_public_key_pem(&pem)
                .map_err(|e| crate::failure::invalid_input(format!("{}: {e}", path.display())))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(TrustedKeys::new(keys)?))
}

/// Parse a bundle's saved proto3 JSON and verify it, without a server.
/// This checks integrity only: it has no trusted keys, so a signature on
/// the bundle is reported as present and not checked.
pub fn verify_json(json: &str) -> Result<(pb::AttestedModelBundle, VerifiedBundle), BundleError> {
    let bundle = attested_bundle::bundle_from_json(json)?;
    let verified = attested_bundle::verify(&bundle)?;
    Ok((bundle, verified))
}

/// Read a saved bundle and verify it, requiring a signature by one of
/// `trusted` when given. A malformed, tampered or untrusted file is
/// reported as invalid input.
pub fn verify_file(
    path: &Path,
    trusted: Option<&TrustedKeys>,
) -> Result<(pb::AttestedModelBundle, VerifiedBundle)> {
    let json = crate::manifest::read_text(path)?;
    let bundle = attested_bundle::bundle_from_json(&json).map_err(|e| {
        crate::failure::invalid_input(format!("{} is not an attested bundle: {e}", path.display()))
    })?;
    let verified = match trusted {
        Some(trusted) => attested_bundle::verify_trusted(&bundle, trusted),
        None => attested_bundle::verify(&bundle),
    }
    .map_err(|e| {
        crate::failure::invalid_input(format!("{} failed verification: {e}", path.display()))
    })?;
    Ok((bundle, verified))
}

/// The incomplete reasons of a verified bundle, by proto enum name. Empty
/// when it is complete.
#[must_use]
pub fn incomplete_reason_names(verified: &VerifiedBundle) -> Vec<&'static str> {
    match &verified.completeness {
        Completeness::Complete => Vec::new(),
        Completeness::Incomplete(reasons) => reasons
            .iter()
            .map(pb::attested_completeness::Reason::as_str_name)
            .collect(),
    }
}

/// What a verified bundle says about itself, in the shape every surface
/// reports: its digest, whether it is complete, and when it is not, why
/// and which entities it could not attest, plus what it proves about who
/// sealed it.
#[must_use]
pub fn outcome_json(bundle: &pb::AttestedModelBundle, verified: &VerifiedBundle) -> Value {
    let unresolved: Vec<Value> = bundle
        .unresolved
        .iter()
        .map(|row| {
            let entity = row.entity.as_ref().and_then(|entity| {
                let kind = pb::EntityKind::try_from(entity.kind).ok()?;
                Some(pb::canonical::id_string(kind, entity.id.as_ref()?))
            });
            json!({
                "key": row.key,
                "entity": entity,
                "cause": row.cause().as_str_name(),
            })
        })
        .collect();
    json!({
        "bundle_digest": verified.digest.to_hex(),
        "complete": verified.completeness.is_complete(),
        "incomplete_reasons": incomplete_reason_names(verified),
        "unresolved": unresolved,
        "authenticity": verified.authenticity.as_str(),
        "signing_key_id": verified.authenticity.key_id().map(|id| id.to_hex()),
    })
}
