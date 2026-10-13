use super::options::InstrumentationOptions;

#[derive(Clone, Debug)]
pub struct Meter(opentelemetry::metrics::Meter);

impl Meter {
    pub fn new<P: opentelemetry::metrics::MeterProvider + ?Sized>(
        provider: &P,
        options: InstrumentationOptions,
    ) -> Self {
        let meter = provider.meter_with_scope(options.scope.into());
        Self(meter)
    }
    pub fn inner(&self) -> &opentelemetry::metrics::Meter {
        &self.0
    }
}

impl Default for Meter {
    fn default() -> Self {
        Self::new(
            &opentelemetry::metrics::noop::NoopMeterProvider::new(),
            InstrumentationOptions::default(),
        )
    }
}
