//! Instruments record through the `MeterProvider` they are built from, with
//! the names and attributes `otel/registry` declares.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::BTreeMap, convert::Infallible};

use opentelemetry_sdk::metrics::{
    data::{AggregatedMetrics, MetricData, ResourceMetrics, ScopeMetrics},
    InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
};
use tower::Layer as _;
use trogon_atlas_semconv::semconv::{
    attribute, meter::Meter, metric, options::InstrumentationOptions,
};
use trogon_atlas_server::{metrics::ServerMetrics, telemetry::RpcMetricsLayer};

type Attributes = BTreeMap<String, String>;

struct Telemetry {
    provider: SdkMeterProvider,
    exporter: InMemoryMetricExporter,
    metrics: ServerMetrics,
}

impl Telemetry {
    fn new() -> Self {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build())
            .build();
        let metrics = ServerMetrics::new(&Meter::new(&provider, InstrumentationOptions::default()));
        Self {
            provider,
            exporter,
            metrics,
        }
    }

    fn rpc_metrics_layer(&self) -> RpcMetricsLayer {
        RpcMetricsLayer::new(self.metrics.clone())
    }

    /// Every data point recorded so far under `name`, as its attributes and
    /// its count (histograms) or value (counters).
    fn data_points(&self, name: &str) -> Vec<(Attributes, u64)> {
        self.provider.force_flush().unwrap();
        let exported = self.exporter.get_finished_metrics().unwrap();
        exported
            .iter()
            .flat_map(ResourceMetrics::scope_metrics)
            .flat_map(ScopeMetrics::metrics)
            .filter(|m| m.name() == name)
            .flat_map(|m| match m.data() {
                AggregatedMetrics::F64(MetricData::Histogram(h)) => h
                    .data_points()
                    .map(|dp| (attributes(dp.attributes()), dp.count()))
                    .collect::<Vec<_>>(),
                AggregatedMetrics::U64(MetricData::Sum(s)) => s
                    .data_points()
                    .map(|dp| (attributes(dp.attributes()), dp.value()))
                    .collect(),
                other => panic!("{name} exported unexpected data {other:?}"),
            })
            .collect()
    }
}

fn attributes<'a>(kvs: impl Iterator<Item = &'a opentelemetry::KeyValue>) -> Attributes {
    kvs.map(|kv| (kv.key.to_string(), kv.value.to_string()))
        .collect()
}

fn expected(pairs: &[(&str, &str)]) -> Attributes {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

async fn call<S>(mut service: S, path: &str, content_type: &str) -> Result<(), String>
where
    S: tower::Service<http::Request<()>, Response = http::Response<()>>,
    S::Error: std::fmt::Display,
{
    let req = http::Request::builder()
        .uri(path)
        .header(http::header::CONTENT_TYPE, content_type)
        .body(())
        .unwrap();
    service.call(req).await.map(drop).map_err(|e| e.to_string())
}

fn response(grpc_status: Option<&'static str>) -> http::Response<()> {
    let mut resp = http::Response::new(());
    if let Some(code) = grpc_status {
        resp.headers_mut()
            .insert("grpc-status", http::HeaderValue::from_static(code));
    }
    resp
}

#[tokio::test]
async fn successful_call_records_duration_with_ok_status() {
    let telemetry = Telemetry::new();
    let service = telemetry
        .rpc_metrics_layer()
        .layer(tower::service_fn(|_| async {
            Ok::<_, Infallible>(response(None))
        }));
    call(service, "/test.v1.OkService/Get", "application/grpc")
        .await
        .unwrap();

    let points = telemetry.data_points(metric::RpcServerCallDuration::NAME);
    let want = expected(&[
        ("rpc.system.name", "grpc"),
        ("rpc.method", "test.v1.OkService/Get"),
        ("rpc.response.status_code", "OK"),
    ]);
    assert!(
        points
            .iter()
            .any(|(attrs, count)| *attrs == want && *count == 1),
        "no OK data point with {want:?} in {points:?}"
    );
}

#[tokio::test]
async fn failed_call_records_status_code_as_error_type() {
    let telemetry = Telemetry::new();
    let service = telemetry
        .rpc_metrics_layer()
        .layer(tower::service_fn(|_| async {
            Ok::<_, Infallible>(response(Some("5")))
        }));
    call(
        service,
        "/test.v1.MissingService/Get",
        "application/grpc+proto",
    )
    .await
    .unwrap();

    let points = telemetry.data_points(metric::RpcServerCallDuration::NAME);
    let want = expected(&[
        ("rpc.system.name", "grpc"),
        ("rpc.method", "test.v1.MissingService/Get"),
        ("rpc.response.status_code", "NOT_FOUND"),
        ("error.type", "NOT_FOUND"),
    ]);
    assert!(
        points
            .iter()
            .any(|(attrs, count)| *attrs == want && *count == 1),
        "no NOT_FOUND data point with {want:?} in {points:?}"
    );
}

#[tokio::test]
async fn transport_failure_records_transport_error_without_status() {
    let telemetry = Telemetry::new();
    let failing = tower::service_fn(|_req: http::Request<()>| async {
        Err::<http::Response<()>, _>("connection reset")
    });
    let service = telemetry.rpc_metrics_layer().layer(failing);
    call(service, "/test.v1.BrokenService/Get", "application/grpc")
        .await
        .unwrap_err();

    let points = telemetry.data_points(metric::RpcServerCallDuration::NAME);
    let want = expected(&[
        ("rpc.system.name", "grpc"),
        ("rpc.method", "test.v1.BrokenService/Get"),
        ("error.type", "transport_error"),
    ]);
    assert!(
        points
            .iter()
            .any(|(attrs, count)| *attrs == want && *count == 1),
        "no transport_error data point with {want:?} in {points:?}"
    );
}

#[tokio::test]
async fn non_grpc_requests_are_not_recorded() {
    let telemetry = Telemetry::new();
    let service = telemetry
        .rpc_metrics_layer()
        .layer(tower::service_fn(|_| async {
            Ok::<_, Infallible>(response(None))
        }));
    call(service, "/test.v1.JsonService/Get", "application/json")
        .await
        .unwrap();

    let points = telemetry.data_points(metric::RpcServerCallDuration::NAME);
    assert!(
        points
            .iter()
            .all(|(attrs, _)| !attrs.values().any(|v| v.contains("JsonService"))),
        "a non-gRPC request was recorded: {points:?}"
    );
}

#[test]
fn cache_lookups_record_cache_name_and_result() {
    let telemetry = Telemetry::new();
    telemetry.metrics.record_snapshot_cache_hit();
    telemetry.metrics.record_snapshot_cache_hit();
    telemetry.metrics.record_branch_search_index_miss();

    let points = telemetry.data_points(metric::CacheLookups::NAME);
    let snapshot_hits = expected(&[
        (attribute::CacheName::KEY, "snapshot"),
        (attribute::CacheResult::KEY, "hit"),
    ]);
    let index_misses = expected(&[
        (attribute::CacheName::KEY, "branch_search_index"),
        (attribute::CacheResult::KEY, "miss"),
    ]);
    assert!(
        points
            .iter()
            .any(|(attrs, n)| *attrs == snapshot_hits && *n == 2),
        "no snapshot hit count of 2 in {points:?}"
    );
    assert!(
        points
            .iter()
            .any(|(attrs, n)| *attrs == index_misses && *n == 1),
        "no branch search index miss count of 1 in {points:?}"
    );
}
