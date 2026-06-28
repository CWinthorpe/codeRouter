//! OpenAI Codex (ChatGPT device-login) provider integration.
//!
//! Handles:
//! - Parsing Codex CLI `auth.json` credentials.
//! - Building Codex-specific auth headers from those credentials.
//! - Token expiry detection via JWT `exp` claim and refresh via OAuth token endpoint.
//! - Translating OpenAI chat-completion requests to the Codex Responses API.
//! - SSE translation: Codex Responses SSE events → OpenAI chat-completion SSE chunks.

use base64::Engine;
use serde::de;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

const CODEX_AUTH_ISSUER: &str = "https://auth.openai.com";
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_DEVICE_EXPIRES_IN_SECONDS: u64 = 15 * 60;
pub const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.135.0";
const CODEX_ORIGINATOR: &str = "opencode";

// ── Codex auth types ──

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CodexTokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CodexAuth {
    pub tokens: CodexTokens,
}

/// Parses the credential string. It may be a raw access token or a JSON
/// object (the full `auth.json`).  If JSON, extracts the token fields.
///
/// Returns `(access_token, account_id, id_token, refresh_token)`.
pub fn parse_codex_credential(
    credential: &str,
) -> (String, Option<String>, Option<String>, Option<String>) {
    if credential.trim().starts_with('{') {
        if let Ok(auth) = serde_json::from_str::<CodexAuth>(credential) {
            let access_token = auth.tokens.access_token;
            let id_token = auth.tokens.id_token;
            let refresh_token = auth.tokens.refresh_token;
            let (acct_from_id, _) = decode_id_token_claims(id_token.as_deref());
            let (acct_from_access, _) = decode_id_token_claims(Some(&access_token));
            let account_id = auth.tokens.account_id.or(acct_from_id).or(acct_from_access);
            return (access_token, account_id, id_token, refresh_token);
        }
        if let Ok(tokens) = serde_json::from_str::<CodexTokens>(credential) {
            let access_token = tokens.access_token;
            let id_token = tokens.id_token;
            let refresh_token = tokens.refresh_token;
            let (acct_from_id, _) = decode_id_token_claims(id_token.as_deref());
            let (acct_from_access, _) = decode_id_token_claims(Some(&access_token));
            let account_id = tokens.account_id.or(acct_from_id).or(acct_from_access);
            return (access_token, account_id, id_token, refresh_token);
        }
        // Fallback: raw JSON string extraction
        if let Ok(value) = serde_json::from_str::<Value>(credential) {
            let access = value
                .get("access_token")
                .or_else(|| value.pointer("/tokens/access_token"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if access.is_empty() {
                return (credential.to_string(), None, None, None);
            }
            let refresh = value
                .get("refresh_token")
                .or_else(|| value.pointer("/tokens/refresh_token"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let account = value
                .get("account_id")
                .or_else(|| value.pointer("/tokens/account_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let id = value
                .get("id_token")
                .or_else(|| value.pointer("/tokens/id_token"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let (acct_from_id, _) = decode_id_token_claims(id.as_deref());
            let (acct_from_access, _) = decode_id_token_claims(Some(access));
            let account_id = account.or(acct_from_id).or(acct_from_access);
            return (access.to_string(), account_id, id, refresh);
        }
    }

    // Raw token — treat as access_token only
    (credential.to_string(), None, None, None)
}

/// Decodes a Codex JWT payload and returns `(chatgpt_account_id, is_fedramp)`.
///
/// Checks both top-level `fedramp` and the nested `https://api.openai.com/auth`
/// namespace used by Codex JWTs.
fn decode_id_token_claims(id_token: Option<&str>) -> (Option<String>, bool) {
    let id = match id_token {
        Some(t) if !t.is_empty() => t,
        _ => return (None, false),
    };
    let parts: Vec<&str> = id.split('.').collect();
    if parts.len() < 2 {
        return (None, false);
    }
    let payload = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(bytes) => bytes,
        Err(_) => return (None, false),
    };
    let value: Value = match serde_json::from_slice(&payload) {
        Ok(v) => v,
        Err(_) => return (None, false),
    };

    let mut account_id = None;
    let mut is_fedramp = false;

    if let Some(acct) = value.get("chatgpt_account_id").and_then(|v| v.as_str()) {
        account_id = Some(acct.to_string());
    }

    if let Some(fr) = value.get("fedramp").and_then(|v| v.as_bool()) {
        is_fedramp = fr;
    }

    // OpenAI namespace claims: https://api.openai.com/auth
    if let Some(auth_ns) = value.get("https://api.openai.com/auth") {
        if let Some(acct) = auth_ns.get("chatgpt_account_id").and_then(|v| v.as_str()) {
            account_id = Some(acct.to_string());
        }
        if let Some(fr) = auth_ns
            .get("chatgpt_account_is_fedramp")
            .and_then(|v| v.as_bool())
        {
            is_fedramp = fr;
        }
    }

    if account_id.is_none() {
        account_id = value
            .get("organizations")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }

    (account_id, is_fedramp)
}

/// Returns true when the `id_token` JWT payload indicates FedRAMP.
pub fn check_fedramp(id_token: &str) -> bool {
    let (_, fed) = decode_id_token_claims(Some(id_token));
    fed
}

/// Builds Codex auth headers from parsed credentials.
///
/// Headers:
/// - `Authorization: Bearer <access_token>`
/// - Codex client identity headers (`originator`, `User-Agent`)
/// - `ChatGPT-Account-Id: <account_id>` when present
/// - `X-OpenAI-Fedramp: true` when the id token claims FedRAMP
pub fn build_codex_auth_headers(
    access_token: &str,
    account_id: Option<&str>,
    id_token: Option<&str>,
) -> Vec<(String, String)> {
    let mut headers = vec![
        (
            "Authorization".to_string(),
            format!("Bearer {access_token}"),
        ),
        ("content-type".to_string(), "application/json".to_string()),
        ("originator".to_string(), CODEX_ORIGINATOR.to_string()),
        ("User-Agent".to_string(), codex_user_agent()),
    ];

    if let Some(aid) = account_id {
        if !aid.is_empty() {
            headers.push(("ChatGPT-Account-Id".to_string(), aid.to_string()));
        }
    }

    if let Some(id) = id_token {
        if check_fedramp(id) {
            headers.push(("X-OpenAI-Fedramp".to_string(), "true".to_string()));
        }
    }

    headers
}

pub fn codex_client_version() -> String {
    std::env::var("CODEROUTER_CODEX_CLIENT_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_CODEX_CLIENT_VERSION.to_string())
}

fn codex_user_agent() -> String {
    format!(
        "{}/{} ({} {}; {})",
        CODEX_ORIGINATOR,
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        platform_release(),
        std::env::consts::ARCH
    )
}

fn platform_release() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn new_codex_request_id() -> String {
    uuid_short()
}

pub fn build_codex_responses_headers(request_id: &str) -> Vec<(String, String)> {
    vec![("session-id".to_string(), request_id.to_string())]
}

// ── Device auth ──

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CodexDeviceAuthStart {
    pub verification_url: String,
    pub user_code: String,
    pub device_auth_id: String,
    pub interval: u64,
    pub expires_in: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CodexDeviceAuthPoll {
    pub status: String,
    #[serde(default)]
    pub credential: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Deserialize)]
struct DeviceUserCodeResponse {
    device_auth_id: String,
    #[serde(alias = "user_code", alias = "usercode")]
    user_code: String,
    #[serde(
        default = "default_device_poll_interval",
        deserialize_with = "deserialize_interval"
    )]
    interval: u64,
}

#[derive(Deserialize)]
struct DeviceTokenResponse {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Deserialize)]
struct AuthCodeTokenResponse {
    access_token: String,
    refresh_token: String,
    id_token: String,
}

fn default_device_poll_interval() -> u64 {
    5
}

fn deserialize_interval<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| de::Error::custom("interval must be a positive integer")),
        Value::String(s) => s
            .trim()
            .parse::<u64>()
            .map_err(|e| de::Error::custom(format!("invalid interval: {e}"))),
        _ => Err(de::Error::custom("interval must be a string or integer")),
    }
}

/// Starts Codex's ChatGPT device login flow and returns the browser URL plus
/// one-time user code to display to the user.
pub async fn start_device_auth(
    client: &reqwest::Client,
) -> Result<CodexDeviceAuthStart, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!(
        "{}/api/accounts/deviceauth/usercode",
        CODEX_AUTH_ISSUER.trim_end_matches('/')
    );
    let resp = client
        .post(url)
        .header("User-Agent", codex_user_agent())
        .json(&serde_json::json!({ "client_id": CODEX_CLIENT_ID }))
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(format!("Codex device code request failed: HTTP {}", resp.status()).into());
    }

    let body: DeviceUserCodeResponse = resp.json().await?;
    Ok(CodexDeviceAuthStart {
        verification_url: format!("{}/codex/device", CODEX_AUTH_ISSUER.trim_end_matches('/')),
        user_code: body.user_code,
        device_auth_id: body.device_auth_id,
        interval: body.interval.max(1),
        expires_in: CODEX_DEVICE_EXPIRES_IN_SECONDS,
    })
}

