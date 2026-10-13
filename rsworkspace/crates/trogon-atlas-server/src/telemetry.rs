use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Instant,
};

use tower::{Layer, Service};
use tracing::Span;
use trogon_atlas_semconv::semconv::{
    attribute, meter::Meter, metric::RpcServerCallDurationAttr, options::InstrumentationOptions,
};

use crate::metrics::ServerMetrics;

pub const GRPC_SYSTEM: &str = "grpc";
pub const SERVICE_NAME: &str = "trogon-atlas-server";
pub const SERVICE_NAMESPACE: &str = "trogon-atlas";
/// `error.type` for a call whose inner service failed before producing any
/// response, so no gRPC status exists.
const ERROR_TYPE_TRANSPORT: &str = "transport_error";

/// `OTEL_SERVICE_NAME` wins over a `service.name` in
/// `OTEL_RESOURCE_ATTRIBUTES`, which wins over [`SERVICE_NAME`]. A
/// `service.namespace` in `OTEL_RESOURCE_ATTRIBUTES` wins over
/// [`SERVICE_NAMESPACE`].
/// `Resource::builder()` lets `OTEL_RESOURCE_ATTRIBUTES` override
/// `OTEL_SERVICE_NAME`, so the precedence is built explicitly.
fn resource() -> opentelemetry_sdk::Resource {
    use opentelemetry_sdk::resource::EnvResourceDetector;

    let resource_attributes = opentelemetry_sdk::Resource::builder_empty()
        .with_detector(Box::new(EnvResourceDetector::new()))
        .build();
    resource_from(
        &resource_attributes,
        std::env::var("OTEL_SERVICE_NAME").ok(),
    )
}

/// Each step overwrites the keys set before it, which is what gives
/// [`resource`] its precedence.
fn resource_from(
    resource_attributes: &opentelemetry_sdk::Resource,
    otel_service_name: Option<String>,
) -> opentelemetry_sdk::Resource {
    use opentelemetry_sdk::resource::TelemetryResourceDetector;

    let mut builder = opentelemetry_sdk::Resource::builder_empty()
        .with_service_name(SERVICE_NAME)
        .with_attribute(opentelemetry::KeyValue::new(
            "service.namespace",
            SERVICE_NAMESPACE,
        ))
        .with_detector(Box::new(TelemetryResourceDetector))
        .with_attributes(
            resource_attributes
                .iter()
                .map(|(key, value)| opentelemetry::KeyValue::new(key.clone(), value.clone())),
        );

    if let Some(name) = otel_service_name.filter(|name| !name.is_empty()) {
        builder = builder.with_service_name(name);
    }

    builder.build()
}

/// Shutdown handle returned by `init_telemetry`. When OTLP is enabled, this
/// wraps the batch `TracerProvider` and the periodic `SdkMeterProvider` so
/// the graceful-shutdown path can flush queued spans and metrics before the
/// process exits. When OTLP is disabled, `shutdown` is a no-op.
pub struct TelemetryGuard {
    trace_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    meter_provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
}

impl TelemetryGuard {
    pub fn exports_metrics(&self) -> bool {
        self.meter_provider.is_some()
    }

    /// The meter every instrument should be built from: bound to the OTLP
    /// meter provider when one is configured, recording nothing otherwise.
    pub fn meter(&self) -> Meter {
        match &self.meter_provider {
            Some(provider) => Meter::new(provider, InstrumentationOptions::default()),
            None => Meter::default(),
        }
    }

    /// Flush any spans and metrics buffered by the batch/periodic exporters
    /// and tear down both providers. Safe to call multiple times; subsequent
    /// calls are no-ops.
    pub fn shutdown(mut self) {
        if let Some(provider) = self.trace_provider.take() {
            if let Err(err) = provider.force_flush() {
                tracing::warn!(error = ?err, "otlp force_flush failed");
            }
            if let Err(err) = provider.shutdown() {
                tracing::warn!(error = ?err, "otlp tracer provider shutdown failed");
            }
        }
        if let Some(provider) = self.meter_provider.take() {
            if let Err(err) = provider.force_flush() {
                tracing::warn!(error = ?err, "otlp metrics force_flush failed");
            }
            if let Err(err) = provider.shutdown() {
                tracing::warn!(error = ?err, "otlp meter provider shutdown failed");
            }
        }
    }
}

