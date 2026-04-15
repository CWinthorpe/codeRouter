//! Tauri IPC command handlers and shared application state.
//!
//! This module contains every `#[tauri::command]` function that the frontend
//! can invoke, plus helper structs (`ProviderResponse`, `OpenCodeAgentMapping`,
//! `AppState`, etc.) and sidecar lifecycle functions (`spawn_sidecar`,
//! `kill_sidecar`).

use coderouter_proxy::config::models::{AppConfig, Group, Provider};
use coderouter_proxy::config::store;
use coderouter_proxy::credentials::keychain;
use coderouter_proxy::metrics::db;
use coderouter_proxy::metrics::queries;
use coderouter_proxy::opencode::config_writer::{self, AgentMapping};
use coderouter_proxy::proxy::router;
use coderouter_proxy::proxy::ssrf;
use chrono::NaiveDate;
use reqwest::Client;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Child;
use std::sync::Mutex;
use tauri::menu::MenuItem;
use tauri_plugin_updater::UpdaterExt;

/// Global singleton SQLite connection for metrics queries.
///
/// Initialized once at startup via [`init_metrics_db`] and accessed through
/// [`with_metrics_db`].
static METRICS_DB: Mutex<Option<Connection>> = Mutex::new(None);

/// Opens (or creates) the metrics database and stores the connection in the
/// global [`METRICS_DB`] singleton.
///
/// # Errors
/// Returns an error string if the database cannot be opened or if the global
/// mutex is poisoned.
pub fn init_metrics_db() -> Result<(), String> {
    let conn = db::init_db().map_err(|e| e.to_string())?;
    let mut guard = METRICS_DB.lock().map_err(|e| e.to_string())?;
    *guard = Some(conn);
    Ok(())
}

/// Executes a closure with a reference to the global metrics database connection.
///
/// # Errors
/// Returns an error string if the mutex is poisoned or if the database has not
/// been initialized via [`init_metrics_db`].
fn with_metrics_db<T, F: FnOnce(&Connection) -> Result<T, String>>(f: F) -> Result<T, String> {
    let guard = METRICS_DB.lock().map_err(|e| e.to_string())?;
    let conn = guard.as_ref().ok_or_else(|| "Metrics database not initialized".to_string())?;
    f(conn)
}

/// JSON-serializable provider representation for the frontend.
///
/// Mirrors [`Provider`] but flattens and renames fields for the API contract
/// (e.g. `baseUrl`, `credentialKey`, `dailyTokenQuota`).
#[derive(Serialize)]
pub struct ProviderResponse {
    pub id: String,
    pub name: String,
    pub protocol: String,
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    #[serde(rename = "credentialKey")]
    pub credential_key: String,
    #[serde(default, rename = "dailyTokenQuota")]
    pub daily_token_quota: Option<u64>,
    #[serde(default, rename = "quotaResetUtcHour")]
    pub quota_reset_utc_hour: u32,
    #[serde(default, rename = "dailyRequestQuota")]
    pub daily_request_quota: Option<u64>,
    #[serde(default, rename = "modelOverrides")]
    pub model_overrides: Vec<ProviderModelResponse>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub models: Vec<ProviderModelResponse>,
}

/// JSON-serializable model metadata within a provider response.
#[derive(Serialize)]
pub struct ProviderModelResponse {
    pub id: String,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub input_cost_per_1m: Option<f64>,
    #[serde(default)]
    pub output_cost_per_1m: Option<f64>,
    #[serde(default)]
    pub last_refreshed: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
}

impl From<&Provider> for ProviderResponse {
    fn from(p: &Provider) -> Self {
        ProviderResponse {
            id: p.id.clone(),
            name: p.name.clone(),
            protocol: p.protocol.clone(),
            base_url: p.base_url.clone(),
            credential_key: p.credential_key.clone(),
            daily_token_quota: p.daily_token_quota,
            daily_request_quota: p.daily_request_quota,
            quota_reset_utc_hour: p.quota_reset_utc_hour,
            model_overrides: p.model_overrides.as_ref().map(|v| v.iter().map(|m| ProviderModelResponse {
                id: m.id.clone(),
                context_window: m.context_window,
                max_output_tokens: m.max_output_tokens,
                input_cost_per_1m: m.input_cost_per_1m,
                output_cost_per_1m: m.output_cost_per_1m,
                last_refreshed: m.last_refreshed.clone(),
                protocol: m.protocol.clone(),
            }).collect()).unwrap_or_default(),
            // When no auto-discovered models exist, fall back to model overrides
            // so the UI always has something to display.
            enabled: p.enabled,
            models: {
                let auto: Vec<ProviderModelResponse> = p.models.iter().map(|m| ProviderModelResponse {
                    id: m.id.clone(),
                    context_window: m.context_window,
                    max_output_tokens: m.max_output_tokens,
                    input_cost_per_1m: m.input_cost_per_1m,
                    output_cost_per_1m: m.output_cost_per_1m,
                    last_refreshed: m.last_refreshed.clone(),
                    protocol: m.protocol.clone(),
                }).collect();
                if auto.is_empty() {
                    p.model_overrides.as_ref().map(|v| v.iter().map(|m| ProviderModelResponse {
                        id: m.id.clone(),
                        context_window: m.context_window,
                        max_output_tokens: m.max_output_tokens,
                        input_cost_per_1m: m.input_cost_per_1m,
                        output_cost_per_1m: m.output_cost_per_1m,
                        last_refreshed: m.last_refreshed.clone(),
                        protocol: m.protocol.clone(),
                    }).collect()).unwrap_or_default()
                } else {
                    auto
                }
            },
        }
    }
}

/// Returns all configured providers.
///
/// # Errors
/// Returns an error string if the provider configuration file cannot be read.
#[tauri::command]
pub async fn get_providers() -> Result<Vec<ProviderResponse>, String> {
    let providers = store::load_providers().map_err(|e| e.to_string())?;
    Ok(providers.iter().map(|p| p.into()).collect())
}

/// Sends a config-reload signal to the running sidecar process.
///
/// Posts to the `/internal/config/reload` endpoint so the proxy picks up
/// configuration changes without a full restart. Retries up to 3 times with
/// a 500 ms delay between attempts because the sidecar may need a moment to
/// become ready after a config write.
async fn notify_sidecar_config_reload() {
    let config = store::load_app_config().unwrap_or_default();
    let url = format!("http://{}:{}/internal/config/reload", config.proxy_host, config.proxy_port);
    let client = Client::new();

    for attempt in 0..3 {
        // Wait before retrying (skip delay on first attempt)
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        match client.post(&url).timeout(std::time::Duration::from_secs(2)).send().await {
            Ok(_) => return,
            Err(e) => {
                eprintln!("[notify-sidecar] attempt {} failed: {e}", attempt + 1);
            }
        }
    }
    eprintln!("[notify-sidecar] all retry attempts failed");
}