/// Polls Codex's device login flow once. Returns `pending` while the user has
/// not approved the code yet, or `authorized` with a CodeRouter credential JSON.
pub async fn poll_device_auth(
    client: &reqwest::Client,
    device_auth_id: &str,
    user_code: &str,
) -> Result<CodexDeviceAuthPoll, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!(
        "{}/api/accounts/deviceauth/token",
        CODEX_AUTH_ISSUER.trim_end_matches('/')
    );
    let resp = client
        .post(url)
        .header("User-Agent", codex_user_agent())
        .json(&serde_json::json!({
            "device_auth_id": device_auth_id,
            "user_code": user_code,
        }))
        .send()
        .await?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::NOT_FOUND
    {
        return Ok(CodexDeviceAuthPoll {
            status: "pending".to_string(),
            credential: None,
            message: Some("Waiting for ChatGPT approval".to_string()),
        });
    }

    if !resp.status().is_success() {
        return Err(format!("Codex device auth failed: HTTP {}", resp.status()).into());
    }

    let code: DeviceTokenResponse = resp.json().await?;
    let token = exchange_device_authorization_code(client, &code).await?;
    let credential = credential_from_device_tokens(&token);

    Ok(CodexDeviceAuthPoll {
        status: "authorized".to_string(),
        credential: Some(credential),
        message: Some("ChatGPT sign-in complete".to_string()),
    })
}

async fn exchange_device_authorization_code(
    client: &reqwest::Client,
    code: &DeviceTokenResponse,
) -> Result<AuthCodeTokenResponse, Box<dyn std::error::Error + Send + Sync>> {
    let token_endpoint = format!("{}/oauth/token", CODEX_AUTH_ISSUER.trim_end_matches('/'));
    let redirect_uri = format!(
        "{}/deviceauth/callback",
        CODEX_AUTH_ISSUER.trim_end_matches('/')
    );
    let resp = client
        .post(token_endpoint)
        .header("User-Agent", codex_user_agent())
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.authorization_code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", CODEX_CLIENT_ID),
            ("code_verifier", code.code_verifier.as_str()),
        ])
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(format!("Codex token exchange failed: HTTP {}", resp.status()).into());
    }

    Ok(resp.json().await?)
}

fn credential_from_device_tokens(token: &AuthCodeTokenResponse) -> String {
    let (account_id, _) = decode_id_token_claims(Some(&token.id_token));
    build_auth_json(
        &token.access_token,
        Some(&token.refresh_token),
        account_id.as_deref(),
        Some(&token.id_token),
    )
}

// ── JWT expiry & token refresh ──

/// Checks whether a JWT access token is expired (or will expire within `skew_secs`).
/// Returns true if the token does not appear to be a JWT (no `exp` claim),
/// because raw opaque tokens cannot be checked this way and should be tried.
fn token_needs_refresh(access_token: &str, skew_secs: i64) -> bool {
    let parts: Vec<&str> = access_token.split('.').collect();
    if parts.len() < 2 {
        // Not a JWT — cannot check expiry; assume it does not need refresh
        return false;
    }
    let payload = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(bytes) => bytes,
        Err(_) => return true,
    };
    let value: Value = match serde_json::from_slice(&payload) {
        Ok(v) => v,
        Err(_) => return true,
    };
    let exp = match value.get("exp").and_then(|v| v.as_i64()) {
        Some(e) => e,
        None => return false,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    now + skew_secs >= exp
}

/// Rebuilds a Codex auth JSON string from tokens, for persisting after refresh.
fn build_auth_json(
    access_token: &str,
    refresh_token: Option<&str>,
    account_id: Option<&str>,
    id_token: Option<&str>,
) -> String {
    let mut tokens_map = serde_json::Map::new();
    tokens_map.insert(
        "access_token".to_string(),
        Value::String(access_token.to_string()),
    );
    if let Some(rt) = refresh_token {
        tokens_map.insert("refresh_token".to_string(), Value::String(rt.to_string()));
    }
    if let Some(aid) = account_id {
        tokens_map.insert("account_id".to_string(), Value::String(aid.to_string()));
    }
    if let Some(idt) = id_token {
        tokens_map.insert("id_token".to_string(), Value::String(idt.to_string()));
    }
    let auth = serde_json::json!({ "tokens": tokens_map });
    auth.to_string()
}

/// Response from the OAuth token refresh endpoint.
#[derive(Deserialize)]
struct TokenRefreshResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

