use anyhow::{Context, Result};
use serde_json::Value;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

use crate::client::Client;
pub use crate::responsibility::parse_request;

pub async fn list_channel_messages(
    client: &mut Client,
    request: pb::ListChannelMessagesRequest,
) -> Result<Value> {
    let response = client
        .list_channel_messages(request)
        .await
        .context("ListChannelMessages failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListChannelMessagesResponse",
            &response,
        )
        .context("encode ListChannelMessages response")
}
