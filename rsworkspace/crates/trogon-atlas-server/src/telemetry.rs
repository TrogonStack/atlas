use std::{
    future::Future,
    net::SocketAddr,
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

pub const GRPC_SYSTEM: &str = "grpc";

/// Shutdown handle returned by `init_tracing`. When OTLP is enabled, this
/// wraps the batch `TracerProvider` so the graceful-shutdown path can
/// flush queued spans before the process exits. When OTLP is disabled (or
/// failed to initialize), `shutdown` is a no-op.
pub struct TracingGuard {
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl TracingGuard {
    /// Flush any spans buffered by the batch exporter and tear down the
    /// provider. Safe to call multiple times; subsequent calls are no-ops.
    pub fn shutdown(mut self) {
        if let Some(provider) = self.provider.take() {
            if let Err(err) = provider.force_flush() {
                tracing::warn!(error = ?err, "otlp force_flush failed");
            }
            if let Err(err) = provider.shutdown() {
                tracing::warn!(error = ?err, "otlp tracer provider shutdown failed");
            }
        }
    }
}

/// Initialize JSON logging plus, when `OTEL_EXPORTER_OTLP_ENDPOINT` is set,
/// an OTLP span exporter so the `rpc`/`llm.complete` spans leave the
/// process instead of dying in stdout. Returns a `TracingGuard` whose
/// `shutdown` method MUST be called on graceful shutdown so the batch
/// exporter flushes its in-memory queue.
pub fn init_tracing() -> anyhow::Result<TracingGuard> {
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
            use opentelemetry_otlp::WithExportConfig as _;
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint.clone())
                .build()
                .map_err(|e| anyhow::anyhow!("building OTLP exporter for {endpoint}: {e}"))?;
            let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(
                    opentelemetry_sdk::Resource::builder()
                        .with_service_name("trogon-atlas-server")
                        .build(),
                )
                .build();
            let tracer = provider.tracer("trogon-atlas-server");
            // Set the global provider so opentelemetry::global::tracer works,
            // and keep a clone for explicit shutdown so the batch exporter
            // flushes queued spans before the process exits.
            opentelemetry::global::set_tracer_provider(provider.clone());
            registry
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .init();
            tracing::info!(endpoint = %endpoint, "otlp trace exporter enabled");
            Ok(TracingGuard {
                provider: Some(provider),
            })
        }
        _ => {
            registry.init();
            Ok(TracingGuard { provider: None })
        }
    }
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

