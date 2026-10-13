#![allow(deprecated)]
/// Changesets that could not be appended after their entity writes committed.
#[derive(Clone)]
pub struct LostCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl LostCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.changesets.lost")
            .with_description(
                "Changesets that could not be appended after their entity writes committed.",
            )
            .with_unit("{changeset}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for LostCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct LostCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl LostCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct LostObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl LostObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(LostCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.changesets.lost")
            .with_description(
                "Changesets that could not be appended after their entity writes committed.",
            )
            .with_unit("{changeset}")
            .with_callback(move |observer| callback(LostCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for LostObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
