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
