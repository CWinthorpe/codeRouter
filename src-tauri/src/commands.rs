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
use std::process::Child;
use std::sync::Mutex;
use tauri::menu::MenuItem;

static METRICS_DB: Mutex<Option<Connection>> = Mutex::new(None);

pub fn init_metrics_db() -> Result<(), String> {
    let conn = db::init_db().map_err(|e| e.to_string())?;
    let mut guard = METRICS_DB.lock().map_err(|e| e.to_string())?;
    *guard = Some(conn);
    Ok(())
}

fn with_metrics_db<T, F: FnOnce(&Connection) -> Result<T, String>>(f: F) -> Result<T, String> {
    let guard = METRICS_DB.lock().map_err(|e| e.to_string())?;
    let conn = guard.as_ref().ok_or_else(|| "Metrics database not initialized".to_string())?;
    f(conn)
}

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

#[tauri::command]
pub async fn get_providers() -> Result<Vec<ProviderResponse>, String> {
    let providers = store::load_providers().map_err(|e| e.to_string())?;
    Ok(providers.iter().map(|p| p.into()).collect())
}

async fn notify_sidecar_config_reload() {
    let config = store::load_app_config().unwrap_or_default();
    let url = format!("http://{}:{}/internal/config/reload", config.proxy_host, config.proxy_port);
    let client = Client::new();

    for attempt in 0..3 {
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

    if !api_key.is_empty() {
        keychain::store_credential(&provider.id, &api_key)
            .await
            .map_err(|e| e.to_string())?;
    }

    notify_sidecar_config_reload().await;

    Ok(())
}

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

#[tauri::command]
pub async fn delete_provider(provider_id: String) -> Result<(), String> {
    let mut providers = store::load_providers().map_err(|e| e.to_string())?;
    providers.retain(|p| p.id != provider_id);
    store::save_providers(&providers).map_err(|e| e.to_string())?;

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

#[tauri::command]
pub async fn get_groups() -> Result<Vec<Group>, String> {
    store::load_groups().map_err(|e| e.to_string())
}

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
    Ok(())
}

#[tauri::command]
pub async fn delete_group(group_id: String) -> Result<(), String> {
    let mut groups = store::load_groups().map_err(|e| e.to_string())?;
    groups.retain(|g| g.id != group_id);
    store::save_groups(&groups).map_err(|e| e.to_string())?;
    notify_sidecar_config_reload().await;
    Ok(())
}

#[tauri::command]
pub async fn get_app_config() -> Result<AppConfig, String> {
    store::load_app_config().or_else(|_| Ok(AppConfig::default()))
}

#[tauri::command]
pub async fn save_app_config(config: AppConfig) -> Result<(), String> {
    store::save_app_config(&config).map_err(|e| e.to_string())?;
    notify_sidecar_config_reload().await;
    Ok(())
}

#[tauri::command]
pub async fn dismiss_onboarding() -> Result<(), String> {
    let mut config = store::load_app_config().unwrap_or_default();
    config.onboarding_dismissed = true;
    store::save_app_config(&config).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct TestConnectionResult {
    pub success: bool,
    pub status_code: Option<u16>,
    pub message: String,
}

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

#[tauri::command]
pub fn get_recent_requests(limit: usize) -> Result<Vec<queries::RequestRow>, String> {
    with_metrics_db(|conn| {
        queries::get_recent_requests(conn, limit)
            .map_err(|e| e.to_string())
    })
}

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

#[tauri::command]
pub fn get_opencode_config_path() -> Option<String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    config_writer::resolve_opencode_config_path(stored.as_deref())
        .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub fn set_opencode_config_path(path: String) -> Result<(), String> {
    config_writer::save_opencode_config_path(&path)
        .map_err(|e| e.to_string())
}

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

#[tauri::command]
pub fn remove_opencode_provider() -> Result<(), String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    let config_path = config_writer::resolve_opencode_config_path(stored.as_deref())
        .ok_or_else(|| "Could not determine OpenCode config path".to_string())?;

    config_writer::remove_provider(&config_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_opencode_agent_models(mapping: OpenCodeAgentMapping) -> Result<(), String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    let config_path = config_writer::resolve_opencode_config_path(stored.as_deref())
        .ok_or_else(|| "Could not determine OpenCode config path".to_string())?;

    config_writer::set_agent_models(&config_path, &mapping.into())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_opencode_agent_models() -> Result<(), String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    let config_path = config_writer::resolve_opencode_config_path(stored.as_deref())
        .ok_or_else(|| "Could not determine OpenCode config path".to_string())?;

    config_writer::remove_agent_models(&config_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_opencode_agent_models() -> Result<OpenCodeAgentMapping, String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    let config_path = config_writer::resolve_opencode_config_path(stored.as_deref())
        .ok_or_else(|| "Could not determine OpenCode config path".to_string())?;

    config_writer::get_current_agent_mapping(&config_path)
        .map(|m| m.into())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn preview_opencode_config(proxy_port: u16, mapping: Option<OpenCodeAgentMapping>) -> Result<String, String> {
    let groups = store::load_groups().unwrap_or_default();
    let providers = store::load_providers().unwrap_or_default();
    let entry_statuses = build_entry_statuses().await;

    let agent_mapping = mapping.map(|m| m.into());

    config_writer::preview_opencode_config(&groups, &providers, proxy_port, agent_mapping.as_ref(), &entry_statuses)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn is_group_referenced_in_opencode(group_alias: String) -> bool {
    config_writer::is_group_alias_referenced(&group_alias)
}

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

#[tauri::command]
pub fn remove_coderouter_from_opencode() -> Result<(), String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    let config_path = config_writer::resolve_opencode_config_path(stored.as_deref())
        .ok_or_else(|| "Could not determine OpenCode config path".to_string())?;

    config_writer::remove_provider(&config_path).map_err(|e| e.to_string())?;
    config_writer::remove_agent_models(&config_path).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct HealthCheckResult {
    pub running: bool,
    pub status: Option<String>,
    pub uptime_seconds: Option<u64>,
}

#[tauri::command]
pub async fn check_proxy_health() -> Result<HealthCheckResult, String> {
    let config = store::load_app_config().unwrap_or_default();
    let host = config.proxy_host;
    let port = config.proxy_port;
    let url = format!("http://{}:{}/health", host, port);
    let client = Client::new();

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

#[tauri::command]
pub fn clear_metrics_data() -> Result<(), String> {
    db::clear_metrics().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reset_all_config() -> Result<(), String> {
    store::reset_all_config().map_err(|e| e.to_string())
}

pub struct AppState {
    pub app_handle: tauri::AppHandle,
    pub sidecar: Mutex<Option<Child>>,
    pub proxy_running: Mutex<bool>,
    pub proxy_status_item: MenuItem<tauri::Wry>,
    pub toggle_proxy_item: MenuItem<tauri::Wry>,
}

pub fn kill_sidecar(child: &mut Child) {
    let pid = child.id() as i32;
    let _ = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGTERM,
    );
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
    let _ = child.kill();
    let _ = child.wait();
}

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

pub fn spawn_sidecar() -> Result<Child, String> {
    let target_suffix = sidecar_target_suffix();
    let sidecar_path = if cfg!(debug_assertions) {
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

#[tauri::command]
pub fn restart_proxy(state: tauri::State<AppState>) -> Result<(), String> {
    let mut sidecar_guard = state.sidecar.lock().map_err(|e| e.to_string())?;
    if let Some(child) = sidecar_guard.as_mut() {
        kill_sidecar(child);
    }
    *sidecar_guard = None;
    *state.proxy_running.lock().map_err(|e| e.to_string())? = false;

    let child = spawn_sidecar()?;
    *sidecar_guard = Some(child);
    *state.proxy_running.lock().map_err(|e| e.to_string())? = true;

    crate::update_tray_icon(&state.app_handle, true);
    crate::update_menu_labels(&state, true);

    Ok(())
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