/// Creates or updates a provider and persists its API key in the keychain.
///
/// Validates that the protocol is either `"openai"` or `"anthropic"` and
/// that the base URL passes SSRF checks. If a provider with the same id
/// already exists it is replaced; otherwise a new entry is appended.
///
/// # Arguments
/// * `provider` — The provider configuration to save.
/// * `api_key`  — The API key to store in the system keychain. If empty,
///   no keychain operation is performed.
///
/// # Errors
/// Returns an error string if the protocol is invalid, the base URL fails
/// SSRF validation, persisted storage fails, or keychain storage fails.
#[tauri::command]
pub async fn save_provider(provider: Provider, api_key: String) -> Result<(), String> {
    if provider.protocol != "openai" && provider.protocol != "anthropic" {
        return Err(format!("Invalid protocol '{}'. Must be 'openai' or 'anthropic'.", provider.protocol));
    }

    ssrf::validate_base_url(&provider.base_url)?;

    let mut providers = match store::load_providers() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[save_provider] Warning: failed to load existing providers ({e}), starting fresh");
            Vec::new()
        }
    };

    if let Some(pos) = providers.iter().position(|p| p.id == provider.id) {
        providers[pos] = provider.clone();
    } else {
        providers.push(provider.clone());
    }

    store::save_providers(&providers).map_err(|e| e.to_string())?;

    // Only store a credential when a non-empty key is provided so that
    // updates to other fields don't erase an existing keychain entry.
    if !api_key.is_empty() {
        keychain::store_credential(&provider.id, &api_key)
            .await
            .map_err(|e| e.to_string())?;
    }

    notify_sidecar_config_reload().await;

    Ok(())
}

/// Toggles a provider's enabled flag and persists the change.
///
/// # Arguments
/// * `provider_id` — The id of the provider to toggle.
/// * `enabled`      — The new enabled state.
///
/// # Errors
/// Returns an error string if the provider list cannot be loaded or saved,
/// or if the sidecar config reload fails.
#[tauri::command]
pub async fn toggle_provider_enabled(provider_id: String, enabled: bool) -> Result<(), String> {
    let mut providers = store::load_providers().map_err(|e| e.to_string())?;
    if let Some(provider) = providers.iter_mut().find(|p| p.id == provider_id) {
        provider.enabled = enabled;
    }
    store::save_providers(&providers).map_err(|e| e.to_string())?;
    notify_sidecar_config_reload().await;
    Ok(())
}

/// Deletes a provider and cleans up related data.
///
/// Removes the provider from the config, strips its entries from all groups
/// (re-indexing priorities), and deletes the stored API key from the keychain.
///
/// # Arguments
/// * `provider_id` — The id of the provider to delete.
///
/// # Errors
/// Returns an error string if loading/saving providers or groups fails, or
/// if keychain deletion fails.
#[tauri::command]
pub async fn delete_provider(provider_id: String) -> Result<(), String> {
    let mut providers = store::load_providers().map_err(|e| e.to_string())?;
    providers.retain(|p| p.id != provider_id);
    store::save_providers(&providers).map_err(|e| e.to_string())?;

    // Remove entries referencing this provider and re-index priorities
    let mut groups = store::load_groups().map_err(|e| e.to_string())?;
    for group in &mut groups {
        group.entries.retain(|e| e.provider_id != provider_id);
        for (idx, entry) in group.entries.iter_mut().enumerate() {
            entry.priority = (idx + 1) as u32;
        }
    }
    store::save_groups(&groups).map_err(|e| e.to_string())?;

    keychain::delete_credential(&provider_id)
        .await
        .map_err(|e| e.to_string())?;

    notify_sidecar_config_reload().await;
    Ok(())
}

/// Returns all configured routing groups.
///
/// # Errors
/// Returns an error string if the group configuration file cannot be read.
#[tauri::command]
pub async fn get_groups() -> Result<Vec<Group>, String> {
    store::load_groups().map_err(|e| e.to_string())
}

/// Creates or updates a routing group.
///
/// If a group with the same id already exists it is replaced; otherwise a new
/// entry is appended.
///
/// # Arguments
/// * `group` — The group configuration to save.
///
/// # Errors
/// Returns an error string if loading or saving groups fails.
#[tauri::command]
pub async fn save_group(group: Group) -> Result<(), String> {
    let mut groups = match store::load_groups() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[save_group] Warning: failed to load existing groups ({e}), starting fresh");
            Vec::new()
        }
    };

    if let Some(pos) = groups.iter().position(|g| g.id == group.id) {
        groups[pos] = group;
    } else {
        groups.push(group);
    }

    store::save_groups(&groups).map_err(|e| e.to_string())?;
    notify_sidecar_config_reload().await;
    reinject_opencode_provider_if_enabled().await;
    Ok(())
}

/// Deletes a routing group by id.
///
/// # Arguments
/// * `group_id` — The id of the group to delete.
///
/// # Errors
/// Returns an error string if loading or saving groups fails.
#[tauri::command]
pub async fn delete_group(group_id: String) -> Result<(), String> {
    let mut groups = store::load_groups().map_err(|e| e.to_string())?;
    groups.retain(|g| g.id != group_id);
    store::save_groups(&groups).map_err(|e| e.to_string())?;
    notify_sidecar_config_reload().await;
    reinject_opencode_provider_if_enabled().await;
    Ok(())
}

/// Returns the application configuration, falling back to defaults.
///
/// # Errors
/// Never — missing config files produce the default [`AppConfig`].
#[tauri::command]
pub async fn get_app_config() -> Result<AppConfig, String> {
    store::load_app_config().or_else(|_| Ok(AppConfig::default()))
}

/// Persists the application configuration and notifies the sidecar.
///
/// # Arguments
/// * `config` — The full application configuration to save.
///
/// # Errors
/// Returns an error string if saving fails.
#[tauri::command]
pub async fn save_app_config(config: AppConfig) -> Result<(), String> {
    store::save_app_config(&config).map_err(|e| e.to_string())?;
    notify_sidecar_config_reload().await;
    Ok(())
}

/// Marks the onboarding flow as dismissed so it will not reappear.
///
/// # Errors
/// Returns an error string if the config cannot be saved.
#[tauri::command]
pub async fn dismiss_onboarding() -> Result<(), String> {
    let mut config = store::load_app_config().unwrap_or_default();
    config.onboarding_dismissed = true;
    store::save_app_config(&config).map_err(|e| e.to_string())
}

/// Result of a provider connection test returned to the frontend.
#[derive(Serialize)]
pub struct TestConnectionResult {
    /// Whether the provider could be reached.
    pub success: bool,
    /// HTTP status code received, if any.
    pub status_code: Option<u16>,
    /// Human-readable description of the outcome.
    pub message: String,
}

