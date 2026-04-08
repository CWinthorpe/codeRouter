use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use axum::{
    extract::State,
    http::{header::CONTENT_TYPE, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use axum::response::Response;
use futures::StreamExt;
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use tokio::net::TcpListener;

use crate::config::{
    models::{AppConfig, Group},
    store::{load_app_config, load_groups, load_providers},
};
use crate::credentials::keychain::get_credential;
use crate::metrics::db as metrics_db;
use crate::metrics::recorder::{MetricsRecorder, RequestEvent};
use crate::metrics::scheduler::spawn_scheduler;
use crate::metrics::queries::get_latency_percentiles;
use crate::models::refresher::refresh_all_providers;
use crate::proxy::router::{
    self, SharedRouterState,
};
use crate::proxy::ssrf;
use crate::proxy::translator;
use crate::proxy::upstream::{self, UpstreamError};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub groups: Arc<Vec<Group>>,
    pub client: Client,
    pub router_state: SharedRouterState,
    pub metrics_recorder: Arc<MetricsRecorder>,
}

static START_TIME: AtomicI64 = AtomicI64::new(0);

pub async fn start_server() -> anyhow::Result<()> {
    let start_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    START_TIME.store(start_epoch, Ordering::Relaxed);

    let config = load_app_config().unwrap_or_default();
    let groups = load_groups().unwrap_or_default();
    let providers = load_providers().unwrap_or_default();

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let router_state = router::init_and_set_global_router_state(&groups, &providers);

    router::init_daily_totals_from_db(&router_state);

    let conn = metrics_db::init_db().expect("Failed to initialize metrics database");
    let (metrics_recorder, _metrics_handle) = MetricsRecorder::new(conn);
    let metrics_recorder = Arc::new(metrics_recorder);

    let state = AppState {
        config: Arc::new(config),
        groups: Arc::new(groups),
        client,
        router_state,
        metrics_recorder: metrics_recorder.clone(),
    };

    let scheduler_groups = state.groups.clone();
    let scheduler_client = state.client.clone();
    let scheduler_state = state.router_state.clone();
    let _scheduler_handle = spawn_scheduler(scheduler_state, scheduler_groups, scheduler_client);

    let refresh_client = state.client.clone();
    tokio::spawn(async move {
        refresh_all_providers(&refresh_client).await;
    });

    let host = state.config.proxy_host.clone();
    let port = state.config.proxy_port;
    let addr = format!("{host}:{port}");
    let state = Arc::new(state);
    let app = Router::new()
        .route("/v1/models", get(handle_models))
        .route("/v1/chat/completions", post(handle_chat_completions))
        .route("/v1/completions", post(handle_completions))
        .route("/health", get(handle_health))
        .with_state(state.clone());

    let listener = TcpListener::bind(&addr).await?;

    eprintln!("CodeRouter proxy listening on {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Serialize)]
struct ModelResponse {
    object: String,
    data: Vec<ModelObject>,
}

#[derive(Serialize)]
struct ModelObject {
    id: String,
    object: String,
    created: u64,
    owned_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
}

async fn handle_models(State(state): State<Arc<AppState>>) -> Json<ModelResponse> {
    let providers = load_providers().unwrap_or_default();
    let router_state = state.router_state.lock().unwrap();
    let data = state
        .groups
        .iter()
        .filter(|g| {
            g.entries.iter().enumerate().any(|(idx, e)| {
                if !e.enabled {
                    return false;
                }
                let key = format!("{}:{}", e.provider_id, idx);
                if let Some(entry_state) = router_state.entries.get(&key) {
                    entry_state.status == router::EntryStatus::Active
                } else {
                    true
                }
            })
        })
        .map(|g| {
            let highest_active_entry = g
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.enabled)
                .min_by_key(|(_, e)| e.priority);

            let (context_window, max_output_tokens) = if let Some((idx, entry)) = highest_active_entry {
                let key = format!("{}:{}", entry.provider_id, idx);
                let is_active = router_state.entries.get(&key)
                    .map(|es| es.status == router::EntryStatus::Active)
                    .unwrap_or(true);

                if is_active {
                    if let Some(provider) = providers.iter().find(|p| p.id == entry.provider_id) {
                        if let Some(model_meta) = provider.models.iter().find(|m| m.id == entry.model_id) {
                            (model_meta.context_window, model_meta.max_output_tokens)
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

            ModelObject {
                id: g.alias.clone(),
                object: "model".to_string(),
                created: 0,
                owned_by: "coderouter".to_string(),
                context_window,
                max_output_tokens,
            }
        })
        .collect();

    Json(ModelResponse {
        object: "list".to_string(),
        data,
    })
}

async fn handle_chat_completions(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Response, AppError> {
    route_request(&state, body, "chat/completions").await
}

async fn handle_completions(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Response, AppError> {
    route_request(&state, body, "completions").await
}

async fn route_request(
    state: &AppState,
    body: Value,
    endpoint: &str,
) -> Result<Response, AppError> {
    let start = Instant::now();

    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("missing model field".into()))?
        .to_string();

    let stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    let group = state
        .groups
        .iter()
        .find(|g| g.alias == model)
        .ok_or_else(|| AppError::NotFound(format!("no group found for model '{model}'")))?;

    let providers = load_providers().map_err(|e| {
        eprintln!("failed to load providers: {e}");
        AppError::InternalError(e.to_string())
    })?;

    let max_retries = group.entries.len();
    let mut skip_indices = HashSet::new();

    for _attempt in 0..max_retries {
        let (entry, entry_index) = {
            let router_state = state.router_state.lock().unwrap();
            match router::select_entry(group, &router_state, &providers, &skip_indices) {
                Some(result) => result,
                None => break,
            }
        };

        let provider = providers
            .iter()
            .find(|p| p.id == entry.provider_id)
            .ok_or_else(|| AppError::InternalError(format!("provider '{}' not found", entry.provider_id)))?
            .clone();

        let api_key = get_credential(&provider.credential_key).await.map_err(|e| {
            eprintln!("failed to get credential for '{}': {e}", provider.credential_key);
            AppError::InternalError(e.to_string())
        })?;

        let is_anthropic = provider.protocol == "anthropic";
        let upstream_model = entry.model_id.clone();

        ssrf::validate_base_url(&provider.base_url).map_err(|e| {
            eprintln!("SSRF validation failed for provider {}: {}", provider.id, e);
            AppError::InternalError(format!("invalid provider base_url: {}", e))
        })?;

        let url = if is_anthropic {
            format!("{}/v1/messages", provider.base_url.trim_end_matches('/'))
        } else if endpoint == "completions" {
            format!("{}/v1/completions", provider.base_url.trim_end_matches('/'))
        } else {
            format!("{}/v1/chat/completions", provider.base_url.trim_end_matches('/'))
        };

        let timeout_ms = group.failover_config.latency_timeout_ms;
        let req = if endpoint == "completions" {
            upstream::build_completion_request(&state.client, &body, &api_key, &upstream_model, &url, is_anthropic)
        } else {
            upstream::build_upstream_request(&state.client, &body, &api_key, &upstream_model, &url, is_anthropic)
        };

        let result = match upstream::send_with_timeout(req, timeout_ms, group.failover_config.on_latency_timeout).await {
            Ok(resp) => process_response(resp, stream, &group.alias, is_anthropic).await,
            Err(UpstreamError::Timeout) => {
                eprintln!("request timed out for provider {}", provider.id);
                if group.failover_config.on_latency_timeout {
                    let mut rs = state.router_state.lock().unwrap();
                    let _ = router::record_latency_timeout(&mut rs, &provider.id, entry_index);
                }
                skip_indices.insert(entry_index);
                continue;
            }
            Err(UpstreamError::Network(e)) => {
                eprintln!("upstream request error: {e}");
                Err(RequestError::Network(e))
            }
        };

        match result {
            Ok((resp, prompt_tokens, output_tokens)) => {
                let latency_ms = start.elapsed().as_millis() as i64;
                let tokens_used = prompt_tokens + output_tokens;
                {
                    let mut rs = state.router_state.lock().unwrap();
                    router::record_success(&mut rs, &provider.id, entry_index, tokens_used);
                }
                let model_id = entry.model_id.clone();
                let provider_id = provider.id.clone();
                let group_alias = group.alias.clone();
                let metrics_recorder = state.metrics_recorder.clone();
                tokio::spawn(async move {
                    let event = RequestEvent {
                        ts: chrono::Utc::now().timestamp(),
                        group_alias,
                        provider_id,
                        model_id,
                        prompt_tokens: prompt_tokens as i64,
                        output_tokens: output_tokens as i64,
                        latency_ms,
                        status: "success".to_string(),
                        error_type: None,
                        input_cost_per_1m: None,
                        output_cost_per_1m: None,
                    };
                    let _ = metrics_recorder.record_request(event).await;
                });
                return Ok(resp);
            }
            Err(RequestError::RateLimited) => {
                if group.failover_config.on_429 {
                    let mut rs = state.router_state.lock().unwrap();
                    router::record_429(&mut rs, &provider.id, entry_index, 60);
                }
                skip_indices.insert(entry_index);
                continue;
            }
            Err(RequestError::QuotaExhausted) => {
                if group.failover_config.on_quota_exhausted {
                    let mut rs = state.router_state.lock().unwrap();
                    router::record_quota_exhausted(&mut rs, &provider.id, entry_index);
                }
                skip_indices.insert(entry_index);
                continue;
            }
            Err(RequestError::Network(_msg)) => {
                if group.failover_config.on_consecutive_errors {
                    let mut rs = state.router_state.lock().unwrap();
                    router::record_consecutive_error(
                        &mut rs,
                        &provider.id,
                        entry_index,
                        group.failover_config.consecutive_error_threshold,
                        true,
                    );
                }
                skip_indices.insert(entry_index);
                continue;
            }
            Err(RequestError::ServerError(_msg)) => {
                if group.failover_config.on_consecutive_errors {
                    let mut rs = state.router_state.lock().unwrap();
                    router::record_consecutive_error(
                        &mut rs,
                        &provider.id,
                        entry_index,
                        group.failover_config.consecutive_error_threshold,
                        true,
                    );
                }
                skip_indices.insert(entry_index);
                continue;
            }
        }
    }

    let exhausted_body = router::build_exhausted_response(&model);
    Ok((StatusCode::SERVICE_UNAVAILABLE, Json(exhausted_body)).into_response())
}

#[allow(dead_code)]
enum RequestError {
    RateLimited,
    QuotaExhausted,
    Network(String),
    ServerError(String),
}

async fn process_response(
    resp: reqwest::Response,
    stream: bool,
    group_alias: &str,
    is_anthropic: bool,
) -> Result<(Response, u64, u64), RequestError> {
    let status = resp.status();

    if status.as_u16() == 429 {
        return Err(RequestError::RateLimited);
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(RequestError::ServerError(format!("HTTP {}: {}", status, body)));
    }

    if stream {
        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            if is_anthropic {
                let stream = resp.bytes_stream().map(|result| {
                    result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
                });
                return Ok((translator::translate_anthropic_stream(stream, group_alias.to_string()).into_response(), 0, 0));
            } else {
                let raw_stream = resp.bytes_stream().map(|result| {
                    result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
                });
                let body = axum::body::Body::from_stream(raw_stream);
                let mut resp = Response::new(body);
                resp.headers_mut().insert(
                    CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("text/event-stream"),
                );
                return Ok((resp, 0, 0));
            }
        } else {
            let bytes = resp.bytes().await.map_err(|e| {
                eprintln!("failed to read response body: {e}");
                RequestError::Network(e.to_string())
            })?;

            if is_anthropic {
                let anthropic_resp: translator::MessagesResponse =
                    serde_json::from_slice(&bytes).map_err(|e| {
                        eprintln!("failed to parse anthropic response: {e}");
                        RequestError::Network(e.to_string())
                    })?;
                let (prompt_tokens, output_tokens) = extract_anthropic_tokens(&anthropic_resp);
                let openai_resp = translator::anthropic_to_openai(&anthropic_resp, group_alias);
                return Ok((Json(openai_resp).into_response(), prompt_tokens, output_tokens));
            } else {
                let mut json: Value = serde_json::from_slice(&bytes).map_err(|e| {
                    eprintln!("failed to parse upstream JSON response: {e}");
                    RequestError::ServerError(format!("invalid upstream JSON: {e}"))
                })?;
                let (prompt_tokens, output_tokens) = extract_openai_tokens(&json);
                if let Some(obj) = json.as_object_mut() {
                    obj.insert("model".to_string(), Value::String(group_alias.to_string()));
                }
                let mut resp = Json(json).into_response();
                *resp.status_mut() = status;
                return Ok((resp, prompt_tokens, output_tokens));
            }
        }
    } else {
        let bytes = resp.bytes().await.map_err(|e| {
            eprintln!("failed to read response body: {e}");
            RequestError::Network(e.to_string())
        })?;

        if is_anthropic {
            let anthropic_resp: translator::MessagesResponse =
                serde_json::from_slice(&bytes).map_err(|e| {
                    eprintln!("failed to parse anthropic response: {e}");
                    RequestError::Network(e.to_string())
                })?;
            let (prompt_tokens, output_tokens) = extract_anthropic_tokens(&anthropic_resp);
            let openai_resp = translator::anthropic_to_openai(&anthropic_resp, group_alias);
            return Ok((Json(openai_resp).into_response(), prompt_tokens, output_tokens));
        } else {
            let mut json: Value = serde_json::from_slice(&bytes).map_err(|e| {
                eprintln!("failed to parse upstream JSON response: {e}");
                RequestError::ServerError(format!("invalid upstream JSON: {e}"))
            })?;
            let (prompt_tokens, output_tokens) = extract_openai_tokens(&json);
            if let Some(obj) = json.as_object_mut() {
                obj.insert("model".to_string(), Value::String(group_alias.to_string()));
            }

            let mut resp = Json(json).into_response();
            *resp.status_mut() = status;
            return Ok((resp, prompt_tokens, output_tokens));
        }
    }
}

fn extract_openai_tokens(json: &Value) -> (u64, u64) {
    let prompt_tokens = json
        .get("usage")
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let completion_tokens = json
        .get("usage")
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    (prompt_tokens, completion_tokens)
}

fn extract_anthropic_tokens(resp: &translator::MessagesResponse) -> (u64, u64) {
    let prompt_tokens = resp.usage.input_tokens as u64;
    let output_tokens = resp.usage.output_tokens as u64;
    (prompt_tokens, output_tokens)
}

async fn handle_health(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let start = START_TIME.load(Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let uptime_seconds = if start > 0 { now - start } else { 0 };

    let providers = load_providers().unwrap_or_default();
    let groups = load_groups().unwrap_or_default();
    let router_state = state.router_state.lock().unwrap();

    let providers_info: Vec<serde_json::Value> = providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "protocol": p.protocol,
                "enabled": p.enabled,
            })
        })
        .collect();

    let mut failover_states = Vec::new();
    for group in &groups {
        for (idx, entry) in group.entries.iter().enumerate() {
            let key = format!("{}:{}", entry.provider_id, idx);
            let entry_state = router_state.entries.get(&key);
            let status = entry_state
                .map(|es| format!("{:?}", es.status).to_lowercase())
                .unwrap_or_else(|| "unknown".to_string());
            let cooldown_until = entry_state.and_then(|es| es.cooldown_until);
            failover_states.push(serde_json::json!({
                "group_id": group.id,
                "group_alias": group.alias,
                "provider_id": entry.provider_id,
                "model_id": entry.model_id,
                "priority": entry.priority,
                "entry_index": idx,
                "status": status,
                "cooldown_until": cooldown_until,
            }));
        }
    }

    Json(serde_json::json!({
        "status": "ok",
        "proxy": "coderouter",
        "uptime_seconds": uptime_seconds,
        "providers": providers_info,
        "failover_states": failover_states,
    }))
}

#[derive(Debug)]
enum AppError {
    BadRequest(String),
    NotFound(String),
    UpstreamError(String),
    InternalError(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::UpstreamError(msg) => {
                eprintln!("upstream error: {msg}");
                (StatusCode::BAD_GATEWAY, msg)
            }
            AppError::InternalError(msg) => {
                eprintln!("internal error: {msg}");
                (StatusCode::INTERNAL_SERVER_ERROR, msg)
            }
        };

        let body = Json(serde_json::json!({
            "error": {
                "message": message,
                "type": "coderouter_error",
                "code": status.as_u16()
            }
        }));

        (status, body).into_response()
    }
}
