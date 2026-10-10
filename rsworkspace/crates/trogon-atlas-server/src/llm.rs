use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tracing::Instrument as _;

/// Sonnet is the deliberate default: the analysis prompts are structured
/// JSON inference where it matches Opus-class quality at a fraction of the
/// cost. Override with `TROGON_ATLAS_LLM_MODEL` when stronger reasoning is
/// worth the price.
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-6";
pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";
const MAX_ATTEMPTS: u32 = 3;
const RETRY_BASE_DELAY: Duration = Duration::from_millis(250);
/// Cap on the response body the LLM provider may stream back. The Anthropic
/// /v1/messages contract bounds output via `max_tokens`, but a misconfigured
/// base URL pointing at a hostile proxy could otherwise send unbounded
/// bytes. 16 MiB covers any realistic completion plus margin.
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
/// Default concurrent in-flight LLM calls per process. A handler burst
/// (e.g. CI sweep over 200 slices) otherwise fans out to the provider in
/// parallel and burns budget on 429-retries. Tunable via env in `build`.
const DEFAULT_CONCURRENCY_LIMIT: usize = 4;
/// Sane ceiling on a `Retry-After` value to prevent a hostile or
/// misconfigured provider from stalling the process indefinitely.
const MAX_RETRY_AFTER: Duration = Duration::from_mins(1);

/// Provider-agnostic completion contract used by `InferDataFlow` /
/// `CheckInformationCompleteness` handlers when the LLM path is enabled.
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Issue a single-turn completion. The implementation is responsible for
    /// model selection, timeouts, retry policy, and token-budget enforcement.
    async fn complete(&self, system: &str, user: &str) -> Result<String>;

    /// Model identifier surfaced via `GetServerInfo`.
    fn model(&self) -> &str;

    /// Human-readable provider name (e.g. `"anthropic"`).
    fn provider(&self) -> &'static str;
}

#[derive(Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub max_tokens: u32,
    pub request_timeout: Duration,
}

impl std::fmt::Debug for AnthropicConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicConfig")
            .field("api_key", &"[redacted]")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("max_tokens", &self.max_tokens)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

impl AnthropicConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: DEFAULT_ANTHROPIC_MODEL.to_string(),
            base_url: DEFAULT_ANTHROPIC_BASE_URL.to_string(),
            max_tokens: 4096,
            request_timeout: Duration::from_mins(1),
        }
    }
}

pub struct AnthropicClient {
    config: AnthropicConfig,
    http: reqwest::Client,
    /// Global concurrency gate. All `complete()` callers acquire one permit
    /// for the duration of the call (including retries) so burst load can't
    /// fan out unbounded to the provider.
    concurrency: Arc<Semaphore>,
}

impl AnthropicClient {
    pub fn new(config: AnthropicConfig) -> Result<Self> {
        Self::with_concurrency(config, DEFAULT_CONCURRENCY_LIMIT)
    }

    pub fn with_concurrency(config: AnthropicConfig, max_in_flight: usize) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .build()
            .context("building reqwest client for AnthropicClient")?;
        Ok(Self {
            config,
            http,
            concurrency: Arc::new(Semaphore::new(max_in_flight.max(1))),
        })
    }

    fn build_request_body(&self, system: &str, user: &str) -> MessagesRequest {
        MessagesRequest {
            model: self.config.model.clone(),
            max_tokens: self.config.max_tokens,
            system: system.to_owned(),
            messages: vec![Message {
                role: "user".to_owned(),
                content: user.to_owned(),
            }],
        }
    }
}

/// Returned when a streaming response body exceeds `MAX_RESPONSE_BYTES`.
/// Kept as a distinct type so `complete_once` can classify it as non-retryable
/// without matching on error message strings.
#[derive(Debug)]
struct ResponseTruncated {
    cap: usize,
    seen: usize,
}

impl std::fmt::Display for ResponseTruncated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LLM response body exceeded {} byte cap (at least {} bytes seen); \
             increase MAX_RESPONSE_BYTES or reduce max_tokens",
            self.cap, self.seen
        )
    }
}

impl std::error::Error for ResponseTruncated {}

enum RequestError {
    /// Transport failures and throttling/server statuses worth retrying.
    /// `retry_after` overrides the exponential delay when the provider
    /// supplies a parseable `Retry-After: <seconds>` header.
    Retryable {
        err: anyhow::Error,
        retry_after: Option<Duration>,
    },
    /// Client errors and malformed responses; retrying cannot help.
    Fatal(anyhow::Error),
}

