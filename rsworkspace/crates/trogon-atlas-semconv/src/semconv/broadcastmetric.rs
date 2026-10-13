#![allow(deprecated)]
/// In-process change events a StreamChanges subscriber missed because it fell behind the broadcast channel.
#[derive(Clone)]
pub struct LagEventsCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl LagEventsCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter.inner().u64_counter("trogon_atlas.broadcast.lag_events")
            .with_description("In-process change events a StreamChanges subscriber missed because it fell behind the broadcast channel.")
            .with_unit("{event}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for LagEventsCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct LagEventsCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl LagEventsCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct LagEventsObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl LagEventsObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(LagEventsCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter.inner().u64_observable_counter("trogon_atlas.broadcast.lag_events")
            .with_description("In-process change events a StreamChanges subscriber missed because it fell behind the broadcast channel.")
            .with_unit("{event}")
            .with_callback(move |observer| callback(LagEventsCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for LagEventsObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
