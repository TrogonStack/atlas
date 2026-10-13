#![allow(deprecated)]
#[derive(Clone, Debug)]
pub enum ServerCallDurationHistogramAttr {
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

impl From<ServerCallDurationHistogramAttr> for ::opentelemetry::KeyValue {
    fn from(value: ServerCallDurationHistogramAttr) -> Self {
        match value {
            ServerCallDurationHistogramAttr::ErrorType(value) => {
                ::opentelemetry::KeyValue::new("error.type", value)
            }
            ServerCallDurationHistogramAttr::RpcMethod(value) => {
                ::opentelemetry::KeyValue::new("rpc.method", value)
            }
            ServerCallDurationHistogramAttr::RpcResponseStatusCode(value) => {
                ::opentelemetry::KeyValue::new("rpc.response.status_code", value)
            }
            ServerCallDurationHistogramAttr::ServerAddress(value) => {
                ::opentelemetry::KeyValue::new("server.address", value)
            }
            ServerCallDurationHistogramAttr::ServerPort(value) => {
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
pub struct ServerCallDurationHistogram {
    instrument: ::opentelemetry::metrics::Histogram<f64>,
}

impl ServerCallDurationHistogram {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .f64_histogram("rpc.server.call.duration")
            .with_description("Measures the duration of an incoming Remote Procedure Call (RPC).")
            .with_unit("s")
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
        options: impl IntoIterator<Item = ServerCallDurationHistogramAttr>,
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

impl Default for ServerCallDurationHistogram {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
/// Terminal outcomes of server-streaming RPCs, which end past the point a Tower layer can observe the response head.
#[derive(Clone)]
pub struct ServerStreamTerminationsCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl ServerStreamTerminationsCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter.inner().u64_counter("trogon_atlas.rpc.server.stream.terminations")
            .with_description("Terminal outcomes of server-streaming RPCs, which end past the point a Tower layer can observe the response head.")
            .with_unit("{stream}")
            .build();
        Self { instrument }
    }
    pub fn add(
        &self,
        value: u64,
        r#rpc_method: ::opentelemetry::StringValue,
        r#stream_outcome: super::streamattr::OutcomeAttr,
    ) {
        let attributes = [
            ::opentelemetry::KeyValue::new("rpc.method", r#rpc_method),
            ::opentelemetry::KeyValue::from(r#stream_outcome),
        ];
        self.instrument.add(value, &attributes);
    }
}

impl Default for ServerStreamTerminationsCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct ServerStreamTerminationsCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl ServerStreamTerminationsCounterObserver<'_> {
    pub fn observe(
        &self,
        value: u64,
        r#rpc_method: ::opentelemetry::StringValue,
        r#stream_outcome: super::streamattr::OutcomeAttr,
    ) {
        let attributes = [
            ::opentelemetry::KeyValue::new("rpc.method", r#rpc_method),
            ::opentelemetry::KeyValue::from(r#stream_outcome),
        ];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct ServerStreamTerminationsObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl ServerStreamTerminationsObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(ServerStreamTerminationsCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter.inner().u64_observable_counter("trogon_atlas.rpc.server.stream.terminations")
            .with_description("Terminal outcomes of server-streaming RPCs, which end past the point a Tower layer can observe the response head.")
            .with_unit("{stream}")
            .with_callback(move |observer| callback(ServerStreamTerminationsCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for ServerStreamTerminationsObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
