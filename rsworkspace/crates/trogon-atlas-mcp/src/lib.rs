pub mod error;
pub mod server;
pub mod tools;

use anyhow::Result;
pub use error::McpError;
use rmcp::ServiceExt;

pub async fn run(
    grpc_endpoint: &str,
    auth_token: Option<&str>,
    max_message_bytes: Option<usize>,
    rpc_timeout_secs: u64,
    branch: Option<String>,
) -> Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env();
    if let Err(ref e) = env_filter {
        eprintln!(
            "trogon-atlas-mcp: RUST_LOG could not be parsed ({e}); falling back to info level"
        );
    }
    let filter = env_filter.unwrap_or_else(|_| "info,trogon_atlas_mcp=info".into());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();

    let server = server::McpServer::connect(
        grpc_endpoint,
        auth_token,
        max_message_bytes,
        rpc_timeout_secs,
        branch,
    )
    .await?;
    let transport = (tokio::io::stdin(), tokio::io::stdout());
    let service = server.serve(transport).await?;
    service.waiting().await?;
    Ok(())
}
