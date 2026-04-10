//! Axum HTTP server for the sidecar proxy.
//!
//! Handles incoming OpenAI-compatible API requests, routes them to the
//! appropriate upstream provider via the router module, translates between
//! OpenAI and Anthropic protocols as needed, streams responses back to the
//! client, and records metrics/usage.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::RwLock;
use std::time::Instant;

use axum::{
    extract::State,
    http::{header::CONTENT_TYPE, StatusCode},
    response::IntoResponse,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use axum::response::Response;
use bytes::Bytes;
use futures::stream::Stream;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::net::TcpListener;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::{broadcast, oneshot};

use crate::config::{
    models::{AppConfig, Group, Provider},
    store::{load_app_config, load_groups, load_providers},
};

/// Maximum time (ms) to wait between SSE chunks before declaring a timeout.
/// Long timeout accounts for models that "think" (reasoning) before emitting tokens.
const STREAM_INTER_CHUNK_TIMEOUT_MS: u64 = 120_000;
use crate::credentials::keychain::get_credential;
use crate::metrics::db as metrics_db;
use crate::metrics::recorder::{MetricsRecorder, RequestEvent};
use crate::metrics::scheduler;
use crate::metrics::scheduler::spawn_scheduler;
use crate::models::refresher::refresh_all_providers;
use crate::proxy::router::{
    self, SharedRouterState,
};
use crate::proxy::ssrf;
use crate::proxy::translator;
use crate::proxy::upstream::{self, UpstreamError};

/// Shared application state passed to every Axum route handler via [`State`].
///
/// Wraps config, groups, providers, the HTTP client pool, router state,
/// metrics recorder, and a broadcast channel for real-time metric streaming.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<Arc<AppConfig>>>,
    pub groups: Arc<RwLock<Arc<Vec<Group>>>>,
    pub providers: Arc<RwLock<Arc<Vec<Provider>>>>,
    pub client: Client,
    pub router_state: SharedRouterState,
    pub metrics_recorder: Arc<MetricsRecorder>,
    pub metrics_broadcast: broadcast::Sender<String>,
}

/// Unix epoch timestamp recorded at server start, used to compute uptime.
static START_TIME: AtomicI64 = AtomicI64::new(0);

/// Bootstraps and starts the Axum HTTP server.
///
/// Loads config, groups, and providers from disk; initialises the router
/// state, metrics DB, scheduler, and model-refresher background tasks;
/// binds to the configured `proxy_host:proxy_port`; and serves requests
/// until a SIGTERM or SIGINT is received, then performs graceful shutdown.
///
/// # Errors
///
/// Returns an error if the TCP listener cannot bind, the HTTP client
/// cannot be built, or the metrics DB cannot be initialised.
pub async fn start_server() -> anyhow::Result<()> {
    let start_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    START_TIME.store(start_epoch, Ordering::Relaxed);

    let config = load_app_config().unwrap_or_default();
    let groups = load_groups().unwrap_or_default();
    let providers = load_providers().unwrap_or_default();

    // No total timeout: streaming responses can run for minutes (reasoning/thinking).
    // Per-layer timeouts handle each phase: connect (below), TTFB (upstream.rs), inter-chunk (TimeoutStream), non-streaming body (tokio::timeout).
    let client = Client::builder()
        // 30 s connect timeout prevents hanging on unreachable upstreams
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()?;

    let router_state = router::init_and_set_global_router_state(&groups, &providers);

    let conn = metrics_db::init_db().expect("Failed to initialize metrics database");
    router::init_daily_totals_from_db(&router_state, &providers, &conn);
    let (metrics_recorder, metrics_handle) = MetricsRecorder::new(conn);
    let metrics_recorder = Arc::new(metrics_recorder);
    let metrics_handle = Arc::new(tokio::sync::Mutex::new(Some(metrics_handle)));

    let (metrics_broadcast_tx, _) = broadcast::channel::<String>(256);

    let state = AppState {
        config: Arc::new(RwLock::new(Arc::new(config))),
        groups: Arc::new(RwLock::new(Arc::new(groups))),
        providers: Arc::new(RwLock::new(Arc::new(providers))),
        client,
        router_state,
        metrics_recorder: metrics_recorder.clone(),
        metrics_broadcast: metrics_broadcast_tx,
    };

    let scheduler_groups = state.groups.clone();
    let scheduler_client = state.client.clone();
    let scheduler_state = state.router_state.clone();
    let (scheduler_handle, scheduler_shutdown) = spawn_scheduler(scheduler_state, scheduler_groups, scheduler_client);
    let scheduler_handle = Arc::new(tokio::sync::Mutex::new(Some(scheduler_handle)));

    let refresh_client = state.client.clone();
    let (refresh_shutdown, mut refresh_shutdown_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut refresh_shutdown_rx => {
                    eprintln!("[model-refresher] Shutting down gracefully");
                    break;
                }
                _ = refresh_all_providers(&refresh_client) => {}
            }
            let interval_hours = {
                match load_app_config() {
                    Ok(c) => c.refresh_interval_hours.max(1),
                    Err(_) => 24,
                }
            };
            tokio::time::sleep(std::time::Duration::from_secs(
                (interval_hours as u64).saturating_mul(3600),
            ))
            .await;
        }
    });

    let host = {
        let config = state.config.read().unwrap();
        config.proxy_host.clone()
    };
    let port = {
        let config = state.config.read().unwrap();
        config.proxy_port
    };
    let addr = format!("{host}:{port}");
    let state = Arc::new(state);
    let app = Router::new()
        .route("/v1/models", get(handle_models))
        .route("/v1/chat/completions", post(handle_chat_completions))
        .route("/v1/completions", post(handle_completions))
        .route("/health", get(handle_health))
        .route("/internal/router/status", get(handle_internal_router_status))
        .route("/internal/router/entry", post(handle_internal_router_set_entry))
        .route("/internal/config/reload", post(handle_internal_config_reload))
        .route("/internal/metrics/stream", get(handle_metrics_stream))
        .with_state(state.clone());

    let listener = TcpListener::bind(&addr).await?;

    eprintln!("CodeRouter proxy listening on {addr}");

    let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");

    let shutdown_recorder = metrics_recorder.clone();
    let shutdown_metrics_handle = metrics_handle.clone();
    let shutdown_scheduler_handle = scheduler_handle.clone();
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        tokio::select! {
            _ = sigterm.recv() => {
                eprintln!("Received SIGTERM, shutting down gracefully");
            }
            _ = sigint.recv() => {
                eprintln!("Received SIGINT, shutting down gracefully");
            }
        }
        let _ = scheduler_shutdown.send(());
        let _ = refresh_shutdown.send(());
        drop(shutdown_recorder);
    });

    server.await?;

    drop(state);
    drop(metrics_recorder);
    if let Some(h) = shutdown_scheduler_handle.lock().await.take() {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), h).await;
    }
    if let Some(h) = shutdown_metrics_handle.lock().await.take() {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), h).await;
    }

    Ok(())
}