/// Attempts to refresh an expired access token using the stored refresh token.
///
/// Returns a new `(access_token, refresh_token, id_token)` tuple on success.
/// The new `id_token` from the refresh response may carry updated account claims.
pub async fn refresh_access_token(
    client: &reqwest::Client,
    refresh_token: &str,
) -> Result<(String, Option<String>, Option<String>), Box<dyn std::error::Error + Send + Sync>> {
    let resp = client
        .post("https://auth.openai.com/oauth/token")
        .header("User-Agent", codex_user_agent())
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CODEX_CLIENT_ID),
        ])
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(format!("token refresh failed: HTTP {}", resp.status()).into());
    }

    let body: TokenRefreshResponse = resp.json().await?;
    Ok((
        body.access_token,
        body.refresh_token.or(Some(refresh_token.to_string())),
        body.id_token,
    ))
}

/// Resolves the credential for a codex provider: parses, optionally refreshes
/// an expiring/expired access token, persists updated tokens, and returns
/// `(access_token, account_id, id_token)` ready for auth header building.
///
/// Raw access-token credentials (no refresh_token) skip refresh entirely.
pub async fn resolve_codex_credential(
    client: &reqwest::Client,
    raw_credential: &str,
    credential_key: &str,
) -> Result<(String, Option<String>, Option<String>), Box<dyn std::error::Error + Send + Sync>> {
    let (access_token, account_id, id_token, refresh_token) =
        parse_codex_credential(raw_credential);

    // If there's no refresh token, we can't refresh — use as-is
    let rt = match refresh_token {
        Some(ref rt) if !rt.is_empty() => rt.clone(),
        _ => return Ok((access_token, account_id, id_token)),
    };

    // Check expiry
    if !token_needs_refresh(&access_token, 60) {
        return Ok((access_token, account_id, id_token));
    }

    // Attempt refresh
    match refresh_access_token(client, &rt).await {
        Ok((new_access, new_refresh, new_id_token)) => {
            // Use the new id_token from refresh if provided; otherwise keep old
            let idt = new_id_token.or(id_token.clone());
            // Re-derive account_id from refreshed id_token
            let (acct_jwt, _) = decode_id_token_claims(idt.as_deref());
            let acct = account_id.or(acct_jwt);

            // Persist updated credential
            let updated_json = build_auth_json(
                &new_access,
                new_refresh.as_deref().or(Some(&rt)),
                acct.as_deref(),
                idt.as_deref(),
            );
            let _ =
                crate::credentials::keychain::store_credential(credential_key, &updated_json).await;

            Ok((new_access, acct, idt))
        }
        Err(_e) => {
            // Refresh failed — proceed with existing (possibly expired) token
            Ok((access_token, account_id, id_token))
        }
    }
}

// ── Request translation: OpenAI chat → Codex Responses ──

/// Converts an OpenAI chat-completion request body into a Codex Responses API request.
///
/// Mapping rules:
/// - `system` / `developer` messages → top-level `instructions`.
/// - `user` messages → `user` input items with `input_text` content.
/// - `assistant` text messages → `assistant` input items with `output_text` content.
/// - `tool` result messages → `function_call_output` items.
/// - Assistant `tool_calls` → `function_call` items.
/// - Chat `tools` → Responses function tools.
/// - Always forces `stream: true`.
pub fn openai_to_codex_request(openai_body: &Value, upstream_model: &str) -> Value {
    let request_id = new_codex_request_id();
    openai_to_codex_request_with_id(openai_body, upstream_model, &request_id)
}

pub fn openai_to_codex_request_with_id(
    openai_body: &Value,
    upstream_model: &str,
    request_id: &str,
) -> Value {
    let messages = openai_body
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut instructions_parts: Vec<String> = Vec::new();
    let mut input_items: Vec<Value> = Vec::new();

    for msg in &messages {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");

        match role {
            "system" | "developer" => {
                let content = extract_text_content(msg);
                if !content.is_empty() {
                    instructions_parts.push(content);
                }
            }
            "user" => {
                let item = serde_json::json!({
                    "role": "user",
                    "content": lower_user_content(msg)
                });
                input_items.push(item);
            }
            "assistant" => {
                let content = extract_text_content(msg);
                if !content.is_empty() {
                    let item = serde_json::json!({
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": content }]
                    });
                    input_items.push(item);
                }
                if let Some(tool_calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tool_calls {
                        let func = tc.get("function");
                        let name = func
                            .and_then(|f| f.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let arguments = func
                            .and_then(|f| f.get("arguments"))
                            .map(json_value_as_string)
                            .unwrap_or_else(|| "{}".to_string());
                        let call_id = tc
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let item = serde_json::json!({
                            "type": "function_call",
                            "name": name,
                            "arguments": arguments,
                            "call_id": call_id
                        });
                        input_items.push(item);
                    }
                }
            }
            "tool" => {
                let call_id = msg
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let output = extract_tool_content(msg);
                let item = serde_json::json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output
                });
                input_items.push(item);
            }
            _ => {}
        }
    }

    let mut body = serde_json::Map::new();
    body.insert(
        "model".to_string(),
        Value::String(upstream_model.to_string()),
    );
    body.insert("input".to_string(), Value::Array(input_items));
    body.insert("stream".to_string(), Value::Bool(true));
    body.insert("store".to_string(), Value::Bool(false));
    body.insert(
        "prompt_cache_key".to_string(),
        Value::String(request_id.to_string()),
    );

    if uses_openai_gpt5_defaults(upstream_model) {
        body.insert(
            "reasoning".to_string(),
            serde_json::json!({ "effort": "medium", "summary": "auto" }),
        );
        body.insert(
            "include".to_string(),
            serde_json::json!(["reasoning.encrypted_content"]),
        );

        if uses_openai_gpt5_text_verbosity(upstream_model) {
            body.insert(
                "text".to_string(),
                serde_json::json!({ "verbosity": "low" }),
            );
        }
    }

    if let Some(instructions) = openai_body
        .get("instructions")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        instructions_parts.insert(0, instructions.to_string());
    }
    let instructions = if instructions_parts.is_empty() {
        "You are a helpful assistant.".to_string()
    } else {
        instructions_parts.join("\n\n")
    };
    body.insert("instructions".to_string(), Value::String(instructions));

    // Pass through reasoning / reasoning_effort
    if let Some(reasoning) = openai_body.get("reasoning") {
        body.insert("reasoning".to_string(), reasoning.clone());
    } else if let Some(effort) = openai_body.get("reasoning_effort").and_then(|v| v.as_str()) {
        let mut reasoning = serde_json::Map::new();
        reasoning.insert("effort".to_string(), Value::String(effort.to_string()));
        if openai_body
            .get("reasoning_summary")
            .and_then(|v| v.as_str())
            == Some("auto")
            || uses_openai_gpt5_defaults(upstream_model)
        {
            reasoning.insert("summary".to_string(), Value::String("auto".to_string()));
        }
        body.insert("reasoning".to_string(), Value::Object(reasoning));
    }

    if let Some(include) = openai_body.get("include") {
        if include.as_array().is_some_and(|items| !items.is_empty()) {
            body.insert("include".to_string(), include.clone());
        } else {
            body.remove("include");
        }
    }

    if let Some(text) = openai_body.get("text").filter(|v| v.is_object()) {
        body.insert("text".to_string(), text.clone());
    }

    // Tools: chat → codex function tools
    if let Some(tools) = openai_body.get("tools").and_then(|v| v.as_array()) {
        let codex_tools: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                if t.get("type").and_then(|v| v.as_str()) == Some("function") {
                    let func = t.get("function")?;
                    let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if name.is_empty() {
                        return None;
                    }
                    let mut tool = serde_json::Map::new();
                    tool.insert("type".to_string(), Value::String("function".to_string()));
                    tool.insert("name".to_string(), Value::String(name.to_string()));
                    tool.insert(
                        "description".to_string(),
                        Value::String(
                            func.get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                        ),
                    );
                    tool.insert(
                        "parameters".to_string(),
                        func.get("parameters").cloned().unwrap_or_else(
                            || serde_json::json!({ "type": "object", "properties": {} }),
                        ),
                    );
                    if let Some(strict) = func.get("strict").and_then(|v| v.as_bool()) {
                        tool.insert("strict".to_string(), Value::Bool(strict));
                    }
                    Some(Value::Object(tool))
                } else {
                    None
                }
            })
            .collect();
        if !codex_tools.is_empty() {
            body.insert("tools".to_string(), Value::Array(codex_tools));
        }
    }

    if let Some(tool_choice) = lower_tool_choice(openai_body.get("tool_choice")) {
        body.insert("tool_choice".to_string(), tool_choice);
    }

    if supports_codex_sampling_params(upstream_model) {
        if let Some(temp) = openai_body.get("temperature") {
            body.insert("temperature".to_string(), temp.clone());
        }
        if let Some(top_p) = openai_body.get("top_p") {
            body.insert("top_p".to_string(), top_p.clone());
        }
    }
    if let Some(store) = openai_body.get("store").and_then(|v| v.as_bool()) {
        body.insert("store".to_string(), Value::Bool(store));
    }
    if let Some(prompt_cache_key) = openai_body.get("prompt_cache_key").and_then(|v| v.as_str()) {
        body.insert(
            "prompt_cache_key".to_string(),
            Value::String(prompt_cache_key.to_string()),
        );
    }

    Value::Object(body)
}