/// Build an RPC span seeded with the caller-supplied `x-request-id` (or a
/// generated one when the caller did not propagate one). Use this from
/// `Server::trace_fn` so every span carries a correlation id readable by
/// log/trace consumers downstream.
pub fn rpc_span_from_request<B>(req: &http::Request<B>) -> Span {
    let path = req.uri().path();
    let span = rpc_span_from_path(path);
    let request_id =
        request_id_from_headers(req.headers()).unwrap_or_else(|| short_request_id(req));
    span.record("request_id", tracing::field::display(&request_id));
    span
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

/// Record the time a caller spent blocked acquiring the global mutation
/// lock. Exposes contention as a histogram alongside the other RPC metrics.
pub fn record_mutation_lock_wait(seconds: f64) {
    metrics::histogram!("mutation_lock_wait_seconds").record(seconds);
}

/// Snapshot-cache hit/miss counters. Both are emitted as counters (not
/// gauges) so the rate of misses is alertable.
pub fn record_snapshot_cache_hit() {
    metrics::counter!("snapshot_cache_hits_total").increment(1);
}

pub fn record_snapshot_cache_miss() {
    metrics::counter!("snapshot_cache_misses_total").increment(1);
}

/// Branch-search index cache hit/miss counters. A miss means one branch
/// query rebuilt a whole index, which is the cost this cache exists to
/// avoid; a sustained miss rate means branches are being written between
/// every search, or the cache is too small for the number of branches under
/// review.
pub fn record_branch_search_index_hit() {
    metrics::counter!("branch_search_index_cache_hits_total").increment(1);
}

pub fn record_branch_search_index_miss() {
    metrics::counter!("branch_search_index_cache_misses_total").increment(1);
}

fn split_grpc_path(path: &str) -> (&str, &str) {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    match trimmed.rsplit_once('/') {
        Some((svc, method)) => (svc, method),
        None => ("", trimmed),
    }
}

/// Install the global Prometheus metrics recorder and spawn an HTTP exporter.
///
/// The exporter serves `/metrics` (and reports its own healthcheck on `/`).
/// Returns an error if a recorder is already installed or the listener cannot bind.
///
/// When `addr` binds a non-loopback interface the caller must also pass
/// `allow_external = true`; otherwise startup is refused with a clear error.
/// Loopback addresses (127.x.x.x / `::1`) are always permitted. Set
/// `TROGON_ATLAS_METRICS_ALLOW_EXTERNAL=true` to opt in to external binding.
pub fn install_prometheus_exporter(addr: SocketAddr, allow_external: bool) -> anyhow::Result<()> {
    let is_loopback = addr.ip().is_loopback();
    if !is_loopback && !allow_external {
        anyhow::bail!(
            "metrics listen address {addr} is non-loopback but \
             TROGON_ATLAS_METRICS_ALLOW_EXTERNAL is not set. Set it to `true` to \
             explicitly opt in to exposing /metrics on a non-loopback interface, \
             or change --metrics-listen to a loopback address (e.g. 127.0.0.1:9090)."
        );
    }
    use metrics_exporter_prometheus::PrometheusBuilder;
    PrometheusBuilder::new()
        .with_http_listener(addr)
        .install()
        .map_err(|e| anyhow::anyhow!("install prometheus exporter on {addr}: {e}"))?;
    Ok(())
}

/// Periodically poll the store for change-stream retention stats and
/// publish them as Prometheus gauges. SREs alert on these to detect
/// imminent `JetStream` retention pressure before the stream evicts data.
///
/// Emitted gauges (no labels, the stream is a singleton per server):
/// - `changes_stream_messages`: current message count.
/// - `changes_stream_bytes`: current byte count.
/// - `changes_stream_oldest_age_seconds`: age of the oldest retained
///   message; `0` when the stream is empty.
///
/// Backends that do not report stats yield a `None`
/// from `change_stream_stats` and this loop becomes a no-op heartbeat.
pub fn spawn_change_stream_stats_publisher(
    store: std::sync::Arc<dyn trogon_atlas_store::Store>,
    interval: std::time::Duration,
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
                    #[allow(clippy::cast_precision_loss)]
                    {
                        metrics::gauge!("changes_stream_messages").set(stats.messages as f64);
                        metrics::gauge!("changes_stream_bytes").set(stats.bytes as f64);
                    }
                    let age = match stats.oldest_message_unix_seconds {
                        Some(secs) => {
                            let now = chrono::Utc::now().timestamp();
                            #[allow(clippy::cast_precision_loss)]
                            let age_f = (now - secs).max(0) as f64;
                            age_f
                        }
                        None => 0.0,
                    };
                    metrics::gauge!("changes_stream_oldest_age_seconds").set(age);
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

/// Tower middleware that records per-RPC counters and a duration histogram.
///
/// Emitted metrics (labelled by `service` + `method`, plus `code` on the
/// counters):
/// - `rpc_requests_total{service, method, code}`: every RPC, success or fail.
/// - `rpc_request_duration_seconds{service, method}`: wall-clock latency
///   measured from request entry to response head.
/// - `rpc_errors_total{service, method, code}`: subset of `rpc_requests_total`
///   where `code != "ok"`.
///
/// gRPC status is read from the `grpc-status` response header. For
/// "trailers-only" error responses tonic emits that header inline, so we
/// observe it directly. For successful unary responses tonic emits the
/// status only in trailers, which a Tower layer cannot observe without
/// wrapping the body; we default the code to `"ok"` when the header is
/// absent, which is accurate for unary RPCs. Server-streaming RPCs record
/// their terminal outcome separately via [`StreamTerminationGuard`]
/// (`rpc_stream_terminations_total{method, outcome}`).
#[derive(Clone, Default)]
pub struct RpcMetricsLayer;

/// Records the terminal outcome of a server-streaming RPC, which Tower
/// layers cannot observe past the response head. Handlers create one per
/// stream and `mark` the outcome; dropping it unmarked means the client
/// went away first.
pub struct StreamTerminationGuard {
    method: &'static str,
    outcome: Option<&'static str>,
}

impl StreamTerminationGuard {
    #[must_use]
    pub fn new(method: &'static str) -> Self {
        Self {
            method,
            outcome: None,
        }
    }

    pub fn mark(&mut self, outcome: &'static str) {
        self.outcome = Some(outcome);
    }
}

impl Drop for StreamTerminationGuard {
    fn drop(&mut self) {
        metrics::counter!(
            "rpc_stream_terminations_total",
            "method" => self.method,
            "outcome" => self.outcome.unwrap_or("client_disconnected"),
        )
        .increment(1);
    }
}

impl RpcMetricsLayer {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for RpcMetricsLayer {
    type Service = RpcMetricsService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        RpcMetricsService { inner }
    }
}

#[derive(Clone)]
pub struct RpcMetricsService<S> {
    inner: S,
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
        let (service, method) = {
            let (s, m) = split_grpc_path(req.uri().path());
            (s.to_owned(), m.to_owned())
        };
        let started = Instant::now();

        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            let result = inner.call(req).await;
            let elapsed = started.elapsed().as_secs_f64();
            let code = match &result {
                Ok(resp) => header_grpc_status(resp).unwrap_or_else(|| "ok".to_string()),
                Err(_) => "transport_error".to_string(),
            };

            metrics::counter!(
                "rpc_requests_total",
                "service" => service.clone(),
                "method" => method.clone(),
                "code" => code.clone(),
            )
            .increment(1);

            metrics::histogram!(
                "rpc_request_duration_seconds",
                "service" => service.clone(),
                "method" => method.clone(),
            )
            .record(elapsed);

            // Treat "ok" (tonic's label) and numeric 0 (gRPC spec) as
            // success. Parse the raw header value to a tonic::Code so any
            // future label changes or non-string representations are handled
            // uniformly rather than by growing the string-match list.
            let is_ok = code == "ok"
                || code
                    .parse::<i32>()
                    .ok()
                    .map(tonic::Code::from)
                    .is_some_and(|c| c == tonic::Code::Ok);
            if !is_ok {
                metrics::counter!(
                    "rpc_errors_total",
                    "service" => service,
                    "method" => method,
                    "code" => code,
                )
                .increment(1);
            }

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

fn header_grpc_status<B>(resp: &http::Response<B>) -> Option<String> {
    resp.headers()
        .get("grpc-status")
        .and_then(|v| v.to_str().ok())
        .map(std::borrow::ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::split_grpc_path;

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
}