impl AnthropicClient {
    async fn complete_once(&self, system: &str, user: &str) -> Result<String, RequestError> {
        let body = self.build_request_body(system, user);
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        let response = self
            .http
            .post(url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|e| RequestError::Retryable {
                err: anyhow::Error::new(e).context("POST /v1/messages"),
                retry_after: None,
            })?;

        let status = response.status();
        // Before consuming the body, capture the Retry-After header from 429
        // responses so we can honor it in the retry delay below.
        let retry_after = if status.as_u16() == 429 {
            parse_retry_after(response.headers())
        } else {
            None
        };

        // Stream the body so we can enforce MAX_RESPONSE_BYTES instead of
        // pulling an unbounded blob into memory. Anthropic's contract bounds
        // output via max_tokens, but a hostile / misconfigured base URL
        // could otherwise drown the process.
        let bytes = read_capped_bytes(response, MAX_RESPONSE_BYTES)
            .await
            .map_err(|e| {
                if e.is::<ResponseTruncated>() {
                    // Truncation is non-retryable: retrying a request that
                    // already returned too many bytes will just truncate again.
                    RequestError::Fatal(anyhow::anyhow!("{e}"))
                } else {
                    RequestError::Retryable {
                        err: anyhow::anyhow!("reading response body: {e}"),
                        retry_after: None,
                    }
                }
            })?;
        if !status.is_success() {
            let snippet = String::from_utf8_lossy(&bytes);
            let err = anyhow::anyhow!("anthropic returned HTTP {status}: {snippet}");
            return if status.as_u16() == 429 || status.is_server_error() {
                Err(RequestError::Retryable { err, retry_after })
            } else {
                Err(RequestError::Fatal(err))
            };
        }

        let parsed: MessagesResponse = serde_json::from_slice(&bytes).map_err(|e| {
            let snippet = String::from_utf8_lossy(&bytes);
            RequestError::Fatal(
                anyhow::Error::new(e).context(format!("parsing MessagesResponse: {snippet}")),
            )
        })?;
        if let Some(usage) = parsed.usage.as_ref() {
            metrics::counter!("llm_tokens_total", "model" => self.config.model.clone(), "type" => "input")
                .increment(usage.input_tokens);
            metrics::counter!("llm_tokens_total", "model" => self.config.model.clone(), "type" => "output")
                .increment(usage.output_tokens);
        }
        extract_text(&parsed).map_err(RequestError::Fatal)
    }

    async fn complete_with_retry(&self, system: &str, user: &str) -> Result<String> {
        let mut next_delay: Option<Duration> = None;
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                // Use the provider's Retry-After when available; cap to
                // MAX_RETRY_AFTER so a misbehaving server cannot stall forever.
                // Fall back to exponential backoff when no header was present.
                let delay = next_delay.map_or_else(
                    || RETRY_BASE_DELAY * 2u32.pow((attempt - 1).min(6)),
                    |d| d.min(MAX_RETRY_AFTER),
                );
                tokio::time::sleep(delay).await;
            }
            match self.complete_once(system, user).await {
                Ok(text) => return Ok(text),
                Err(RequestError::Retryable { err, retry_after }) => {
                    // Strip anything that could carry the API key from the
                    // error chain before logging. Reqwest's Display impl can
                    // occasionally surface request headers in TLS / redirect
                    // errors; scrub defensively.
                    let scrubbed = scrub_secret(&format!("{err:#}"), &self.config.api_key);
                    tracing::warn!(attempt, error = %scrubbed, "retryable llm failure");
                    next_delay = retry_after;
                    last_err = Some(err);
                }
                Err(RequestError::Fatal(err)) => return Err(err),
            }
        }
        Err(last_err
            .unwrap_or_else(|| anyhow::anyhow!("llm retries exhausted without a recorded error")))
    }
}

/// Replace a secret substring with `***` in any text destined for logs.
/// Belt-and-suspenders defense against reqwest's `Display` impl leaking
/// request headers via TLS / redirect error wrapping.
fn scrub_secret(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "***")
}

/// Extract a `Retry-After` delay from response headers when the value is an
/// integer number of seconds. Non-integer forms (HTTP-date) are ignored.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let val = headers.get("retry-after")?.to_str().ok()?;
    let secs: u64 = val.trim().parse().ok()?;
    Some(Duration::from_secs(secs))
}