/// Response body for `GET /v1/models`.
#[derive(Serialize)]
struct ModelResponse {
    object: String,
    data: Vec<ModelObject>,
}

/// Single model entry inside [`ModelResponse`].
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

/// `GET /v1/models` — lists groups as OpenAI model objects.
///
/// Only groups that have at least one **active, under-quota** entry are
/// included. Context-window and max-output-token metadata are taken from
/// the highest-priority active entry's provider model definition.
async fn handle_models(State(state): State<Arc<AppState>>) -> Json<ModelResponse> {
    let providers = state.providers.read().unwrap();
    let router_state = state.router_state.lock().unwrap();
    let groups = state.groups.read().unwrap();
    let data = groups
        .iter()
        .filter(|g| {
            g.entries.iter().enumerate().any(|(idx, e)| {
                if !e.enabled {
                    return false;
                }
                let key = format!("{}:{}", e.provider_id, idx);
                if let Some(entry_state) = router_state.entries.get(&key) {
                    if entry_state.status != router::EntryStatus::Active {
                        return false;
                    }
                    let effective_quota = e.daily_token_quota_override.or_else(|| {
                        providers
                            .iter()
                            .find(|p| p.id == e.provider_id)
                            .and_then(|p| p.daily_token_quota)
                    });
                    if let Some(quota) = effective_quota {
                        if entry_state.daily_tokens_used >= quota {
                            return false;
                        }
                    }
                    let effective_request_quota = providers
                        .iter()
                        .find(|p| p.id == e.provider_id)
                        .and_then(|p| p.daily_request_quota);
                    if let Some(quota) = effective_request_quota {
                        if entry_state.daily_requests_used >= quota {
                            return false;
                        }
                    }
                    true
                } else {
                    true
                }
            })
        })
        .map(|g| {
            let (context_window, max_output_tokens) = {
                let mut found = (None, None);
                let sorted_entries: Vec<_> = g
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.enabled)
                    .map(|(idx, e)| (idx, e))
                    .collect();
                let mut sorted_by_priority = sorted_entries.clone();
                sorted_by_priority.sort_by_key(|(_, e)| e.priority);

                for (idx, entry) in sorted_by_priority {
                    let key = format!("{}:{}", entry.provider_id, idx);
                    let is_active = router_state.entries.get(&key)
                        .map(|es| es.status == router::EntryStatus::Active)
                        .unwrap_or(true);

                    if is_active {
                        if let Some(provider) = providers.iter().find(|p| p.id == entry.provider_id) {
                            if let Some(model_meta) = provider.models.iter().find(|m| m.id == entry.model_id) {
                                found = (model_meta.context_window, model_meta.max_output_tokens);
                            }
                        }
                        break;
                    }
                }
                found
            };

            ModelObject {
                id: g.alias.clone(),
                object: "model".to_string(),
                created: chrono::Utc::now().timestamp() as u64,
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

/// `POST /v1/chat/completions` — OpenAI-compatible chat completion endpoint.
///
/// Validates the request body is a JSON object, then delegates to
/// [`route_request`].
async fn handle_chat_completions(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Response, AppError> {
    if !body.is_object() {
        return Err(AppError::BadRequest("request body must be a JSON object".into()));
    }
    route_request(&state, body, "chat/completions").await
}

/// `POST /v1/completions` — OpenAI-compatible legacy completion endpoint.
///
/// Validates the request body is a JSON object, then delegates to
/// [`route_request`]. Note: Anthropic entries are skipped for this
/// endpoint because Anthropic has no `/completions` equivalent.
async fn handle_completions(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Response, AppError> {
    if !body.is_object() {
        return Err(AppError::BadRequest("request body must be a JSON object".into()));
    }
    route_request(&state, body, "completions").await
}

/// Core request routing logic shared by both chat and legacy completions.
///
/// Resolves the requested model alias to a group, then iterates over
/// entries in priority order (with failover). For each attempt it:
///
/// 1. Picks the highest-priority available entry via [`router::select_entry`].
/// 2. Resolves the provider, API key, and protocol (OpenAI vs Anthropic).
/// 3. Validates the upstream URL against SSRF rules.
/// 4. Builds and sends the upstream request with a latency timeout.
/// 5. Processes the response — streaming or non-streaming — translating
///    Anthropic protocol back to OpenAI format when necessary.
/// 6. Records metrics and updates router state (success, 429, quota, errors).
///
/// If all entries are exhausted, returns `503 All providers unavailable`.
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

    let group = {
        let groups = state.groups.read().map_err(|_| AppError::InternalError("groups lock poisoned".into()))?;
        groups
            .iter()
            .find(|g| g.alias == model)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("no group found for model '{model}'")))?
    };

    let providers = {
        let guard = state.providers.read().map_err(|_| AppError::InternalError("providers lock poisoned".into()))?;
        guard.clone()
    };

    let max_retries = group.entries.len();
    let mut skip_indices = HashSet::new();

    // Failover loop: try each eligible entry in priority order until one succeeds
    for _attempt in 0..max_retries {
        let (entry, entry_index) = {
            let mut router_state = state.router_state.lock().unwrap();
            match router::select_entry(&group, &mut router_state, &providers, &skip_indices) {
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
            AppError::InternalError("upstream provider configuration error".to_string())
        })?;

        // Determine protocol: per-model override takes precedence over provider default
        let model_protocol = provider.models.iter()
            .find(|m| m.id == entry.model_id)
            .and_then(|m| m.protocol.clone())
            .or_else(|| provider.model_overrides.as_ref()
                .and_then(|overrides| overrides.iter()
                    .find(|m| m.id == entry.model_id)
                    .and_then(|m| m.protocol.clone())));
        let is_anthropic = model_protocol.as_deref().unwrap_or(&provider.protocol) == "anthropic";
        let upstream_model = entry.model_id.clone();

        ssrf::validate_base_url(&provider.base_url).map_err(|e| {
            eprintln!("SSRF validation failed for provider {}: {}", provider.id, e);
            AppError::InternalError(format!("invalid provider base_url: {}", e))
        })?;

        let base = provider.base_url.trim_end_matches('/');
        let url = if is_anthropic {
            format!("{base}/messages")
        } else if endpoint == "completions" {
            format!("{base}/completions")
        } else {
            format!("{base}/chat/completions")
        };

        // Anthropic doesn't have a /completions equivalent — skip this entry
        if endpoint == "completions" && is_anthropic {
            skip_indices.insert(entry_index);
            continue;
        }

        let timeout_ms = group.failover_config.latency_timeout_ms;
        let req = if endpoint == "completions" {
            upstream::build_completion_request(&state.client, &body, &api_key, &upstream_model, &url, is_anthropic)
        } else {
            upstream::build_upstream_request(&state.client, &body, &api_key, &upstream_model, &url, is_anthropic)
        };

        let result = match upstream::send_with_timeout(req, timeout_ms, group.failover_config.on_latency_timeout).await {
            Ok(resp) => process_response(resp, stream, &group.alias, is_anthropic, timeout_ms).await,
            Err(UpstreamError::Timeout) => {
                eprintln!("request timed out for provider {}", provider.id);
                {
                    let mut rs = state.router_state.lock().unwrap();
                    let _ = router::record_latency_timeout(&mut rs, &provider.id, entry_index, group.failover_config.latency_timeout_cooldown_ms);
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
            Ok(StreamProcessResult::Streaming(raw_stream, token_counts)) => {
                let provider_id = provider.id.clone();
                let model_id = entry.model_id.clone();
                let group_alias_str = group.alias.clone();
                let router_state = state.router_state.clone();
                let metrics_recorder = state.metrics_recorder.clone();
                let metrics_broadcast = state.metrics_broadcast.clone();
                let entry_index_clone = entry_index;
                let effective_quota = entry.daily_token_quota_override.or_else(|| {
                    providers
                        .iter()
                        .find(|p| p.id == entry.provider_id)
                        .and_then(|p| p.daily_token_quota)
                });
                let effective_request_quota = providers
                    .iter()
                    .find(|p| p.id == entry.provider_id)
                    .and_then(|p| p.daily_request_quota);
                let on_quota_exhausted = group.failover_config.on_quota_exhausted;
                let latency_start = start;

                let consecutive_error_threshold = group.failover_config.consecutive_error_threshold;
                let on_consecutive_errors = group.failover_config.on_consecutive_errors;
                let consecutive_error_cooldown_ms = group.failover_config.consecutive_error_cooldown_ms;

                let (pricing_input, pricing_output) = provider.models.iter()
                    .find(|m| m.id == entry.model_id)
                    .map(|m| (m.input_cost_per_1m, m.output_cost_per_1m))
                    .unwrap_or((None, None));

                // Wrap the stream in MetricsRecordingStream so success/error is captured
                // after the entire stream has been consumed by the client.
                let body_with_metrics = MetricsRecordingStream::new(raw_stream, move |success: bool| {
                    let latency_ms = latency_start.elapsed().as_millis() as i64;
                    let counts = token_counts.lock().unwrap();
                    let prompt_tokens = counts.input_tokens;
                    let output_tokens = counts.output_tokens;
                    let tokens_used = prompt_tokens + output_tokens;
                    drop(counts);

                    if success {
                        let event = RequestEvent {
                            ts: chrono::Utc::now().timestamp(),
                            group_alias: group_alias_str.clone(),
                            provider_id: provider_id.clone(),
                            model_id: model_id.clone(),
                            prompt_tokens: prompt_tokens as i64,
                            output_tokens: output_tokens as i64,
                            latency_ms,
                            status: "success".to_string(),
                            error_type: None,
                            input_cost_per_1m: pricing_input,
                            output_cost_per_1m: pricing_output,
                        };
                        let _ = metrics_recorder.record_request_sync(event.clone());
                        if let Ok(json) = serde_json::to_string(&event) {
                            let _ = metrics_broadcast.send(json);
                        }

                        if tokens_used > 0 {
                            let mut rs = router_state.lock().unwrap();
                            router::record_success(&mut rs, &provider_id, entry_index_clone, tokens_used, effective_quota, effective_request_quota, on_quota_exhausted);
                        }
                    } else {
                        let event = RequestEvent {
                            ts: chrono::Utc::now().timestamp(),
                            group_alias: group_alias_str.clone(),
                            provider_id: provider_id.clone(),
                            model_id: model_id.clone(),
                            prompt_tokens: prompt_tokens as i64,
                            output_tokens: output_tokens as i64,
                            latency_ms,
                            status: "error".to_string(),
                            error_type: Some("stream_error".to_string()),
                            input_cost_per_1m: pricing_input,
                            output_cost_per_1m: pricing_output,
                        };
                        let _ = metrics_recorder.record_request_sync(event.clone());
                        if let Ok(json) = serde_json::to_string(&event) {
                            let _ = metrics_broadcast.send(json);
                        }

                        let mut rs = router_state.lock().unwrap();
                        router::record_consecutive_error(
                            &mut rs,
                            &provider_id,
                            entry_index_clone,
                            consecutive_error_threshold,
                            on_consecutive_errors,
                            consecutive_error_cooldown_ms,
                        );
                    }
                });

                let body = axum::body::Body::from_stream(body_with_metrics);
                let mut final_resp = Response::new(body);
                final_resp.headers_mut().insert(
                    CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("text/event-stream"),
                );
                return Ok(final_resp);
            }
            Ok(StreamProcessResult::NonStreaming(resp, prompt_tokens, output_tokens)) => {
                let latency_ms = start.elapsed().as_millis() as i64;
                let tokens_used = prompt_tokens + output_tokens;
                {
                    let mut rs = state.router_state.lock().unwrap();
                    let effective_quota = entry.daily_token_quota_override.or_else(|| {
                        providers
                            .iter()
                            .find(|p| p.id == entry.provider_id)
                            .and_then(|p| p.daily_token_quota)
                    });
                    let effective_request_quota = providers
                        .iter()
                        .find(|p| p.id == entry.provider_id)
                        .and_then(|p| p.daily_request_quota);
                    router::record_success(&mut rs, &provider.id, entry_index, tokens_used, effective_quota, effective_request_quota, group.failover_config.on_quota_exhausted);
                }
                let model_id = entry.model_id.clone();
                let provider_id = provider.id.clone();
                let group_alias = group.alias.clone();
                let metrics_recorder = state.metrics_recorder.clone();
                let metrics_broadcast = state.metrics_broadcast.clone();
                let (pricing_input, pricing_output) = provider.models.iter()
                    .find(|m| m.id == entry.model_id)
                    .map(|m| (m.input_cost_per_1m, m.output_cost_per_1m))
                    .unwrap_or((None, None));
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
                        input_cost_per_1m: pricing_input,
                        output_cost_per_1m: pricing_output,
                    };
                    let _ = metrics_recorder.record_request(event.clone()).await;
                    if let Ok(json) = serde_json::to_string(&event) {
                        let _ = metrics_broadcast.send(json);
                    }
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
                {
                    let mut rs = state.router_state.lock().unwrap();
                    router::record_consecutive_error(
                        &mut rs,
                        &provider.id,
                        entry_index,
                        group.failover_config.consecutive_error_threshold,
                        group.failover_config.on_consecutive_errors,
                        group.failover_config.consecutive_error_cooldown_ms,
                    );
                }
                skip_indices.insert(entry_index);
                continue;
            }
            Err(RequestError::ServerError(_msg)) => {
                {
                    let mut rs = state.router_state.lock().unwrap();
                    router::record_consecutive_error(
                        &mut rs,
                        &provider.id,
                        entry_index,
                        group.failover_config.consecutive_error_threshold,
                        group.failover_config.on_consecutive_errors,
                        group.failover_config.consecutive_error_cooldown_ms,
                    );
                }
                skip_indices.insert(entry_index);
                continue;
            }
        }
    }

    // All entries exhausted — return 503
    let exhausted_body = router::build_exhausted_response(&model);
    Ok((StatusCode::SERVICE_UNAVAILABLE, Json(exhausted_body)).into_response())
}

/// Errors that can occur while processing a proxied request.
///
/// Each variant maps to a specific failover behaviour in [`route_request`].
#[allow(dead_code)]
enum RequestError {
    /// Upstream returned HTTP 429 (rate-limited).
    RateLimited,
    /// Daily token/request quota has been exhausted.
    QuotaExhausted,
    /// Network-level error (connection, DNS, etc.).
    Network(String),
    /// Upstream returned a non-success HTTP status.
    ServerError(String),
}

/// Result of processing an upstream response.
enum StreamProcessResult {
    /// Streaming response (SSE) — the body is a translated byte stream,
    /// and `Arc<Mutex<StreamTokenCounts>>` accumulates token counts.
    Streaming(
        Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send + Unpin + 'static>,
        Arc<std::sync::Mutex<translator::StreamTokenCounts>>,
    ),
    /// Non-streaming response — contains the final HTTP response plus
    /// prompt/output token counts extracted from the body.
    NonStreaming(Response, u64, u64),
}

/// Processes the raw upstream HTTP response.
///
/// Handles three major branches:
///
/// - **429**: immediately returns [`RequestError::RateLimited`] so the
///   caller can trigger failover.
/// - **Streaming** (`stream: true` + `text/event-stream` content type):
///   wraps the byte stream in [`TimeoutStream`] for inter-chunk timeouts,
///   then translates via the appropriate translator.
/// - **Non-streaming** or unexpected content type: reads the full body,
///   translates if Anthropic, and returns token counts.
///
/// # Arguments
///
/// * `resp` — The raw [`reqwest::Response`] from the upstream provider.
/// * `stream` — Whether the client requested SSE streaming.
/// * `group_alias` — The group alias used as the model name in responses.
/// * `is_anthropic` — Whether the upstream speaks the Anthropic protocol.
/// * `_latency_timeout_ms` — Currently unused; inter-chunk timeout is
///   configured via [`STREAM_INTER_CHUNK_TIMEOUT_MS`].
///
/// # Errors
///
/// Returns [`RequestError`] variants for rate-limiting, server errors,
/// network failures, or body-parse failures.
async fn process_response(
    resp: reqwest::Response,
    stream: bool,
    group_alias: &str,
    is_anthropic: bool,
    _latency_timeout_ms: u64,
) -> Result<StreamProcessResult, RequestError> {
    let status = resp.status();

    if status.as_u16() == 429 {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), resp.bytes()).await;
        return Err(RequestError::RateLimited);
    }

    if !status.is_success() {
        let body = match tokio::time::timeout(std::time::Duration::from_secs(10), resp.text()).await {
            Ok(text) => text.unwrap_or_default(),
            Err(_) => "timed out reading error body".to_string(),
        };
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
            let token_counts = Arc::new(std::sync::Mutex::new(translator::StreamTokenCounts {
                input_tokens: 0,
                output_tokens: 0,
            }));

            let raw_stream = TimeoutStream::new(
                resp.bytes_stream().map(|result| {
                    result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
                }),
                STREAM_INTER_CHUNK_TIMEOUT_MS,
            );

            if is_anthropic {
                let (body, token_counts) = translator::translate_anthropic_stream(raw_stream, group_alias.to_string(), token_counts);
                let stream: Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send + Unpin + 'static> =
                    Box::new(body.into_data_stream().map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))));
                return Ok(StreamProcessResult::Streaming(stream, token_counts));
            } else {
                let (body, token_counts) = translator::translate_openai_stream(raw_stream, token_counts);
                let stream: Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send + Unpin + 'static> =
                    Box::new(body.into_data_stream().map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))));
                return Ok(StreamProcessResult::Streaming(stream, token_counts));
            }
        } else {
            let bytes = tokio::time::timeout(
                std::time::Duration::from_secs(120),
                resp.bytes()
            )
            .await
            .map_err(|_| RequestError::Network("response body read timed out".into()))?
            .map_err(|e| {
                eprintln!("failed to read response body: {e}");
                RequestError::Network(e.to_string())
            })?;

            if is_anthropic {
                let anthropic_resp: translator::MessagesResponse =
                    serde_json::from_slice(&bytes).map_err(|e| {
                        eprintln!("failed to parse anthropic response: {e}");
                        RequestError::ServerError(format!("invalid upstream JSON: {e}"))
                    })?;
                let (prompt_tokens, output_tokens) = extract_anthropic_tokens(&anthropic_resp);
                let openai_resp = translator::anthropic_to_openai(&anthropic_resp, group_alias);
                return Ok(StreamProcessResult::NonStreaming(Json(openai_resp).into_response(), prompt_tokens, output_tokens));
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
                return Ok(StreamProcessResult::NonStreaming(resp, prompt_tokens, output_tokens));
            }
        }
    } else {
        let bytes = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            resp.bytes()
        )
        .await
        .map_err(|_| RequestError::Network("response body read timed out".into()))?
        .map_err(|e| {
            eprintln!("failed to read response body: {e}");
            RequestError::Network(e.to_string())
        })?;

        if is_anthropic {
            let anthropic_resp: translator::MessagesResponse =
                serde_json::from_slice(&bytes).map_err(|e| {
                    eprintln!("failed to parse anthropic response: {e}");
                    RequestError::ServerError(format!("invalid upstream JSON: {e}"))
                })?;
            let (prompt_tokens, output_tokens) = extract_anthropic_tokens(&anthropic_resp);
            let openai_resp = translator::anthropic_to_openai(&anthropic_resp, group_alias);
            return Ok(StreamProcessResult::NonStreaming(Json(openai_resp).into_response(), prompt_tokens, output_tokens));
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
            return Ok(StreamProcessResult::NonStreaming(resp, prompt_tokens, output_tokens));
        }
    }
}

