use anyhow::{Context, Result};
use serde_json::Value;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

use crate::client::Client;
pub use crate::responsibility::parse_request;

fn response<M: prost::Message>(name: &str, message: &M) -> Result<Value> {
    TranscodePool::builtin()?
        .message_json_value(
            &format!("trogonatlas.api.eventmodel.v1alpha1.{name}"),
            message,
        )
        .with_context(|| format!("encode {name}"))
}

pub async fn list_glossary_entries(
    client: &mut Client,
    request: pb::ListGlossaryEntriesRequest,
) -> Result<Value> {
    let value = client
        .list_glossary_entries(request)
        .await
        .context("ListGlossaryEntries failed")?
        .into_inner();
    response("ListGlossaryEntriesResponse", &value)
}

pub async fn get_glossary_term(
    client: &mut Client,
    request: pb::GetGlossaryTermRequest,
) -> Result<Value> {
    let value = client
        .get_glossary_term(request)
        .await
        .context("GetGlossaryTerm failed")?
        .into_inner();
    response("GetGlossaryTermResponse", &value)
}

pub async fn inspect_glossary_sources(
    client: &mut Client,
    request: pb::InspectGlossarySourcesRequest,
) -> Result<Value> {
    let value = client
        .inspect_glossary_sources(request)
        .await
        .context("InspectGlossarySources failed")?
        .into_inner();
    response("InspectGlossarySourcesResponse", &value)
}

pub async fn review_glossary_term(
    client: &mut Client,
    request: pb::ReviewGlossaryTermRequest,
) -> Result<Value> {
    let value = client
        .review_glossary_term(request)
        .await
        .context("ReviewGlossaryTerm failed")?
        .into_inner();
    response("GetGlossaryTermResponse", &value)
}

pub async fn compare_glossaries(
    client: &mut Client,
    request: pb::CompareGlossariesRequest,
) -> Result<Value> {
    let value = client
        .compare_glossaries(request)
        .await
        .context("CompareGlossaries failed")?
        .into_inner();
    response("CompareGlossariesResponse", &value)
}

pub async fn check_glossary_usage(
    client: &mut Client,
    request: pb::CheckGlossaryUsageRequest,
) -> Result<Value> {
    let value = client
        .check_glossary_usage(request)
        .await
        .context("CheckGlossaryUsage failed")?
        .into_inner();
    response("CheckGlossaryUsageResponse", &value)
}

pub async fn inspect_glossary_refresh(
    client: &mut Client,
    request: pb::InspectGlossaryRefreshRequest,
) -> Result<Value> {
    let value = client
        .inspect_glossary_refresh(request)
        .await
        .context("InspectGlossaryRefresh failed")?
        .into_inner();
    response("InspectGlossaryRefreshResponse", &value)
}
