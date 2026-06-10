//! Upstream HTTP request construction and dispatch.
//!
//! Builds `reqwest` requests for OpenAI, Anthropic, and Codex providers,
//! handles protocol-specific headers and body translation, and sends
//! requests with an optional latency timeout.

use reqwest::Client;
use serde_json::Value;

/// Builds a `reqwest::RequestBuilder` for a chat-completion request.
///
/// - `is_anthropic`: translates to Anthropic `/messages` format.
/// - `codex_tokens`: if `Some((account_id, id_token))`, translates to Codex
///   `/responses` format and attaches Codex auth headers. The `api_key` is
///   the access_token.
/// - Otherwise, injects `upstream_model` and uses a standard `Bearer` token.
pub fn build_upstream_request(
    client: &Client,
    body: &Value,
    api_key: &str,
    upstream_model: &str,
    url: &str,
    is_anthropic: bool,
    codex_tokens: Option<(Option<&str>, Option<&str>)>,
) -> reqwest::RequestBuilder {
    let mut req = client.post(url);

    if let Some((account_id, id_token)) = codex_tokens {
        let request_id = crate::proxy::codex::new_codex_request_id();
        let codex_req =
            crate::proxy::codex::openai_to_codex_request_with_id(body, upstream_model, &request_id);
        let mut headers =
            crate::proxy::codex::build_codex_auth_headers(api_key, account_id, id_token);
        headers.extend(crate::proxy::codex::build_codex_responses_headers(
            &request_id,
        ));
        for (key, value) in &headers {
            req = req.header(key, value);
        }
        req = req.json(&codex_req);
    } else if is_anthropic {
        let anthropic_req = crate::proxy::translator::openai_to_anthropic(body, upstream_model);
        let anthropic_headers = crate::proxy::translator::anthropic_headers(api_key);
        for (key, value) in &anthropic_headers {
            req = req.header(key, value);
        }
        req = req.json(&anthropic_req);
    } else {
        let mut body = body.clone();
        body["model"] = Value::String(upstream_model.to_string());
        req = req.json(&body);
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }

    req
}

/// Builds a `reqwest::RequestBuilder` for a legacy `/completions` request.
///
/// For Anthropic and Codex providers, converts the `prompt` field into a
/// single-user `messages` array since neither has a `/completions` endpoint.
pub fn build_completion_request(
    client: &Client,
    body: &Value,
    api_key: &str,
    upstream_model: &str,
    url: &str,
    is_anthropic: bool,
) -> reqwest::RequestBuilder {
    let mut req = client.post(url);

    if is_anthropic {
        let anthropic_headers = crate::proxy::translator::anthropic_headers(api_key);
        for (key, value) in &anthropic_headers {
            req = req.header(key, value);
        }
        let mut anthropic_body = serde_json::Map::new();
        if let Some(model) = body.get("model").and_then(|v| v.as_str()) {
            anthropic_body.insert("model".to_string(), Value::String(model.to_string()));
        } else {
            anthropic_body.insert(
                "model".to_string(),
                Value::String(upstream_model.to_string()),
            );
        }
        if let Some(prompt) = body.get("prompt") {
            let messages = vec![serde_json::json!({
                "role": "user",
                "content": prompt.as_str().unwrap_or("")
            })];
            anthropic_body.insert("messages".to_string(), Value::Array(messages));
        }
        if let Some(max_tokens) = body.get("max_tokens").and_then(|v| v.as_u64()) {
            anthropic_body.insert(
                "max_tokens".to_string(),
                Value::Number(serde_json::Number::from(max_tokens)),
            );
        }
        req = req.json(&anthropic_body);
    } else {
        let mut body = body.clone();
        body["model"] = Value::String(upstream_model.to_string());
        req = req.json(&body);
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }

    req
}

/// Sends a request with an optional wall-clock latency timeout.
///
/// When `on_latency_timeout` is true, the request is wrapped in a
/// `tokio::time::timeout` of `timeout_ms`. If the upstream does not
/// respond within that window, [`UpstreamError::Timeout`] is returned so
/// the caller can trigger failover.
pub async fn send_with_timeout(
    req: reqwest::RequestBuilder,
    timeout_ms: u64,
    on_latency_timeout: bool,
) -> Result<reqwest::Response, UpstreamError> {
    if on_latency_timeout {
        let duration = std::time::Duration::from_millis(timeout_ms);
        match tokio::time::timeout(duration, req.send()).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(e)) => Err(UpstreamError::Network(e.to_string())),
            Err(_) => Err(UpstreamError::Timeout),
        }
    } else {
        match req.send().await {
            Ok(resp) => Ok(resp),
            Err(e) => Err(UpstreamError::Network(e.to_string())),
        }
    }
}

/// Errors that can occur when dispatching an upstream request.
#[derive(Debug)]
pub enum UpstreamError {
    /// A network-level error (DNS, connection refused, TLS, etc.).
    Network(String),
    /// The upstream did not respond within the configured latency timeout.
    Timeout,
}

/// Creates a new `reqwest::Client` with a 30 s connect timeout.
///
/// No total timeout is configured because streaming responses can run for
/// minutes (e.g., reasoning/thinking). Per-layer timeouts handle each
/// phase: connect, TTFB, inter-chunk, and non-streaming body read.
pub fn create_client(_timeout_secs: u64) -> Result<Client, reqwest::Error> {
    Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
}