fn uses_openai_gpt5_defaults(model: &str) -> bool {
    let id = model.to_ascii_lowercase();
    id.contains("gpt-5") && !id.contains("gpt-5-chat") && !id.contains("gpt-5-pro")
}

fn uses_openai_gpt5_text_verbosity(model: &str) -> bool {
    let id = model.to_ascii_lowercase();
    id.contains("gpt-5.") && !id.contains("codex") && !id.contains("-chat")
}

fn supports_codex_sampling_params(model: &str) -> bool {
    !model.to_ascii_lowercase().contains("gpt-5.")
}

fn json_value_as_string(value: &Value) -> String {
    value
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()))
}

fn lower_user_content(msg: &Value) -> Vec<Value> {
    let Some(content) = msg.get("content") else {
        return vec![serde_json::json!({ "type": "input_text", "text": "" })];
    };

    if let Some(text) = content.as_str() {
        return vec![serde_json::json!({ "type": "input_text", "text": text })];
    }

    if let Some(parts) = content.as_array() {
        let lowered: Vec<Value> = parts
            .iter()
            .filter_map(|part| match part.get("type").and_then(|v| v.as_str()) {
                Some("text") | Some("input_text") => part
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|text| serde_json::json!({ "type": "input_text", "text": text })),
                Some("image_url") | Some("input_image") => extract_image_url(part)
                    .map(|url| serde_json::json!({ "type": "input_image", "image_url": url })),
                _ => None,
            })
            .collect();
        if !lowered.is_empty() {
            return lowered;
        }
    }

    vec![serde_json::json!({
        "type": "input_text",
        "text": extract_text_content(msg)
    })]
}

fn extract_image_url(part: &Value) -> Option<String> {
    let image = part.get("image_url")?;
    if let Some(url) = image.as_str() {
        return Some(url.to_string());
    }
    image
        .get("url")
        .and_then(|v| v.as_str())
        .map(|url| url.to_string())
}

fn lower_tool_choice(value: Option<&Value>) -> Option<Value> {
    let value = value?;
    if let Some(choice) = value.as_str() {
        return Some(Value::String(choice.to_string()));
    }

    if value.get("type").and_then(|v| v.as_str()) == Some("function") {
        let name = value
            .get("function")
            .and_then(|v| v.get("name"))
            .or_else(|| value.get("name"))
            .and_then(|v| v.as_str())?;
        return Some(serde_json::json!({ "type": "function", "name": name }));
    }

    None
}

fn extract_text_content(msg: &Value) -> String {
    if let Some(content) = msg.get("content") {
        if let Some(s) = content.as_str() {
            return s.to_string();
        }
        if let Some(arr) = content.as_array() {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            return parts.join("");
        }
    }
    String::new()
}

fn extract_tool_content(msg: &Value) -> String {
    if let Some(content) = msg.get("content") {
        if let Some(s) = content.as_str() {
            return s.to_string();
        }
        return serde_json::to_string(content).unwrap_or_default();
    }
    String::new()
}

// ── SSE translation: Codex Responses → OpenAI chat chunks ──

use bytes::Bytes;
use futures::Stream;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::proxy::translator::StreamTokenCounts;

/// Accumulated state for the Codex SSE translation process.
pub struct CodexToOpenAIStream<S> {
    inner: S,
    group_alias: String,
    chat_id: String,
    buffer: Vec<u8>,
    state: CodexStreamState,
    token_counts: Arc<Mutex<StreamTokenCounts>>,
    has_sent_role: bool,
    pending_event_type: Option<String>,
}

enum CodexStreamState {
    Waiting,
    Done,
    Errored(std::io::Error),
}

impl<S> CodexToOpenAIStream<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>>,
{
    pub fn new(inner: S, group_alias: String, token_counts: Arc<Mutex<StreamTokenCounts>>) -> Self {
        Self {
            inner,
            group_alias,
            chat_id: format!("chatcmpl-{}", uuid_short()),
            buffer: Vec::new(),
            state: CodexStreamState::Waiting,
            token_counts,
            has_sent_role: false,
            pending_event_type: None,
        }
    }

    fn translate_sse_data(&mut self, data: &str) -> Result<Option<String>, std::io::Error> {
        let owned;
        let data = if let Some(event_type) = self.pending_event_type.take() {
            owned = apply_sse_event_type(data, &event_type);
            owned.as_str()
        } else {
            data
        };
        self.translate_event(data)
    }

