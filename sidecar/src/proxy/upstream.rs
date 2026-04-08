use reqwest::Client;
use serde_json::Value;

pub fn build_upstream_request(
    client: &Client,
    body: &Value,
    api_key: &str,
    upstream_model: &str,
    url: &str,
    is_anthropic: bool,
) -> reqwest::RequestBuilder {
    let mut req = client.post(url);

    if is_anthropic {
        use crate::proxy::translator;
        let anthropic_req = translator::openai_to_anthropic(body, upstream_model);
        let anthropic_headers = translator::anthropic_headers(api_key);
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
            anthropic_body.insert("model".to_string(), Value::String(upstream_model.to_string()));
        }
        if let Some(prompt) = body.get("prompt") {
            let messages = vec![serde_json::json!({
                "role": "user",
                "content": prompt.as_str().unwrap_or("")
            })];
            anthropic_body.insert("messages".to_string(), Value::Array(messages));
        }
        if let Some(max_tokens) = body.get("max_tokens").and_then(|v| v.as_u64()) {
            anthropic_body.insert("max_tokens".to_string(), Value::Number(serde_json::Number::from(max_tokens)));
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

pub async fn send_with_timeout(
    req: reqwest::RequestBuilder,
    timeout_ms: u64,
    on_latency_timeout: bool,
) -> Result<reqwest::Response, UpstreamError> {
    if on_latency_timeout {
        let req = req.timeout(std::time::Duration::from_millis(timeout_ms));
        match req.send().await {
            Ok(resp) => Ok(resp),
            Err(e) => {
                if e.is_timeout() {
                    Err(UpstreamError::Timeout)
                } else {
                    Err(UpstreamError::Network(e.to_string()))
                }
            }
        }
    } else {
        match req.send().await {
            Ok(resp) => Ok(resp),
            Err(e) => Err(UpstreamError::Network(e.to_string())),
        }
    }
}

#[derive(Debug)]
pub enum UpstreamError {
    Network(String),
    Timeout,
}

pub fn create_client(timeout_secs: u64) -> Result<Client, reqwest::Error> {
    Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
}
