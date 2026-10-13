#![allow(deprecated)]
/// Time a caller spent blocked acquiring the global mutation lock.
#[derive(Clone)]
pub struct WaitDurationHistogram {
    instrument: ::opentelemetry::metrics::Histogram<f64>,
}

impl WaitDurationHistogram {
    pub fn new(meter: &super::meter::Meter) -> Self {
        let instrument = meter
            .inner()
            .f64_histogram("trogon_atlas.mutation_lock.wait.duration")
            .with_description("Time a caller spent blocked acquiring the global mutation lock.")
            .with_unit("s")
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

impl Default for WaitDurationHistogram {
    fn default() -> Self {
        Self::new(&super::meter::Meter::default())
    }
}