/// Initialize JSON logging plus, when `OTEL_EXPORTER_OTLP_ENDPOINT` is set,
/// OTLP span and metric exporters. Returns a `TelemetryGuard` whose
/// `shutdown` method MUST be called on graceful shutdown so the batch and
/// periodic exporters flush their in-memory queues.
pub fn init_telemetry() -> anyhow::Result<TelemetryGuard> {
    use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

    let env_filter = match tracing_subscriber::EnvFilter::try_from_default_env() {
        Ok(f) => f,
        Err(e) => {
            // RUST_LOG is set but could not be parsed; warn before falling back
            // to info so operators know their filter directive was ignored.
            if std::env::var_os("RUST_LOG").is_some() {
                eprintln!(
                    "WARN trogon-atlas-server: RUST_LOG is set but could not be parsed ({e}); \
                     falling back to 'info'"
                );
            }
            "info".into()
        }
    };
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
        .json();
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    match std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        Ok(endpoint) if !endpoint.is_empty() => {
            use opentelemetry::trace::TracerProvider as _;
            use opentelemetry_otlp::{WithExportConfig as _, WithTonicConfig as _};

            let resource = resource();

            let span_exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint.clone())
                .with_tls_config(otlp_tls_config())
                .build()
                .map_err(|e| anyhow::anyhow!("building OTLP span exporter for {endpoint}: {e}"))?;
            let trace_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
                .with_batch_exporter(span_exporter)
                .with_resource(resource.clone())
                .build();
            let tracer = trace_provider.tracer(SERVICE_NAME);

            let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint.clone())
                .with_tls_config(otlp_tls_config())
                .build()
                .map_err(|e| {
                    anyhow::anyhow!("building OTLP metric exporter for {endpoint}: {e}")
                })?;
            let meter_provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
                .with_periodic_exporter(metric_exporter)
                .with_resource(resource)
                .build();

            registry
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .init();
            tracing::info!(endpoint = %endpoint, "otlp trace and metric exporters enabled");
            Ok(TelemetryGuard {
                trace_provider: Some(trace_provider),
                meter_provider: Some(meter_provider),
            })
        }
        _ => {
            registry.init();
            Ok(TelemetryGuard {
                trace_provider: None,
                meter_provider: None,
            })
        }
    }
}

/// Trusts the system roots for an `https://` collector; the exporter only
/// applies it when the endpoint scheme is `https`.
pub fn otlp_tls_config() -> opentelemetry_otlp::tonic_types::transport::ClientTlsConfig {
    opentelemetry_otlp::tonic_types::transport::ClientTlsConfig::new().with_native_roots()
}

fn rpc_span_from_path(path: &str) -> Span {
    let (service, method) = split_grpc_path(path);
    tracing::info_span!(
        "rpc",
        otel.name = %format!("{service}/{method}"),
        otel.kind = "server",
        rpc.system = GRPC_SYSTEM,
        rpc.service = %service,
        rpc.method = %method,
        request_id = tracing::field::Empty,
    )
}

/// `http.route` for a request that is not gRPC, when the path names a route
/// this listener knows about outside the gRPC services it serves.
fn http_route_for_path(path: &str) -> Option<&'static str> {
    (path == trogon_atlas_mcp::MCP_PATH).then_some(trogon_atlas_mcp::MCP_PATH)
}

