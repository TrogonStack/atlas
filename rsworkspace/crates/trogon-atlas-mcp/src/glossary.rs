use rmcp::model::{CallToolResult, ContentBlock};

use crate::{error::McpError, server::MAX_JSON_BYTES, tools::GlossaryRequestParams};

pub(crate) fn request<M: prost::Message + Default>(
    name: &str,
    parameters: &GlossaryRequestParams,
) -> Result<M, rmcp::ErrorData> {
    let text = parameters.request_json.to_json_text();
    if text.len() > MAX_JSON_BYTES {
        return Err(rmcp::ErrorData::invalid_params(
            "request_json exceeds the 4 MiB limit",
            None,
        ));
    }
    let value = serde_json::from_str(&text).map_err(|error| {
        rmcp::ErrorData::invalid_params(format!("invalid request JSON: {error}"), None)
    })?;
    trogon_atlas_client::glossary::parse_request(name, &value)
        .map_err(|error| rmcp::ErrorData::from(McpError::from(error)))
}

pub(crate) fn response(
    result: anyhow::Result<serde_json::Value>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let value = result.map_err(|error| rmcp::ErrorData::from(McpError::from(error)))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(
        value.to_string(),
    )]))
}
