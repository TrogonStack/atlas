use anyhow::{Context, Result};
use serde_json::Value;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

use crate::client::Client;
pub use crate::responsibility::parse_request;

pub async fn inspect_human_interaction(
    client: &mut Client,
    request: pb::InspectHumanInteractionRequest,
) -> Result<Value> {
    let response = client
        .inspect_human_interaction(request)
        .await
        .context("InspectHumanInteraction failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.InspectHumanInteractionResponse",
            &response,
        )
        .context("encode InspectHumanInteraction response")
}
