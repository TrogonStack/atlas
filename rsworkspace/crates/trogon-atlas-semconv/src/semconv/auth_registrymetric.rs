#![allow(deprecated)]
/// Failures publishing principal memberships after a token registry reload.
#[derive(Clone)]
pub struct MembershipPublishFailuresCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl MembershipPublishFailuresCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.auth_registry.membership_publish_failures")
            .with_description(
                "Failures publishing principal memberships after a token registry reload.",
            )
            .with_unit("{failure}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for MembershipPublishFailuresCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct MembershipPublishFailuresCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl MembershipPublishFailuresCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct MembershipPublishFailuresObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl MembershipPublishFailuresObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(MembershipPublishFailuresCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.auth_registry.membership_publish_failures")
            .with_description(
                "Failures publishing principal memberships after a token registry reload.",
            )
            .with_unit("{failure}")
            .with_callback(move |observer| {
                callback(MembershipPublishFailuresCounterObserver { observer })
            })
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for MembershipPublishFailuresObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Token registry reloads rejected by validation; the previous registry stays in force.
#[derive(Clone)]
pub struct ReloadFailuresCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl ReloadFailuresCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter.inner().u64_counter("trogon_atlas.auth_registry.reload_failures")
            .with_description("Token registry reloads rejected by validation; the previous registry stays in force.")
            .with_unit("{failure}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for ReloadFailuresCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct ReloadFailuresCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl ReloadFailuresCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct ReloadFailuresObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl ReloadFailuresObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(ReloadFailuresCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter.inner().u64_observable_counter("trogon_atlas.auth_registry.reload_failures")
            .with_description("Token registry reloads rejected by validation; the previous registry stays in force.")
            .with_unit("{failure}")
            .with_callback(move |observer| callback(ReloadFailuresCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for ReloadFailuresObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Token registry files successfully reloaded from disk.
#[derive(Clone)]
pub struct ReloadsCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl ReloadsCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.auth_registry.reloads")
            .with_description("Token registry files successfully reloaded from disk.")
            .with_unit("{reload}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for ReloadsCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct ReloadsCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl ReloadsCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct ReloadsObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl ReloadsObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(ReloadsCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.auth_registry.reloads")
            .with_description("Token registry files successfully reloaded from disk.")
            .with_unit("{reload}")
            .with_callback(move |observer| callback(ReloadsCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for ReloadsObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