/// Tests connectivity to a provider's API endpoint.
///
/// Attempts a GET to the provider's `/models` endpoint using the appropriate
/// authentication header for the protocol (Bearer token for OpenAI, x-api-key
/// for Anthropic). If `/models` returns 404, falls back to testing the base
/// URL itself — some providers don't expose a models listing.
///
/// # Arguments
/// * `provider_id` — The id of the provider to test.
///
/// # Errors
/// Returns an error string if the provider is not found or the API key cannot
/// be retrieved from the keychain.
#[tauri::command]
pub async fn test_provider_connection(provider_id: String) -> Result<TestConnectionResult, String> {
    let providers = store::load_providers().map_err(|e| e.to_string())?;
    let provider = providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    let api_key = keychain::get_credential(&provider.credential_key)
        .await
        .map_err(|e| format!("Failed to retrieve API key: {}", e))?;

    let client = Client::new();
    let base_url = provider.base_url.trim_end_matches('/');
    let models_url = format!("{base_url}/models");

    // Build the request with protocol-appropriate authentication
    let request = match provider.protocol.as_str() {
        "anthropic" => client
            .get(&models_url)
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2024-06-01"),
        _ => client
            .get(&models_url)
            .header("Authorization", format!("Bearer {}", api_key)),
    };

    match request.timeout(std::time::Duration::from_secs(10)).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            if resp.status().is_success() {
                Ok(TestConnectionResult {
                    success: true,
                    status_code: Some(status),
                    message: format!("Connection successful (HTTP {})", status),
                })
            } else if status == 404 {
                // /models not available — try the base URL as a connectivity check
                let base_request = match provider.protocol.as_str() {
                    "anthropic" => client
                        .get(base_url)
                        .header("x-api-key", &api_key)
                        .header("anthropic-version", "2024-06-01"),
                    _ => client
                        .get(base_url)
                        .header("Authorization", format!("Bearer {}", api_key)),
                };
                match base_request.timeout(std::time::Duration::from_secs(10)).send().await {
                    Ok(r) if r.status().as_u16() != 404 => Ok(TestConnectionResult {
                        success: true,
                        status_code: Some(r.status().as_u16()),
                        message: format!("Base URL reachable (HTTP {}) — no /models endpoint", r.status().as_u16()),
                    }),
                    _ => Ok(TestConnectionResult {
                        success: true,
                        status_code: Some(status),
                        message: "Provider reachable — no /models endpoint (OK if using model overrides)".to_string(),
                    }),
                }
            } else {
                let body = resp.text().await.unwrap_or_default();
                Ok(TestConnectionResult {
                    success: false,
                    status_code: Some(status),
                    message: format!("Connection failed (HTTP {}): {}", status, body.chars().take(200).collect::<String>()),
                })
            }
        }
        Err(e) => Ok(TestConnectionResult {
            success: false,
            status_code: None,
            message: format!("Connection failed: {}", e),
        }),
    }
}

/// Refreshes the list of models for a provider by fetching from its API.
///
/// Updates the provider's `models` field with the latest data and returns
/// the full provider list so the frontend can refresh its state.
///
/// # Arguments
/// * `provider_id` — The id of the provider whose models to refresh.
///
/// # Errors
/// Returns an error string if the provider is not found, the API key cannot
/// be retrieved, the model fetch fails, or persisting the update fails.
#[tauri::command]
pub async fn refresh_provider_models(provider_id: String) -> Result<Vec<ProviderResponse>, String> {
    let providers = store::load_providers().map_err(|e| e.to_string())?;
    let provider = providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?
        .clone();

    let api_key = keychain::get_credential(&provider.credential_key)
        .await
        .map_err(|e| format!("Failed to retrieve API key: {}", e))?;

    let client = Client::new();
    let models = coderouter_proxy::models::refresher::fetch_models_for_provider(&provider, &api_key, &client)
        .await
        .map_err(|e| e.to_string())?;

    let mut all_providers = providers;
    if let Some(existing) = all_providers.iter_mut().find(|p| p.id == provider_id) {
        existing.models = models;
    }
    store::save_providers(&all_providers).map_err(|e| e.to_string())?;

    Ok(all_providers.iter().map(|p| p.into()).collect())
}

/// Fetches the current router status from the running proxy.
///
/// Queries `/internal/router/status` on the proxy and deserializes the
/// response into a [`router::RouterStatusResponse`].
///
/// # Errors
/// Returns an error string if the proxy is unreachable or returns a
/// non-success / malformed response.
#[tauri::command]
pub async fn get_router_status() -> Result<router::RouterStatusResponse, String> {
    let config = store::load_app_config().unwrap_or_default();
    let client = Client::new();
    let url = format!("http://{}:{}/internal/router/status", config.proxy_host, config.proxy_port);
    let resp = client.get(&url).send().await
        .map_err(|e| format!("Failed to connect to proxy: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Proxy returned status {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await
        .map_err(|e| format!("Failed to parse proxy response: {}", e))?;
    if body.get("status").and_then(|v| v.as_str()) != Some("ok") {
        return Err("Proxy returned error".to_string());
    }
    let data = body.get("data").ok_or_else(|| "No data in proxy response".to_string())?;
    let status: router::RouterStatusResponse = serde_json::from_value(data.clone())
        .map_err(|e| format!("Failed to parse router status: {}", e))?;
    Ok(status)
}

/// Enables or disables a single routing entry inside a group via the proxy.
///
/// Posts the change to `/internal/router/entry` so the running proxy updates
/// its routing table without a restart.
///
/// # Arguments
/// * `group_id`    — The id of the group containing the entry.
/// * `entry_index` — Zero-based index of the entry within the group.
/// * `enabled`     — Desired enabled state.
///
/// # Errors
/// Returns an error string if the proxy is unreachable or returns an error.
#[tauri::command]
pub async fn set_entry_enabled(group_id: String, entry_index: usize, enabled: bool) -> Result<(), String> {
    let config = store::load_app_config().unwrap_or_default();
    let client = Client::new();
    let url = format!("http://{}:{}/internal/router/entry", config.proxy_host, config.proxy_port);
    let body = serde_json::json!({
        "group_id": group_id,
        "entry_index": entry_index,
        "enabled": enabled,
    });
    let resp = client.post(&url).json(&body).send().await
        .map_err(|e| format!("Failed to connect to proxy: {}", e))?;
    if !resp.status().is_success() {
        let err_text = resp.text().await.unwrap_or_default();
        return Err(format!("Proxy returned error: {}", err_text));
    }
    Ok(())
}

/// Returns a daily usage summary for a specific provider and date.
///
/// Uses the provider's `quota_reset_utc_hour` to align the UTC day boundary.
///
/// # Arguments
/// * `provider_id` — The provider to query.
/// * `date`        — Date string in `YYYY-MM-DD` format.
///
/// # Errors
/// Returns an error string if the date format is invalid or the metrics
/// database query fails.
#[tauri::command]
pub fn get_daily_summary(provider_id: String, date: String) -> Result<queries::DailySummary, String> {
    let date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid date format (expected YYYY-MM-DD): {}", e))?;
    let providers = store::load_providers().unwrap_or_default();
    let reset_hour = providers.iter()
        .find(|p| p.id == provider_id)
        .map(|p| p.quota_reset_utc_hour)
        .unwrap_or(0);
    with_metrics_db(|conn| {
        queries::get_daily_summary(conn, &provider_id, date, reset_hour)
            .map_err(|e| e.to_string())
    })
}

/// Returns the most recent request log entries.
///
/// # Arguments
/// * `limit` — Maximum number of rows to return.
///
/// # Errors
/// Returns an error string if the metrics database query fails.
#[tauri::command]
pub fn get_recent_requests(limit: usize) -> Result<Vec<queries::RequestRow>, String> {
    with_metrics_db(|conn| {
        queries::get_recent_requests(conn, limit)
            .map_err(|e| e.to_string())
    })
}

/// Returns daily aggregated usage for a provider.
///
/// Uses the provider's `quota_reset_utc_hour` to align the UTC day boundary.
///
/// # Arguments
/// * `provider_id` — The provider to query.
/// * `days`        — Number of days to look back.
///
/// # Errors
/// Returns an error string if the metrics database query fails.
#[tauri::command]
pub fn get_usage_by_day(provider_id: String, days: u32) -> Result<Vec<queries::DailyUsage>, String> {
    let providers = store::load_providers().unwrap_or_default();
    let reset_hour = providers.iter()
        .find(|p| p.id == provider_id)
        .map(|p| p.quota_reset_utc_hour)
        .unwrap_or(0);
    with_metrics_db(|conn| {
        queries::get_usage_by_day(conn, &provider_id, days, reset_hour)
            .map_err(|e| e.to_string())
    })
}

/// Returns usage aggregated by routing group.
///
/// Uses the provider's `quota_reset_utc_hour` when a `provider_id` is given,
/// otherwise defaults to hour 0.
///
/// # Arguments
/// * `days`        — Number of days to look back.
/// * `provider_id` — Optional provider filter and reset-hour source.
///
/// # Errors
/// Returns an error string if the metrics database query fails.
#[tauri::command]
pub fn get_usage_by_group(days: u32, provider_id: Option<String>) -> Result<Vec<queries::GroupUsage>, String> {
    let reset_hour = provider_id.and_then(|pid| {
        let providers = store::load_providers().ok()?;
        providers.iter()
            .find(|p| p.id == pid)
            .map(|p| p.quota_reset_utc_hour)
    }).unwrap_or(0);
    with_metrics_db(|conn| {
        queries::get_usage_by_group(conn, days, reset_hour)
            .map_err(|e| e.to_string())
    })
}

/// Returns per-model usage aggregation over a given number of days.
///
/// Uses the app config's `quota_reset_utc_hour` to align the time window,
/// defaulting to hour 0 if no config is available.
///
/// # Arguments
/// * `days` — Number of days to look back.
///
/// # Errors
/// Returns an error string if the metrics database query fails.
#[tauri::command]
pub fn get_usage_by_model(days: u32) -> Result<Vec<queries::ModelUsage>, String> {
    with_metrics_db(|conn| {
        queries::get_usage_by_model(conn, days, 0)
            .map_err(|e| e.to_string())
    })
}

/// Returns daily cost breakdown per model for chart rendering.
///
/// # Arguments
/// * `days` — Number of days to look back.
///
/// # Errors
/// Returns an error string if the metrics database query fails.
#[tauri::command]
pub fn get_daily_usage_by_model(days: u32) -> Result<Vec<queries::DailyModelUsage>, String> {
    with_metrics_db(|conn| {
        queries::get_daily_usage_by_model(conn, days, 0)
            .map_err(|e| e.to_string())
    })
}

/// Frontend representation of an OpenCode agent-to-model mapping.
///
/// Each field corresponds to an agent role whose requests should be routed to
/// a specific model. `None` means the mapping is not overridden.
#[derive(Serialize, Deserialize)]
pub struct OpenCodeAgentMapping {
    #[serde(default)]
    pub build: Option<String>,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub general: Option<String>,
    #[serde(default)]
    pub explore: Option<String>,
    #[serde(default)]
    pub compaction: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub small_model: Option<String>,
}

impl From<OpenCodeAgentMapping> for AgentMapping {
    fn from(m: OpenCodeAgentMapping) -> Self {
        AgentMapping {
            build: m.build,
            plan: m.plan,
            general: m.general,
            explore: m.explore,
            compaction: m.compaction,
            title: m.title,
            summary: m.summary,
            small_model: m.small_model,
        }
    }
}

impl From<AgentMapping> for OpenCodeAgentMapping {
    fn from(m: AgentMapping) -> Self {
        OpenCodeAgentMapping {
            build: m.build,
            plan: m.plan,
            general: m.general,
            explore: m.explore,
            compaction: m.compaction,
            title: m.title,
            summary: m.summary,
            small_model: m.small_model,
        }
    }
}

/// Fetches each routing entry's current status from the remote proxy.
///
/// Returns a map of `"provider_id:entry_index" → status` strings. Returns an
/// empty map silently if the proxy is unreachable or returns malformed data.
async fn build_entry_statuses() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let config = store::load_app_config().unwrap_or_default();
    let url = format!("http://{}:{}/internal/router/status", config.proxy_host, config.proxy_port);
    let client = Client::new();
    let resp = match client.get(&url).timeout(std::time::Duration::from_secs(2)).send().await {
        Ok(r) => r,
        Err(_) => return map,
    };
    if !resp.status().is_success() {
        return map;
    }
    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(_) => return map,
    };
    let data = match body.get("data") {
        Some(d) => d,
        None => return map,
    };
    let status: router::RouterStatusResponse = match serde_json::from_value(data.clone()) {
        Ok(s) => s,
        Err(_) => return map,
    };
    for entry in status.entries {
        let key = format!("{}:{}", entry.provider_id, entry.entry_index);
        map.insert(key, entry.status);
    }
    map
}