/// Drain a streaming response body into a `Vec<u8>` while enforcing a hard
/// size cap. Returns `Err(ResponseTruncated)` as soon as the cap would be
/// exceeded so the caller can classify the failure as non-retryable.
async fn read_capped_bytes(
    mut response: reqwest::Response,
    cap: usize,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut out = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
    {
        let seen = out.len() + chunk.len();
        if seen > cap {
            return Err(Box::new(ResponseTruncated { cap, seen }));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(&self, system: &str, user: &str) -> Result<String> {
        let span = tracing::info_span!(
            "llm.complete",
            otel.kind = "client",
            llm.provider = self.provider(),
            llm.model = %self.config.model,
        );
        async {
            // Hold one permit for the entire RPC (including retries). The
            // semaphore is never closed; `acquire()` cannot error in steady
            // state, so we surface a non-retryable failure if it does.
            let _permit = self
                .concurrency
                .acquire()
                .await
                .map_err(|e| anyhow::anyhow!("llm concurrency semaphore closed: {e}"))?;
            let started = Instant::now();
            let result = self.complete_with_retry(system, user).await;
            metrics::histogram!(
                "llm_request_duration_seconds",
                "provider" => self.provider(),
                "model" => self.config.model.clone(),
            )
            .record(started.elapsed().as_secs_f64());
            metrics::counter!(
                "llm_requests_total",
                "provider" => self.provider(),
                "model" => self.config.model.clone(),
                "outcome" => if result.is_ok() { "ok" } else { "error" },
            )
            .increment(1);
            result
        }
        .instrument(span)
        .await
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn provider(&self) -> &'static str {
        "anthropic"
    }
}

fn extract_text(resp: &MessagesResponse) -> Result<String> {
    let mut out = String::new();
    for block in &resp.content {
        if block.kind == "text" {
            out.push_str(&block.text);
        }
    }
    if out.is_empty() {
        anyhow::bail!("anthropic response had no text content blocks");
    }
    Ok(out)
}

#[derive(Serialize)]
struct MessagesRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<Message>,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LlmProvider {
    Disabled,
    Anthropic,
}

impl std::str::FromStr for LlmProvider {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "disabled" | "off" | "none" => Ok(Self::Disabled),
            "anthropic" => Ok(Self::Anthropic),
            other => anyhow::bail!("unknown llm provider: {other}"),
        }
    }
}

/// Validate that a base URL string is a well-formed http or https URL.
/// Errors eagerly at construction time so a misconfigured
/// `TROGON_ATLAS_LLM_BASE_URL` produces a clear diagnostic rather than an
/// opaque transport error on the first request.
fn validate_base_url(url: &str) -> Result<()> {
    // reqwest is already a hard dependency; parse through it so validation
    // uses the same parser that the HTTP client does.
    let parsed = reqwest::Url::parse(url)
        .with_context(|| format!("TROGON_ATLAS_LLM_BASE_URL is not a valid URL: {url:?}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!(
            "TROGON_ATLAS_LLM_BASE_URL must use http or https, got scheme {:?}",
            parsed.scheme()
        );
    }
    Ok(())
}

