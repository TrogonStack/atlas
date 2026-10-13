//! Where the `metrics` facade's measurements go: pushed over OTLP when
//! `OTEL_EXPORTER_OTLP_ENDPOINT` is set, and rendered for a Prometheus scrape
//! at `/-/metrics` on the serving port when that is opted into. Either, both,
//! or neither may be on.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{
    http::{header, StatusCode},
    routing::get,
    Router,
};
use metrics::{CounterFn, GaugeFn, HistogramFn, Key, KeyName, Metadata, SharedString, Unit};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle, PrometheusRecorder};
use metrics_util::layers::{Fanout, FanoutBuilder};
use opentelemetry::{
    metrics::{Meter, MeterProvider as _},
    KeyValue,
};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use parking_lot::RwLock;

pub const METRICS_PATH: &str = "/-/metrics";

const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";
const PROMETHEUS_UPKEEP_INTERVAL: Duration = Duration::from_secs(5);
const DURATION_SECONDS_BOUNDARIES: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
];

/// An OTLP meter provider exporting over gRPC to `endpoint`, the way
/// `init_tracing` exports spans.
pub fn otlp_meter_provider(endpoint: &str) -> anyhow::Result<SdkMeterProvider> {
    use opentelemetry_otlp::{WithExportConfig as _, WithTonicConfig as _};
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint.to_owned())
        .with_tls_config(crate::telemetry::otlp_tls_config())
        .build()
        .map_err(|e| anyhow::anyhow!("building OTLP metric exporter for {endpoint}: {e}"))?;
    Ok(SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .with_resource(crate::telemetry::resource())
        .build())
}

/// The metrics backends to feed, before any of them is wired up.
pub struct MetricsExporters {
    otlp: Option<SdkMeterProvider>,
    prometheus: Option<PrometheusRecorder>,
}

impl MetricsExporters {
    pub fn new(otlp: Option<SdkMeterProvider>, prometheus: bool) -> Self {
        Self {
            otlp,
            prometheus: prometheus.then(|| PrometheusBuilder::new().build_recorder()),
        }
    }

    /// OTLP export follows the trace exporter: on exactly when
    /// `OTEL_EXPORTER_OTLP_ENDPOINT` is set and non-empty.
    pub fn from_env(prometheus: bool) -> anyhow::Result<Self> {
        let otlp = match std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
            Ok(endpoint) if !endpoint.is_empty() => {
                let provider = otlp_meter_provider(&endpoint)?;
                tracing::info!(endpoint = %endpoint, "otlp metric exporter enabled");
                Some(provider)
            }
            _ => None,
        };
        Ok(Self::new(otlp, prometheus))
    }

    /// One recorder that feeds every enabled backend, or `None` when none is,
    /// plus the guard that serves and shuts them down.
    pub fn build(self) -> (Option<Fanout>, MetricsGuard) {
        let prometheus = self.prometheus.as_ref().map(PrometheusRecorder::handle);
        let mut fanout = FanoutBuilder::default();
        let mut enabled = false;
        if let Some(provider) = &self.otlp {
            fanout = fanout.add_recorder(OtlpRecorder::new(
                provider.meter(crate::telemetry::SERVICE_NAME),
            ));
            enabled = true;
        }
        if let Some(recorder) = self.prometheus {
            fanout = fanout.add_recorder(recorder);
            enabled = true;
        }
        (
            enabled.then(|| fanout.build()),
            MetricsGuard {
                provider: self.otlp,
                prometheus,
            },
        )
    }

    /// Installs the global recorder. Must run inside a Tokio runtime when
    /// Prometheus is enabled, which needs periodic upkeep.
    pub fn install(self) -> anyhow::Result<MetricsGuard> {
        let (recorder, guard) = self.build();
        if let Some(recorder) = recorder {
            metrics::set_global_recorder(recorder)
                .map_err(|e| anyhow::anyhow!("installing metrics recorder: {e}"))?;
        }
        if let Some(handle) = guard.prometheus.clone() {
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(PROMETHEUS_UPKEEP_INTERVAL);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    ticker.tick().await;
                    handle.run_upkeep();
                }
            });
        }
        Ok(guard)
    }
}

/// Holds the enabled metrics backends. `shutdown` MUST be called on graceful
/// shutdown so the OTLP exporter pushes what it has not sent yet.
pub struct MetricsGuard {
    provider: Option<SdkMeterProvider>,
    prometheus: Option<PrometheusHandle>,
}

impl MetricsGuard {
    pub fn is_recording(&self) -> bool {
        self.provider.is_some() || self.prometheus.is_some()
    }