/// Extracts `prompt_tokens` and `completion_tokens` from an OpenAI-style
/// JSON response's `usage` field. Returns `(0, 0)` if usage is absent.
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

/// Extracts token counts from an Anthropic [`MessagesResponse`].
fn extract_anthropic_tokens(resp: &translator::MessagesResponse) -> (u64, u64) {
    let prompt_tokens = resp.usage.input_tokens as u64;
    let output_tokens = resp.usage.output_tokens as u64;
    (prompt_tokens, output_tokens)
}

/// A [`Stream`] wrapper that enforces a maximum gap between consecutive chunks.
///
/// If no chunk arrives within `timeout` of the last successful chunk (or
/// stream creation), the stream produces an [`std::io::ErrorKind::TimedOut`]
/// error. This prevents silently hanging connections during streaming
/// responses (e.g., when an upstream silently drops the connection).
struct TimeoutStream<S> {
    inner: S,
    timeout: std::time::Duration,
    last_activity: Instant,
}

impl<S> TimeoutStream<S> {
    /// Creates a new `TimeoutStream` wrapping `inner` with the given
    /// inter-chunk timeout in milliseconds.
    fn new(inner: S, timeout_ms: u64) -> Self {
        Self {
            inner,
            timeout: std::time::Duration::from_millis(timeout_ms),
            last_activity: Instant::now(),
        }
    }
}

