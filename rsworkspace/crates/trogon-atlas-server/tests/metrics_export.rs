//! Metrics leave the process over OTLP, and are scraped at `/-/metrics` on
//! the gRPC listener only when Prometheus is opted into, outside auth.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{sync::Arc, time::Duration};

use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_proto::tonic::{
    collector::metrics::v1::{
        metrics_service_server::{MetricsService, MetricsServiceServer},
        ExportMetricsServiceRequest, ExportMetricsServiceResponse,
    },
    common::v1::any_value::Value as AnyValue,
    metrics::v1::{metric::Data, number_data_point::Value as NumberValue},
};
use opentelemetry_sdk::metrics::{
    data::{AggregatedMetrics, MetricData, ResourceMetrics, ScopeMetrics},
    InMemoryMetricExporter, SdkMeterProvider,
};
use tokio::{net::TcpListener, sync::mpsc};
use tonic::transport::{server::TcpIncoming, Server};
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{BearerAuth, RegistryFile, TokenRegistry},
    health::Health,
    mcp::data_plane,
    metrics_export::{otlp_meter_provider, MetricsExporters, OtlpRecorder, METRICS_PATH},
    service::EventModelServiceImpl,
};

const READER_TOKEN: &str = "metrics-reader-aaaa";

fn authenticated_registry() -> TokenRegistry {
    let file: RegistryFile = toml::from_str(&format!(
        "[principals.reader]\nrole = \"reader\"\ntokens = [\"{READER_TOKEN}\"]\n"
    ))
    .expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

/// The gRPC listener with routes merged the way `main.rs` merges them.
struct Listener {
    addr: std::net::SocketAddr,
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}

impl Listener {
    async fn start(metrics_router: axum::Router) -> Self {
        let store = trogon_atlas_testsupport::shared().await.store().await;
        let service =
            EventModelServiceServer::new(EventModelServiceImpl::try_new(Arc::new(store)).unwrap());
        let data_plane = data_plane(service, BearerAuth::new(authenticated_registry()));
        let health = Health::new().await;

        let mut routes = tonic::service::Routes::builder();
        routes.add_service(health.grpc_service());
        routes.add_service(data_plane);
        let mut routes = routes.routes();
        let merged = routes.axum_router_mut().clone().merge(health.router());
        *routes.axum_router_mut() = merged;
        let merged = routes.axum_router_mut().clone().merge(metrics_router);
        *routes.axum_router_mut() = merged;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = TcpIncoming::from(listener);
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(
            Server::builder()
                .accept_http1(true)
                .add_routes(routes)
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = shutdown_rx.await;
                }),
        );
        Self {
            addr,
            shutdown,
            task,
        }
    }

    /// Status line, headers and body of one unauthenticated HTTP/1.1 GET.
    async fn get(&self, path: &str) -> (u16, String, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        let status = response
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .expect("an HTTP status line");
        let (head, body) = response.split_once("\r\n\r\n").expect("an HTTP body");
        (status, head.to_ascii_lowercase(), body.to_owned())
    }

    async fn shut_down(self) {
        let _ = self.shutdown.send(());
        tokio::time::timeout(Duration::from_secs(10), self.task)
            .await
            .expect("the gRPC listener drains on shutdown")
            .unwrap()
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_route_is_not_found_by_default() {
    let (recorder, guard) = MetricsExporters::new(None, false).build();
    assert!(recorder.is_none());
    assert!(!guard.is_recording());

    let listener = Listener::start(guard.router()).await;
    let (status, _, body) = listener.get(METRICS_PATH).await;
    assert_eq!(
        (status, body.as_str()),
        (404, "prometheus metrics are disabled\n")
    );
    listener.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn prometheus_scrape_needs_no_bearer_token_while_the_data_plane_does() {
    let (recorder, guard) = MetricsExporters::new(None, true).build();
    let recorder = recorder.expect("a recorder when Prometheus is enabled");
    metrics::with_local_recorder(&recorder, || {
        metrics::counter!(
            "rpc_requests_total",
            "service" => "svc.X",
            "method" => "Y",
            "code" => "ok",
        )
        .increment(3);
    });

    let listener = Listener::start(guard.router()).await;
    let (status, head, body) = listener.get(METRICS_PATH).await;
    assert_eq!(status, 200);
    assert!(
        head.contains("content-type: text/plain; version=0.0.4"),
        "{head}"
    );
    assert!(
        body.contains(r#"rpc_requests_total{service="svc.X",method="Y",code="ok"} 3"#),
        "expected the counter in the scrape, got:\n{body}"
    );

    let channel = tonic::transport::Endpoint::from_shared(format!("http://{}", listener.addr))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let refused = EventModelServiceClient::new(channel)
        .list_entities(trogon_atlas_proto::ListEntitiesRequest::default())
        .await
        .unwrap_err();
    assert_eq!(refused.code(), tonic::Code::Unauthenticated);

    listener.shut_down().await;
}

fn in_memory_provider() -> (SdkMeterProvider, InMemoryMetricExporter) {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter.clone())
        .build();
    (provider, exporter)
}

fn exported(exporter: &InMemoryMetricExporter, name: &str) -> AggregatedMetrics {
    let finished = exporter.get_finished_metrics().unwrap();
    let metric = finished
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(ScopeMetrics::metrics)
        .filter(|metric| metric.name() == name)
        .last()
        .unwrap_or_else(|| panic!("{name} was not exported"));
    match metric.data() {
        AggregatedMetrics::F64(data) => AggregatedMetrics::F64(clone_data(data)),
        AggregatedMetrics::U64(data) => AggregatedMetrics::U64(clone_data(data)),
        AggregatedMetrics::I64(data) => AggregatedMetrics::I64(clone_data(data)),
    }
}

fn clone_data<T: Copy>(data: &MetricData<T>) -> MetricData<T> {
    match data {
        MetricData::Gauge(gauge) => MetricData::Gauge(gauge.clone()),
        MetricData::Sum(sum) => MetricData::Sum(sum.clone()),
        MetricData::Histogram(histogram) => MetricData::Histogram(histogram.clone()),
        MetricData::ExponentialHistogram(histogram) => {
            MetricData::ExponentialHistogram(histogram.clone())
        }
    }
}

fn labels<'a>(attributes: impl Iterator<Item = &'a opentelemetry::KeyValue>) -> Vec<String> {
    let mut labels: Vec<String> = attributes
        .map(|kv| format!("{}={}", kv.key, kv.value))
        .collect();
    labels.sort();
    labels
}

#[test]
fn the_bridge_keeps_names_labels_and_instrument_kinds() {
    let (provider, exporter) = in_memory_provider();
    let recorder = OtlpRecorder::new(provider.meter("test"));
    metrics::with_local_recorder(&recorder, || {
        let requests = metrics::counter!(
            "rpc_requests_total",
            "service" => "svc.X",
            "method" => "Y",
            "code" => "ok",
        );
        requests.increment(2);
        requests.increment(1);
        metrics::counter!("snapshot_cache_hits_total").absolute(5);
        metrics::counter!("snapshot_cache_hits_total").absolute(4);
        metrics::gauge!("changes_stream_messages").set(10.0);
        metrics::gauge!("changes_stream_messages").increment(2.0);
        metrics::gauge!("changes_stream_messages").decrement(1.0);
        let duration = metrics::histogram!(
            "rpc_request_duration_seconds",
            "service" => "svc.X",
            "method" => "Y",
        );
        duration.record(0.003);
        duration.record(0.2);
        duration.record(42.0);
    });
    provider.force_flush().unwrap();

    let AggregatedMetrics::U64(MetricData::Sum(requests)) =
        exported(&exporter, "rpc_requests_total")
    else {
        panic!("rpc_requests_total is not a u64 sum");
    };
    assert!(requests.is_monotonic());
    let point = requests.data_points().next().unwrap();
    assert_eq!(point.value(), 3);
    assert_eq!(
        labels(point.attributes()),
        ["code=ok", "method=Y", "service=svc.X"]
    );

    let AggregatedMetrics::U64(MetricData::Sum(hits)) =
        exported(&exporter, "snapshot_cache_hits_total")
    else {
        panic!("snapshot_cache_hits_total is not a u64 sum");
    };
    assert_eq!(hits.data_points().next().unwrap().value(), 5);

    let AggregatedMetrics::F64(MetricData::Gauge(messages)) =
        exported(&exporter, "changes_stream_messages")
    else {
        panic!("changes_stream_messages is not an f64 gauge");
    };
    assert!((messages.data_points().next().unwrap().value() - 11.0).abs() < f64::EPSILON);

    let AggregatedMetrics::F64(MetricData::Histogram(duration)) =
        exported(&exporter, "rpc_request_duration_seconds")
    else {
        panic!("rpc_request_duration_seconds is not an f64 histogram");
    };
    let point = duration.data_points().next().unwrap();
    assert_eq!(point.count(), 3);
    assert_eq!(labels(point.attributes()), ["method=Y", "service=svc.X"]);
    let bounds: Vec<f64> = point.bounds().collect();
    assert_eq!(bounds.first(), Some(&0.005));
    assert_eq!(bounds.last(), Some(&120.0));
    let counts: Vec<u64> = point.bucket_counts().collect();
    assert_eq!(counts.first(), Some(&1));
    assert_eq!(counts.last(), Some(&0));
    assert_eq!(counts.iter().sum::<u64>(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn both_backends_see_every_measurement_when_both_are_on() {
    let (provider, exporter) = in_memory_provider();
    let (recorder, guard) = MetricsExporters::new(Some(provider), true).build();
    let recorder = recorder.expect("a recorder when both backends are enabled");
    metrics::with_local_recorder(&recorder, || {
        metrics::counter!("llm_requests_total", "provider" => "test", "outcome" => "ok")
            .increment(1);
    });

    let listener = Listener::start(guard.router()).await;
    let (status, _, body) = listener.get(METRICS_PATH).await;
    listener.shut_down().await;
    assert_eq!(status, 200);
    assert!(body.contains("llm_requests_total"), "{body}");

    tokio::task::spawn_blocking(move || guard.shutdown())
        .await
        .unwrap();
    let AggregatedMetrics::U64(MetricData::Sum(sum)) = exported(&exporter, "llm_requests_total")
    else {
        panic!("llm_requests_total is not a u64 sum");
    };
    assert_eq!(sum.data_points().next().unwrap().value(), 1);
}

struct Collector {
    received: mpsc::UnboundedSender<ExportMetricsServiceRequest>,
}

#[async_trait::async_trait]
impl MetricsService for Collector {
    async fn export(
        &self,
        request: tonic::Request<ExportMetricsServiceRequest>,
    ) -> Result<tonic::Response<ExportMetricsServiceResponse>, tonic::Status> {
        let _ = self.received.send(request.into_inner());
        Ok(tonic::Response::new(ExportMetricsServiceResponse::default()))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn otlp_pushes_metrics_to_a_collector_over_grpc() {
    let (received, mut exports) = mpsc::unbounded_channel();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let collector = tokio::spawn(
        Server::builder()
            .add_service(MetricsServiceServer::new(Collector { received }))
            .serve_with_incoming_shutdown(TcpIncoming::from(listener), async {
                let _ = stopped.await;
            }),
    );

    let provider = otlp_meter_provider(&endpoint).unwrap();
    let (recorder, guard) = MetricsExporters::new(Some(provider), false).build();
    assert!(guard.is_recording());
    assert!(!guard.serves_prometheus());
    let recorder = recorder.expect("a recorder when OTLP is enabled");
    metrics::with_local_recorder(&recorder, || {
        metrics::counter!(
            "rpc_requests_total",
            "service" => "svc.X",
            "method" => "Y",
            "code" => "ok",
        )
        .increment(7);
    });
    tokio::task::spawn_blocking(move || guard.shutdown())
        .await
        .unwrap();

    let request = tokio::time::timeout(Duration::from_secs(10), exports.recv())
        .await
        .expect("the collector received an export")
        .unwrap();
    let _ = stop.send(());
    collector.await.unwrap().unwrap();

    let resource = request.resource_metrics.first().unwrap();
    let service_name = resource
        .resource
        .as_ref()
        .unwrap()
        .attributes
        .iter()
        .find(|kv| kv.key == "service.name")
        .and_then(|kv| kv.value.as_ref()?.value.clone());
    assert_eq!(
        service_name,
        Some(AnyValue::StringValue("trogon-atlas-server".to_owned()))
    );
    let metric = resource
        .scope_metrics
        .iter()
        .flat_map(|scope| &scope.metrics)
        .find(|metric| metric.name == "rpc_requests_total")
        .expect("rpc_requests_total was exported");
    let Some(Data::Sum(sum)) = &metric.data else {
        panic!("rpc_requests_total is not a sum");
    };
    assert!(sum.is_monotonic);
    let point = sum.data_points.first().unwrap();
    assert_eq!(point.value, Some(NumberValue::AsInt(7)));
    let mut labels: Vec<(String, Option<AnyValue>)> = point
        .attributes
        .iter()
        .map(|kv| {
            (
                kv.key.clone(),
                kv.value.as_ref().and_then(|v| v.value.clone()),
            )
        })
        .collect();
    labels.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        labels,
        [
            (
                "code".to_owned(),
                Some(AnyValue::StringValue("ok".to_owned()))
            ),
            (
                "method".to_owned(),
                Some(AnyValue::StringValue("Y".to_owned()))
            ),
            (
                "service".to_owned(),
                Some(AnyValue::StringValue("svc.X".to_owned()))
            ),
        ]
    );
}
