use anyhow::{Context, Result};
use serde_json::Value;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

use crate::client::Client;

pub async fn list_term_domain_declarations(
    client: &mut Client,
    request: pb::ListTermDomainDeclarationsRequest,
) -> Result<Value> {
    let response = client
        .list_term_domain_declarations(request)
        .await
        .context("ListTermDomainDeclarations failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListTermDomainDeclarationsResponse",
            &response,
        )
        .context("encode ListTermDomainDeclarations response")
}