/// Returns the resolved file-system path of the OpenCode configuration file.
///
/// If the user has set a custom path in the app config, that path is used;
/// otherwise the default location is resolved automatically.
#[tauri::command]
pub fn get_opencode_config_path() -> Option<String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    config_writer::resolve_opencode_config_path(stored.as_deref())
        .map(|p| p.to_string_lossy().to_string())
}

/// Persists a custom OpenCode config file path.
///
/// # Arguments
/// * `path` — Absolute or relative path to the OpenCode configuration file.
///
/// # Errors
/// Returns an error string if the path cannot be saved.
#[tauri::command]
pub fn set_opencode_config_path(path: String) -> Result<(), String> {
    config_writer::save_opencode_config_path(&path)
        .map_err(|e| e.to_string())
}

/// Injects the CodeRouter provider into the OpenCode configuration file.
///
/// Reads the current groups and providers, fetches live entry statuses from
/// the proxy, and writes the combined configuration so OpenCode routes its
/// requests through CodeRouter.
///
/// # Arguments
/// * `proxy_port` — The local port on which CodeRouter is listening.
///
/// # Errors
/// Returns an error string if the config path cannot be resolved, the proxy
/// is unreachable, or the config file cannot be written.
#[tauri::command]
pub async fn inject_opencode_provider(proxy_port: u16) -> Result<(), String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    let config_path = config_writer::resolve_opencode_config_path(stored.as_deref())
        .ok_or_else(|| "Could not determine OpenCode config path".to_string())?;

    let groups = store::load_groups().unwrap_or_default();
    let providers = store::load_providers().unwrap_or_default();
    let entry_statuses = build_entry_statuses().await;

    config_writer::inject_provider(&config_path, &groups, &providers, proxy_port, &entry_statuses)
        .map_err(|e| e.to_string())
}

/// Removes the CodeRouter provider from the OpenCode configuration file.
///
/// # Errors
/// Returns an error string if the config path cannot be resolved or the file
/// cannot be modified.
#[tauri::command]
pub fn remove_opencode_provider() -> Result<(), String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    let config_path = config_writer::resolve_opencode_config_path(stored.as_deref())
        .ok_or_else(|| "Could not determine OpenCode config path".to_string())?;

    config_writer::remove_provider(&config_path)
        .map_err(|e| e.to_string())
}

/// Writes agent-specific model overrides into the OpenCode configuration.
///
/// # Arguments
/// * `mapping` — The agent-to-model mapping to apply.
///
/// # Errors
/// Returns an error string if the config path cannot be resolved or the file
/// cannot be modified.
#[tauri::command]
pub fn set_opencode_agent_models(mapping: OpenCodeAgentMapping) -> Result<(), String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    let config_path = config_writer::resolve_opencode_config_path(stored.as_deref())
        .ok_or_else(|| "Could not determine OpenCode config path".to_string())?;

    config_writer::set_agent_models(&config_path, &mapping.into())
        .map_err(|e| e.to_string())
}

/// Removes all agent-specific model overrides from the OpenCode configuration.
///
/// # Errors
/// Returns an error string if the config path cannot be resolved or the file
/// cannot be modified.
#[tauri::command]
pub fn remove_opencode_agent_models() -> Result<(), String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    let config_path = config_writer::resolve_opencode_config_path(stored.as_deref())
        .ok_or_else(|| "Could not determine OpenCode config path".to_string())?;

    config_writer::remove_agent_models(&config_path)
        .map_err(|e| e.to_string())
}

