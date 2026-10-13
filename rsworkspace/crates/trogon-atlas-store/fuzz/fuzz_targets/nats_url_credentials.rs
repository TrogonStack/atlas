#![no_main]

use libfuzzer_sys::fuzz_target;
use trogon_atlas_store::{nats::NatsStoreConfig, StoreMetrics};

fuzz_target!(|data: &[u8]| {
    if let Ok(url) = std::str::from_utf8(data) {
        let _ = NatsStoreConfig::new(url, StoreMetrics::default());
    }
});
