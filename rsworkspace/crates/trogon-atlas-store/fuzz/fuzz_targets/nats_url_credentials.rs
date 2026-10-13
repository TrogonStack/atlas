#![no_main]

use libfuzzer_sys::fuzz_target;
use trogon_atlas_store::nats::NatsStoreConfig;

// NatsStoreConfig::new carries the NATS server URL straight from operator
// config, env vars, or a mounted secret, and immediately strips any
// `user:pass@` userinfo out of it. That split runs on untrusted text before
// anything validates it, so it is the real attack surface, not a synthetic
// one.
fuzz_target!(|data: &[u8]| {
    if let Ok(url) = std::str::from_utf8(data) {
        let _ = NatsStoreConfig::new(url);
    }
});
