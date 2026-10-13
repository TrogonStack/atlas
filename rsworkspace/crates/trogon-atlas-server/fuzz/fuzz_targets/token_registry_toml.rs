#![no_main]

use libfuzzer_sys::fuzz_target;
use trogon_atlas_server::auth::RegistryFile;

// The token registry file is operator-authored config, but it is also the
// one file on disk whose parse errors must never crash the process: a bad
// deploy should fail loudly, not take the server down with a panic.
fuzz_target!(|data: &[u8]| {
    if let Ok(raw) = std::str::from_utf8(data) {
        let _ = toml::from_str::<RegistryFile>(raw);
    }
});