/// Reads the current agent-to-model mapping from the OpenCode configuration.
///
/// # Errors
/// Returns an error string if the config path cannot be resolved or the file
/// cannot be parsed.
#[tauri::command]
pub fn get_opencode_agent_models() -> Result<OpenCodeAgentMapping, String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    let config_path = config_writer::resolve_opencode_config_path(stored.as_deref())
        .ok_or_else(|| "Could not determine OpenCode config path".to_string())?;

    config_writer::get_current_agent_mapping(&config_path)
        .map(|m| m.into())
        .map_err(|e| e.to_string())
}

/// Generates a preview of what the OpenCode config would look like after injection.
///
/// Does **not** modify any files — purely for frontend display.
///
/// # Arguments
/// * `proxy_port` — The local port on which CodeRouter is listening.
/// * `mapping`     — Optional agent model overrides to include in the preview.
///
/// # Errors
/// Returns an error string if the config cannot be generated.
#[tauri::command]
pub async fn preview_opencode_config(proxy_port: u16, mapping: Option<OpenCodeAgentMapping>) -> Result<String, String> {
    let groups = store::load_groups().unwrap_or_default();
    let providers = store::load_providers().unwrap_or_default();
    let entry_statuses = build_entry_statuses().await;

    let agent_mapping = mapping.map(|m| m.into());

    config_writer::preview_opencode_config(&groups, &providers, proxy_port, agent_mapping.as_ref(), &entry_statuses)
        .map_err(|e| e.to_string())
}

/// Checks whether a given group alias is referenced in the OpenCode config.
///
/// Used by the frontend to warn the user before deleting a group that is
/// actively referenced.
///
/// # Arguments
/// * `group_alias` — The alias to check.
#[tauri::command]
pub fn is_group_referenced_in_opencode(group_alias: String) -> bool {
    config_writer::is_group_alias_referenced(&group_alias)
}

/// Returns latency percentile statistics for a provider on a given date.
///
/// Uses the provider's `quota_reset_utc_hour` to align the UTC day boundary.
///
/// # Arguments
/// * `provider_id` — The provider to query.
/// * `date`        — Date string in `YYYY-MM-DD` format.
///
/// # Errors
/// Returns an error string if the date format is invalid or the query fails.
#[tauri::command]
pub fn get_latency_percentiles(provider_id: String, date: String) -> Result<Option<queries::LatencyPercentiles>, String> {
    let date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid date format (expected YYYY-MM-DD): {}", e))?;
    let providers = store::load_providers().unwrap_or_default();
    let reset_hour = providers.iter()
        .find(|p| p.id == provider_id)
        .map(|p| p.quota_reset_utc_hour)
        .unwrap_or(0);
    with_metrics_db(|conn| {
        queries::get_latency_percentiles(conn, &provider_id, date, reset_hour)
            .map_err(|e| e.to_string())
    })
}

/// Returns the total estimated cost for a provider over a given number of days.
///
/// Uses the provider's `quota_reset_utc_hour` to align the UTC day boundary.
///
/// # Arguments
/// * `provider_id` — The provider to query.
/// * `days`        — Number of days to look back.
///
/// # Errors
/// Returns an error string if the metrics database query fails.
#[tauri::command]
pub fn get_cost_summary(provider_id: String, days: u32) -> Result<f64, String> {
    let providers = store::load_providers().unwrap_or_default();
    let reset_hour = providers.iter()
        .find(|p| p.id == provider_id)
        .map(|p| p.quota_reset_utc_hour)
        .unwrap_or(0);
    with_metrics_db(|conn| {
        queries::get_cost_summary(conn, &provider_id, days, reset_hour)
            .map_err(|e| e.to_string())
    })
}

/// Returns the application version as defined in `Cargo.toml`.
#[tauri::command]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Removes all CodeRouter-related configuration from the OpenCode config.
///
/// This removes both the provider entry and any agent model overrides,
/// restoring the OpenCode config to its pre-CodeRouter state.
///
/// # Errors
/// Returns an error string if the config path cannot be resolved or the file
/// cannot be modified.
#[tauri::command]
pub fn remove_coderouter_from_opencode() -> Result<(), String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    let config_path = config_writer::resolve_opencode_config_path(stored.as_deref())
        .ok_or_else(|| "Could not determine OpenCode config path".to_string())?;

    config_writer::remove_provider(&config_path).map_err(|e| e.to_string())?;
    config_writer::remove_agent_models(&config_path).map_err(|e| e.to_string())
}

/// Result of a proxy health check returned to the frontend.
#[derive(Serialize)]
pub struct HealthCheckResult {
    /// Whether the proxy process is responding to health checks.
    pub running: bool,
    /// The `"status"` field from the health endpoint, if present.
    pub status: Option<String>,
    /// Proxy uptime in seconds, if reported.
    pub uptime_seconds: Option<u64>,
}

async fn reinject_opencode_provider_if_enabled() {
    let stored = match store::load_app_config() {
        Ok(c) => c.opencode_config_path,
        Err(_) => return,
    };
    let config_path = match config_writer::resolve_opencode_config_path(stored.as_deref()) {
        Some(p) => p,
        None => return,
    };

    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let config: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    let has_coderouter = config
        .get("provider")
        .and_then(|p| p.get("coderouter"))
        .is_some();

    if !has_coderouter {
        return;
    }

    let groups = store::load_groups().unwrap_or_default();
    let providers = store::load_providers().unwrap_or_default();
    let app_config = store::load_app_config().ok().unwrap_or_default();
    let entry_statuses = build_entry_statuses().await;

    let _ = config_writer::inject_provider(
        &config_path, &groups, &providers, app_config.proxy_port, &entry_statuses,
    );
}

/// Checks whether the proxy sidecar is healthy by querying its `/health` endpoint.
///
/// Times out after 3 seconds to avoid blocking the UI.
///
/// # Errors
/// Never returns `Err` — an unreachable proxy is reported as
/// `HealthCheckResult { running: false, … }`.
#[tauri::command]
pub async fn check_proxy_health() -> Result<HealthCheckResult, String> {
    let config = store::load_app_config().unwrap_or_default();
    let host = config.proxy_host;
    let port = config.proxy_port;
    let url = format!("http://{}:{}/health", host, port);
    let client = Client::new();

    // Timeout prevents the UI from hanging when the proxy is unresponsive
    let controller = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        client.get(&url).send().await
    }).await;

    match controller {
        Ok(Ok(resp)) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            Ok(HealthCheckResult {
                running: true,
                status: body.get("status").and_then(|v| v.as_str()).map(|s| s.to_string()),
                uptime_seconds: body.get("uptime_seconds").and_then(|v| v.as_u64()),
            })
        }
        _ => Ok(HealthCheckResult {
            running: false,
            status: None,
            uptime_seconds: None,
        }),
    }
}

/// Deletes all metrics data from the database.
///
/// # Errors
/// Returns an error string if the database operation fails.
#[tauri::command]
pub fn clear_metrics_data() -> Result<(), String> {
    db::clear_metrics().map_err(|e| e.to_string())
}

/// Resets all CodeRouter configuration (providers, groups, app config) to defaults.
///
/// # Errors
/// Returns an error string if the configuration files cannot be written.
#[tauri::command]
pub fn reset_all_config() -> Result<(), String> {
    store::reset_all_config().map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub available: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub release_notes: Option<String>,
}

