use anyhow::{Context, Result};
use serde_json::Value;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

use crate::client::Client;
pub use crate::source_codec::{prepare_preview_request, SourcePreviewDocument};

pub const MAX_SOURCE_MESSAGE_BYTES: usize = 36 * 1024 * 1024;
pub fn parse_request<T: prost::Message + Default>(name: &str, json: &Value) -> Result<T> {
    let pool = TranscodePool::builtin()?;
    let name = format!("trogonatlas.api.eventmodel.v1alpha1.{name}");
    let descriptor = pool.message_descriptor(&name)?;
    if !crate::source_request::validate(&pool, &descriptor, json) {
        return Err(crate::failure::invalid_input("invalid source request"));
    }
    pool.message_from_json_value(&name, json)
        .map_err(|_| crate::failure::invalid_input("invalid source request"))
}

pub async fn native_payload(
    client: &mut Client,
    source: pb::SourceResourceKey,
    bytes: &[u8],
) -> Result<pb::SourceNativePayload> {
    use trogon_atlas_core::source_bundle::{RegistrationId, SourceCategory, SourceLocalKey};
    RegistrationId::new(&source.registration_id)
        .map_err(|error| crate::failure::invalid_input(error.to_string()))?;
    SourceCategory::new(&source.category)
        .map_err(|error| crate::failure::invalid_input(error.to_string()))?;
    SourceLocalKey::new(&source.key)
        .map_err(|error| crate::failure::invalid_input(error.to_string()))?;
    let text = std::str::from_utf8(bytes)
        .map_err(|_| crate::failure::invalid_input("native owner manifest must be UTF-8"))?;
    let documents = crate::manifest::read_str(text, "supplied native owner")
        .map_err(|_| crate::failure::invalid_input("invalid complete native owner manifest"))?;
    if documents.len() != 1 {
        return Err(crate::failure::invalid_input(
            "each explicit source key requires exactly one complete native owner manifest",
        ));
    }
    let mut loaded = crate::manifest::load_with_tenant_types(client, documents)
        .await
        .map_err(|error| {
            if error
                .downcast_ref::<crate::failure::InvalidInput>()
                .is_some()
            {
                crate::failure::invalid_input("invalid complete native owner manifest")
            } else {
                error
            }
        })?;
    let manifest = loaded
        .manifests
        .pop()
        .ok_or_else(|| crate::failure::invalid_input("complete native owner manifest required"))?;
    Ok(pb::SourceNativePayload {
        source: Some(source),
        entity: Some(manifest.entity),
    })
}

pub async fn create_source_registration(
    client: &mut Client,
    request: pb::CreateSourceRegistrationRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .create_source_registration(request)
        .await
        .context("CreateSourceRegistration failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.CreateSourceRegistrationResponse",
            &response,
        )
        .context("encode CreateSourceRegistration response")
}

pub async fn update_source_registration(
    client: &mut Client,
    request: pb::UpdateSourceRegistrationRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .update_source_registration(request)
        .await
        .context("UpdateSourceRegistration failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.UpdateSourceRegistrationResponse",
            &response,
        )
        .context("encode UpdateSourceRegistration response")
}

pub async fn get_source_registration(
    client: &mut Client,
    request: pb::GetSourceRegistrationRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .get_source_registration(request)
        .await
        .context("GetSourceRegistration failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.GetSourceRegistrationResponse",
            &response,
        )
        .context("encode GetSourceRegistration response")
}

pub async fn list_source_registrations(
    client: &mut Client,
    request: pb::ListSourceRegistrationsRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .list_source_registrations(request)
        .await
        .context("ListSourceRegistrations failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListSourceRegistrationsResponse",
            &response,
        )
        .context("encode ListSourceRegistrations response")
}

pub async fn delete_source_registration(
    client: &mut Client,
    request: pb::DeleteSourceRegistrationRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .delete_source_registration(request)
        .await
        .context("DeleteSourceRegistration failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.DeleteSourceRegistrationResponse",
            &response,
        )
        .context("encode DeleteSourceRegistration response")
}

pub async fn accept_source_snapshot(
    client: &mut Client,
    request: pb::AcceptSourceSnapshotRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .accept_source_snapshot(request)
        .await
        .context("AcceptSourceSnapshot failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.AcceptSourceSnapshotResponse",
            &response,
        )
        .context("encode AcceptSourceSnapshot response")
}

pub async fn get_source_snapshot(
    client: &mut Client,
    request: pb::GetSourceSnapshotRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .get_source_snapshot(request)
        .await
        .context("GetSourceSnapshot failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.GetSourceSnapshotResponse",
            &response,
        )
        .context("encode GetSourceSnapshot response")
}

pub async fn list_source_snapshots(
    client: &mut Client,
    request: pb::ListSourceSnapshotsRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .list_source_snapshots(request)
        .await
        .context("ListSourceSnapshots failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListSourceSnapshotsResponse",
            &response,
        )
        .context("encode ListSourceSnapshots response")
}

pub async fn create_source_mapping(
    client: &mut Client,
    request: pb::CreateSourceMappingRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .create_source_mapping(request)
        .await
        .context("CreateSourceMapping failed")?
        .into_inner();
    crate::source_codec::response_json(
        client,
        "trogonatlas.api.eventmodel.v1alpha1.CreateSourceMappingResponse",
        &response,
    )
    .await
    .context("encode CreateSourceMapping response")
}

