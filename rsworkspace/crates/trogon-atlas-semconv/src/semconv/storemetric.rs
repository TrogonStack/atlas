#![allow(deprecated)]
/// Batches left for background recovery after a partial apply.
#[derive(Clone)]
pub struct BatchesPendingRecoveryCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl BatchesPendingRecoveryCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.store.batches.pending_recovery")
            .with_description("Batches left for background recovery after a partial apply.")
            .with_unit("{batch}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for BatchesPendingRecoveryCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct BatchesPendingRecoveryCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl BatchesPendingRecoveryCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct BatchesPendingRecoveryObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl BatchesPendingRecoveryObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(BatchesPendingRecoveryCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.store.batches.pending_recovery")
            .with_description("Batches left for background recovery after a partial apply.")
            .with_unit("{batch}")
            .with_callback(move |observer| {
                callback(BatchesPendingRecoveryCounterObserver { observer })
            })
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for BatchesPendingRecoveryObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Change events that could not be published after their write committed.
#[derive(Clone)]
pub struct ChangeEventsLostCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl ChangeEventsLostCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.store.change_events.lost")
            .with_description(
                "Change events that could not be published after their write committed.",
            )
            .with_unit("{event}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for ChangeEventsLostCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct ChangeEventsLostCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl ChangeEventsLostCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct ChangeEventsLostObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl ChangeEventsLostObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(ChangeEventsLostCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.store.change_events.lost")
            .with_description(
                "Change events that could not be published after their write committed.",
            )
            .with_unit("{event}")
            .with_callback(move |observer| callback(ChangeEventsLostCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for ChangeEventsLostObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Live change-tail subscriptions that had to reconnect.
#[derive(Clone)]
pub struct ChangeTailReconnectsCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl ChangeTailReconnectsCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.store.change_tail.reconnects")
            .with_description("Live change-tail subscriptions that had to reconnect.")
            .with_unit("{reconnect}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for ChangeTailReconnectsCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct ChangeTailReconnectsCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl ChangeTailReconnectsCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct ChangeTailReconnectsObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl ChangeTailReconnectsObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(ChangeTailReconnectsCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.store.change_tail.reconnects")
            .with_description("Live change-tail subscriptions that had to reconnect.")
            .with_unit("{reconnect}")
            .with_callback(move |observer| {
                callback(ChangeTailReconnectsCounterObserver { observer })
            })
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for ChangeTailReconnectsObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// NATS client disconnect events.
#[derive(Clone)]
pub struct NatsDisconnectsCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl NatsDisconnectsCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.store.nats.disconnects")
            .with_description("NATS client disconnect events.")
            .with_unit("{disconnect}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for NatsDisconnectsCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct NatsDisconnectsCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl NatsDisconnectsCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct NatsDisconnectsObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl NatsDisconnectsObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(NatsDisconnectsCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.store.nats.disconnects")
            .with_description("NATS client disconnect events.")
            .with_unit("{disconnect}")
            .with_callback(move |observer| callback(NatsDisconnectsCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for NatsDisconnectsObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// NATS client reconnect events.
#[derive(Clone)]
pub struct NatsReconnectsCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl NatsReconnectsCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.store.nats.reconnects")
            .with_description("NATS client reconnect events.")
            .with_unit("{reconnect}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for NatsReconnectsCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct NatsReconnectsCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl NatsReconnectsCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct NatsReconnectsObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl NatsReconnectsObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(NatsReconnectsCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.store.nats.reconnects")
            .with_description("NATS client reconnect events.")
            .with_unit("{reconnect}")
            .with_callback(move |observer| callback(NatsReconnectsCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for NatsReconnectsObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Ephemeral JetStream pull consumers created to serve a read_changes call.
#[derive(Clone)]
pub struct ReadChangesConsumersCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl ReadChangesConsumersCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.store.read_changes.consumers")
            .with_description(
                "Ephemeral JetStream pull consumers created to serve a read_changes call.",
            )
            .with_unit("{consumer}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for ReadChangesConsumersCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct ReadChangesConsumersCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl ReadChangesConsumersCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct ReadChangesConsumersObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl ReadChangesConsumersObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(ReadChangesConsumersCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.store.read_changes.consumers")
            .with_description(
                "Ephemeral JetStream pull consumers created to serve a read_changes call.",
            )
            .with_unit("{consumer}")
            .with_callback(move |observer| {
                callback(ReadChangesConsumersCounterObserver { observer })
            })
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for ReadChangesConsumersObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Entity revisions that could not be recorded after their write committed.
#[derive(Clone)]
pub struct RevisionsLostCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl RevisionsLostCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.store.revisions.lost")
            .with_description(
                "Entity revisions that could not be recorded after their write committed.",
            )
            .with_unit("{revision}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for RevisionsLostCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct RevisionsLostCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl RevisionsLostCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct RevisionsLostObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl RevisionsLostObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(RevisionsLostCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.store.revisions.lost")
            .with_description(
                "Entity revisions that could not be recorded after their write committed.",
            )
            .with_unit("{revision}")
            .with_callback(move |observer| callback(RevisionsLostCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for RevisionsLostObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
