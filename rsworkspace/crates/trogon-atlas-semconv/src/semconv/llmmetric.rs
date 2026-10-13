#![allow(deprecated)]
/// Analysis prompts whose link list was truncated to fit the LLM's context budget.
#[derive(Clone)]
pub struct PromptTruncationsCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl PromptTruncationsCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter.inner().u64_counter("trogon_atlas.llm.prompt_truncations")
            .with_description("Analysis prompts whose link list was truncated to fit the LLM\u{0027}s context budget.")
            .with_unit("{truncation}")
            .build();
        Self { instrument }
    }
    pub fn add(&self, value: u64) {
        let attributes = [];
        self.instrument.add(value, &attributes);
    }
}

impl Default for PromptTruncationsCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct PromptTruncationsCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl PromptTruncationsCounterObserver<'_> {
    pub fn observe(&self, value: u64) {
        let attributes = [];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct PromptTruncationsObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl PromptTruncationsObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(PromptTruncationsCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter.inner().u64_observable_counter("trogon_atlas.llm.prompt_truncations")
            .with_description("Analysis prompts whose link list was truncated to fit the LLM\u{0027}s context budget.")
            .with_unit("{truncation}")
            .with_callback(move |observer| callback(PromptTruncationsCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for PromptTruncationsObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
#[derive(Clone, Debug)]
pub enum RequestDurationHistogramAttr {
    /// Conditionally required: The request did not complete successfully.
    ErrorType(::opentelemetry::StringValue),
}

impl From<RequestDurationHistogramAttr> for ::opentelemetry::KeyValue {
    fn from(value: RequestDurationHistogramAttr) -> Self {
        match value {
            RequestDurationHistogramAttr::ErrorType(value) => {
                ::opentelemetry::KeyValue::new("error.type", value)
            }
        }
    }
}
/// Wall-clock duration of a complete LLM request, including retries.
#[derive(Clone)]
pub struct RequestDurationHistogram {
    instrument: ::opentelemetry::metrics::Histogram<f64>,
}

impl RequestDurationHistogram {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .f64_histogram("trogon_atlas.llm.request.duration")
            .with_description("Wall-clock duration of a complete LLM request, including retries.")
            .with_unit("s")
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
        r#llm_model: super::llmattr::ModelAttr,
        r#llm_provider: super::llmattr::ProviderAttr,
        options: impl IntoIterator<Item = RequestDurationHistogramAttr>,
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

impl Default for RequestDurationHistogram {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
/// Tokens exchanged with the LLM provider.
#[derive(Clone)]
pub struct TokensCounter {
    instrument: ::opentelemetry::metrics::Counter<u64>,
}

impl TokensCounter {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .u64_counter("trogon_atlas.llm.tokens")
            .with_description("Tokens exchanged with the LLM provider.")
            .with_unit("{token}")
            .build();
        Self { instrument }
    }
    pub fn add(
        &self,
        value: u64,
        r#llm_model: super::llmattr::ModelAttr,
        r#llm_token_type: super::llmattr::TokenTypeAttr,
    ) {
        let attributes = [
            ::opentelemetry::KeyValue::from(r#llm_model),
            ::opentelemetry::KeyValue::from(r#llm_token_type),
        ];
        self.instrument.add(value, &attributes);
    }
}

impl Default for TokensCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
pub struct TokensCounterObserver<'a> {
    observer: &'a dyn ::opentelemetry::metrics::AsyncInstrument<u64>,
}

impl TokensCounterObserver<'_> {
    pub fn observe(
        &self,
        value: u64,
        r#llm_model: super::llmattr::ModelAttr,
        r#llm_token_type: super::llmattr::TokenTypeAttr,
    ) {
        let attributes = [
            ::opentelemetry::KeyValue::from(r#llm_model),
            ::opentelemetry::KeyValue::from(r#llm_token_type),
        ];
        self.observer.observe(value, &attributes);
    }
}

#[derive(Clone)]
pub struct TokensObservableCounter {
    _instrument: ::opentelemetry::metrics::ObservableCounter<u64>,
}

impl TokensObservableCounter {
    pub fn new(
        meter: &super::meter::Meter,
        callback: impl Fn(TokensCounterObserver<'_>) + Send + Sync + 'static,
    ) -> Self {
        let instrument = meter
            .inner()
            .u64_observable_counter("trogon_atlas.llm.tokens")
            .with_description("Tokens exchanged with the LLM provider.")
            .with_unit("{token}")
            .with_callback(move |observer| callback(TokensCounterObserver { observer }))
            .build();
        Self {
            _instrument: instrument,
        }
    }
}

impl Default for TokensObservableCounter {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default(), |_| {})
    }
}
