#![no_main]

use axum::http::{HeaderMap, HeaderValue};
use libfuzzer_sys::fuzz_target;
use trogon_atlas_server::mcp::client_grpc_timeout;

fuzz_target!(|data: &[u8]| {
    let Ok(value) = HeaderValue::from_bytes(data) else {
        return;
    };
    let mut headers = HeaderMap::new();
    headers.insert("grpc-timeout", value);
    let _ = client_grpc_timeout(&headers);
});