/// Build an HTTP server span following OTel's HTTP semantic conventions, for
/// a request on the gRPC listener that is not itself gRPC (currently, MCP's
/// `/mcp`).
fn http_span_from_request<B>(req: &http::Request<B>) -> Span {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let route = http_route_for_path(&path);
    let otel_name = match route {
        Some(route) => format!("{method} {route}"),
        None => method.to_string(),
    };
    let span = tracing::info_span!(
        "http",
        otel.name = %otel_name,
        otel.kind = "server",
        http.request.method = %method,
        url.path = %path,
        http.route = tracing::field::Empty,
        request_id = tracing::field::Empty,
    );
    if let Some(route) = route {
        span.record("http.route", route);
    }
    span
}

/// Build a trace span seeded with the caller-supplied `x-request-id` (or a
/// generated one when the caller did not propagate one). Use this from
/// `Server::trace_fn` so every span carries a correlation id readable by
/// log/trace consumers downstream. A gRPC request gets the `rpc` span; any
/// other request on the same listener (MCP's `/mcp`) gets an HTTP server
/// span instead, so it is never mislabeled as an RPC with no real service or
/// method.
pub fn rpc_span_from_request<B>(req: &http::Request<B>) -> Span {
    if is_operational_path(req.uri().path()) {
        return Span::none();
    }
    let span = if is_grpc_request(req) {
        rpc_span_from_path(req.uri().path())
    } else {
        http_span_from_request(req)
    };
    let request_id =
        request_id_from_headers(req.headers()).unwrap_or_else(|| short_request_id(req));
    span.record("request_id", tracing::field::display(&request_id));
    span
}

/// Probes and health checks hit these paths every few seconds; tracing them
/// would spend sampling budget and export bandwidth on requests nobody reads.
fn is_operational_path(path: &str) -> bool {
    path.starts_with("/-/") || path.starts_with("/grpc.health.v1.Health/")
}

/// Whether a request on the gRPC listener is actually gRPC, as opposed to a
/// merged HTTP route such as MCP's `/mcp`. gRPC clients (and tonic's own
/// generated clients, including the in-process one MCP tool calls build)
/// always send `content-type: application/grpc[+proto|+json]`.
fn is_grpc_request<B>(req: &http::Request<B>) -> bool {
    req.headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/grpc"))
}

fn request_id_from_headers(headers: &http::HeaderMap) -> Option<String> {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.len() <= 128)
}

fn short_request_id<B>(req: &http::Request<B>) -> String {
    // Cheap, allocation-bounded id: monotonic counter folded to base36.
    // Avoids pulling in `uuid` for a value that is only meaningful inside
    // this process's tracing output.
    use std::sync::atomic::AtomicU64;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = req.uri().path();
    let last = path.rsplit('/').next().unwrap_or("rpc");
    format!("{last}-{n:08x}")
}

/// Increment-on-construction / decrement-on-drop guard around a shared
/// in-flight counter. Used by the graceful shutdown path to wait for
/// outstanding RPCs to drain. When the counter reaches zero, the watch
/// sender is notified so `await_inflight_drain` can wake immediately
/// instead of polling.
pub struct InflightGuard {
    counter: Arc<AtomicI64>,
    drain_notify: Arc<tokio::sync::watch::Sender<i64>>,
}

impl InflightGuard {
    pub fn new(
        counter: Arc<AtomicI64>,
        drain_notify: Arc<tokio::sync::watch::Sender<i64>>,
    ) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self {
            counter,
            drain_notify,
        }
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        let remaining = self.counter.fetch_sub(1, Ordering::Relaxed) - 1;
        let _ = self.drain_notify.send(remaining);
    }
}

fn split_grpc_path(path: &str) -> (&str, &str) {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    match trimmed.rsplit_once('/') {
        Some((svc, method)) => (svc, method),
        None => ("", trimmed),
    }
}

