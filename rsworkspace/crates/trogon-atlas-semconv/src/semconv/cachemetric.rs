#![allow(deprecated)]
/// In-process cache lookups, by cache and result.
#[derive(Clone)]
pub struct LookupsCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl LookupsCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.cache.lookups")
            .with_description("In-process cache lookups, by cache and result.")
            .with_unit("{lookup}")
            .build();
        Self { instrument }
    }
    pub fn add(
        &self,
        value: u64,
        r#cache_name: super::cacheattr::NameAttr,
        r#cache_result: super::cacheattr::ResultAttr,
    ) {
        let attributes = [
            ::opentelemetry::KeyValue::from(r#cache_name),
            ::opentelemetry::KeyValue::from(r#cache_result),
        ];
        self.instrument.add(value, &attributes);
    }
}

impl Default for LookupsCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct LookupsCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl LookupsCounterObserver<'_> {
    pub fn observe(
        &self,
        value: u64,
        r#cache_name: super::cacheattr::NameAttr,
        r#cache_result: super::cacheattr::ResultAttr,
    ) {
        let attributes = [
            ::opentelemetry::KeyValue::from(r#cache_name),
            ::opentelemetry::KeyValue::from(r#cache_result),
        ];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct LookupsObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl LookupsObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(LookupsCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.cache.lookups")
            .with_description("In-process cache lookups, by cache and result.")
            .with_unit("{lookup}")
            .with_callback(move |observer| callback(LookupsCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for LookupsObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