/// Build an LLM client from CLI/env-derived inputs. Returns `Ok(None)` when
/// the provider is disabled; bails when configuration is incoherent
/// (e.g. provider=anthropic but no API key).
pub fn build(
    provider: LlmProvider,
    api_key: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    max_tokens: Option<u32>,
    request_timeout_secs: Option<u64>,
    concurrency: Option<usize>,
) -> Result<Option<Arc<dyn LlmClient>>> {
    match provider {
        LlmProvider::Disabled => Ok(None),
        LlmProvider::Anthropic => {
            let key =
                api_key.context("--llm=anthropic requires --llm-api-key or ANTHROPIC_API_KEY")?;
            let mut config = AnthropicConfig::new(key);
            if let Some(m) = model {
                config.model = m;
            }
            if let Some(u) = base_url {
                validate_base_url(&u)?;
                config.base_url = u;
            }
            if let Some(t) = max_tokens {
                config.max_tokens = t;
            }
            if let Some(secs) = request_timeout_secs {
                config.request_timeout = Duration::from_secs(secs);
            }
            let max_in_flight = concurrency.unwrap_or(DEFAULT_CONCURRENCY_LIMIT);
            let client = AnthropicClient::with_concurrency(config, max_in_flight)?;
            Ok(Some(Arc::new(client)))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn anthropic_request_body_shape() {
        let client = AnthropicClient::new(AnthropicConfig::new("test-key")).unwrap();
        let body = client.build_request_body("sys", "hi");
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], DEFAULT_ANTHROPIC_MODEL);
        assert_eq!(json["max_tokens"], 4096);
        assert_eq!(json["system"], "sys");
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["messages"][0]["content"], "hi");
    }

    #[test]
    fn extract_text_concatenates_text_blocks() {
        let resp = MessagesResponse {
            content: vec![
                ContentBlock {
                    kind: "text".into(),
                    text: "hello ".into(),
                },
                ContentBlock {
                    kind: "thinking".into(),
                    text: "ignored".into(),
                },
                ContentBlock {
                    kind: "text".into(),
                    text: "world".into(),
                },
            ],
            usage: None,
        };
        assert_eq!(extract_text(&resp).unwrap(), "hello world");
    }

    #[test]
    fn extract_text_fails_on_empty_content() {
        let resp = MessagesResponse {
            content: vec![],
            usage: None,
        };
        assert!(extract_text(&resp).is_err());
    }

    #[test]
    fn provider_parses_known_values() {
        use std::str::FromStr;
        assert_eq!(
            LlmProvider::from_str("anthropic").unwrap(),
            LlmProvider::Anthropic
        );
        assert_eq!(
            LlmProvider::from_str("disabled").unwrap(),
            LlmProvider::Disabled
        );
        assert!(LlmProvider::from_str("openai").is_err());
    }

    #[test]
    fn build_disabled_returns_none() {
        match build(LlmProvider::Disabled, None, None, None, None, None, None) {
            Ok(opt) => assert!(opt.is_none()),
            Err(e) => panic!("disabled should not error: {e}"),
        }
    }

    #[test]
    fn build_anthropic_without_key_errors() {
        match build(LlmProvider::Anthropic, None, None, None, None, None, None) {
            Ok(_) => panic!("expected error when api key is missing"),
            Err(e) => assert!(e.to_string().contains("api-key")),
        }
    }

    #[test]
    fn build_anthropic_with_key_succeeds() {
        let client = match build(
            LlmProvider::Anthropic,
            Some("k".into()),
            None,
            None,
            None,
            None,
            None,
        ) {
            Ok(Some(c)) => c,
            other => panic!("expected Ok(Some(_)), got {:?}", other.is_ok()),
        };
        assert_eq!(client.provider(), "anthropic");
        assert_eq!(client.model(), DEFAULT_ANTHROPIC_MODEL);
    }

    #[test]
    fn build_anthropic_with_custom_model() {
        let Ok(Some(client)) = build(
            LlmProvider::Anthropic,
            Some("k".into()),
            Some("claude-opus-4-7".into()),
            None,
            None,
            None,
            None,
        ) else {
            panic!("expected Ok(Some(_))");
        };
        assert_eq!(client.model(), "claude-opus-4-7");
    }

    #[test]
    fn build_anthropic_with_custom_base_url() {
        let Ok(Some(client)) = build(
            LlmProvider::Anthropic,
            Some("k".into()),
            None,
            Some("https://proxy.example.com".into()),
            None,
            None,
            None,
        ) else {
            panic!("expected Ok(Some(_))");
        };
        assert_eq!(client.model(), DEFAULT_ANTHROPIC_MODEL);
    }

    #[test]
    fn build_anthropic_rejects_malformed_base_url() {
        match build(
            LlmProvider::Anthropic,
            Some("k".into()),
            None,
            Some("not a url at all :::".into()),
            None,
            None,
            None,
        ) {
            Ok(_) => panic!("expected error for malformed base url"),
            Err(e) => {
                let msg = format!("{e:#}");
                assert!(
                    msg.contains("TROGON_ATLAS_LLM_BASE_URL"),
                    "error message should mention the env var, got: {msg}"
                );
            }
        }
    }

    #[test]
    fn build_anthropic_rejects_non_http_base_url() {
        match build(
            LlmProvider::Anthropic,
            Some("k".into()),
            None,
            Some("ftp://files.example.com".into()),
            None,
            None,
            None,
        ) {
            Ok(_) => panic!("expected error for non-http scheme"),
            Err(e) => {
                let msg = format!("{e:#}");
                assert!(
                    msg.contains("http") || msg.contains("https"),
                    "error message should mention required schemes, got: {msg}"
                );
            }
        }
    }
}