    /// Translates a single Codex SSE event.
    ///
    /// Returns `Ok(Some(sse_text))` for normal output, `Ok(None)` for
    /// skip-able events, and `Err(...)` on upstream errors (`response.failed`
    /// / `error`).
    fn translate_event(&mut self, data: &str) -> Result<Option<String>, std::io::Error> {
        let event: Value = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return Ok(None),
        };

        let mut event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if event_type.is_empty() && event.get("delta").and_then(|v| v.as_str()).is_some() {
            event_type = "response.output_text.delta";
        }

        match event_type {
            "response.output_text.delta" => {
                let delta_text = event.get("delta").and_then(|v| v.as_str()).unwrap_or("");

                let mut output = String::new();
                if !self.has_sent_role {
                    output.push_str(&build_openai_chunk(
                        &self.chat_id,
                        &self.group_alias,
                        Some("assistant"),
                        None,
                        None,
                    ));
                    self.has_sent_role = true;
                }

                if delta_text.is_empty() {
                    return if output.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(output))
                    };
                }

                output.push_str(&build_openai_chunk(
                    &self.chat_id,
                    &self.group_alias,
                    None,
                    Some(delta_text),
                    None,
                ));
                Ok(Some(output))
            }

            "response.output_item.done" => {
                let item = event.get("item");
                if let Some(func_name) = item.and_then(|i| i.get("name")).and_then(|v| v.as_str()) {
                    let call_id = item
                        .and_then(|i| i.get("call_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let arguments = item
                        .and_then(|i| i.get("arguments"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}")
                        .to_string();

                    let mut output = String::new();
                    if !self.has_sent_role {
                        output.push_str(&build_openai_chunk(
                            &self.chat_id,
                            &self.group_alias,
                            Some("assistant"),
                            None,
                            None,
                        ));
                        self.has_sent_role = true;
                    }

                    let tool_chunk = serde_json::json!({
                        "id": self.chat_id,
                        "object": "chat.completion.chunk",
                        "model": self.group_alias,
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "tool_calls": [{
                                    "index": 0,
                                    "id": call_id,
                                    "type": "function",
                                    "function": {
                                        "name": func_name,
                                        "arguments": arguments,
                                    }
                                }]
                            },
                            "finish_reason": null
                        }]
                    });
                    if let Ok(json) = serde_json::to_string(&tool_chunk) {
                        output.push_str(&format!("data: {json}\n\n"));
                    }
                    return Ok(Some(output));
                }
                Ok(None)
            }

            "response.completed" | "response.incomplete" => {
                // Usage is nested under `event.response.usage` in Codex Responses
                let usage = event
                    .get("response")
                    .and_then(|r| r.get("usage"))
                    .or_else(|| event.get("usage"));
                if let Some(usage) = usage {
                    let mut counts = self.token_counts.lock().unwrap();
                    if let Some(input) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                        counts.input_tokens = input;
                    }
                    if let Some(output) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                        counts.output_tokens = output;
                    }
                }

                let finish_reason = codex_finish_reason(&event);
                let mut output = build_openai_chunk(
                    &self.chat_id,
                    &self.group_alias,
                    None,
                    None,
                    Some(finish_reason),
                );
                output.push_str("data: [DONE]\n\n");
                self.state = CodexStreamState::Done;
                Ok(Some(output))
            }

            "response.failed" | "error" => {
                let detail = codex_error_message(&event, event_type);
                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("codex upstream error: {detail}"),
                ))
            }

            _ => Ok(None),
        }
    }
}

fn codex_finish_reason(event: &Value) -> &'static str {
    match event
        .get("response")
        .and_then(|r| r.get("incomplete_details"))
        .and_then(|d| d.get("reason"))
        .and_then(|v| v.as_str())
    {
        Some("max_output_tokens") => "length",
        Some("content_filter") => "content_filter",
        _ => "stop",
    }
}

fn codex_error_message(event: &Value, fallback: &str) -> String {
    let nested = event.get("response").and_then(|r| r.get("error"));
    let message = event
        .get("message")
        .or_else(|| nested.and_then(|e| e.get("message")))
        .or_else(|| event.get("response").and_then(|r| r.get("status_details")))
        .and_then(|v| v.as_str());
    let code = event
        .get("code")
        .or_else(|| nested.and_then(|e| e.get("code")))
        .and_then(|v| v.as_str());

    match (code, message) {
        (Some(code), Some(message)) if !code.is_empty() && !message.is_empty() => {
            format!("{code}: {message}")
        }
        (_, Some(message)) if !message.is_empty() => message.to_string(),
        (Some(code), _) if !code.is_empty() => code.to_string(),
        _ => fallback.to_string(),
    }
}

fn apply_sse_event_type(data: &str, event_type: &str) -> String {
    match serde_json::from_str::<Value>(data) {
        Ok(Value::Object(mut obj)) => {
            obj.entry("type".to_string())
                .or_insert_with(|| Value::String(event_type.to_string()));
            serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|_| data.to_string())
        }
        Ok(Value::String(text)) if event_type.contains("delta") => {
            serde_json::json!({ "type": event_type, "delta": text }).to_string()
        }
        Ok(value) => serde_json::json!({ "type": event_type, "data": value }).to_string(),
        Err(_) if event_type.contains("delta") => {
            serde_json::json!({ "type": event_type, "delta": data }).to_string()
        }
        Err(_) => serde_json::json!({ "type": event_type, "data": data }).to_string(),
    }
}

fn build_openai_chunk(
    chat_id: &str,
    model: &str,
    role: Option<&str>,
    content: Option<&str>,
    finish_reason: Option<&str>,
) -> String {
    let mut delta = serde_json::Map::new();
    if let Some(r) = role {
        delta.insert("role".to_string(), Value::String(r.to_string()));
    }
    if let Some(c) = content {
        delta.insert("content".to_string(), Value::String(c.to_string()));
    }

    let chunk = serde_json::json!({
        "id": chat_id,
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": finish_reason,
        }]
    });

    format!(
        "data: {}\n\n",
        serde_json::to_string(&chunk).unwrap_or_default()
    )
}