/// Periodically poll the store for change-stream retention stats and
/// publish them as gauges. SREs alert on these to detect
/// imminent `JetStream` retention pressure before the stream evicts data.
///
/// Emitted gauges (no attributes, the stream is a singleton per server):
/// - `trogon_atlas.change_stream.message.count`: current message count.
/// - `trogon_atlas.change_stream.size`: current byte count.
/// - `trogon_atlas.change_stream.oldest_message.age`: age of the oldest
///   retained message; `0` when the stream is empty.
///
/// Backends that do not report stats yield a `None`
/// from `change_stream_stats` and this loop becomes a no-op heartbeat.
pub fn spawn_change_stream_stats_publisher(
    store: std::sync::Arc<dyn trogon_atlas_store::Store>,
    interval: std::time::Duration,
    metrics: ServerMetrics,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut consecutive_failures: u32 = 0;
        loop {
            ticker.tick().await;
            match store.change_stream_stats().await {
                Ok(Some(stats)) => {
                    consecutive_failures = 0;
                    metrics.record_change_stream_stats(&stats);
                }
                Ok(None) => {
                    consecutive_failures = 0;
                }
                Err(err) => {
                    consecutive_failures += 1;
                    // Log at warn on the first failure and every 10th after,
                    // to alert without flooding logs during sustained outages.
                    if consecutive_failures == 1 || consecutive_failures.is_multiple_of(10) {
                        tracing::warn!(
                            error = %err,
                            consecutive_failures,
                            "change_stream_stats failed"
                        );
                    }
                }
            }
        }
    });
}

/// Tower middleware that records the `rpc.server.call.duration` histogram
/// for every incoming gRPC call. Server-streaming RPCs end past the response
/// head this layer observes, so they also record their terminal outcome via
/// [`StreamTerminationGuard`].
#[derive(Clone)]
pub struct RpcMetricsLayer {
    metrics: ServerMetrics,
}

/// Records the terminal outcome of a server-streaming RPC, which Tower
/// layers cannot observe past the response head. Handlers create one per
/// stream and `mark` the outcome; dropping it unmarked means the client
/// went away first.
pub struct StreamTerminationGuard {
    metrics: ServerMetrics,
    rpc_method: String,
    outcome: Option<attribute::StreamOutcome>,
}

impl StreamTerminationGuard {
    #[must_use]
    pub fn new(metrics: ServerMetrics, service: &str, method: &str) -> Self {
        Self {
            metrics,
            rpc_method: format!("{service}/{method}"),
            outcome: None,
        }
    }

    pub fn mark(&mut self, outcome: attribute::StreamOutcome) {
        self.outcome = Some(outcome);
    }
}

impl Drop for StreamTerminationGuard {
    fn drop(&mut self) {
        self.metrics.record_stream_termination(
            std::mem::take(&mut self.rpc_method),
            self.outcome
                .unwrap_or(attribute::StreamOutcome::ClientDisconnected),
        );
    }
}

impl RpcMetricsLayer {
    #[must_use]
    pub fn new(metrics: ServerMetrics) -> Self {
        Self { metrics }
    }
}

impl<S> Layer<S> for RpcMetricsLayer {
    type Service = RpcMetricsService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        RpcMetricsService {
            inner,
            metrics: self.metrics.clone(),
        }
    }
}

#[derive(Clone)]
pub struct RpcMetricsService<S> {
    inner: S,
    metrics: ServerMetrics,
}

impl<S, ReqBody, ResBody> Service<http::Request<ReqBody>> for RpcMetricsService<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + std::fmt::Display + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = http::Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let rpc_method = is_grpc_request(&req).then(|| rpc_method_from_path(req.uri().path()));
        let started = Instant::now();
        let metrics = self.metrics.clone();

        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            let result = inner.call(req).await;
            // A merged non-gRPC route (MCP's `/mcp`): its own tool calls
            // already record RPC metrics through the inner `data_plane`
            // stack, so this outer layer would otherwise double-count every
            // MCP request under a fake method.
            let Some(rpc_method) = rpc_method else {
                return result;
            };
            let elapsed = started.elapsed();
            let mut attributes = vec![RpcServerCallDurationAttr::RpcMethod(rpc_method.into())];
            match &result {
                Ok(resp) => {
                    let code = grpc_status_code(resp);
                    let name = grpc_status_code_name(code);
                    attributes.push(RpcServerCallDurationAttr::RpcResponseStatusCode(
                        name.into(),
                    ));
                    if code != tonic::Code::Ok {
                        attributes.push(RpcServerCallDurationAttr::ErrorType(name.into()));
                    }
                }
                Err(_) => {
                    attributes.push(RpcServerCallDurationAttr::ErrorType(
                        ERROR_TYPE_TRANSPORT.into(),
                    ));
                }
            }
            metrics.record_rpc_call(elapsed, attributes);
            result
        })
    }
}

