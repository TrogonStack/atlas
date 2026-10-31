//! Attested model export (`ExportAttestedModel`) and its offline check.
//! Verification reads only the saved bundle, so it needs no server and
//! proves integrity, not who produced it.

use std::path::Path;

use anyhow::{Context as _, Result};
use trogon_atlas_core::attested_bundle::{self, VerifiedBundle};
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

/// Read a saved bundle and verify it. A malformed or tampered file is
/// reported as invalid input.
pub fn verify_file(path: &Path) -> Result<VerifiedBundle> {
    let json =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let bundle = attested_bundle::bundle_from_json(&json).map_err(|e| {
        crate::failure::invalid_input(format!("{} is not an attested bundle: {e}", path.display()))
    })?;
    attested_bundle::verify(&bundle).map_err(|e| {
        crate::failure::invalid_input(format!("{} failed verification: {e}", path.display()))
    })
}
