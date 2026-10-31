//! Attested model export (`ExportAttestedModel`) and its offline check.
//! Verification reads only the saved bundle, so it needs no server and
//! proves integrity, not who produced it.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::{json, Value};
use trogon_atlas_core::attested_bundle::{self, BundleError, Completeness, VerifiedBundle};
use trogon_atlas_proto as pb;

use crate::{
    client::Client,
    view::{self, EntityAddress},
};

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
    let id = if let Some(version) = address.version {
        pb::Id {
            namespace: address.namespace.clone(),
            slug: address.slug.clone(),
            version,
        }
    } else {
        let entity = view::fetch_entity(client, address).await?;
        trogon_atlas_core::entity_id(&entity)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("stored event model has no id"))?
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

/// Parse a bundle's saved proto3 JSON and verify it, without a server.
pub fn verify_json(json: &str) -> Result<(pb::AttestedModelBundle, VerifiedBundle), BundleError> {
    let bundle = attested_bundle::bundle_from_json(json)?;
    let verified = attested_bundle::verify(&bundle)?;
    Ok((bundle, verified))
}

/// Read a saved bundle and verify it. A malformed or tampered file is
/// reported as invalid input.
pub fn verify_file(path: &Path) -> Result<(pb::AttestedModelBundle, VerifiedBundle)> {
    let json = crate::manifest::read_text(path)?;
    verify_json(&json).map_err(|e| {
        crate::failure::invalid_input(format!("{} failed verification: {e}", path.display()))
    })
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
/// and which entities it could not attest.
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
    })
}
