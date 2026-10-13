//! MCP over Streamable HTTP, served by this process so an agent reaches the
//! same deployment, and the same authentication and authorization, as every
//! gRPC caller.

use std::{convert::Infallible, time::Duration};

use axum::{extract::Request, middleware, middleware::Next, response::Response};
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

/// The deadline `tonic::transport::Server::timeout` would apply, scoped to
/// the routes already on `grpc_router`: `axum::Router::layer` skips routes
/// merged later, which is how `/mcp` keeps only its per-RPC timeout.
pub fn with_grpc_timeout(grpc_router: axum::Router, server_timeout: Duration) -> axum::Router {
    grpc_router.layer(middleware::from_fn(move |request: Request, next: Next| {
        enforce_grpc_timeout(request, next, server_timeout)
    }))
}

async fn enforce_grpc_timeout(request: Request, next: Next, server_timeout: Duration) -> Response {
    let timeout = client_grpc_timeout(request.headers()).map_or(server_timeout, |client_timeout| {
        client_timeout.min(server_timeout)
    });
    match tokio::time::timeout(timeout, next.run(request)).await {
        Ok(response) => response,
        Err(_elapsed) => grpc_timeout_response(),
    }
}

/// Matches tonic's own timeout response, which is `CANCELLED`, not
/// `DEADLINE_EXCEEDED`.
fn grpc_timeout_response() -> Response {
    let status = tonic::Status::try_from_error(Box::new(tonic::TimeoutExpired(())))
        .unwrap_or_else(|_| tonic::Status::cancelled("request timed out"));
    let (parts, ()) = status.into_http::<()>().into_parts();
    Response::from_parts(parts, axum::body::Body::empty())
}

fn client_grpc_timeout(headers: &axum::http::HeaderMap) -> Option<Duration> {
    let value = headers.get("grpc-timeout")?.to_str().ok()?;
    if value.is_empty() || value.len() > 9 {
        return None;
    }
    let (digits, unit) = value.split_at(value.len() - 1);
    let amount: u64 = digits.parse().ok()?;
    match unit {
        "H" => Some(Duration::from_secs(amount.saturating_mul(3600))),
        "M" => Some(Duration::from_secs(amount.saturating_mul(60))),
        "S" => Some(Duration::from_secs(amount)),
        "m" => Some(Duration::from_millis(amount)),
        "u" => Some(Duration::from_micros(amount)),
        "n" => Some(Duration::from_nanos(amount)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use axum::{body::Body, http::StatusCode, routing::post};
    use tower::ServiceExt;

    use super::*;

    const SERVER_TIMEOUT: Duration = Duration::from_secs(5);
    const SLOW: Duration = Duration::from_secs(60);

    async fn slow_handler() -> &'static str {
        tokio::time::sleep(SLOW).await;
        "done"
    }

    fn listener_router() -> axum::Router {
        let grpc = axum::Router::new().route("/grpc.Service/Call", post(slow_handler));
        with_grpc_timeout(grpc, SERVER_TIMEOUT)
            .merge(axum::Router::new().route(MCP_PATH, post(slow_handler)))
    }

    fn request(path: &str, grpc_timeout: Option<&str>) -> Request {
        let mut builder = Request::post(path);
        if let Some(value) = grpc_timeout {
            builder = builder.header("grpc-timeout", value);
        }
        builder.body(Body::empty()).unwrap()
    }

    fn grpc_status(response: &Response) -> Option<tonic::Code> {
        tonic::Status::from_header_map(response.headers()).map(|status| status.code())
    }

    #[tokio::test(start_paused = true)]
    async fn grpc_routes_are_cut_off_at_the_server_timeout() {
        let started = tokio::time::Instant::now();
        let response = listener_router()
            .oneshot(request("/grpc.Service/Call", None))
            .await
            .unwrap();

        assert_eq!(grpc_status(&response), Some(tonic::Code::Cancelled));
        assert_eq!(started.elapsed(), SERVER_TIMEOUT);
    }

    #[tokio::test(start_paused = true)]
    async fn a_shorter_client_grpc_timeout_wins() {
        let started = tokio::time::Instant::now();
        let response = listener_router()
            .oneshot(request("/grpc.Service/Call", Some("250m")))
            .await
            .unwrap();

        assert_eq!(grpc_status(&response), Some(tonic::Code::Cancelled));
        assert_eq!(started.elapsed(), Duration::from_millis(250));
    }

    #[tokio::test(start_paused = true)]
    async fn mcp_merged_after_the_layer_outlives_the_grpc_timeout() {
        let response = listener_router()
            .oneshot(request(MCP_PATH, None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(grpc_status(&response), None);
    }

    #[test]
    fn parses_every_grpc_timeout_unit() {
        let parse = |value: &str| {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert("grpc-timeout", value.parse().unwrap());
            client_grpc_timeout(&headers)
        };

        assert_eq!(parse("2H"), Some(Duration::from_secs(7200)));
        assert_eq!(parse("3M"), Some(Duration::from_secs(180)));
        assert_eq!(parse("4S"), Some(Duration::from_secs(4)));
        assert_eq!(parse("5m"), Some(Duration::from_millis(5)));
        assert_eq!(parse("6u"), Some(Duration::from_micros(6)));
        assert_eq!(parse("7n"), Some(Duration::from_nanos(7)));
        assert_eq!(parse("123456789S"), None);
        assert_eq!(parse("1x"), None);
        assert_eq!(parse("S"), None);
    }
}
