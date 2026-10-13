#![allow(deprecated)]
/// Batch recovery passes that could not list the journal.
#[derive(Clone)]
pub struct FailuresCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl FailuresCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.batch_recovery.failures")
            .with_description("Batch recovery passes that could not list the journal.")
            .with_unit("{failure}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for FailuresCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct FailuresCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl FailuresCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct FailuresObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl FailuresObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(FailuresCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.batch_recovery.failures")
            .with_description("Batch recovery passes that could not list the journal.")
            .with_unit("{failure}")
            .with_callback(move |observer| callback(FailuresCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for FailuresObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Interrupted batches a recovery pass found still unresolved.
#[derive(Clone)]
pub struct UnresolvedCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl UnresolvedCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.batch_recovery.unresolved")
            .with_description("Interrupted batches a recovery pass found still unresolved.")
            .with_unit("{batch}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for UnresolvedCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct UnresolvedCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl UnresolvedCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct UnresolvedObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl UnresolvedObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(UnresolvedCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.batch_recovery.unresolved")
            .with_description("Interrupted batches a recovery pass found still unresolved.")
            .with_unit("{batch}")
            .with_callback(move |observer| callback(UnresolvedCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for UnresolvedObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
