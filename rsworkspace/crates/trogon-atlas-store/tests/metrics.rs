//! A store records through the `StoreMetrics` it was configured with.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use opentelemetry_sdk::metrics::{
    data::{AggregatedMetrics, MetricData, ResourceMetrics, ScopeMetrics, SumDataPoint},
    InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
};
use trogon_atlas_semconv::semconv::{meter::Meter, metric, options::InstrumentationOptions};
use trogon_atlas_store::{NatsStore, NatsStoreConfig, Store, StoreMetrics};

fn counter_total(
    provider: &SdkMeterProvider,
    exporter: &InMemoryMetricExporter,
    name: &str,
) -> u64 {
    provider.force_flush().unwrap();
    exporter
        .get_finished_metrics()
        .unwrap()
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(ScopeMetrics::metrics)
        .filter(|m| m.name() == name)
        .map(|m| match m.data() {
            AggregatedMetrics::U64(MetricData::Sum(s)) => {
                s.data_points().map(SumDataPoint::value).sum::<u64>()
            }
            other => panic!("{name} exported unexpected data {other:?}"),
        })
        .max()
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn read_changes_records_each_consumer_through_the_injected_metrics() {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let meter = Meter::new(&provider, InstrumentationOptions::default());

    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = NatsStoreConfig::new(&nats.url, StoreMetrics::new(&meter));
    config.bucket = format!("metrics-{suffix}");
    config.branches_bucket = format!("metrics-branches-{suffix}");
    config.lease_bucket = format!("metrics-writer-lease-{suffix}");
    config.stream = format!("METRICS_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("metrics.{suffix}");
    config.changes_max_age = None;
    let store = NatsStore::connect_with(config).await.expect("connect");

    store.read_changes(0, 10).await.expect("read changes");
    store.read_changes(0, 10).await.expect("read changes");

    assert_eq!(
        counter_total(
            &provider,
            &exporter,
            metric::StoreReadChangesConsumers::NAME
        ),
        2
    );
}
