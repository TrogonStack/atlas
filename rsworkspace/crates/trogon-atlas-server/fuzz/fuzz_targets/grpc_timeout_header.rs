#![no_main]

use axum::http::{HeaderMap, HeaderValue};
use libfuzzer_sys::fuzz_target;
use trogon_atlas_server::mcp::client_grpc_timeout;

// grpc-timeout is a header a caller sets on every RPC, parsed into a
// Duration that then bounds how long the server will wait. It is untrusted
// input on every request, not just a config-time value.
fuzz_target!(|data: &[u8]| {
    let Ok(value) = HeaderValue::from_bytes(data) else {
        return;
    };
    let mut headers = HeaderMap::new();
    headers.insert("grpc-timeout", value);
    let _ = client_grpc_timeout(&headers);
});