pub async fn update_source_mapping(
    client: &mut Client,
    request: pb::UpdateSourceMappingRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .update_source_mapping(request)
        .await
        .context("UpdateSourceMapping failed")?
        .into_inner();
    crate::source_codec::response_json(
        client,
        "trogonatlas.api.eventmodel.v1alpha1.UpdateSourceMappingResponse",
        &response,
    )
    .await
    .context("encode UpdateSourceMapping response")
}

pub async fn get_source_mapping(
    client: &mut Client,
    request: pb::GetSourceMappingRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .get_source_mapping(request)
        .await
        .context("GetSourceMapping failed")?
        .into_inner();
    crate::source_codec::response_json(
        client,
        "trogonatlas.api.eventmodel.v1alpha1.GetSourceMappingResponse",
        &response,
    )
    .await
    .context("encode GetSourceMapping response")
}

pub async fn list_source_mappings(
    client: &mut Client,
    request: pb::ListSourceMappingsRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .list_source_mappings(request)
        .await
        .context("ListSourceMappings failed")?
        .into_inner();
    crate::source_codec::response_json(
        client,
        "trogonatlas.api.eventmodel.v1alpha1.ListSourceMappingsResponse",
        &response,
    )
    .await
    .context("encode ListSourceMappings response")
}

pub async fn delete_source_mapping(
    client: &mut Client,
    request: pb::DeleteSourceMappingRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .delete_source_mapping(request)
        .await
        .context("DeleteSourceMapping failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.DeleteSourceMappingResponse",
            &response,
        )
        .context("encode DeleteSourceMapping response")
}

pub async fn get_source_overlay(
    client: &mut Client,
    request: pb::GetSourceOverlayRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .get_source_overlay(request)
        .await
        .context("GetSourceOverlay failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.GetSourceOverlayResponse",
            &response,
        )
        .context("encode GetSourceOverlay response")
}

pub async fn preview_source_run(
    client: &mut Client,
    request: pb::PreviewSourceRunRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .preview_source_run(request)
        .await
        .context("PreviewSourceRun failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.PreviewSourceRunResponse",
            &response,
        )
        .context("encode PreviewSourceRun response")
}

pub async fn apply_source_run(
    client: &mut Client,
    request: pb::ApplySourceRunRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .apply_source_run(request)
        .await
        .context("ApplySourceRun failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ApplySourceRunResponse",
            &response,
        )
        .context("encode ApplySourceRun response")
}

pub async fn cancel_source_run(
    client: &mut Client,
    request: pb::CancelSourceRunRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .cancel_source_run(request)
        .await
        .context("CancelSourceRun failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.CancelSourceRunResponse",
            &response,
        )
        .context("encode CancelSourceRun response")
}

pub async fn get_source_run(
    client: &mut Client,
    request: pb::GetSourceRunRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .get_source_run(request)
        .await
        .context("GetSourceRun failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.GetSourceRunResponse",
            &response,
        )
        .context("encode GetSourceRun response")
}

pub async fn list_source_runs(
    client: &mut Client,
    request: pb::ListSourceRunsRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .list_source_runs(request)
        .await
        .context("ListSourceRuns failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListSourceRunsResponse",
            &response,
        )
        .context("encode ListSourceRuns response")
}

pub async fn recover_source_run(
    client: &mut Client,
    request: pb::RecoverSourceRunRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .recover_source_run(request)
        .await
        .context("RecoverSourceRun failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.RecoverSourceRunResponse",
            &response,
        )
        .context("encode RecoverSourceRun response")
}

pub async fn list_source_run_results(
    client: &mut Client,
    request: pb::ListSourceRunResultsRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .list_source_run_results(request)
        .await
        .context("ListSourceRunResults failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListSourceRunResultsResponse",
            &response,
        )
        .context("encode ListSourceRunResults response")
}

pub fn snapshot_request(
    registration_id: String,
    registration_revision: String,
    bundle: &trogon_atlas_core::source_bundle::ValidatedBundle,
) -> pb::AcceptSourceSnapshotRequest {
    pb::AcceptSourceSnapshotRequest {
        registration_id,
        registration_revision,
        inventory_bytes: bundle.inventory_bytes().to_vec(),
        files: bundle
            .files()
            .iter()
            .map(|file| pb::SourceFile {
                logical_name: file.logical_name().as_str().to_owned(),
                media_type: file.media_type().as_str().to_owned(),
                content: file.bytes().to_vec(),
                content_digest: file.digest().hex(),
            })
            .collect(),
    }
}

pub async fn list_source_run_operations(
    client: &mut Client,
    request: pb::ListSourceRunOperationsRequest,
) -> Result<Value> {
    let response = client
        .clone()
        .max_encoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .max_decoding_message_size(MAX_SOURCE_MESSAGE_BYTES)
        .list_source_run_operations(request)
        .await
        .context("ListSourceRunOperations failed")?
        .into_inner();
    crate::source_codec::response_json(
        client,
        "trogonatlas.api.eventmodel.v1alpha1.ListSourceRunOperationsResponse",
        &response,
    )
    .await
    .context("encode ListSourceRunOperations response")
}
