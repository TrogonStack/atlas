pub mod caller;
pub mod error;
pub mod profile;
pub mod prompts;
pub mod reference;
pub mod server;
pub mod tools;

use std::{str::FromStr, sync::Arc};

use axum::Router;
pub use error::McpError;
use rmcp::transport::streamable_http_server::{
    session::never::NeverSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};

pub const MCP_PATH: &str = "/mcp";

/// Host header allow-list guarding the MCP endpoint against DNS rebinding.
/// Entries without a port match any port; `*` turns the check off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowedHosts {
    Any,
    Only(Vec<String>),
}

impl FromStr for AllowedHosts {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim() == "*" {
            return Ok(Self::Any);
        }
        let hosts: Vec<String> = s
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_owned)
            .collect();
        if hosts.is_empty() {
            return Err("list at least one host, or `*` to allow any".to_owned());
        }
        Ok(Self::Only(hosts))
    }
}

/// MCP over Streamable HTTP at [`MCP_PATH`].
///
/// MCP runs stateless (one fresh handler per request, JSON responses), so
/// replicas need no session affinity and a restart loses nothing.
pub fn router(server: server::McpServer, allowed_hosts: AllowedHosts) -> Router {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None);
    let config = match allowed_hosts {
        AllowedHosts::Any => config.disable_allowed_hosts(),
        AllowedHosts::Only(hosts) => config.with_allowed_hosts(hosts),
    };
    let mcp = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new().route_service(MCP_PATH, mcp)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn allowed_hosts_star_disables_the_check() {
        assert_eq!("*".parse::<AllowedHosts>().unwrap(), AllowedHosts::Any);
    }

    #[test]
    fn allowed_hosts_splits_and_trims() {
        assert_eq!(
            "localhost, 127.0.0.1,,mcp.example.com:443"
                .parse::<AllowedHosts>()
                .unwrap(),
            AllowedHosts::Only(vec![
                "localhost".into(),
                "127.0.0.1".into(),
                "mcp.example.com:443".into(),
            ])
        );
    }

    #[test]
    fn allowed_hosts_refuses_an_empty_list() {
        assert!(" , ".parse::<AllowedHosts>().is_err());
    }
}
