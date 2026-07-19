//! Upstream HTTP request construction and dispatch.
//!
//! Builds `reqwest` requests for OpenAI, Anthropic, and Codex providers,
//! handles protocol-specific headers and body translation, and sends
//! requests with an optional latency timeout.

use reqwest::Client;
use serde_json::Value;
use std::borrow::Cow;

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
    codex_session_id: Option<&str>,
    output_token_limit: Option<u32>,
) -> reqwest::RequestBuilder {
    let mut req = client.post(url);
    let body = match output_token_limit {
        Some(limit) => Cow::Owned(apply_output_token_limit(
            body,
            upstream_model,
            is_anthropic,
            codex_tokens.is_some(),
            limit,
        )),
        None => Cow::Borrowed(body),
    };
    let body = body.as_ref();

    if let Some((account_id, id_token)) = codex_tokens {
        let generated_session_id = crate::proxy::codex::new_codex_request_id();
        let session_id = codex_session_id.unwrap_or(&generated_session_id);
        let mut codex_body = body.clone();
        if codex_session_id.is_some() {
            codex_body["prompt_cache_key"] = Value::String(session_id.to_string());
        }
        let codex_req = crate::proxy::codex::openai_to_codex_request_with_id(
            &codex_body,
            upstream_model,
            session_id,
        );
        let mut headers =
            crate::proxy::codex::build_codex_auth_headers(api_key, account_id, id_token);
        headers.extend(crate::proxy::codex::build_codex_responses_headers(
            session_id,
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
        let body = build_openai_compatible_body(body, upstream_model, url);
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
        let body = build_openai_compatible_body(body, upstream_model, url);
        req = req.json(&body);
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }

    req
}

fn build_openai_compatible_body(body: &Value, upstream_model: &str, url: &str) -> Value {
    let mut body = body.clone();
    body["model"] = Value::String(upstream_model.to_string());

    if requires_default_sampling(upstream_model, url) {
        if let Some(obj) = body.as_object_mut() {
            obj.remove("temperature");
            obj.remove("top_p");
        }
    }

    body
}

fn apply_output_token_limit(
    body: &Value,
    upstream_model: &str,
    is_anthropic: bool,
    is_codex: bool,
    limit: u32,
) -> Value {
    let mut body = body.clone();
    let Some(obj) = body.as_object_mut() else {
        return body;
    };
    obj.remove("max_tokens");
    obj.remove("max_completion_tokens");

    // The ChatGPT Codex Responses endpoint intentionally exposes no output
    // limit for primary agent calls. Keep the advisor limit best-effort there.
    if is_codex {
        return body;
    }
    let field = if !is_anthropic && requires_max_completion_tokens(upstream_model) {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    obj.insert(field.to_string(), Value::Number(limit.into()));
    body
}

fn requires_max_completion_tokens(model: &str) -> bool {
    let model = model
        .rsplit_once('/')
        .map(|(_, model)| model)
        .unwrap_or(model)
        .to_ascii_lowercase();
    model.contains("gpt-5")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
}

fn requires_default_sampling(model: &str, url: &str) -> bool {
    let model = model.to_ascii_lowercase();
    let url = url.to_ascii_lowercase();
    url.contains("opencode.ai/zen/go")
        && (model.contains("kimi-k2.7") || model.contains("kimi-k2-7"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_body_drops_sampling_for_kimi_k2_7() {
        let body = serde_json::json!({
            "model": "kimi-k2-7",
            "messages": [{"role": "user", "content": "hello"}],
            "temperature": 0.3,
            "top_p": 0.9
        });

        let result = build_openai_compatible_body(
            &body,
            "kimi-k2.7-code",
            "https://opencode.ai/zen/go/v1/chat/completions",
        );

        assert_eq!(result["model"], "kimi-k2.7-code");
        assert!(result.get("temperature").is_none());
        assert!(result.get("top_p").is_none());
    }

    #[test]
    fn test_openai_body_preserves_sampling_for_other_models() {
        let body = serde_json::json!({
            "model": "glm-5-2",
            "messages": [{"role": "user", "content": "hello"}],
            "temperature": 0.3,
            "top_p": 0.9
        });

        let result = build_openai_compatible_body(
            &body,
            "glm-5.2",
            "https://opencode.ai/zen/go/v1/chat/completions",
        );

        assert_eq!(result["model"], "glm-5.2");
        assert_eq!(result["temperature"], 0.3);
        assert_eq!(result["top_p"], 0.9);
    }

    #[test]
    fn test_openai_body_preserves_sampling_for_non_opencode_kimi() {
        let body = serde_json::json!({
            "model": "kimi-k2-7",
            "messages": [{"role": "user", "content": "hello"}],
            "temperature": 0.3,
            "top_p": 0.9
        });

        let result = build_openai_compatible_body(
            &body,
            "kimi-k2-7-code",
            "https://api.venice.ai/api/v1/chat/completions",
        );

        assert_eq!(result["model"], "kimi-k2-7-code");
        assert_eq!(result["temperature"], 0.3);
        assert_eq!(result["top_p"], 0.9);
    }

    #[test]
    fn test_reference_limit_uses_provider_compatible_token_field() {
        let body = serde_json::json!({
            "model": "alias",
            "messages": [{"role": "user", "content": "hello"}],
            "max_tokens": 9999
        });

        for model in ["gpt-5.5", "openai/gpt-5.6-sol", "o3-mini"] {
            let result = apply_output_token_limit(&body, model, false, false, 600);
            assert!(result.get("max_tokens").is_none());
            assert_eq!(result["max_completion_tokens"], 600);
        }

        let anthropic = apply_output_token_limit(&body, "claude", true, false, 600);
        assert_eq!(anthropic["max_tokens"], 600);
        assert!(anthropic.get("max_completion_tokens").is_none());

        let codex = apply_output_token_limit(&body, "gpt-5.5", false, true, 600);
        assert!(codex.get("max_tokens").is_none());
        assert!(codex.get("max_completion_tokens").is_none());
    }

    #[test]
    fn test_codex_request_reuses_stable_session_for_header_and_cache_key() {
        let client = Client::new();
        let body = serde_json::json!({
            "model": "alias",
            "messages": [{"role": "user", "content": "hello"}],
            "prompt_cache_key": "conflicting-body-key"
        });
        let request = build_upstream_request(
            &client,
            &body,
            "access-token",
            "gpt-5.5",
            "https://chatgpt.com/backend-api/codex/responses",
            false,
            Some((Some("account-id"), None)),
            Some("ses_stable"),
            Some(600),
        )
        .build()
        .unwrap();

        assert_eq!(request.headers()["session-id"], "ses_stable");
        let request_body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(request_body["prompt_cache_key"], "ses_stable");
    }
}
