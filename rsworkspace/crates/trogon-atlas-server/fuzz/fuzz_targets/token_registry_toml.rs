#![no_main]

use libfuzzer_sys::fuzz_target;
use trogon_atlas_server::auth::RegistryFile;

fuzz_target!(|data: &[u8]| {
    if let Ok(raw) = std::str::from_utf8(data) {
        let _ = toml::from_str::<RegistryFile>(raw);
    }
});