#[tauri::command]
pub async fn check_for_updates(app: tauri::AppHandle) -> Result<UpdateStatus, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;
    match update {
        Some(update) => Ok(UpdateStatus {
            available: true,
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            latest_version: Some(update.version.clone()),
            release_notes: update.body.clone(),
        }),
        None => Ok(UpdateStatus {
            available: false,
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            latest_version: None,
            release_notes: None,
        }),
    }
}

#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;
    match update {
        Some(update) => {
            update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
            app.restart();
            #[allow(unreachable_code)]
            Ok(())
        }
        None => Err("No update available".to_string()),
    }
}

/// Global application state managed by Tauri.
///
/// Holds references to the sidecar process, proxy running flag, and tray menu
/// items so commands and tray event handlers can coordinate.
pub struct AppState {
    /// Handle to the Tauri application for emitting events and updating UI.
    pub app_handle: tauri::AppHandle,
    /// The child process of the running proxy sidecar, if any.
    pub sidecar: Mutex<Option<Child>>,
    /// Whether the proxy health endpoint is currently responding.
    pub proxy_running: Mutex<bool>,
    /// Tray menu item showing "Proxy: Running" / "Proxy: Stopped".
    pub proxy_status_item: MenuItem<tauri::Wry>,
    /// Tray menu item showing "Start Proxy" / "Stop Proxy".
    pub toggle_proxy_item: MenuItem<tauri::Wry>,
}

/// Sends SIGTERM to the sidecar process and waits up to 5 seconds for it to
/// exit. Falls back to `kill()` if the process hasn't exited by then.
///
/// # Arguments
/// * `child` — Mutable reference to the sidecar [`Child`] process.
pub fn kill_sidecar(child: &mut Child) {
    let pid = child.id() as i32;
    // Send SIGTERM for a graceful shutdown first
    let _ = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGTERM,
    );
    // Wait up to 5 seconds for the process to exit gracefully
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(_) => break,
        }
    }
    // Force kill if it didn't exit gracefully
    let _ = child.kill();
    let _ = child.wait();
}

/// Builds the target-specific binary name for the proxy sidecar.
///
/// Produces a string like `coderouter-proxy-x86_64-apple-darwin` based on the
/// current architecture and OS so the correct binary is located at runtime.
fn sidecar_target_suffix() -> String {
    let arch = std::env::consts::ARCH;
    let triple = if cfg!(target_os = "macos") {
        format!("{arch}-apple-darwin")
    } else if cfg!(target_os = "windows") {
        format!("{arch}-pc-windows-msvc")
    } else {
        format!("{arch}-unknown-linux-gnu")
    };
    format!("coderouter-proxy-{triple}")
}

/// Locates and spawns the proxy sidecar process.
///
/// In debug builds, looks in `src-tauri/sidecar/` next to the manifest directory.
/// In release builds, checks the AppImage `APPDIR` environment variable first
/// (for Linux AppImage packaging), then falls back to a `sidecar/` directory
/// next to the main binary, and finally tries the system `PATH`.
///
/// # Returns
/// A [`Child`] handle to the spawned process on success.
///
/// # Errors
/// Returns an error string if the sidecar binary cannot be found or spawned.
pub fn spawn_sidecar() -> Result<Child, String> {
    let target_suffix = sidecar_target_suffix();
    let sidecar_path = if cfg!(debug_assertions) {
        // Debug: look for the sidecar binary next to the crate manifest
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let dev_path = std::path::Path::new(manifest_dir)
            .join(format!("sidecar/{target_suffix}"));
        if dev_path.exists() {
            dev_path
        } else {
            let fallback = std::path::Path::new(manifest_dir)
                .join("sidecar/coderouter-proxy");
            if fallback.exists() {
                fallback
            } else {
                std::path::PathBuf::from("coderouter-proxy")
            }
        }
    } else {
        // Release: check AppImage layout first, then fall back
        if let Ok(appdir) = std::env::var("APPDIR") {
            let appimage_sidecar = std::path::Path::new(&appdir)
                .join(format!("usr/bin/sidecar/{target_suffix}"));
            if appimage_sidecar.exists() {
                appimage_sidecar
            } else {
                find_sidecar_fallback()
            }
        } else {
            find_sidecar_fallback()
        }
    };

    std::process::Command::new(&sidecar_path)
        .spawn()
        .map_err(|e| format!("Failed to spawn sidecar {}: {}", sidecar_path.display(), e))
}

/// Searches for the sidecar binary in common release directories.
///
/// Looks next to the running executable in a `sidecar/` subdirectory, then
/// falls back to assuming `coderouter-proxy` is on the system `PATH`.
fn find_sidecar_fallback() -> std::path::PathBuf {
    // Try release layout: sidecar/ next to the main binary
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let release_path = parent.join("sidecar/coderouter-proxy");
            if release_path.exists() {
                return release_path;
            }
        }
    }
    // Last resort: assume it's in PATH
    std::path::PathBuf::from("coderouter-proxy")
}

/// Restarts the proxy sidecar process.
///
/// Kills the existing sidecar (if any), spawns a new one, and updates the tray
/// icon and labels to reflect the running state.
///
/// # Errors
/// Returns an error string if the mutex is poisoned or spawning fails.
#[tauri::command]
pub fn restart_proxy(state: tauri::State<AppState>) -> Result<(), String> {
    let mut sidecar_guard = state.sidecar.lock().map_err(|e| e.to_string())?;
    // Kill the existing sidecar process, if one is running
    if let Some(child) = sidecar_guard.as_mut() {
        kill_sidecar(child);
    }
    *sidecar_guard = None;
    *state.proxy_running.lock().map_err(|e| e.to_string())? = false;

    // Spawn a fresh sidecar process
    let child = spawn_sidecar()?;
    *sidecar_guard = Some(child);
    *state.proxy_running.lock().map_err(|e| e.to_string())? = true;

    crate::update_tray_icon(&state.app_handle, true);
    crate::update_menu_labels(&state, true);

    Ok(())
}

// ─── Custom Agent Management ───────────────────────────────────────────────

use coderouter_proxy::opencode::custom_agents::{
    self, AgentMode, AgentPermissions, BashPermission, CustomAgent, PermissionLevel,
};

/// JSON-serializable permission level.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum PermissionLevelResponse {
    Allow,
    Deny,
    Ask,
}

impl From<PermissionLevel> for PermissionLevelResponse {
    fn from(p: PermissionLevel) -> Self {
        match p {
            PermissionLevel::Allow => PermissionLevelResponse::Allow,
            PermissionLevel::Deny => PermissionLevelResponse::Deny,
            PermissionLevel::Ask => PermissionLevelResponse::Ask,
        }
    }
}

impl From<PermissionLevelResponse> for PermissionLevel {
    fn from(p: PermissionLevelResponse) -> Self {
        match p {
            PermissionLevelResponse::Allow => PermissionLevel::Allow,
            PermissionLevelResponse::Deny => PermissionLevel::Deny,
            PermissionLevelResponse::Ask => PermissionLevel::Ask,
        }
    }
}

/// JSON-serializable bash permission.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum BashPermissionResponse {
    Simple(PermissionLevelResponse),
    Commands(HashMap<String, PermissionLevelResponse>),
}

impl From<BashPermission> for BashPermissionResponse {
    fn from(p: BashPermission) -> Self {
        match p {
            BashPermission::Simple(level) => BashPermissionResponse::Simple(level.into()),
            BashPermission::Commands(map) => {
                BashPermissionResponse::Commands(map.into_iter().map(|(k, v)| (k, v.into())).collect())
            }
        }
    }
}