/// Implements the [`Stream`] trait for `TimeoutStream`.
///
/// On each `poll_next`:
/// - A successful chunk resets `last_activity` and is yielded.
/// - An error is forwarded as-is.
/// - A `Pending` result checks whether the elapsed time since the last
///   activity exceeds `timeout`; if so, yields a `TimedOut` error.
/// - Stream end (`None`) is forwarded.
impl<S> Stream for TimeoutStream<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(item))) => {
                self.last_activity = Instant::now();
                Poll::Ready(Some(Ok(item)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => {
                if self.last_activity.elapsed() > self.timeout {
                    Poll::Ready(Some(Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "streaming response timed out (inter-chunk gap exceeded)",
                    ))))
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

/// A [`Stream`] wrapper that invokes a callback when the inner stream ends
/// (success) or encounters an error (failure). Used to record metrics on
/// stream completion.
struct MetricsRecordingStream<S> {
    inner: S,
    on_complete: Option<Box<dyn FnOnce(bool) + Send + 'static>>,
}

impl<S> MetricsRecordingStream<S> {
    /// Creates a new `MetricsRecordingStream` that calls `on_complete(bool)`
    /// exactly once — `true` for a clean end-of-stream, `false` for an error.
    fn new<F>(inner: S, on_complete: F) -> Self
    where
        F: FnOnce(bool) + Send + 'static,
    {
        Self {
            inner,
            on_complete: Some(Box::new(on_complete)),
        }
    }
}

/// Implements [`Stream`] for `MetricsRecordingStream`.
///
/// Delegates to the inner stream and fires `on_complete(false)` on error
/// or `on_complete(true)` on clean end-of-stream. The callback is called
/// at most once.
impl<S> Stream for MetricsRecordingStream<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(item))) => Poll::Ready(Some(Ok(item))),
            Poll::Ready(Some(Err(e))) => {
                if let Some(cb) = self.on_complete.take() {
                    cb(false);
                }
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                if let Some(cb) = self.on_complete.take() {
                    cb(true);
                }
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// `GET /internal/metrics/stream` — SSE endpoint that pushes real-time
/// request metrics to subscribers as JSON events.
async fn handle_metrics_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.metrics_broadcast.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(json) => {
                    yield Ok(Event::default().data(json));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    };
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keepalive"),
    )
}

