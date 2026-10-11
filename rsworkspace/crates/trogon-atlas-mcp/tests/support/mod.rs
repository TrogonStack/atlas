use rmcp::model::CallToolResult;
use trogon_atlas_client::client::{endpoint_for, ConnectOptions, Transport};
use trogon_atlas_mcp::server::McpServer;

/// An `McpServer` calling a test gRPC server over the network, standing in
/// for the in-process transport trogon-atlas-server gives it.
pub fn mcp_server(grpc_endpoint: &str, branch: Option<String>) -> McpServer {
    let channel = endpoint_for(grpc_endpoint, 30)
        .expect("valid gRPC endpoint")
        .connect_lazy();
    McpServer::new(
        Transport::new(channel),
        ConnectOptions {
            request_id_prefix: Some("mcp"),
            max_message_bytes: None,
            branch,
        },
    )
    .expect("valid connect options")
}

/// The JSON a successful tool returned. Clients that do not read
/// `structuredContent` see only the text block, so both must carry it.
#[allow(dead_code, reason = "not every test binary reads tool results")]
pub fn structured(result: &CallToolResult) -> serde_json::Value {
    assert_ne!(result.is_error, Some(true), "{result:?}");
    structured_body(result)
}

/// The JSON body of a failed tool call. A failure the tool reports comes
/// back as an `isError` result, which clients show the model, rather than
/// as a JSON-RPC error, which they may not.
#[allow(dead_code, reason = "not every test binary reads tool results")]
pub fn tool_error(result: &CallToolResult) -> serde_json::Value {
    assert_eq!(result.is_error, Some(true), "{result:?}");
    let body = structured_body(result);
    for field in ["message", "category", "next_action"] {
        assert!(body[field].is_string(), "{field} missing: {body}");
    }
    body
}

fn structured_body(result: &CallToolResult) -> serde_json::Value {
    let value = result
        .structured_content
        .clone()
        .unwrap_or_else(|| panic!("tool result carries no structured content: {result:?}"));
    let text: serde_json::Value = serde_json::from_str(
        &result.content[0]
            .as_text()
            .expect("first content block is text")
            .text,
    )
    .expect("text block is JSON");
    assert_eq!(text, value, "text block must mirror the structured content");
    value
}

/// A tool's JSON read back as the API response message `name`. Unknown
/// fields fail the read, so this pins the result to the proto3 JSON of that
/// message rather than to some ad-hoc shape.
#[allow(dead_code, reason = "not every test binary reads tool results")]
pub fn structured_as<M: prost::Message + Default>(result: &CallToolResult, name: &str) -> M {
    let full_name = format!("trogonatlas.api.eventmodel.v1alpha1.{name}");
    trogon_atlas_core::transcode::message_from_json_value(&full_name, &structured(result))
        .unwrap_or_else(|e| panic!("result is not proto3 JSON of {full_name}: {e}"))
}
