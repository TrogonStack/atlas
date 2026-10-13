#![allow(deprecated)]
/// Current message count of the durable change stream.
#[derive(Clone)]
pub struct MessageCountGauge {
    instrument: ::opentelemetry::metrics::Gauge<i64>,
}

impl MessageCountGauge {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .i64_gauge("trogon_atlas.change_stream.message.count")
            .with_description("Current message count of the durable change stream.")
            .with_unit("{message}")
            .build();
        Self { instrument }
    }
    pub fn record(&self, value: i64) {
        let attributes = [];
        self.instrument.record(value, &attributes);
    }
}

impl Default for MessageCountGauge {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct MessageCountGaugeObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<i64>,
}

impl MessageCountGaugeObserver<'_> {
    pub fn observe(&self, value: i64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct MessageCountObservableGauge {
    _instrument: ::opentelemetry::metrics::ObservableGauge<i64>,
}

impl MessageCountObservableGauge {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(MessageCountGaugeObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .i64_observable_gauge("trogon_atlas.change_stream.message.count")
            .with_description("Current message count of the durable change stream.")
            .with_unit("{message}")
            .with_callback(move |observer| callback(MessageCountGaugeObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for MessageCountObservableGauge {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Age of the oldest retained message in the durable change stream; zero when the stream is empty.
#[derive(Clone)]
pub struct OldestMessageAgeGauge {
    instrument: ::opentelemetry::metrics::Gauge<i64>,
}

impl OldestMessageAgeGauge {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter.inner().i64_gauge("trogon_atlas.change_stream.oldest_message.age")
            .with_description("Age of the oldest retained message in the durable change stream; zero when the stream is empty.")
            .with_unit("s")
            .build();
        Self { instrument }
    }
    pub fn record(&self, value: i64) {
        let attributes = [];
        self.instrument.record(value, &attributes);
    }
}

impl Default for OldestMessageAgeGauge {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct OldestMessageAgeGaugeObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<i64>,
}

impl OldestMessageAgeGaugeObserver<'_> {
    pub fn observe(&self, value: i64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct OldestMessageAgeObservableGauge {
    _instrument: ::opentelemetry::metrics::ObservableGauge<i64>,
}

impl OldestMessageAgeObservableGauge {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(OldestMessageAgeGaugeObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter.inner().i64_observable_gauge("trogon_atlas.change_stream.oldest_message.age")
            .with_description("Age of the oldest retained message in the durable change stream; zero when the stream is empty.")
            .with_unit("s")
            .with_callback(move |observer| callback(OldestMessageAgeGaugeObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for OldestMessageAgeObservableGauge {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
/// Current byte size of the durable change stream.
#[derive(Clone)]
pub struct SizeGauge {
    instrument: ::opentelemetry::metrics::Gauge<i64>,
}

impl SizeGauge {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .i64_gauge("trogon_atlas.change_stream.size")
            .with_description("Current byte size of the durable change stream.")
            .with_unit("By")
            .build();
        Self { instrument }
    }
    pub fn record(&self, value: i64) {
        let attributes = [];
        self.instrument.record(value, &attributes);
    }
}

impl Default for SizeGauge {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct SizeGaugeObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<i64>,
}

impl SizeGaugeObserver<'_> {
    pub fn observe(&self, value: i64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct SizeObservableGauge {
    _instrument: ::opentelemetry::metrics::ObservableGauge<i64>,
}

impl SizeObservableGauge {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(SizeGaugeObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .i64_observable_gauge("trogon_atlas.change_stream.size")
            .with_description("Current byte size of the durable change stream.")
            .with_unit("By")
            .with_callback(move |observer| callback(SizeGaugeObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for SizeObservableGauge {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