/// `GET /health` — returns a JSON object with server uptime, provider info,
/// and the current failover state of every group entry.
async fn handle_health(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let start = START_TIME.load(Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let uptime_seconds = if start > 0 { now - start } else { 0 };

    let providers = state.providers.read().unwrap().clone();
    let groups = state.groups.read().unwrap().clone();
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
    for group in groups.iter() {
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

/// Application-level error type mapped to HTTP status codes.
#[derive(Debug)]
enum AppError {
    BadRequest(String),
    NotFound(String),
    InternalError(String),
}

/// Converts [`AppError`] into an Axum [`Response`] with an OpenAI-style
/// error JSON body: `{"error":{"message":…,"type":"coderouter_error","code":…}}`.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
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

/// Response body for `GET /internal/router/status`.
#[derive(Serialize)]
struct InternalRouterStatusResponse {
    pub status: String,
    pub data: Option<router::RouterStatusResponse>,
}

/// `GET /internal/router/status` — returns the full failover/state
/// snapshot for every group entry (for admin dashboards).
async fn handle_internal_router_status(
    State(state): State<Arc<AppState>>,
) -> Json<InternalRouterStatusResponse> {
    let groups = load_groups().unwrap_or_default();
    let router_state = state.router_state.lock().unwrap();
    let status = router::get_router_status(&groups, &router_state);
    Json(InternalRouterStatusResponse {
        status: "ok".to_string(),
        data: Some(status),
    })
}

/// Request body for `POST /internal/router/entry`.
#[derive(Deserialize)]
struct InternalSetEntryRequest {
    pub group_id: String,
    pub entry_index: usize,
    pub enabled: bool,
}

/// `POST /internal/router/entry` — enable or disable a specific group entry.
///
/// After updating the entry's enabled state, reloads groups and providers
/// from disk so the scheduler picks up the change.
async fn handle_internal_router_set_entry(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InternalSetEntryRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let groups = state.groups.read().unwrap().clone();
    match scheduler::set_entry_enabled(
        &state.router_state,
        groups,
        &req.group_id,
        req.entry_index,
        req.enabled,
    ) {
        Ok(()) => {
            let reloaded = load_groups().unwrap_or_else(|_| {
                state.groups.read().unwrap().as_ref().clone()
            });
            *state.groups.write().unwrap() = Arc::new(reloaded);
            // The scheduler holds a clone of state.groups (same Arc<RwLock<...>>),
            // so this write is visible on its next tick when it reads groups_clone.
            let reloaded_providers = load_providers().unwrap_or_else(|_| {
                state.providers.read().unwrap().as_ref().clone()
            });
            *state.providers.write().unwrap() = Arc::new(reloaded_providers);
            Ok(Json(serde_json::json!({ "status": "ok" })))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )),
    }
}

/// `POST /internal/config/reload` — hot-reloads `app_config`, groups, and
/// providers from disk without restarting the process.
///
/// Preserves router state (cooldowns, quotas, consecutive errors) for
/// entries that still exist after the reload, and removes entries that
/// are no longer present.
async fn handle_internal_config_reload(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let new_config = match load_app_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[config-reload] failed to load app config: {e}");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to load app config: {e}") })),
            ));
        }
    };
    let new_groups = match load_groups() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[config-reload] failed to load groups: {e}");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to load groups: {e}") })),
            ));
        }
    };
    let new_providers = match load_providers() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[config-reload] failed to load providers: {e}");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to load providers: {e}") })),
            ));
        }
    };

    let old_state = {
        let guard = state.router_state.lock().unwrap();
        let mut map: HashMap<String, router::EntryState> = HashMap::new();
        for (key, entry) in guard.entries.iter() {
            map.insert(key.clone(), entry.clone());
        }
        map
    };

    *state.config.write().unwrap() = Arc::new(new_config);
    *state.groups.write().unwrap() = Arc::new(new_groups.clone());
    *state.providers.write().unwrap() = Arc::new(new_providers.clone());

    let new_router_state = router::init_router_state(&new_groups, &new_providers);

    {
        let new_entries = {
            let new_guard = new_router_state.lock().unwrap();
            new_guard.entries.clone()
        };
        let mut guard = state.router_state.lock().unwrap();
        for (key, new_entry) in &new_entries {
            if let Some(existing) = old_state.get(key) {
                let entry = guard.entries.entry(key.clone()).or_insert_with(|| new_entry.clone());
                entry.status = existing.status.clone();
                entry.consecutive_errors = existing.consecutive_errors;
                entry.cooldown_until = existing.cooldown_until;
                entry.cooldown_duration_seconds = existing.cooldown_duration_seconds;
                entry.daily_tokens_used = existing.daily_tokens_used;
                entry.daily_requests_used = existing.daily_requests_used;
                entry.daily_reset_at = existing.daily_reset_at;
            } else {
                guard.entries.insert(key.clone(), new_entry.clone());
            }
        }
        let new_keys: HashSet<String> = new_entries.keys().cloned().collect();
        guard.entries.retain(|key, _| new_keys.contains(key));
    }

    match metrics_db::init_db() {
        Ok(reload_conn) => {
            router::init_daily_totals_from_db(&state.router_state, &new_providers, &reload_conn);
        }
        Err(e) => {
            eprintln!("[config-reload] failed to reload daily totals: {e}");
        }
    }

    eprintln!("[config-reload] config reloaded successfully");
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures::stream::StreamExt;
    use std::sync::{Arc, Mutex};
    use crate::config::models::ProviderModel;

    #[tokio::test]
    async fn test_timeout_stream_passes_chunks_without_delay() {
        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from("chunk1")),
            Ok(Bytes::from("chunk2")),
            Ok(Bytes::from("chunk3")),
        ];
        let stream = futures::stream::iter(chunks);
        let mut timeout_stream = TimeoutStream::new(stream, 5000);

        let mut results = Vec::new();
        while let Some(item) = timeout_stream.next().await {
            results.push(item);
        }
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.is_ok()));
    }

    #[tokio::test]
    async fn test_timeout_stream_propagates_end() {
        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![];
        let stream = futures::stream::iter(chunks);
        let mut timeout_stream = TimeoutStream::new(stream, 5000);

        let result = timeout_stream.next().await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_timeout_stream_propagates_error() {
        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from("chunk1")),
            Err(std::io::Error::new(std::io::ErrorKind::Other, "test error")),
        ];
        let stream = futures::stream::iter(chunks);
        let mut timeout_stream = TimeoutStream::new(stream, 5000);

        let r1 = timeout_stream.next().await.unwrap().unwrap();
        assert_eq!(r1, Bytes::from("chunk1"));

        let r2 = timeout_stream.next().await.unwrap();
        assert!(r2.is_err());
        assert_eq!(r2.unwrap_err().kind(), std::io::ErrorKind::Other);
    }

    #[tokio::test]
    async fn test_metrics_recording_stream_success_callback() {
        let called = Arc::new(Mutex::new(None));
        let called_clone = called.clone();

        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from("chunk1")),
            Ok(Bytes::from("chunk2")),
        ];
        let stream = futures::stream::iter(chunks);
        let mut metrics_stream = MetricsRecordingStream::new(stream, move |success: bool| {
            *called_clone.lock().unwrap() = Some(success);
        });

        while let Some(_) = metrics_stream.next().await {}

        assert_eq!(*called.lock().unwrap(), Some(true));
    }

    #[tokio::test]
    async fn test_metrics_recording_stream_error_callback() {
        let called = Arc::new(Mutex::new(None));
        let called_clone = called.clone();

        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from("chunk1")),
            Err(std::io::Error::new(std::io::ErrorKind::Other, "stream error")),
        ];
        let stream = futures::stream::iter(chunks);
        let mut metrics_stream = MetricsRecordingStream::new(stream, move |success: bool| {
            *called_clone.lock().unwrap() = Some(success);
        });

        while let Some(item) = metrics_stream.next().await {
            if item.is_err() {
                break;
            }
        }

        assert_eq!(*called.lock().unwrap(), Some(false));
    }

    #[tokio::test]
    async fn test_metrics_recording_stream_empty_success() {
        let called = Arc::new(Mutex::new(None));
        let called_clone = called.clone();

        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![];
        let stream = futures::stream::iter(chunks);
        let mut metrics_stream = MetricsRecordingStream::new(stream, move |success: bool| {
            *called_clone.lock().unwrap() = Some(success);
        });

        while let Some(_) = metrics_stream.next().await {}

        assert_eq!(*called.lock().unwrap(), Some(true));
    }

    #[test]
    fn test_pricing_lookup_from_provider_models() {
        let provider = Provider {
            id: "test-provider".to_string(),
            name: "Test".to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.test.com".to_string(),
            credential_key: "test".to_string(),
            daily_token_quota: None,
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![
                ProviderModel {
                    id: "gpt-4".to_string(),
                    context_window: Some(128000),
                    max_output_tokens: Some(4096),
                    input_cost_per_1m: Some(30.0),
                    output_cost_per_1m: Some(60.0),
                    last_refreshed: None,
                    protocol: None,
                },
                ProviderModel {
                    id: "gpt-3.5".to_string(),
                    context_window: Some(16384),
                    max_output_tokens: Some(4096),
                    input_cost_per_1m: Some(0.5),
                    output_cost_per_1m: Some(1.5),
                    last_refreshed: None,
                    protocol: None,
                },
            ],
            model_overrides: None,
        };

        let (input_cost, output_cost) = provider.models.iter()
            .find(|m| m.id == "gpt-4")
            .map(|m| (m.input_cost_per_1m, m.output_cost_per_1m))
            .unwrap_or((None, None));

        assert_eq!(input_cost, Some(30.0));
        assert_eq!(output_cost, Some(60.0));

        let (input_cost, output_cost) = provider.models.iter()
            .find(|m| m.id == "gpt-3.5")
            .map(|m| (m.input_cost_per_1m, m.output_cost_per_1m))
            .unwrap_or((None, None));

        assert_eq!(input_cost, Some(0.5));
        assert_eq!(output_cost, Some(1.5));

        let (input_cost, output_cost) = provider.models.iter()
            .find(|m| m.id == "nonexistent")
            .map(|m| (m.input_cost_per_1m, m.output_cost_per_1m))
            .unwrap_or((None, None));

        assert_eq!(input_cost, None);
        assert_eq!(output_cost, None);
    }

    #[test]
    fn test_request_event_calculate_cost_with_pricing() {
        let event = RequestEvent {
            ts: 0,
            group_alias: "test".to_string(),
            provider_id: "provider".to_string(),
            model_id: "model".to_string(),
            prompt_tokens: 1_000_000,
            output_tokens: 500_000,
            latency_ms: 100,
            status: "success".to_string(),
            error_type: None,
            input_cost_per_1m: Some(30.0),
            output_cost_per_1m: Some(60.0),
        };

        let cost = event.calculate_cost();
        assert!((cost - 60.0).abs() < 0.001);
    }

    #[test]
    fn test_request_event_calculate_cost_without_pricing() {
        let event = RequestEvent {
            ts: 0,
            group_alias: "test".to_string(),
            provider_id: "provider".to_string(),
            model_id: "model".to_string(),
            prompt_tokens: 1_000_000,
            output_tokens: 500_000,
            latency_ms: 100,
            status: "success".to_string(),
            error_type: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
        };

        let cost = event.calculate_cost();
        assert_eq!(cost, 0.0);
    }
}
