use std::{fmt, str::FromStr};

use anyhow::Result;
use clap::Parser;
use trogon_atlas_mcp::run;

#[derive(Clone)]
struct Redacted(String);

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<redacted>")
    }
}

impl fmt::Display for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<redacted>")
    }
}

impl FromStr for Redacted {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Redacted(s.to_string()))
    }
}

#[derive(Parser, Debug, Clone)]
#[command(name = "trogon-atlas-mcp")]
struct Args {
    /// gRPC endpoint of trogon-atlas-server. `https://` endpoints use TLS
    /// with native roots.
    #[arg(
        long,
        env = "TROGON_ATLAS_GRPC",
        default_value = "http://127.0.0.1:50069"
    )]
    grpc: String,
    /// Shared bearer token matching the server's `TROGON_ATLAS_AUTH_TOKEN`.
    #[arg(long, env = "TROGON_ATLAS_AUTH_TOKEN", hide_env_values = true)]
    auth_token: Option<Redacted>,
    /// Maximum gRPC message size in bytes for encoding and decoding.
    /// Defaults to 4 MiB. Must match the server's cap; set both sides
    /// in lock-step when raising the limit.
    #[arg(long, env = "TROGON_ATLAS_MCP_MAX_MESSAGE_BYTES")]
    max_message_bytes: Option<usize>,
    /// Per-RPC timeout in seconds applied to every outgoing gRPC call.
    /// LLM-backed tools (`infer_data_flow`, `check_information_completeness`)
    /// can take 60+ seconds; the default of 120s gives them headroom while
    /// still bounding stuck calls. The connect timeout is always 5 seconds.
    #[arg(long, env = "TROGON_ATLAS_MCP_RPC_TIMEOUT_SECS", default_value_t = 120)]
    rpc_timeout_secs: u64,
    /// Scope every RPC to this branch via the `x-trogon-atlas-branch` metadata
    /// header (Phase 1: Isolation). Omit to operate against baseline.
    #[arg(long, env = "TROGON_ATLAS_BRANCH")]
    branch: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run(
        &args.grpc,
        args.auth_token.as_ref().map(|r| r.0.as_str()),
        args.max_message_bytes,
        args.rpc_timeout_secs,
        args.branch,
    ))
}
