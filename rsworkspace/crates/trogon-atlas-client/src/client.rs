// gRPC client plumbing: the same connect pattern shared by every surface that
// talks to the trogon-atlas gRPC service: a multiplexing Channel, optional
// bearer token, x-request-id correlation.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};

use anyhow::{Context as _, Result};
use tonic::{
    metadata::MetadataValue,
    service::interceptor::InterceptedService,
    transport::{Channel, ClientTlsConfig},
};
use trogon_atlas_proto::event_model_service_client::EventModelServiceClient;

const DEFAULT_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_request_id(prefix: &str) -> String {
    static PID: OnceLock<u32> = OnceLock::new();
    let pid = PID.get_or_init(std::process::id);
    let n = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{pid:x}-{n:08x}")
}

#[derive(Clone)]
pub struct BearerAuth {
    header: Option<MetadataValue<tonic::metadata::Ascii>>,
    request_id_prefix: &'static str,
    /// Branch name sent as `x-trogon-atlas-branch` on every outgoing request
    /// (see `docs/explanation/branching.md`, Phase 1: Isolation). `None`
    /// means every RPC operates against baseline, unchanged from before
    /// branching existed.
    branch: Option<MetadataValue<tonic::metadata::Ascii>>,
}

impl tonic::service::Interceptor for BearerAuth {
    fn call(&mut self, mut req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        if let Some(header) = self.header.as_ref() {
            req.metadata_mut().insert("authorization", header.clone());
        }
        if let Ok(val) =
            next_request_id(self.request_id_prefix).parse::<MetadataValue<tonic::metadata::Ascii>>()
        {
            req.metadata_mut().insert("x-request-id", val);
        }
        if let Some(branch) = self.branch.as_ref() {
            req.metadata_mut()
                .insert("x-trogon-atlas-branch", branch.clone());
        }
        Ok(req)
    }
}

pub type Client = EventModelServiceClient<InterceptedService<Channel, BearerAuth>>;

/// Connection options beyond the bare endpoint/token/timeout: every caller
/// so far wants "emclient" defaults, so these are optional overrides rather
/// than required positional args.
#[derive(Debug, Clone, Default)]
pub struct ConnectOptions {
    /// x-request-id prefix; defaults to "emclient" when `None`.
    pub request_id_prefix: Option<&'static str>,
    /// Max encode/decode message size in bytes; defaults to 4 MiB when `None`.
    pub max_message_bytes: Option<usize>,
    /// Branch to scope every RPC to via the `x-trogon-atlas-branch` metadata
    /// header (Phase 1: Isolation). `None` operates against baseline.
    pub branch: Option<String>,
}

/// Connect with the "emclient" default request-id prefix and the fixed
/// 4 MiB message cap. Convenience wrapper around [`connect_with_options`]
/// for the common case (trogon-atlas and any other default caller).
pub async fn connect(
    endpoint: &str,
    auth_token: Option<&str>,
    rpc_timeout_secs: u64,
) -> Result<Client> {
    connect_with_options(
        endpoint,
        auth_token,
        rpc_timeout_secs,
        ConnectOptions::default(),
    )
    .await
}

/// Connect with explicit overrides for the x-request-id prefix and the max
/// message size (e.g. the MCP adapter uses prefix "mcp" and a configurable
/// message cap).
pub async fn connect_with_options(
    endpoint: &str,
    auth_token: Option<&str>,
    rpc_timeout_secs: u64,
    options: ConnectOptions,
) -> Result<Client> {
    let mut channel_builder = Channel::from_shared(endpoint.to_string())
        .with_context(|| format!("invalid endpoint {endpoint:?}"))?;
    if endpoint.starts_with("https://") {
        channel_builder = channel_builder
            .tls_config(ClientTlsConfig::new().with_native_roots())
            .context("configuring client TLS")?;
    }
    let channel = channel_builder
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(rpc_timeout_secs))
        .connect()
        .await
        .with_context(|| format!("connecting to {endpoint}"))?;
    let header = match auth_token {
        Some(token) => Some(
            format!("Bearer {token}")
                .parse()
                .context("auth token contains characters invalid in a header")?,
        ),
        None => None,
    };
    let request_id_prefix = options.request_id_prefix.unwrap_or("emclient");
    let max_message_bytes = options
        .max_message_bytes
        .unwrap_or(DEFAULT_MAX_MESSAGE_BYTES);
    let branch = match options.branch.as_deref() {
        Some(name) => Some(
            name.parse()
                .with_context(|| format!("branch name {name:?} is not a valid header value"))?,
        ),
        None => None,
    };
    Ok(EventModelServiceClient::with_interceptor(
        channel,
        BearerAuth {
            header,
            request_id_prefix,
            branch,
        },
    )
    .max_decoding_message_size(max_message_bytes)
    .max_encoding_message_size(max_message_bytes))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use tonic::service::Interceptor as _;

    use super::*;

    #[test]
    fn bearer_auth_attaches_header_when_configured() {
        let mut auth = BearerAuth {
            header: Some("Bearer secret".parse().unwrap()),
            request_id_prefix: "emclient",
            branch: None,
        };
        let req = auth.call(tonic::Request::new(())).unwrap();
        assert_eq!(
            req.metadata().get("authorization").unwrap(),
            "Bearer secret"
        );

        let mut no_auth = BearerAuth {
            header: None,
            request_id_prefix: "emclient",
            branch: None,
        };
        let req = no_auth.call(tonic::Request::new(())).unwrap();
        assert!(req.metadata().get("authorization").is_none());
    }

    #[test]
    fn bearer_auth_attaches_x_request_id_with_prefix() {
        let mut auth = BearerAuth {
            header: None,
            request_id_prefix: "mcp",
            branch: None,
        };
        let req = auth.call(tonic::Request::new(())).unwrap();
        let id = req
            .metadata()
            .get("x-request-id")
            .expect("x-request-id must be present");
        let id_str = id.to_str().unwrap();
        assert!(
            id_str.starts_with("mcp-"),
            "id should start with mcp-: {id_str}"
        );
        assert!(
            id_str.len() <= 128,
            "id must be within server limit: {id_str}"
        );
    }

    #[test]
    fn bearer_auth_attaches_branch_header_when_configured() {
        let mut auth = BearerAuth {
            header: None,
            request_id_prefix: "emclient",
            branch: Some("alex/retention-rework".parse().unwrap()),
        };
        let req = auth.call(tonic::Request::new(())).unwrap();
        assert_eq!(
            req.metadata().get("x-trogon-atlas-branch").unwrap(),
            "alex/retention-rework"
        );

        let mut no_branch = BearerAuth {
            header: None,
            request_id_prefix: "emclient",
            branch: None,
        };
        let req = no_branch.call(tonic::Request::new(())).unwrap();
        assert!(req.metadata().get("x-trogon-atlas-branch").is_none());
    }

    #[test]
    fn consecutive_requests_get_distinct_ids() {
        let mut auth = BearerAuth {
            header: None,
            request_id_prefix: "emclient",
            branch: None,
        };
        let r1 = auth.call(tonic::Request::new(())).unwrap();
        let r2 = auth.call(tonic::Request::new(())).unwrap();
        let id1 = r1
            .metadata()
            .get("x-request-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let id2 = r2
            .metadata()
            .get("x-request-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_ne!(id1, id2, "consecutive requests must get distinct ids");
    }
}