impl From<BashPermissionResponse> for BashPermission {
    fn from(p: BashPermissionResponse) -> Self {
        match p {
            BashPermissionResponse::Simple(level) => BashPermission::Simple(level.into()),
            BashPermissionResponse::Commands(map) => {
                BashPermission::Commands(map.into_iter().map(|(k, v)| (k, v.into())).collect())
            }
        }
    }
}

/// JSON-serializable agent permissions.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct AgentPermissionsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit: Option<PermissionLevelResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bash: Option<BashPermissionResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webfetch: Option<PermissionLevelResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<HashMap<String, PermissionLevelResponse>>,
}

impl From<AgentPermissions> for AgentPermissionsResponse {
    fn from(p: AgentPermissions) -> Self {
        AgentPermissionsResponse {
            edit: p.edit.map(|l| l.into()),
            bash: p.bash.map(|b| b.into()),
            webfetch: p.webfetch.map(|l| l.into()),
            task: p.task.map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect()),
        }
    }
}

impl From<AgentPermissionsResponse> for AgentPermissions {
    fn from(p: AgentPermissionsResponse) -> Self {
        AgentPermissions {
            edit: p.edit.map(|l| l.into()),
            bash: p.bash.map(|b| b.into()),
            webfetch: p.webfetch.map(|l| l.into()),
            task: p.task.map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect()),
        }
    }
}

/// JSON-serializable agent mode.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentModeResponse {
    Primary,
    #[default]
    Subagent,
    All,
}

impl From<AgentMode> for AgentModeResponse {
    fn from(m: AgentMode) -> Self {
        match m {
            AgentMode::Primary => AgentModeResponse::Primary,
            AgentMode::Subagent => AgentModeResponse::Subagent,
            AgentMode::All => AgentModeResponse::All,
        }
    }
}

impl From<AgentModeResponse> for AgentMode {
    fn from(m: AgentModeResponse) -> Self {
        match m {
            AgentModeResponse::Primary => AgentMode::Primary,
            AgentModeResponse::Subagent => AgentMode::Subagent,
            AgentModeResponse::All => AgentMode::All,
        }
    }
}

/// JSON-serializable custom agent returned to the frontend.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CustomAgentResponse {
    pub name: String,
    pub description: String,
    pub mode: AgentModeResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, rename = "topP", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, rename = "permissions", skip_serializing_if = "Option::is_none")]
    pub permission: Option<AgentPermissionsResponse>,
    #[serde(default)]
    pub additional: HashMap<String, serde_json::Value>,
}

impl From<CustomAgent> for CustomAgentResponse {
    fn from(a: CustomAgent) -> Self {
        CustomAgentResponse {
            name: a.name,
            description: a.description,
            mode: a.mode.into(),
            model: a.model,
            prompt: a.prompt,
            temperature: a.temperature,
            steps: a.steps,
            disable: a.disable,
            hidden: a.hidden,
            color: a.color,
            top_p: a.top_p,
            permission: a.permission.map(|p| p.into()),
            additional: a.additional,
        }
    }
}

impl From<CustomAgentResponse> for CustomAgent {
    fn from(a: CustomAgentResponse) -> Self {
        CustomAgent {
            name: a.name,
            description: a.description,
            mode: a.mode.into(),
            model: a.model,
            prompt: a.prompt,
            temperature: a.temperature,
            steps: a.steps,
            disable: a.disable,
            hidden: a.hidden,
            color: a.color,
            top_p: a.top_p,
            permission: a.permission.map(|p| p.into()),
            additional: a.additional,
        }
    }
}

/// JSON-serializable template agent (pre-filled config without name).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TemplateAgentResponse {
    pub description: String,
    pub mode: AgentModeResponse,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, rename = "topP", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, rename = "permissions", skip_serializing_if = "Option::is_none")]
    pub permission: Option<AgentPermissionsResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable: Option<bool>,
    #[serde(default)]
    pub additional: HashMap<String, serde_json::Value>,
}

/// JSON-serializable agent template.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AgentTemplateResponse {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub agent: TemplateAgentResponse,
}

impl From<custom_agents::AgentTemplate> for AgentTemplateResponse {
    fn from(t: custom_agents::AgentTemplate) -> Self {
        AgentTemplateResponse {
            id: t.id,
            name: t.name,
            description: t.description,
            icon: t.icon,
            agent: TemplateAgentResponse {
                description: t.agent.description,
                mode: t.agent.mode.into(),
                prompt: t.agent.prompt,
                temperature: t.agent.temperature,
                steps: t.agent.steps,
                hidden: t.agent.hidden,
                color: t.agent.color,
                top_p: t.agent.top_p,
                permission: t.agent.permission.map(|p| p.into()),
                model: t.agent.model,
                disable: t.agent.disable,
                additional: t.agent.additional,
            },
        }
    }
}

