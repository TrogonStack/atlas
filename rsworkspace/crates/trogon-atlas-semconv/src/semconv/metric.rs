#![allow(deprecated)]
#[derive(Clone, Debug)]
pub enum RpcServerCallDurationAttr {
    /// Conditionally required: If and only if the operation failed.
    ErrorType(::opentelemetry::StringValue),
    /// Conditionally required: if available.
    RpcMethod(::opentelemetry::StringValue),
    /// Conditionally required: if available.
    RpcResponseStatusCode(::opentelemetry::StringValue),
    /// Opt-in: the convention records it only when a user asks for it.
    ServerAddress(::opentelemetry::StringValue),
    /// Opt-in: the convention records it only when a user asks for it.
    ServerPort(i64),
}

impl From<RpcServerCallDurationAttr> for ::opentelemetry::KeyValue {
    fn from(value: RpcServerCallDurationAttr) -> Self {
        match value {
            RpcServerCallDurationAttr::ErrorType(value) => {
                ::opentelemetry::KeyValue::new("error.type", value)
            }
            RpcServerCallDurationAttr::RpcMethod(value) => {
                ::opentelemetry::KeyValue::new("rpc.method", value)
            }
            RpcServerCallDurationAttr::RpcResponseStatusCode(value) => {
                ::opentelemetry::KeyValue::new("rpc.response.status_code", value)
            }
            RpcServerCallDurationAttr::ServerAddress(value) => {
                ::opentelemetry::KeyValue::new("server.address", value)
            }
            RpcServerCallDurationAttr::ServerPort(value) => {
                ::opentelemetry::KeyValue::new("server.port", value)
            }
        }
    }
}
/// Measures the duration of an incoming Remote Procedure Call (RPC).
///
/// When this metric is reported alongside an RPC server span, the metric value
/// SHOULD be the same as the RPC server span duration.
#[derive(Clone)]
pub struct RpcServerCallDuration {
    instrument: ::opentelemetry::metrics::Histogram<f64>,
}

impl RpcServerCallDuration {
    pub const NAME: &'static str = "rpc.server.call.duration";
    pub const UNIT: &'static str = "s";
    pub const DESCRIPTION: &'static str =
        "Measures the duration of an incoming Remote Procedure Call (RPC).";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .f64_histogram(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_boundaries(vec![
                0.005_f64, 0.01_f64, 0.025_f64, 0.05_f64, 0.075_f64, 0.1_f64, 0.25_f64, 0.5_f64,
                0.75_f64, 1_f64, 2.5_f64, 5_f64, 7.5_f64, 10_f64,
            ])
            .build();
        Self { instrument }
    }
    pub fn record(
        &self,
        value: f64,
        r#rpc_system_name: ::opentelemetry::StringValue,
        options: impl IntoIterator<Item = RpcServerCallDurationAttr>,
    ) {
        let mut attributes = ::smallvec::SmallVec::<[::opentelemetry::KeyValue; 6]>::new();
        attributes.extend([::opentelemetry::KeyValue::new(
            "rpc.system.name",
            r#rpc_system_name,
        )]);
        attributes.extend(options.into_iter().map(::opentelemetry::KeyValue::from));
        self.instrument.record(value, &attributes);
    }
}