/// Tower layer that bumps a shared in-flight counter for the duration of
/// every RPC. The graceful-shutdown path in `main.rs` waits on the drain
/// channel to know when all outstanding work has finished.
#[derive(Clone)]
pub struct InflightLayer {
    counter: Arc<AtomicI64>,
    drain_notify: Arc<tokio::sync::watch::Sender<i64>>,
}

impl InflightLayer {
    pub fn new(
        counter: Arc<AtomicI64>,
        drain_notify: Arc<tokio::sync::watch::Sender<i64>>,
    ) -> Self {
        Self {
            counter,
            drain_notify,
        }
    }
}

impl<S> Layer<S> for InflightLayer {
    type Service = InflightService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        InflightService {
            inner,
            counter: self.counter.clone(),
            drain_notify: self.drain_notify.clone(),
        }
    }
}

#[derive(Clone)]
pub struct InflightService<S> {
    inner: S,
    counter: Arc<AtomicI64>,
    drain_notify: Arc<tokio::sync::watch::Sender<i64>>,
}

impl<S, ReqBody, ResBody> Service<http::Request<ReqBody>> for InflightService<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = http::Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let counter = self.counter.clone();
        let drain_notify = self.drain_notify.clone();
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        Box::pin(async move {
            let _guard = InflightGuard::new(counter, drain_notify);
            inner.call(req).await
        })
    }
}

/// The call's gRPC status from the `grpc-status` response header. Tonic only
/// puts it there for trailers-only responses; a successful unary response
/// carries it in trailers this layer cannot see, so an absent header is OK.
fn grpc_status_code<B>(resp: &http::Response<B>) -> tonic::Code {
    resp.headers()
        .get("grpc-status")
        .map_or(tonic::Code::Ok, |v| tonic::Code::from_bytes(v.as_bytes()))
}

fn grpc_status_code_name(code: tonic::Code) -> &'static str {
    match code {
        tonic::Code::Ok => "OK",
        tonic::Code::Cancelled => "CANCELLED",
        tonic::Code::Unknown => "UNKNOWN",
        tonic::Code::InvalidArgument => "INVALID_ARGUMENT",
        tonic::Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
        tonic::Code::NotFound => "NOT_FOUND",
        tonic::Code::AlreadyExists => "ALREADY_EXISTS",
        tonic::Code::PermissionDenied => "PERMISSION_DENIED",
        tonic::Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        tonic::Code::FailedPrecondition => "FAILED_PRECONDITION",
        tonic::Code::Aborted => "ABORTED",
        tonic::Code::OutOfRange => "OUT_OF_RANGE",
        tonic::Code::Unimplemented => "UNIMPLEMENTED",
        tonic::Code::Internal => "INTERNAL",
        tonic::Code::Unavailable => "UNAVAILABLE",
        tonic::Code::DataLoss => "DATA_LOSS",
        tonic::Code::Unauthenticated => "UNAUTHENTICATED",
    }
}

