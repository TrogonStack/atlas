//! HTTP-path tests for the Anthropic client against a mock server:
//! success, retry-on-429, and fatal client errors.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use trogon_atlas_server::llm::{AnthropicClient, AnthropicConfig, LlmClient};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, Request, Respond, ResponseTemplate,
};

fn client_for(server: &MockServer) -> AnthropicClient {
    let mut config = AnthropicConfig::new("test-key");
    config.base_url = server.uri();
    config.request_timeout = Duration::from_secs(5);
    AnthropicClient::new(config).unwrap()
}

fn success_body() -> serde_json::Value {
    serde_json::json!({
        "content": [{"type": "text", "text": "{\"mappings\": []}"}],
        "usage": {"input_tokens": 10, "output_tokens": 5}
    })
}

#[tokio::test]
async fn complete_returns_text_on_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
        .expect(1)
        .mount(&server)
        .await;

    let text = client_for(&server).complete("sys", "user").await.unwrap();
    assert_eq!(text, "{\"mappings\": []}");
}

struct FailThenSucceed {
    failures: std::sync::atomic::AtomicU32,
}

impl Respond for FailThenSucceed {
    fn respond(&self, _req: &Request) -> ResponseTemplate {
        use std::sync::atomic::Ordering;
        if self.failures.fetch_sub(1, Ordering::SeqCst) > 0 {
            ResponseTemplate::new(429).set_body_string("rate limited")
        } else {
            ResponseTemplate::new(200).set_body_json(success_body())
        }
    }
}

#[tokio::test]
async fn complete_retries_throttling_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(FailThenSucceed {
            failures: std::sync::atomic::AtomicU32::new(2),
        })
        .expect(3)
        .mount(&server)
        .await;

    let text = client_for(&server).complete("sys", "user").await.unwrap();
    assert_eq!(text, "{\"mappings\": []}");
}

#[tokio::test]
async fn complete_gives_up_after_persistent_throttling() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
        .expect(3)
        .mount(&server)
        .await;

    let err = client_for(&server)
        .complete("sys", "user")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("429"), "got: {err}");
}

#[tokio::test]
async fn complete_does_not_retry_client_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .expect(1)
        .mount(&server)
        .await;

    let err = client_for(&server)
        .complete("sys", "user")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("400"), "got: {err}");
}

#[tokio::test]
async fn complete_fails_on_malformed_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .expect(1)
        .mount(&server)
        .await;

    let err = client_for(&server)
        .complete("sys", "user")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("parsing"), "got: {err}");
}

/// A `Respond` impl that returns a body larger than `MAX_RESPONSE_BYTES`.
///
/// The body is a 200 OK so no status-code retry fires -- only the size cap
/// should trigger. The test verifies that (a) the error mentions the cap and
/// (b) the mock is called exactly once (no retries).
struct OversizedBody {
    bytes: usize,
}

impl Respond for OversizedBody {
    fn respond(&self, _req: &Request) -> ResponseTemplate {
        let body = " ".repeat(self.bytes);
        ResponseTemplate::new(200).set_body_string(body)
    }
}

#[tokio::test]
async fn complete_oversized_body_is_fatal_no_retries() {
    // 17 MiB exceeds MAX_RESPONSE_BYTES (16 MiB).
    const BODY_SIZE: usize = 17 * 1024 * 1024;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(OversizedBody { bytes: BODY_SIZE })
        // Fatal errors must not be retried -- expect exactly one call.
        .expect(1)
        .mount(&server)
        .await;

    let mut config = AnthropicConfig::new("test-key");
    config.base_url = server.uri();
    config.request_timeout = Duration::from_secs(30);
    let client = AnthropicClient::new(config).unwrap();

    let err = client.complete("sys", "user").await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("cap") || msg.contains("byte"),
        "error should mention size cap, got: {msg}"
    );
}