impl Default for RpcServerCallDuration {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
/// Failures publishing principal memberships after a token registry reload.
#[derive(Clone)]
pub struct AuthRegistryMembershipPublishFailures {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl AuthRegistryMembershipPublishFailures {
    pub const NAME: &'static str = "trogon_atlas.auth_registry.membership_publish_failures";
    pub const UNIT: &'static str = "{failure}";
    pub const DESCRIPTION: &'static str =
        "Failures publishing principal memberships after a token registry reload.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for AuthRegistryMembershipPublishFailures {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct AuthRegistryMembershipPublishFailuresObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl AuthRegistryMembershipPublishFailuresObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct AuthRegistryMembershipPublishFailuresObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl AuthRegistryMembershipPublishFailuresObservable {
    pub const NAME: &'static str = AuthRegistryMembershipPublishFailures::NAME;
    pub const UNIT: &'static str = AuthRegistryMembershipPublishFailures::UNIT;
    pub const DESCRIPTION: &'static str = AuthRegistryMembershipPublishFailures::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(AuthRegistryMembershipPublishFailuresObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| {
                callback(AuthRegistryMembershipPublishFailuresObserver { observer })
            })
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for AuthRegistryMembershipPublishFailuresObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Token registry reloads rejected by validation; the previous registry stays in force.
#[derive(Clone)]
pub struct AuthRegistryReloadFailures {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl AuthRegistryReloadFailures {
    pub const NAME: &'static str = "trogon_atlas.auth_registry.reload_failures";
    pub const UNIT: &'static str = "{failure}";
    pub const DESCRIPTION: &'static str =
        "Token registry reloads rejected by validation; the previous registry stays in force.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for AuthRegistryReloadFailures {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct AuthRegistryReloadFailuresObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl AuthRegistryReloadFailuresObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct AuthRegistryReloadFailuresObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl AuthRegistryReloadFailuresObservable {
    pub const NAME: &'static str = AuthRegistryReloadFailures::NAME;
    pub const UNIT: &'static str = AuthRegistryReloadFailures::UNIT;
    pub const DESCRIPTION: &'static str = AuthRegistryReloadFailures::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(AuthRegistryReloadFailuresObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| {
                callback(AuthRegistryReloadFailuresObserver { observer })
            })
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for AuthRegistryReloadFailuresObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Token registry files successfully reloaded from disk.
#[derive(Clone)]
pub struct AuthRegistryReloads {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl AuthRegistryReloads {
    pub const NAME: &'static str = "trogon_atlas.auth_registry.reloads";
    pub const UNIT: &'static str = "{reload}";
    pub const DESCRIPTION: &'static str = "Token registry files successfully reloaded from disk.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for AuthRegistryReloads {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct AuthRegistryReloadsObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl AuthRegistryReloadsObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct AuthRegistryReloadsObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl AuthRegistryReloadsObservable {
    pub const NAME: &'static str = AuthRegistryReloads::NAME;
    pub const UNIT: &'static str = AuthRegistryReloads::UNIT;
    pub const DESCRIPTION: &'static str = AuthRegistryReloads::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(AuthRegistryReloadsObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(AuthRegistryReloadsObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for AuthRegistryReloadsObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Batch recovery passes that could not list the journal.
#[derive(Clone)]
pub struct BatchRecoveryFailures {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl BatchRecoveryFailures {
    pub const NAME: &'static str = "trogon_atlas.batch_recovery.failures";
    pub const UNIT: &'static str = "{failure}";
    pub const DESCRIPTION: &'static str = "Batch recovery passes that could not list the journal.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for BatchRecoveryFailures {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct BatchRecoveryFailuresObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl BatchRecoveryFailuresObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct BatchRecoveryFailuresObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl BatchRecoveryFailuresObservable {
    pub const NAME: &'static str = BatchRecoveryFailures::NAME;
    pub const UNIT: &'static str = BatchRecoveryFailures::UNIT;
    pub const DESCRIPTION: &'static str = BatchRecoveryFailures::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(BatchRecoveryFailuresObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(BatchRecoveryFailuresObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for BatchRecoveryFailuresObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Interrupted batches a recovery pass found still unresolved.
#[derive(Clone)]
pub struct BatchRecoveryUnresolved {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl BatchRecoveryUnresolved {
    pub const NAME: &'static str = "trogon_atlas.batch_recovery.unresolved";
    pub const UNIT: &'static str = "{batch}";
    pub const DESCRIPTION: &'static str =
        "Interrupted batches a recovery pass found still unresolved.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for BatchRecoveryUnresolved {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct BatchRecoveryUnresolvedObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl BatchRecoveryUnresolvedObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct BatchRecoveryUnresolvedObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl BatchRecoveryUnresolvedObservable {
    pub const NAME: &'static str = BatchRecoveryUnresolved::NAME;
    pub const UNIT: &'static str = BatchRecoveryUnresolved::UNIT;
    pub const DESCRIPTION: &'static str = BatchRecoveryUnresolved::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(BatchRecoveryUnresolvedObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(BatchRecoveryUnresolvedObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for BatchRecoveryUnresolvedObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// In-process change events a StreamChanges subscriber missed because it fell behind the broadcast channel.
#[derive(Clone)]
pub struct BroadcastLagEvents {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl BroadcastLagEvents {
    pub const NAME: &'static str = "trogon_atlas.broadcast.lag_events";
    pub const UNIT: &'static str = "{event}";
    pub const DESCRIPTION: &'static str = "In-process change events a StreamChanges subscriber missed because it fell behind the broadcast channel.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for BroadcastLagEvents {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct BroadcastLagEventsObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl BroadcastLagEventsObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct BroadcastLagEventsObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl BroadcastLagEventsObservable {
    pub const NAME: &'static str = BroadcastLagEvents::NAME;
    pub const UNIT: &'static str = BroadcastLagEvents::UNIT;
    pub const DESCRIPTION: &'static str = BroadcastLagEvents::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(BroadcastLagEventsObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(BroadcastLagEventsObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for BroadcastLagEventsObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// In-process cache lookups, by cache and result.
#[derive(Clone)]
pub struct CacheLookups {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl CacheLookups {
    pub const NAME: &'static str = "trogon_atlas.cache.lookups";
    pub const UNIT: &'static str = "{lookup}";
    pub const DESCRIPTION: &'static str = "In-process cache lookups, by cache and result.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(
        &self,
        value: u64,
        r#cache_name: super::attribute::CacheName,
        r#cache_result: super::attribute::CacheResult,
    ) {
        let attributes = [
            ::opentelemetry::KeyValue::from(r#cache_name),
            ::opentelemetry::KeyValue::from(r#cache_result),
        ];
        self.instrument.add(value, &attributes);
    }
}

impl Default for CacheLookups {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct CacheLookupsObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl CacheLookupsObserver<'_> {
    pub fn observe(
        &self,
        value: u64,
        r#cache_name: super::attribute::CacheName,
        r#cache_result: super::attribute::CacheResult,
    ) {
        let attributes = [
            ::opentelemetry::KeyValue::from(r#cache_name),
            ::opentelemetry::KeyValue::from(r#cache_result),
        ];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct CacheLookupsObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl CacheLookupsObservable {
    pub const NAME: &'static str = CacheLookups::NAME;
    pub const UNIT: &'static str = CacheLookups::UNIT;
    pub const DESCRIPTION: &'static str = CacheLookups::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(CacheLookupsObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(CacheLookupsObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for CacheLookupsObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Current message count of the durable change stream.
#[derive(Clone)]
pub struct ChangeStreamMessageCount {
    instrument: ::opentelemetry::metrics::Gauge<i64>,
}

impl ChangeStreamMessageCount {
    pub const NAME: &'static str = "trogon_atlas.change_stream.message.count";
    pub const UNIT: &'static str = "{message}";
    pub const DESCRIPTION: &'static str = "Current message count of the durable change stream.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .i64_gauge(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn record(&self, value: i64) {
        let attributes = [];
        self.instrument.record(value, &attributes);
    }
}

impl Default for ChangeStreamMessageCount {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct ChangeStreamMessageCountObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<i64>,
}

impl ChangeStreamMessageCountObserver<'_> {
    pub fn observe(&self, value: i64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct ChangeStreamMessageCountObservable {
    _instrument: ::opentelemetry::metrics::ObservableGauge<i64>,
}

impl ChangeStreamMessageCountObservable {
    pub const NAME: &'static str = ChangeStreamMessageCount::NAME;
    pub const UNIT: &'static str = ChangeStreamMessageCount::UNIT;
    pub const DESCRIPTION: &'static str = ChangeStreamMessageCount::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(ChangeStreamMessageCountObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .i64_observable_gauge(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(ChangeStreamMessageCountObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for ChangeStreamMessageCountObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Age of the oldest retained message in the durable change stream; zero when the stream is empty.
#[derive(Clone)]
pub struct ChangeStreamOldestMessageAge {
    instrument: ::opentelemetry::metrics::Gauge<i64>,
}

impl ChangeStreamOldestMessageAge {
    pub const NAME: &'static str = "trogon_atlas.change_stream.oldest_message.age";
    pub const UNIT: &'static str = "s";
    pub const DESCRIPTION: &'static str = "Age of the oldest retained message in the durable change stream; zero when the stream is empty.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .i64_gauge(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn record(&self, value: i64) {
        let attributes = [];
        self.instrument.record(value, &attributes);
    }
}

impl Default for ChangeStreamOldestMessageAge {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct ChangeStreamOldestMessageAgeObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<i64>,
}

impl ChangeStreamOldestMessageAgeObserver<'_> {
    pub fn observe(&self, value: i64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct ChangeStreamOldestMessageAgeObservable {
    _instrument: ::opentelemetry::metrics::ObservableGauge<i64>,
}

impl ChangeStreamOldestMessageAgeObservable {
    pub const NAME: &'static str = ChangeStreamOldestMessageAge::NAME;
    pub const UNIT: &'static str = ChangeStreamOldestMessageAge::UNIT;
    pub const DESCRIPTION: &'static str = ChangeStreamOldestMessageAge::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(ChangeStreamOldestMessageAgeObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .i64_observable_gauge(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| {
                callback(ChangeStreamOldestMessageAgeObserver { observer })
            })
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for ChangeStreamOldestMessageAgeObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Current byte size of the durable change stream.
#[derive(Clone)]
pub struct ChangeStreamSize {
    instrument: ::opentelemetry::metrics::Gauge<i64>,
}

impl ChangeStreamSize {
    pub const NAME: &'static str = "trogon_atlas.change_stream.size";
    pub const UNIT: &'static str = "By";
    pub const DESCRIPTION: &'static str = "Current byte size of the durable change stream.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .i64_gauge(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn record(&self, value: i64) {
        let attributes = [];
        self.instrument.record(value, &attributes);
    }
}

impl Default for ChangeStreamSize {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct ChangeStreamSizeObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<i64>,
}

impl ChangeStreamSizeObserver<'_> {
    pub fn observe(&self, value: i64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct ChangeStreamSizeObservable {
    _instrument: ::opentelemetry::metrics::ObservableGauge<i64>,
}

impl ChangeStreamSizeObservable {
    pub const NAME: &'static str = ChangeStreamSize::NAME;
    pub const UNIT: &'static str = ChangeStreamSize::UNIT;
    pub const DESCRIPTION: &'static str = ChangeStreamSize::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(ChangeStreamSizeObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .i64_observable_gauge(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(ChangeStreamSizeObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for ChangeStreamSizeObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Changesets that could not be appended after their entity writes committed.
#[derive(Clone)]
pub struct ChangesetsLost {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl ChangesetsLost {
    pub const NAME: &'static str = "trogon_atlas.changesets.lost";
    pub const UNIT: &'static str = "{changeset}";
    pub const DESCRIPTION: &'static str =
        "Changesets that could not be appended after their entity writes committed.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for ChangesetsLost {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct ChangesetsLostObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl ChangesetsLostObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct ChangesetsLostObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl ChangesetsLostObservable {
    pub const NAME: &'static str = ChangesetsLost::NAME;
    pub const UNIT: &'static str = ChangesetsLost::UNIT;
    pub const DESCRIPTION: &'static str = ChangesetsLost::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(ChangesetsLostObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(ChangesetsLostObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for ChangesetsLostObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Git mirror apply calls that failed.
#[derive(Clone)]
pub struct GitMirrorFailures {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl GitMirrorFailures {
    pub const NAME: &'static str = "trogon_atlas.git_mirror.failures";
    pub const UNIT: &'static str = "{failure}";
    pub const DESCRIPTION: &'static str = "Git mirror apply calls that failed.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for GitMirrorFailures {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct GitMirrorFailuresObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl GitMirrorFailuresObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct GitMirrorFailuresObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl GitMirrorFailuresObservable {
    pub const NAME: &'static str = GitMirrorFailures::NAME;
    pub const UNIT: &'static str = GitMirrorFailures::UNIT;
    pub const DESCRIPTION: &'static str = GitMirrorFailures::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(GitMirrorFailuresObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(GitMirrorFailuresObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for GitMirrorFailuresObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Graph closures that hit their entity cap and were truncated.
#[derive(Clone)]
pub struct GraphClosureTruncations {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl GraphClosureTruncations {
    pub const NAME: &'static str = "trogon_atlas.graph.closure_truncations";
    pub const UNIT: &'static str = "{truncation}";
    pub const DESCRIPTION: &'static str =
        "Graph closures that hit their entity cap and were truncated.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for GraphClosureTruncations {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct GraphClosureTruncationsObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl GraphClosureTruncationsObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct GraphClosureTruncationsObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl GraphClosureTruncationsObservable {
    pub const NAME: &'static str = GraphClosureTruncations::NAME;
    pub const UNIT: &'static str = GraphClosureTruncations::UNIT;
    pub const DESCRIPTION: &'static str = GraphClosureTruncations::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(GraphClosureTruncationsObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(GraphClosureTruncationsObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for GraphClosureTruncationsObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Analysis prompts whose link list was truncated to fit the LLM's context budget.
#[derive(Clone)]
pub struct LlmPromptTruncations {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl LlmPromptTruncations {
    pub const NAME: &'static str = "trogon_atlas.llm.prompt_truncations";
    pub const UNIT: &'static str = "{truncation}";
    pub const DESCRIPTION: &'static str =
        "Analysis prompts whose link list was truncated to fit the LLM\u{0027}s context budget.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for LlmPromptTruncations {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct LlmPromptTruncationsObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl LlmPromptTruncationsObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct LlmPromptTruncationsObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl LlmPromptTruncationsObservable {
    pub const NAME: &'static str = LlmPromptTruncations::NAME;
    pub const UNIT: &'static str = LlmPromptTruncations::UNIT;
    pub const DESCRIPTION: &'static str = LlmPromptTruncations::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(LlmPromptTruncationsObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(LlmPromptTruncationsObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for LlmPromptTruncationsObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
#[derive(Clone, Debug)]
pub enum LlmRequestDurationAttr {
    /// Conditionally required: The request did not complete successfully.
    ErrorType(::opentelemetry::StringValue),
}

impl From<LlmRequestDurationAttr> for ::opentelemetry::KeyValue {
    fn from(value: LlmRequestDurationAttr) -> Self {
        match value {
            LlmRequestDurationAttr::ErrorType(value) => {
                ::opentelemetry::KeyValue::new("error.type", value)
            }
        }
    }
}
/// Wall-clock duration of a complete LLM request, including retries.
#[derive(Clone)]
pub struct LlmRequestDuration {
    instrument: ::opentelemetry::metrics::Histogram<f64>,
}

impl LlmRequestDuration {
    pub const NAME: &'static str = "trogon_atlas.llm.request.duration";
    pub const UNIT: &'static str = "s";
    pub const DESCRIPTION: &'static str =
        "Wall-clock duration of a complete LLM request, including retries.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .f64_histogram(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_boundaries(vec![
                0.005_f64, 0.01_f64, 0.025_f64, 0.05_f64, 0.1_f64, 0.25_f64, 0.5_f64, 1_f64,
                2.5_f64, 5_f64, 10_f64, 30_f64, 60_f64, 120_f64,
            ])
            .build();
        Self { instrument }
    }
    pub fn record(
        &self,
        value: f64,
        r#llm_model: super::attribute::LlmModel,
        r#llm_provider: super::attribute::LlmProvider,
        options: impl IntoIterator<Item = LlmRequestDurationAttr>,
    ) {
        let mut attributes = ::smallvec::SmallVec::<[::opentelemetry::KeyValue; 3]>::new();
        attributes.extend([
            ::opentelemetry::KeyValue::from(r#llm_model),
            ::opentelemetry::KeyValue::from(r#llm_provider),
        ]);
        attributes.extend(options.into_iter().map(::opentelemetry::KeyValue::from));
        self.instrument.record(value, &attributes);
    }
}

impl Default for LlmRequestDuration {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
/// Tokens exchanged with the LLM provider.
#[derive(Clone)]
pub struct LlmTokens {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl LlmTokens {
    pub const NAME: &'static str = "trogon_atlas.llm.tokens";
    pub const UNIT: &'static str = "{token}";
    pub const DESCRIPTION: &'static str = "Tokens exchanged with the LLM provider.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(
        &self,
        value: u64,
        r#llm_model: super::attribute::LlmModel,
        r#llm_token_type: super::attribute::LlmTokenType,
    ) {
        let attributes = [
            ::opentelemetry::KeyValue::from(r#llm_model),
            ::opentelemetry::KeyValue::from(r#llm_token_type),
        ];
        self.instrument.add(value, &attributes);
    }
}

impl Default for LlmTokens {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct LlmTokensObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl LlmTokensObserver<'_> {
    pub fn observe(
        &self,
        value: u64,
        r#llm_model: super::attribute::LlmModel,
        r#llm_token_type: super::attribute::LlmTokenType,
    ) {
        let attributes = [
            ::opentelemetry::KeyValue::from(r#llm_model),
            ::opentelemetry::KeyValue::from(r#llm_token_type),
        ];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct LlmTokensObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl LlmTokensObservable {
    pub const NAME: &'static str = LlmTokens::NAME;
    pub const UNIT: &'static str = LlmTokens::UNIT;
    pub const DESCRIPTION: &'static str = LlmTokens::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(LlmTokensObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(LlmTokensObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for LlmTokensObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Time a caller spent blocked acquiring the global mutation lock.
#[derive(Clone)]
pub struct MutationLockWaitDuration {
    instrument: ::opentelemetry::metrics::Histogram<f64>,
}

impl MutationLockWaitDuration {
    pub const NAME: &'static str = "trogon_atlas.mutation_lock.wait.duration";
    pub const UNIT: &'static str = "s";
    pub const DESCRIPTION: &'static str =
        "Time a caller spent blocked acquiring the global mutation lock.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .f64_histogram(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_boundaries(vec![
                0.0001_f64, 0.001_f64, 0.005_f64, 0.01_f64, 0.025_f64, 0.05_f64, 0.1_f64, 0.25_f64,
                0.5_f64, 1_f64, 2.5_f64,
            ])
            .build();
        Self { instrument }
    }
    pub fn record(&self, value: f64) {
        let attributes = [];
        self.instrument.record(value, &attributes);
    }
}

impl Default for MutationLockWaitDuration {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
/// Terminal outcomes of server-streaming RPCs, which end past the point a Tower layer can observe the response head.
#[derive(Clone)]
pub struct RpcServerStreamTerminations {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl RpcServerStreamTerminations {
    pub const NAME: &'static str = "trogon_atlas.rpc.server.stream.terminations";
    pub const UNIT: &'static str = "{stream}";
    pub const DESCRIPTION: &'static str = "Terminal outcomes of server-streaming RPCs, which end past the point a Tower layer can observe the response head.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(
        &self,
        value: u64,
        r#rpc_method: ::opentelemetry::StringValue,
        r#stream_outcome: super::attribute::StreamOutcome,
    ) {
        let attributes = [
            ::opentelemetry::KeyValue::new("rpc.method", r#rpc_method),
            ::opentelemetry::KeyValue::from(r#stream_outcome),
        ];
        self.instrument.add(value, &attributes);
    }
}

impl Default for RpcServerStreamTerminations {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct RpcServerStreamTerminationsObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl RpcServerStreamTerminationsObserver<'_> {
    pub fn observe(
        &self,
        value: u64,
        r#rpc_method: ::opentelemetry::StringValue,
        r#stream_outcome: super::attribute::StreamOutcome,
    ) {
        let attributes = [
            ::opentelemetry::KeyValue::new("rpc.method", r#rpc_method),
            ::opentelemetry::KeyValue::from(r#stream_outcome),
        ];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct RpcServerStreamTerminationsObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl RpcServerStreamTerminationsObservable {
    pub const NAME: &'static str = RpcServerStreamTerminations::NAME;
    pub const UNIT: &'static str = RpcServerStreamTerminations::UNIT;
    pub const DESCRIPTION: &'static str = RpcServerStreamTerminations::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(RpcServerStreamTerminationsObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| {
                callback(RpcServerStreamTerminationsObserver { observer })
            })
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for RpcServerStreamTerminationsObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Search index put/delete calls that failed; the index will rebuild.
#[derive(Clone)]
pub struct SearchIndexErrors {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl SearchIndexErrors {
    pub const NAME: &'static str = "trogon_atlas.search_index.errors";
    pub const UNIT: &'static str = "{error}";
    pub const DESCRIPTION: &'static str =
        "Search index put/delete calls that failed; the index will rebuild.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for SearchIndexErrors {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct SearchIndexErrorsObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl SearchIndexErrorsObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct SearchIndexErrorsObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl SearchIndexErrorsObservable {
    pub const NAME: &'static str = SearchIndexErrors::NAME;
    pub const UNIT: &'static str = SearchIndexErrors::UNIT;
    pub const DESCRIPTION: &'static str = SearchIndexErrors::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(SearchIndexErrorsObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(SearchIndexErrorsObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for SearchIndexErrorsObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Batches left for background recovery after a partial apply.
#[derive(Clone)]
pub struct StoreBatchesPendingRecovery {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl StoreBatchesPendingRecovery {
    pub const NAME: &'static str = "trogon_atlas.store.batches.pending_recovery";
    pub const UNIT: &'static str = "{batch}";
    pub const DESCRIPTION: &'static str =
        "Batches left for background recovery after a partial apply.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for StoreBatchesPendingRecovery {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct StoreBatchesPendingRecoveryObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl StoreBatchesPendingRecoveryObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct StoreBatchesPendingRecoveryObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl StoreBatchesPendingRecoveryObservable {
    pub const NAME: &'static str = StoreBatchesPendingRecovery::NAME;
    pub const UNIT: &'static str = StoreBatchesPendingRecovery::UNIT;
    pub const DESCRIPTION: &'static str = StoreBatchesPendingRecovery::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(StoreBatchesPendingRecoveryObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| {
                callback(StoreBatchesPendingRecoveryObserver { observer })
            })
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for StoreBatchesPendingRecoveryObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Change events that could not be published after their write committed.
#[derive(Clone)]
pub struct StoreChangeEventsLost {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl StoreChangeEventsLost {
    pub const NAME: &'static str = "trogon_atlas.store.change_events.lost";
    pub const UNIT: &'static str = "{event}";
    pub const DESCRIPTION: &'static str =
        "Change events that could not be published after their write committed.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for StoreChangeEventsLost {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct StoreChangeEventsLostObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl StoreChangeEventsLostObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct StoreChangeEventsLostObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl StoreChangeEventsLostObservable {
    pub const NAME: &'static str = StoreChangeEventsLost::NAME;
    pub const UNIT: &'static str = StoreChangeEventsLost::UNIT;
    pub const DESCRIPTION: &'static str = StoreChangeEventsLost::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(StoreChangeEventsLostObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(StoreChangeEventsLostObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for StoreChangeEventsLostObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Live change-tail subscriptions that had to reconnect.
#[derive(Clone)]
pub struct StoreChangeTailReconnects {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl StoreChangeTailReconnects {
    pub const NAME: &'static str = "trogon_atlas.store.change_tail.reconnects";
    pub const UNIT: &'static str = "{reconnect}";
    pub const DESCRIPTION: &'static str = "Live change-tail subscriptions that had to reconnect.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for StoreChangeTailReconnects {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct StoreChangeTailReconnectsObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl StoreChangeTailReconnectsObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct StoreChangeTailReconnectsObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl StoreChangeTailReconnectsObservable {
    pub const NAME: &'static str = StoreChangeTailReconnects::NAME;
    pub const UNIT: &'static str = StoreChangeTailReconnects::UNIT;
    pub const DESCRIPTION: &'static str = StoreChangeTailReconnects::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(StoreChangeTailReconnectsObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(StoreChangeTailReconnectsObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for StoreChangeTailReconnectsObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// NATS client disconnect events.
#[derive(Clone)]
pub struct StoreNatsDisconnects {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl StoreNatsDisconnects {
    pub const NAME: &'static str = "trogon_atlas.store.nats.disconnects";
    pub const UNIT: &'static str = "{disconnect}";
    pub const DESCRIPTION: &'static str = "NATS client disconnect events.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for StoreNatsDisconnects {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct StoreNatsDisconnectsObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl StoreNatsDisconnectsObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct StoreNatsDisconnectsObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl StoreNatsDisconnectsObservable {
    pub const NAME: &'static str = StoreNatsDisconnects::NAME;
    pub const UNIT: &'static str = StoreNatsDisconnects::UNIT;
    pub const DESCRIPTION: &'static str = StoreNatsDisconnects::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(StoreNatsDisconnectsObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(StoreNatsDisconnectsObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for StoreNatsDisconnectsObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// NATS client reconnect events.
#[derive(Clone)]
pub struct StoreNatsReconnects {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl StoreNatsReconnects {
    pub const NAME: &'static str = "trogon_atlas.store.nats.reconnects";
    pub const UNIT: &'static str = "{reconnect}";
    pub const DESCRIPTION: &'static str = "NATS client reconnect events.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for StoreNatsReconnects {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct StoreNatsReconnectsObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl StoreNatsReconnectsObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct StoreNatsReconnectsObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl StoreNatsReconnectsObservable {
    pub const NAME: &'static str = StoreNatsReconnects::NAME;
    pub const UNIT: &'static str = StoreNatsReconnects::UNIT;
    pub const DESCRIPTION: &'static str = StoreNatsReconnects::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(StoreNatsReconnectsObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(StoreNatsReconnectsObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for StoreNatsReconnectsObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Ephemeral JetStream pull consumers created to serve a read_changes call.
#[derive(Clone)]
pub struct StoreReadChangesConsumers {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl StoreReadChangesConsumers {
    pub const NAME: &'static str = "trogon_atlas.store.read_changes.consumers";
    pub const UNIT: &'static str = "{consumer}";
    pub const DESCRIPTION: &'static str =
        "Ephemeral JetStream pull consumers created to serve a read_changes call.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for StoreReadChangesConsumers {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct StoreReadChangesConsumersObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl StoreReadChangesConsumersObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct StoreReadChangesConsumersObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl StoreReadChangesConsumersObservable {
    pub const NAME: &'static str = StoreReadChangesConsumers::NAME;
    pub const UNIT: &'static str = StoreReadChangesConsumers::UNIT;
    pub const DESCRIPTION: &'static str = StoreReadChangesConsumers::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(StoreReadChangesConsumersObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(StoreReadChangesConsumersObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for StoreReadChangesConsumersObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Entity revisions that could not be recorded after their write committed.
#[derive(Clone)]
pub struct StoreRevisionsLost {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl StoreRevisionsLost {
    pub const NAME: &'static str = "trogon_atlas.store.revisions.lost";
    pub const UNIT: &'static str = "{revision}";
    pub const DESCRIPTION: &'static str =
        "Entity revisions that could not be recorded after their write committed.";

    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for StoreRevisionsLost {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct StoreRevisionsLostObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl StoreRevisionsLostObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct StoreRevisionsLostObservable {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl StoreRevisionsLostObservable {
    pub const NAME: &'static str = StoreRevisionsLost::NAME;
    pub const UNIT: &'static str = StoreRevisionsLost::UNIT;
    pub const DESCRIPTION: &'static str = StoreRevisionsLost::DESCRIPTION;

    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(StoreRevisionsLostObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter(Self::NAME)
            .with_description(Self::DESCRIPTION)
            .with_unit(Self::UNIT)
            .with_callback(move |observer| callback(StoreRevisionsLostObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for StoreRevisionsLostObservable {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
