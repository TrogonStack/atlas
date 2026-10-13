#![allow(deprecated)]
/// Graph closures that hit their entity cap and were truncated.
#[derive(Clone)]
pub struct ClosureTruncationsCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl ClosureTruncationsCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.graph.closure_truncations")
            .with_description("Graph closures that hit their entity cap and were truncated.")
            .with_unit("{truncation}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for ClosureTruncationsCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct ClosureTruncationsCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl ClosureTruncationsCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct ClosureTruncationsObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl ClosureTruncationsObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(ClosureTruncationsCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.graph.closure_truncations")
            .with_description("Graph closures that hit their entity cap and were truncated.")
            .with_unit("{truncation}")
            .with_callback(move |observer| callback(ClosureTruncationsCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for ClosureTruncationsObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
