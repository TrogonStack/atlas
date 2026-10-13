//! Generated from [`otel/registry`](../../../otel/registry) by OpenTelemetry
//! Weaver. Never edit `src/semconv` by hand; run `mise run otel:generate`.
#[allow(
    clippy::all,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used
)]
#[path = "semconv/mod.rs"]
pub mod semconv;

/// Resolves meters through `opentelemetry::global`, whose `Arc<dyn
/// MeterProvider>` cannot satisfy [`semconv::meter::Meter::new`]'s
/// `&impl MeterProvider` bound directly. A meter built from it stays bound to
/// whichever provider was installed at that moment.
#[derive(Clone, Copy, Debug, Default)]
pub struct GlobalMeterProvider;

impl opentelemetry::metrics::MeterProvider for GlobalMeterProvider {
    fn meter_with_scope(
        &self,
        scope: opentelemetry::InstrumentationScope,
    ) -> opentelemetry::metrics::Meter {
        opentelemetry::global::meter_with_scope(scope)
    }
}