/// Lists all custom agents stored as markdown files.
#[tauri::command]
pub async fn list_custom_agents() -> Result<Vec<CustomAgentResponse>, String> {
    tokio::task::spawn_blocking(|| {
        custom_agents::list_agents()
            .map(|agents| agents.into_iter().map(|a| a.into()).collect())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Creates a new custom agent from the given configuration.
#[tauri::command]
pub async fn create_custom_agent(agent: CustomAgentResponse) -> Result<CustomAgentResponse, String> {
    tokio::task::spawn_blocking(move || {
        let agent: CustomAgent = agent.into();
        let path = custom_agents::create_agent(&agent).map_err(|e| e.to_string())?;
        custom_agents::parse_agent_file(&path)
            .map(|a| a.into())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Updates an existing custom agent by name.
#[tauri::command]
pub async fn update_custom_agent(name: String, agent: CustomAgentResponse) -> Result<CustomAgentResponse, String> {
    tokio::task::spawn_blocking(move || {
        let agent: CustomAgent = agent.into();
        let path = custom_agents::update_agent(&name, &agent).map_err(|e| e.to_string())?;
        custom_agents::parse_agent_file(&path)
            .map(|a| a.into())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Deletes a custom agent by name.
#[tauri::command]
pub async fn delete_custom_agent(name: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        custom_agents::delete_agent(&name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Returns the built-in agent templates available for selection.
#[tauri::command]
pub fn get_agent_templates() -> Result<Vec<AgentTemplateResponse>, String> {
    Ok(custom_agents::get_templates().into_iter().map(|t| t.into()).collect())
}

/// Request body for AI enhancement.
#[derive(Deserialize)]
pub struct AgentEnhanceRequest {
    /// The text to enhance (description or prompt).
    pub text: String,
    #[serde(rename = "enhanceType")]
    pub enhance_type: String,
    #[serde(rename = "modelGroup")]
    pub model_group: String,
}

/// Result of an AI enhancement request.
#[derive(Serialize)]
pub struct AgentEnhanceResponse {
    /// The enhanced text or suggestion JSON string.
    pub result: String,
}

/// Uses the proxy's chat completion endpoint to enhance agent text or suggest settings.
///
/// Sends a system prompt to the proxy asking it to improve the given text
/// or provide configuration suggestions.
#[tauri::command]
pub async fn enhance_agent_text(request: AgentEnhanceRequest) -> Result<AgentEnhanceResponse, String> {
    if request.model_group.trim().is_empty() {
        return Err("model_group must not be empty".to_string());
    }

    let app_config = store::load_app_config().unwrap_or_default();
    let proxy_url = format!(
        "http://{}:{}/v1/chat/completions",
        app_config.proxy_host, app_config.proxy_port
    );

    let system_prompt = match request.enhance_type.as_str() {
        "description" => {
            r#"You are an expert at writing clear, concise descriptions for AI coding agents.
Improve the given description to be more specific about what the agent does and when to use it.
Keep it to 1-2 sentences. Return ONLY the improved description text, no explanations."#
        }
        "prompt" => {
            r#"You are an expert at writing effective system prompts for AI coding agents.
Improve the given prompt to be more structured, specific, and actionable.
Use clear sections and bullet points where appropriate.
Return ONLY the improved prompt text, no explanations."#
        }
        "suggestions" => {
            r#"You are an expert at configuring AI coding agents.
Based on the following agent configuration, suggest optimal settings.
Return a JSON object with these optional keys: "temperature" (0.0-1.0), "edit_permission" ("allow"/"deny"/"ask"), "bash_permission" ("allow"/"deny"/"ask"), "webfetch_permission" ("allow"/"deny"/"ask").
Only include keys where you have a specific recommendation.
Return ONLY the JSON object, no explanations."#
        }
        _ => return Err(format!("Unknown enhance_type: {}", request.enhance_type)),
    };

    let user_content = request.text;

    let model = request.model_group.strip_prefix("coderouter/")
        .unwrap_or(&request.model_group);

    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_content}
        ],
        "temperature": 0.3,
        "max_tokens": 2000
    });

    let client = Client::new();
    let resp = client
        .post(&proxy_url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Failed to call proxy: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_text = resp.text().await.unwrap_or_default();
        return Err(format!("Proxy returned {}: {}", status, err_text.chars().take(200).collect::<String>()));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse proxy response: {}", e))?;

    let content = json
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("No response content")
        .to_string();

    Ok(AgentEnhanceResponse { result: content })
}

#[cfg(test)]
mod tests {
    use super::*;
    use coderouter_proxy::config::models::{FailoverConfig, GroupEntry, ProviderModel};

    fn test_provider(id: &str) -> Provider {
        Provider {
            id: id.to_string(),
            name: id.to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.test.com/v1".to_string(),
            credential_key: id.to_string(),
            daily_token_quota: None,
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![ProviderModel {
                id: "test-model".to_string(),
                context_window: Some(128000),
                max_output_tokens: Some(8192),
                input_cost_per_1m: Some(1.0),
                output_cost_per_1m: Some(2.0),
                last_refreshed: Some("2026-04-07T00:00:00Z".to_string()),
                protocol: None,
            }],
            model_overrides: None,
        }
    }

    #[test]
    fn test_provider_response_from_provider() {
        let provider = test_provider("p1");
        let response: ProviderResponse = (&provider).into();
        assert_eq!(response.id, "p1");
        assert_eq!(response.name, "p1");
        assert_eq!(response.protocol, "openai");
        assert_eq!(response.base_url, "https://api.test.com/v1");
        assert_eq!(response.enabled, true);
        assert_eq!(response.models.len(), 1);
        assert_eq!(response.models[0].id, "test-model");
        assert_eq!(response.models[0].context_window, Some(128000));
    }

    #[test]
    fn test_provider_response_with_quota() {
        let mut provider = test_provider("p1");
        provider.daily_token_quota = Some(500_000);
        provider.quota_reset_utc_hour = 6;
        let response: ProviderResponse = (&provider).into();
        assert_eq!(response.daily_token_quota, Some(500_000));
        assert_eq!(response.quota_reset_utc_hour, 6);
    }

    #[test]
    fn test_open_code_agent_mapping_from_mapping() {
        let input = OpenCodeAgentMapping {
            build: Some("glm-5-router".to_string()),
            plan: Some("fast-model".to_string()),
            general: None,
            explore: None,
            compaction: None,
            title: None,
            summary: None,
            small_model: Some("small-model".to_string()),
        };
        let mapping: AgentMapping = input.into();
        assert_eq!(mapping.build, Some("glm-5-router".to_string()));
        assert_eq!(mapping.plan, Some("fast-model".to_string()));
        assert_eq!(mapping.general, None);
        assert_eq!(mapping.small_model, Some("small-model".to_string()));
    }

    #[test]
    fn test_open_code_agent_mapping_all_none() {
        let input = OpenCodeAgentMapping {
            build: None,
            plan: None,
            general: None,
            explore: None,
            compaction: None,
            title: None,
            summary: None,
            small_model: None,
        };
        let mapping: AgentMapping = input.into();
        assert_eq!(mapping.build, None);
        assert_eq!(mapping.plan, None);
        assert_eq!(mapping.small_model, None);
    }

    #[test]
    fn test_open_code_agent_mapping_all_fields() {
        let input = OpenCodeAgentMapping {
            build: Some("a".to_string()),
            plan: Some("b".to_string()),
            general: Some("c".to_string()),
            explore: Some("d".to_string()),
            compaction: Some("e".to_string()),
            title: Some("f".to_string()),
            summary: Some("g".to_string()),
            small_model: Some("h".to_string()),
        };
        let mapping: AgentMapping = input.into();
        assert_eq!(mapping.build, Some("a".to_string()));
        assert_eq!(mapping.plan, Some("b".to_string()));
        assert_eq!(mapping.general, Some("c".to_string()));
        assert_eq!(mapping.explore, Some("d".to_string()));
        assert_eq!(mapping.compaction, Some("e".to_string()));
        assert_eq!(mapping.title, Some("f".to_string()));
        assert_eq!(mapping.summary, Some("g".to_string()));
        assert_eq!(mapping.small_model, Some("h".to_string()));
    }

    #[test]
    fn test_test_connection_result_serialization() {
        let result = TestConnectionResult {
            success: true,
            status_code: Some(200),
            message: "Connection successful (HTTP 200)".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("success"));
        assert!(json.contains("true"));
        assert!(json.contains("200"));
    }

    #[test]
    fn test_test_connection_result_failure() {
        let result = TestConnectionResult {
            success: false,
            status_code: Some(401),
            message: "Connection failed (HTTP 401)".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("false"));
        assert!(json.contains("401"));
    }

    #[test]
    fn test_test_connection_result_no_status_code() {
        let result = TestConnectionResult {
            success: false,
            status_code: None,
            message: "Connection failed: network error".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("null"));
    }

    #[tokio::test]
    async fn test_get_providers_returns_empty_when_no_config() {
        let result = get_providers().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_groups_returns_empty_when_no_config() {
        let result = get_groups().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_app_config_returns_defaults_when_no_config() {
        let result = get_app_config().await;
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.proxy_port, 4141);
        assert_eq!(config.proxy_host, "127.0.0.1");
        assert_eq!(config.refresh_interval_hours, 24);
    }

    #[test]
    fn test_daily_summary_date_parsing() {
        let date = NaiveDate::parse_from_str("2026-04-08", "%Y-%m-%d");
        assert!(date.is_ok());

        let bad_date = NaiveDate::parse_from_str("04-08-2026", "%Y-%m-%d");
        assert!(bad_date.is_err());

        let bad_format = NaiveDate::parse_from_str("not-a-date", "%Y-%m-%d");
        assert!(bad_format.is_err());
    }

    #[tokio::test]
    async fn test_build_entry_statuses_returns_empty_when_sidecar_unavailable() {
        let statuses = build_entry_statuses().await;
        assert!(statuses.is_empty());
    }
}