/// [`Stream`] implementation for Codex SSE to OpenAI SSE translation.
impl<S> Stream for CodexToOpenAIStream<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if matches!(self.state, CodexStreamState::Done) {
            return Poll::Ready(None);
        }
        if matches!(self.state, CodexStreamState::Errored(_)) {
            // Already errored; drain
            let CodexStreamState::Errored(e) =
                std::mem::replace(&mut self.state, CodexStreamState::Done)
            else {
                return Poll::Ready(None);
            };
            return Poll::Ready(Some(Err(e)));
        }

        loop {
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    self.buffer.extend_from_slice(&chunk);

                    if let Some(newline_pos) = self.buffer.iter().position(|&b| b == b'\n') {
                        let line_bytes = self.buffer[..newline_pos].to_vec();
                        self.buffer.drain(..=newline_pos);

                        let line = String::from_utf8_lossy(&line_bytes);
                        let line = line.strip_suffix('\r').unwrap_or(&line).trim();

                        if let Some(event_type) = line
                            .strip_prefix("event: ")
                            .or_else(|| line.strip_prefix("event:"))
                        {
                            self.pending_event_type = Some(event_type.trim().to_string());
                            continue;
                        }

                        let data = if line.starts_with("data: ") {
                            Some(&line[6..])
                        } else if line.starts_with("data:") {
                            Some(&line[5..])
                        } else {
                            None
                        };

                        if let Some(data) = data {
                            if !data.is_empty() {
                                match self.translate_sse_data(data) {
                                    Ok(Some(output)) => {
                                        return Poll::Ready(Some(Ok(Bytes::from(output))));
                                    }
                                    Ok(None) => continue,
                                    Err(e) => {
                                        self.state = CodexStreamState::Errored(
                                            std::io::Error::new(e.kind(), e.to_string()),
                                        );
                                        return Poll::Ready(Some(Err(e)));
                                    }
                                }
                            }
                        }
                    }
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                Poll::Ready(None) => {
                    if !self.buffer.is_empty() {
                        let remaining = String::from_utf8_lossy(&self.buffer);
                        let remaining = remaining.to_string();
                        self.buffer.clear();
                        let mut output = String::new();

                        for line in remaining.lines() {
                            let line = line.strip_suffix('\r').unwrap_or(line).trim();
                            if let Some(event_type) = line
                                .strip_prefix("event: ")
                                .or_else(|| line.strip_prefix("event:"))
                            {
                                self.pending_event_type = Some(event_type.trim().to_string());
                                continue;
                            }

                            let data = if let Some(data) = line.strip_prefix("data: ") {
                                Some(data)
                            } else {
                                line.strip_prefix("data:")
                            };

                            if let Some(data) = data.filter(|data| !data.is_empty()) {
                                match self.translate_sse_data(data) {
                                    Ok(Some(chunk)) => output.push_str(&chunk),
                                    Ok(None) => {}
                                    Err(e) => {
                                        self.state = CodexStreamState::Errored(
                                            std::io::Error::new(e.kind(), e.to_string()),
                                        );
                                        return Poll::Ready(Some(Err(e)));
                                    }
                                }
                            }
                        }

                        if !output.is_empty() {
                            return Poll::Ready(Some(Ok(Bytes::from(output))));
                        }
                    }
                    self.state = CodexStreamState::Done;
                    let mut output = String::new();
                    if !self.has_sent_role {
                        output.push_str(&build_openai_chunk(
                            &self.chat_id,
                            &self.group_alias,
                            Some("assistant"),
                            None,
                            None,
                        ));
                        self.has_sent_role = true;
                    }
                    output.push_str(&build_openai_chunk(
                        &self.chat_id,
                        &self.group_alias,
                        None,
                        None,
                        Some("stop"),
                    ));
                    output.push_str("data: [DONE]\n\n");
                    return Poll::Ready(Some(Ok(Bytes::from(output))));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Wraps a Codex SSE stream into an [`axum::body::Body`] that yields
/// translated OpenAI SSE chunks.
pub fn translate_codex_stream<S>(
    stream: S,
    group_alias: String,
    token_counts: Arc<Mutex<StreamTokenCounts>>,
) -> (axum::body::Body, Arc<Mutex<StreamTokenCounts>>)
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send + Unpin + 'static,
{
    let translated = CodexToOpenAIStream::new(stream, group_alias, token_counts.clone());
    let raw_stream = futures::stream::unfold(translated, |mut stream| async move {
        match futures::StreamExt::next(&mut stream).await {
            Some(Ok(bytes)) => Some((Ok::<_, std::io::Error>(bytes), stream)),
            Some(Err(e)) => Some((Err(e), stream)),
            None => None,
        }
    });
    let body = axum::body::Body::from_stream(raw_stream);
    (body, token_counts)
}

/// Generates a short UUID for use in OpenAI-compatible response IDs.
fn uuid_short() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_codex_credential tests ──

    #[test]
    fn test_parse_raw_access_token() {
        let (access, account, id, refresh) = parse_codex_credential("sk-my-access-token");
        assert_eq!(access, "sk-my-access-token");
        assert!(account.is_none());
        assert!(id.is_none());
        assert!(refresh.is_none());
    }

    #[test]
    fn test_parse_full_auth_json_return_order() {
        let json = serde_json::json!({
            "tokens": {
                "access_token": "at-123",
                "refresh_token": "rt-456",
                "account_id": "acc-789",
                "id_token": "eyJhbGciOiJSUzI1NiJ9.eyJmZWRyYW1wIjp0cnVlfQ.signature"
            }
        });
        let (access, account, id, refresh) = parse_codex_credential(&json.to_string());
        assert_eq!(access, "at-123");
        assert_eq!(account, Some("acc-789".to_string()));
        assert_eq!(
            id,
            Some("eyJhbGciOiJSUzI1NiJ9.eyJmZWRyYW1wIjp0cnVlfQ.signature".to_string())
        );
        assert_eq!(refresh, Some("rt-456".to_string()));
    }

    #[test]
    fn test_parse_flat_auth_json_return_order() {
        let json = serde_json::json!({
            "access_token": "direct-at",
            "refresh_token": "direct-rt",
            "account_id": "direct-acc"
        });
        let (access, account, id, refresh) = parse_codex_credential(&json.to_string());
        assert_eq!(access, "direct-at");
        assert_eq!(account, Some("direct-acc".to_string()));
        assert!(id.is_none());
        assert_eq!(refresh, Some("direct-rt".to_string()));
    }

    #[test]
    fn test_parse_json_with_jwt_namespace_account_id() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct-jwt-123","chatgpt_account_is_fedramp":false}}"#);
        let id_token = format!("h.{payload}.s");
        let json = serde_json::json!({
            "tokens": {
                "access_token": "at-1",
                "refresh_token": "rt-1",
                "id_token": id_token
            }
        });
        let (access, account, id, refresh) = parse_codex_credential(&json.to_string());
        assert_eq!(access, "at-1");
        assert_eq!(account, Some("acct-jwt-123".to_string()));
        assert!(id.is_some());
        assert_eq!(refresh, Some("rt-1".to_string()));
    }

    #[test]
    fn test_parse_json_with_top_level_jwt_account_id() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"chatgpt_account_id":"acct-top"}"#);
        let id_token = format!("h.{payload}.s");
        let json = serde_json::json!({
            "tokens": {
                "access_token": "at-top",
                "id_token": id_token
            }
        });

        let (_, account, _, _) = parse_codex_credential(&json.to_string());
        assert_eq!(account, Some("acct-top".to_string()));
    }

    #[test]
    fn test_parse_json_with_organization_account_id() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"organizations":[{"id":"org-account"}]}"#);
        let id_token = format!("h.{payload}.s");
        let json = serde_json::json!({
            "tokens": {
                "access_token": "at-org",
                "id_token": id_token
            }
        });

        let (_, account, _, _) = parse_codex_credential(&json.to_string());
        assert_eq!(account, Some("org-account".to_string()));
    }

    #[test]
    fn test_parse_json_account_id_from_both_sources() {
        // auth.json top-level account_id takes precedence over JWT claim
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct-from-jwt"}}"#);
        let id_token = format!("h.{payload}.s");
        let json = serde_json::json!({
            "tokens": {
                "access_token": "at-x",
                "account_id": "acct-from-json",
                "id_token": id_token
            }
        });
        let (_, account, _, _) = parse_codex_credential(&json.to_string());
        assert_eq!(account, Some("acct-from-json".to_string()));
    }

    // ── FedRAMP detection tests ──

    #[test]
    fn test_fedramp_top_level_simple() {
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"fedramp":true}"#);
        let id_token = format!("header.{payload}.sig");
        assert!(check_fedramp(&id_token));

        let payload2 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"fedramp":false}"#);
        let id_token2 = format!("header.{payload2}.sig");
        assert!(!check_fedramp(&id_token2));
    }

    #[test]
    fn test_fedramp_openai_namespace() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"https://api.openai.com/auth":{"chatgpt_account_is_fedramp":true}}"#);
        let id_token = format!("h.{payload}.s");
        assert!(check_fedramp(&id_token));

        let payload2 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"https://api.openai.com/auth":{"chatgpt_account_is_fedramp":false}}"#);
        let id_token2 = format!("h.{payload2}.s");
        assert!(!check_fedramp(&id_token2));
    }

    #[test]
    fn test_fedramp_openai_namespace_wins_over_simple() {
        // FedRAMP true in namespace overrides false at top level
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"fedramp":false,"https://api.openai.com/auth":{"chatgpt_account_is_fedramp":true}}"#);
        let id_token = format!("h.{payload}.s");
        assert!(check_fedramp(&id_token));
    }

    #[test]
    fn test_fedramp_no_claims() {
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"sub":"user-1"}"#);
        let id_token = format!("h.{payload}.s");
        assert!(!check_fedramp(&id_token));
    }

    // ── build_codex_auth_headers tests ──

    #[test]
    fn test_build_codex_auth_headers_basic() {
        let headers = build_codex_auth_headers("access-token", Some("account-id"), None);
        assert!(headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer access-token"));
        assert!(headers
            .iter()
            .any(|(k, v)| k == "ChatGPT-Account-Id" && v == "account-id"));
        assert!(headers
            .iter()
            .any(|(k, v)| k == "content-type" && v == "application/json"));
        assert!(headers
            .iter()
            .any(|(k, v)| k == "originator" && v == CODEX_ORIGINATOR));
        assert!(headers
            .iter()
            .any(|(k, v)| k == "User-Agent" && v.starts_with("opencode/")));
        assert!(!headers.iter().any(|(k, _)| k == "X-OpenAI-Fedramp"));
    }

    #[test]
    fn test_build_codex_responses_headers() {
        let headers = build_codex_responses_headers("request-123");
        assert!(headers
            .iter()
            .any(|(k, v)| k == "session-id" && v == "request-123"));
        assert_eq!(headers.len(), 1);
    }

    #[test]
    fn test_build_codex_auth_headers_fedramp_namespace() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"https://api.openai.com/auth":{"chatgpt_account_is_fedramp":true}}"#);
        let id_token = format!("h.{payload}.s");
        let headers = build_codex_auth_headers("at", None, Some(&id_token));
        assert!(headers
            .iter()
            .any(|(k, v)| k == "X-OpenAI-Fedramp" && v == "true"));
    }

    #[test]
    fn test_build_codex_auth_headers_no_account() {
        let headers = build_codex_auth_headers("at", None, None);
        assert!(headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer at"));
        assert!(!headers.iter().any(|(k, _)| k == "ChatGPT-Account-Id"));
    }

    // ── JWT expiry tests ──

    #[test]
    fn test_token_needs_refresh_expired() {
        // Token with exp in the past (epoch 1)
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"exp":1}"#);
        let token = format!("h.{payload}.s");
        assert!(token_needs_refresh(&token, 60));
    }

    #[test]
    fn test_token_needs_refresh_far_future() {
        // Token with exp far in the future
        let far_future = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64)
            + 86400;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(format!(r#"{{"exp":{}}}"#, far_future));
        let token = format!("h.{payload}.s");
        assert!(!token_needs_refresh(&token, 60));
    }

    #[test]
    fn test_token_needs_refresh_within_skew() {
        // Token expiring in 30s, skew is 60s — should be "expired"
        let near_future = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64)
            + 30;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(format!(r#"{{"exp":{}}}"#, near_future));
        let token = format!("h.{payload}.s");
        assert!(token_needs_refresh(&token, 60));
    }

    #[test]
    fn test_token_needs_refresh_opaque() {
        // Non-JWT tokens can't be checked; assume they don't need refresh
        assert!(!token_needs_refresh("sk-opaque-token", 60));
    }

    // ── build_auth_json tests ──

    #[test]
    fn test_build_auth_json_all_fields() {
        let json = build_auth_json("at", Some("rt"), Some("acct"), Some("idt"));
        let parsed: CodexAuth = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tokens.access_token, "at");
        assert_eq!(parsed.tokens.refresh_token, Some("rt".to_string()));
        assert_eq!(parsed.tokens.account_id, Some("acct".to_string()));
        assert_eq!(parsed.tokens.id_token, Some("idt".to_string()));
    }

    #[test]
    fn test_build_auth_json_minimal() {
        let json = build_auth_json("bare-token", None, None, None);
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["tokens"]["access_token"], "bare-token");
        assert!(parsed["tokens"].get("refresh_token").is_none());
    }

    #[test]
    fn test_device_user_code_interval_accepts_string() {
        let json = r#"{
            "device_auth_id": "dev-123",
            "user_code": "ABCD-EFGH",
            "interval": "7"
        }"#;
        let parsed: DeviceUserCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.device_auth_id, "dev-123");
        assert_eq!(parsed.user_code, "ABCD-EFGH");
        assert_eq!(parsed.interval, 7);
    }

    #[test]
    fn test_credential_from_device_tokens_includes_account_id() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct-device"}}"#);
        let token = AuthCodeTokenResponse {
            access_token: "device-access".to_string(),
            refresh_token: "device-refresh".to_string(),
            id_token: format!("h.{payload}.s"),
        };

        let credential = credential_from_device_tokens(&token);
        let parsed: CodexAuth = serde_json::from_str(&credential).unwrap();

        assert_eq!(parsed.tokens.access_token, "device-access");
        assert_eq!(
            parsed.tokens.refresh_token,
            Some("device-refresh".to_string())
        );
        assert_eq!(parsed.tokens.account_id, Some("acct-device".to_string()));
        assert!(parsed.tokens.id_token.is_some());
    }

    // ── Request translation tests ──

    #[test]
    fn test_openai_to_codex_request_basic() {
        let openai_req = serde_json::json!({
            "model": "gpt-5-codex",
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello"}
            ],
            "max_tokens": 100,
            "temperature": 0.7
        });

        let result = openai_to_codex_request_with_id(&openai_req, "gpt-5-codex", "request-123");

        assert_eq!(result["model"], "gpt-5-codex");
        assert_eq!(result["stream"], true);
        assert_eq!(result["store"], false);
        assert_eq!(result["include"][0], "reasoning.encrypted_content");
        assert_eq!(result["reasoning"]["effort"], "medium");
        assert_eq!(result["reasoning"]["summary"], "auto");
        assert!(result.get("tools").is_none());
        assert!(result.get("tool_choice").is_none());
        assert!(result.get("parallel_tool_calls").is_none());
        assert_eq!(result["prompt_cache_key"], "request-123");
        assert!(result.get("client_metadata").is_none());
        assert_eq!(result["instructions"], "You are helpful.");
        assert_eq!(result["input"].as_array().unwrap().len(), 1);
        assert_eq!(result["input"][0]["role"], "user");
        assert_eq!(result["input"][0]["content"][0]["text"], "Hello");
        assert!(result.get("max_output_tokens").is_none());
        assert_eq!(result["temperature"], 0.7);
    }

    #[test]
    fn test_openai_to_codex_request_drops_sampling_for_gpt_5_dot_models() {
        let openai_req = serde_json::json!({
            "messages": [{"role": "user", "content": "Hello"}],
            "temperature": 0.2,
            "top_p": 0.9
        });

        let result = openai_to_codex_request(&openai_req, "gpt-5.5");

        assert!(result.get("temperature").is_none());
        assert!(result.get("top_p").is_none());
    }

    #[test]
    fn test_openai_to_codex_request_defaults_instructions() {
        let openai_req = serde_json::json!({
            "messages": [{"role": "user", "content": "test"}]
        });

        let result = openai_to_codex_request(&openai_req, "model");
        assert_eq!(result["instructions"], "You are a helpful assistant.");
    }

    #[test]
    fn test_openai_to_codex_request_tools() {
        let openai_req = serde_json::json!({
            "messages": [{"role": "user", "content": "test"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": {"type": "object", "properties": {}}
                }
            }]
        });

        let result = openai_to_codex_request(&openai_req, "model");
        assert!(result.get("parallel_tool_calls").is_none());
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "get_weather");
    }

    #[test]
    fn test_openai_to_codex_request_reasoning() {
        let openai_req = serde_json::json!({
            "messages": [{"role": "user", "content": "test"}],
            "reasoning_effort": "high"
        });

        let result = openai_to_codex_request(&openai_req, "model");
        assert_eq!(result["reasoning"]["effort"], "high");
    }

    #[test]
    fn test_openai_to_codex_request_existing_reasoning_object() {
        let openai_req = serde_json::json!({
            "messages": [{"role": "user", "content": "test"}],
            "reasoning": {"effort": "medium", "summary": "auto"}
        });

        let result = openai_to_codex_request(&openai_req, "model");
        assert_eq!(result["reasoning"]["effort"], "medium");
        assert_eq!(result["reasoning"]["summary"], "auto");
    }

    // ── SSE stream translation tests ──

    fn make_streamer() -> CodexToOpenAIStream<futures::stream::Empty<Result<Bytes, std::io::Error>>>
    {
        let counts = Arc::new(Mutex::new(StreamTokenCounts {
            input_tokens: 0,
            output_tokens: 0,
        }));
        let mut streamer =
            CodexToOpenAIStream::new(futures::stream::empty(), "test-group".to_string(), counts);
        streamer.chat_id = "chatcmpl-test".to_string();
        streamer
    }

    #[test]
    fn test_codex_stream_translation_delta() {
        let mut streamer = make_streamer();
        let data = r#"{"type":"response.output_text.delta","delta":"Hello"}"#;
        let result = streamer.translate_event(data).unwrap().unwrap();
        assert!(result.contains("chat.completion.chunk"));
        assert!(result.contains("Hello"));
        assert!(result.contains("assistant")); // role chunk sent first
    }

    #[test]
    fn test_codex_stream_translation_delta_without_type() {
        let mut streamer = make_streamer();
        let data = r#"{"delta":"Hello"}"#;
        let result = streamer.translate_event(data).unwrap().unwrap();
        assert!(result.contains("chat.completion.chunk"));
        assert!(result.contains("Hello"));
    }

    #[test]
    fn test_apply_sse_event_type_adds_missing_type() {
        let typed = apply_sse_event_type(r#"{"delta":"Hello"}"#, "response.output_text.delta");
        let value: Value = serde_json::from_str(&typed).unwrap();
        assert_eq!(value["type"], "response.output_text.delta");
        assert_eq!(value["delta"], "Hello");
    }

    #[test]
    fn test_codex_stream_translation_completed_nested_usage() {
        // Usage under `event.response.usage` — the real Codex shape
        let mut streamer = make_streamer();
        let data = r#"{"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let result = streamer.translate_event(data).unwrap().unwrap();
        assert!(result.contains("[DONE]"));
        assert!(result.contains("\"finish_reason\":\"stop\""));
        let counts = streamer.token_counts.lock().unwrap();
        assert_eq!(counts.input_tokens, 10);
        assert_eq!(counts.output_tokens, 5);
    }

    #[test]
    fn test_codex_stream_translation_completed_top_level_usage() {
        // Backward compat: usage at top-level `event.usage`
        let mut streamer = make_streamer();
        let data = r#"{"type":"response.completed","usage":{"input_tokens":7,"output_tokens":3}}"#;
        let result = streamer.translate_event(data).unwrap().unwrap();
        assert!(result.contains("[DONE]"));
        let counts = streamer.token_counts.lock().unwrap();
        assert_eq!(counts.input_tokens, 7);
        assert_eq!(counts.output_tokens, 3);
    }

    #[test]
    fn test_codex_stream_translation_failed_is_error() {
        let mut streamer = make_streamer();
        let data =
            r#"{"type":"response.failed","response":{"status_details":"rate_limit_exceeded"}}"#;
        let result = streamer.translate_event(data);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("rate_limit_exceeded"));
    }

    #[test]
    fn test_codex_stream_translation_incomplete_finishes_with_length() {
        let mut streamer = make_streamer();
        let data = r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}"#;
        let result = streamer.translate_event(data).unwrap().unwrap();
        assert!(result.contains("[DONE]"));
        assert!(result.contains("\"finish_reason\":\"length\""));
    }
}
