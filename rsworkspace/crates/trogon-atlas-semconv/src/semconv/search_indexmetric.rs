#![allow(deprecated)]
/// Search index put/delete calls that failed; the index will rebuild.
#[derive(Clone)]
pub struct ErrorsCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl ErrorsCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.search_index.errors")
            .with_description("Search index put/delete calls that failed; the index will rebuild.")
            .with_unit("{error}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for ErrorsCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct ErrorsCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl ErrorsCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct ErrorsObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl ErrorsObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(ErrorsCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.search_index.errors")
            .with_description("Search index put/delete calls that failed; the index will rebuild.")
            .with_unit("{error}")
            .with_callback(move |observer| callback(ErrorsCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for ErrorsObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
