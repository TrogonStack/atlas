//! MCP over Streamable HTTP, served by this process so an agent reaches the
//! same deployment, and the same authentication and authorization, as every
//! gRPC caller.

use std::{convert::Infallible, time::Duration};

use tonic::service::interceptor::InterceptedService;
use tower::{timeout::error::Elapsed, BoxError, ServiceBuilder};
use trogon_atlas_client::client::{ConnectOptions, Transport};
use trogon_atlas_mcp::server::McpServer;
pub use trogon_atlas_mcp::{AllowedHosts, MCP_PATH};
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;

use crate::{
    auth::{AuthzLayer, AuthzService, BearerAuth},
    service::EventModelServiceImpl,
    telemetry::{InflightLayer, RpcMetricsLayer},
};

pub type DataPlane =
    InterceptedService<AuthzService<EventModelServiceServer<EventModelServiceImpl>>, BearerAuth>;

/// The one path to the EventModelService data plane, for network and MCP
/// callers alike. `BearerAuth` is the outermost wrapper so it puts the
/// `Principal` in extensions before `AuthzLayer` reads it; inverting the
/// nesting denies every request.
pub fn data_plane(
    service: EventModelServiceServer<EventModelServiceImpl>,
    auth: BearerAuth,
) -> DataPlane {
    InterceptedService::new(
        ServiceBuilder::new().layer(AuthzLayer).service(service),
        auth,
    )
}

/// MCP tools that call `data_plane` in process, with no network hop, under
/// the same RPC metrics, shutdown drain and RPC timeout as the gRPC listener.
pub fn mcp_server(
    data_plane: DataPlane,
    inflight: InflightLayer,
    rpc_timeout: Option<Duration>,
    max_message_bytes: usize,
) -> anyhow::Result<McpServer> {
    let stack = ServiceBuilder::new()
        .layer(RpcMetricsLayer::new())
        .layer(inflight)
        .map_err(deadline_exceeded)
        .option_layer(rpc_timeout.map(tower::timeout::TimeoutLayer::new))
        .map_err(|never: Infallible| -> BoxError { match never {} })
        .service(data_plane);
    McpServer::new(
        Transport::new(stack),
        ConnectOptions {
            request_id_prefix: Some("mcp"),
            max_message_bytes: Some(max_message_bytes),
            branch: None,
        },
    )
}

/// The `/mcp` route, for merging into the same `axum::Router` the gRPC
/// listener serves.
pub fn mcp_router(
    data_plane: DataPlane,
    inflight: InflightLayer,
    rpc_timeout: Option<Duration>,
    max_message_bytes: usize,
    allowed_hosts: AllowedHosts,
) -> anyhow::Result<axum::Router> {
    let server = mcp_server(data_plane, inflight, rpc_timeout, max_message_bytes)?;
    Ok(trogon_atlas_mcp::router(server, allowed_hosts))
}

fn deadline_exceeded(error: BoxError) -> BoxError {
    if error.is::<Elapsed>() {
        Box::new(tonic::Status::deadline_exceeded(error.to_string()))
    } else {
        error
    }
}