/// `rpc.method` in its `{fully.qualified.Service}/{Method}` form.
fn rpc_method_from_path(path: &str) -> String {
    path.strip_prefix('/').unwrap_or(path).to_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{is_grpc_request, is_operational_path, resource_from, split_grpc_path};

    fn resource_attributes(pairs: &[(&'static str, &'static str)]) -> opentelemetry_sdk::Resource {
        opentelemetry_sdk::Resource::builder_empty()
            .with_attributes(
                pairs
                    .iter()
                    .map(|(key, value)| opentelemetry::KeyValue::new(*key, *value)),
            )
            .build()
    }

    fn attribute(resource: &opentelemetry_sdk::Resource, key: &'static str) -> Option<String> {
        resource
            .get(&opentelemetry::Key::from_static_str(key))
            .map(|value| value.to_string())
    }

    #[test]
    fn resource_names_the_service_within_the_trogon_atlas_namespace() {
        let resource = resource_from(&resource_attributes(&[]), None);
        assert_eq!(
            attribute(&resource, "service.name").as_deref(),
            Some("trogon-atlas-server")
        );
        assert_eq!(
            attribute(&resource, "service.namespace").as_deref(),
            Some("trogon-atlas")
        );
    }

    #[test]
    fn resource_attributes_override_the_defaults() {
        let resource = resource_from(
            &resource_attributes(&[
                ("service.name", "from-attributes"),
                ("service.namespace", "custom"),
            ]),
            None,
        );
        assert_eq!(
            attribute(&resource, "service.name").as_deref(),
            Some("from-attributes")
        );
        assert_eq!(
            attribute(&resource, "service.namespace").as_deref(),
            Some("custom")
        );
    }

    #[test]
    fn otel_service_name_overrides_resource_attributes() {
        let resource = resource_from(
            &resource_attributes(&[
                ("service.name", "from-attributes"),
                ("service.namespace", "custom"),
            ]),
            Some("from-service-name".to_owned()),
        );
        assert_eq!(
            attribute(&resource, "service.name").as_deref(),
            Some("from-service-name")
        );
        assert_eq!(
            attribute(&resource, "service.namespace").as_deref(),
            Some("custom")
        );
    }

    #[test]
    fn empty_otel_service_name_keeps_the_resolved_name() {
        let resource = resource_from(&resource_attributes(&[]), Some(String::new()));
        assert_eq!(
            attribute(&resource, "service.name").as_deref(),
            Some("trogon-atlas-server")
        );
    }

    #[test]
    fn parses_canonical_grpc_path() {
        let (svc, method) =
            split_grpc_path("/trogonatlas.api.eventmodel.v1alpha1.EventModelService/PutEntity");
        assert_eq!(svc, "trogonatlas.api.eventmodel.v1alpha1.EventModelService");
        assert_eq!(method, "PutEntity");
    }

    #[test]
    fn handles_path_without_leading_slash() {
        let (svc, method) = split_grpc_path("svc.X/M");
        assert_eq!(svc, "svc.X");
        assert_eq!(method, "M");
    }

    #[test]
    fn handles_path_without_service() {
        let (svc, method) = split_grpc_path("/M");
        assert_eq!(svc, "");
        assert_eq!(method, "M");
    }

    #[test]
    fn probes_and_health_checks_are_operational_paths() {
        assert!(is_operational_path("/-/readiness"));
        assert!(is_operational_path("/-/liveness"));
        assert!(is_operational_path("/grpc.health.v1.Health/Check"));
        assert!(is_operational_path("/grpc.health.v1.Health/Watch"));
        assert!(!is_operational_path("/mcp"));
        assert!(!is_operational_path(
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/PutEntity"
        ));
    }

    #[test]
    fn is_grpc_request_matches_the_grpc_content_type_only() {
        let grpc = http::Request::builder()
            .header(http::header::CONTENT_TYPE, "application/grpc+proto")
            .body(())
            .unwrap();
        assert!(is_grpc_request(&grpc));

        let mcp = http::Request::builder()
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(())
            .unwrap();
        assert!(!is_grpc_request(&mcp));

        let no_content_type = http::Request::builder().body(()).unwrap();
        assert!(!is_grpc_request(&no_content_type));
    }
}
