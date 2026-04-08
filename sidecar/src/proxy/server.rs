use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use axum::{
    extract::State,
    http::{header::CONTENT_TYPE, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
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
use crate::metrics::scheduler::spawn_scheduler;
use crate::proxy::router::{
    self, SharedRouterState,
};
use crate::proxy::translator;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub groups: Arc<Vec<Group>>,
    pub client: Client,
    pub router_state: SharedRouterState,
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

    let state = AppState {
        config: Arc::new(config),
        groups: Arc::new(groups),
        client,
        router_state,
    };

    let scheduler_groups = state.groups.clone();
    let scheduler_client = state.client.clone();
    let scheduler_state = state.router_state.clone();
    let _scheduler_handle = spawn_scheduler(scheduler_state, scheduler_groups, scheduler_client);

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

    let scheduler_groups = state.groups.clone();
    let scheduler_client = state.client.clone();
    let scheduler_state = state.router_state.clone();
    let _scheduler_handle = spawn_scheduler(scheduler_state, scheduler_groups, scheduler_client);

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
}

async fn handle_models(State(state): State<Arc<AppState>>) -> Json<ModelResponse> {
    let _providers = load_providers().unwrap_or_default();
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
        .map(|g| ModelObject {
            id: g.alias.clone(),
            object: "model".to_string(),
            created: 0,
            owned_by: "coderouter".to_string(),
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

        let url = if is_anthropic {
            format!("{}/v1/messages", provider.base_url.trim_end_matches('/'))
        } else if endpoint == "completions" {
            format!("{}/v1/completions", provider.base_url.trim_end_matches('/'))
        } else {
            format!("{}/v1/chat/completions", provider.base_url.trim_end_matches('/'))
        };

        let timeout_ms = group.failover_config.latency_timeout_ms;
        let req = build_request(&state.client, &body, &api_key, &upstream_model, &url, is_anthropic);

        let result = if group.failover_config.on_latency_timeout {
            match tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                req.send(),
            ).await {
                Ok(Ok(resp)) => process_response(resp, stream, &group.alias, is_anthropic).await,
                Ok(Err(e)) => {
                    eprintln!("upstream request error: {e}");
                    Err(RequestError::Network(e.to_string()))
                }
                Err(_) => {
                    eprintln!("request timed out for provider {}", provider.id);
                    let mut rs = state.router_state.lock().unwrap();
                    router::record_latency_timeout(&mut rs, &provider.id, entry_index);
                    skip_indices.insert(entry_index);
                    continue;
                }
            }
        } else {
            match req.send().await {
                Ok(resp) => process_response(resp, stream, &group.alias, is_anthropic).await,
                Err(e) => {
                    eprintln!("upstream request error: {e}");
                    Err(RequestError::Network(e.to_string()))
                }
            }
        };

        match result {
            Ok(resp) => {
                let mut rs = state.router_state.lock().unwrap();
                router::record_success(&mut rs, &provider.id, entry_index, 0);
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

fn build_request(
    client: &Client,
    body: &Value,
    api_key: &str,
    upstream_model: &str,
    url: &str,
    is_anthropic: bool,
) -> reqwest::RequestBuilder {
    let mut req = client.post(url);

    if is_anthropic {
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

async fn process_response(
    resp: reqwest::Response,
    stream: bool,
    group_alias: &str,
    is_anthropic: bool,
) -> Result<Response, RequestError> {
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
                return Ok(translator::translate_anthropic_stream(stream, group_alias.to_string()).into_response());
            } else {
                let stream = resp.bytes_stream().map(move |result| {
                    result
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
                        .map(|chunk| {
                            let text = String::from_utf8_lossy(&chunk);
                            Event::default().data(text.as_ref())
                        })
                });
                return Ok(Sse::new(stream).into_response());
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
                let openai_resp = translator::anthropic_to_openai(&anthropic_resp, group_alias);
                return Ok(Json(openai_resp).into_response());
            } else {
                let mut json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                if let Some(obj) = json.as_object_mut() {
                    obj.insert("model".to_string(), Value::String(group_alias.to_string()));
                }
                let mut resp = Json(json).into_response();
                *resp.status_mut() = status;
                return Ok(resp);
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
            let openai_resp = translator::anthropic_to_openai(&anthropic_resp, group_alias);
            return Ok(Json(openai_resp).into_response());
        } else {
            let mut json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            if let Some(obj) = json.as_object_mut() {
                obj.insert("model".to_string(), Value::String(group_alias.to_string()));
            }

            let mut resp = Json(json).into_response();
            *resp.status_mut() = status;
            return Ok(resp);
        }
    }
}

async fn handle_health() -> Json<serde_json::Value> {
    let start = START_TIME.load(Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let uptime_seconds = if start > 0 { now - start } else { 0 };
    Json(serde_json::json!({
        "status": "ok",
        "proxy": "coderouter",
        "uptime_seconds": uptime_seconds,
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