    /// `/-/metrics`: the Prometheus scrape when enabled, otherwise a 404, so
    /// a scraper pointed at a server that has it off fails loudly instead of
    /// reading the listener's gRPC fallback as an empty, healthy target.
    pub fn router(&self) -> Router {
        let handle = self.prometheus.clone();
        Router::new().route(
            METRICS_PATH,
            get(move || {
                let handle = handle.clone();
                async move {
                    match handle {
                        Some(handle) => (
                            StatusCode::OK,
                            [(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
                            handle.render(),
                        ),
                        None => (
                            StatusCode::NOT_FOUND,
                            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                            "prometheus metrics are disabled\n".to_owned(),
                        ),
                    }
                }
            }),
        )
    }

    pub fn serves_prometheus(&self) -> bool {
        self.prometheus.is_some()
    }

    pub fn shutdown(mut self) {
        if let Some(provider) = self.provider.take() {
            if let Err(err) = provider.force_flush() {
                tracing::warn!(error = ?err, "otlp metrics force_flush failed");
            }
            if let Err(err) = provider.shutdown() {
                tracing::warn!(error = ?err, "otlp meter provider shutdown failed");
            }
        }
    }
}

/// Forwards the `metrics` facade to an OpenTelemetry `Meter`, keeping every
/// metric's name and labels. Counters stay monotonic sums, gauges stay
/// last-value gauges, and histograms become explicit-bucket histograms with
/// boundaries sized for durations in seconds.
pub struct OtlpRecorder {
    meter: Meter,
    counters: RwLock<HashMap<Key, metrics::Counter>>,
    gauges: RwLock<HashMap<Key, metrics::Gauge>>,
    histograms: RwLock<HashMap<Key, metrics::Histogram>>,
}

impl OtlpRecorder {
    pub fn new(meter: Meter) -> Self {
        Self {
            meter,
            counters: RwLock::default(),
            gauges: RwLock::default(),
            histograms: RwLock::default(),
        }
    }
}

fn attributes(key: &Key) -> Vec<KeyValue> {
    key.labels()
        .map(|label| KeyValue::new(label.key().to_owned(), label.value().to_owned()))
        .collect()
}

fn get_or_register<T: Clone>(
    cache: &RwLock<HashMap<Key, T>>,
    key: &Key,
    register: impl FnOnce() -> T,
) -> T {
    if let Some(existing) = cache.read().get(key) {
        return existing.clone();
    }
    cache
        .write()
        .entry(key.clone())
        .or_insert_with(register)
        .clone()
}

impl metrics::Recorder for OtlpRecorder {
    fn describe_counter(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn describe_gauge(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn describe_histogram(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn register_counter(&self, key: &Key, _metadata: &Metadata<'_>) -> metrics::Counter {
        get_or_register(&self.counters, key, || {
            metrics::Counter::from_arc(Arc::new(OtlpCounter {
                counter: self.meter.u64_counter(key.name().to_owned()).build(),
                attributes: attributes(key),
                total: AtomicU64::new(0),
            }))
        })
    }

    fn register_gauge(&self, key: &Key, _metadata: &Metadata<'_>) -> metrics::Gauge {
        get_or_register(&self.gauges, key, || {
            metrics::Gauge::from_arc(Arc::new(OtlpGauge {
                gauge: self.meter.f64_gauge(key.name().to_owned()).build(),
                attributes: attributes(key),
                value: AtomicU64::new(0f64.to_bits()),
            }))
        })
    }

    fn register_histogram(&self, key: &Key, _metadata: &Metadata<'_>) -> metrics::Histogram {
        get_or_register(&self.histograms, key, || {
            metrics::Histogram::from_arc(Arc::new(OtlpHistogram {
                histogram: self
                    .meter
                    .f64_histogram(key.name().to_owned())
                    .with_boundaries(DURATION_SECONDS_BOUNDARIES.to_vec())
                    .build(),
                attributes: attributes(key),
            }))
        })
    }
}

struct OtlpCounter {
    counter: opentelemetry::metrics::Counter<u64>,
    attributes: Vec<KeyValue>,
    total: AtomicU64,
}

impl CounterFn for OtlpCounter {
    fn increment(&self, value: u64) {
        self.total.fetch_add(value, Ordering::Relaxed);
        self.counter.add(value, &self.attributes);
    }

    fn absolute(&self, value: u64) {
        let previous = self.total.fetch_max(value, Ordering::Relaxed);
        self.counter
            .add(value.saturating_sub(previous), &self.attributes);
    }
}

struct OtlpGauge {
    gauge: opentelemetry::metrics::Gauge<f64>,
    attributes: Vec<KeyValue>,
    value: AtomicU64,
}

impl OtlpGauge {
    fn update(&self, change: impl Fn(f64) -> f64) {
        let mut current = self.value.load(Ordering::Acquire);
        loop {
            let next = change(f64::from_bits(current));
            match self.value.compare_exchange_weak(
                current,
                next.to_bits(),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.gauge.record(next, &self.attributes);
                    return;
                }
                Err(actual) => current = actual,
            }
        }
    }
}

impl GaugeFn for OtlpGauge {
    fn increment(&self, value: f64) {
        self.update(|current| current + value);
    }

    fn decrement(&self, value: f64) {
        self.update(|current| current - value);
    }

    fn set(&self, value: f64) {
        self.update(|_| value);
    }
}

struct OtlpHistogram {
    histogram: opentelemetry::metrics::Histogram<f64>,
    attributes: Vec<KeyValue>,
}

impl HistogramFn for OtlpHistogram {
    fn record(&self, value: f64) {
        self.histogram.record(value, &self.attributes);
    }
}
