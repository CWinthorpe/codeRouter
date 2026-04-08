use coderouter_proxy::config::models::{AppConfig, Group, Provider};
use coderouter_proxy::config::store;
use coderouter_proxy::credentials::keychain;
use coderouter_proxy::metrics::db;
use coderouter_proxy::metrics::queries;
use coderouter_proxy::metrics::scheduler;
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
            quota_reset_utc_hour: p.quota_reset_utc_hour,
            enabled: p.enabled,
            models: p.models.iter().map(|m| ProviderModelResponse {
                id: m.id.clone(),
                context_window: m.context_window,
                max_output_tokens: m.max_output_tokens,
                input_cost_per_1m: m.input_cost_per_1m,
                output_cost_per_1m: m.output_cost_per_1m,
                last_refreshed: m.last_refreshed.clone(),
            }).collect(),
        }
    }
}

#[tauri::command]
pub async fn get_providers() -> Result<Vec<ProviderResponse>, String> {
    let providers = store::load_providers().map_err(|e| e.to_string())?;
    Ok(providers.iter().map(|p| p.into()).collect())
}

#[tauri::command]
pub async fn save_provider(provider: Provider, api_key: String) -> Result<(), String> {
    ssrf::validate_base_url(&provider.base_url)?;

    let mut providers = store::load_providers().unwrap_or_default();

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

    Ok(())
}

#[tauri::command]
pub async fn toggle_provider_enabled(provider_id: String, enabled: bool) -> Result<(), String> {
    let mut providers = store::load_providers().map_err(|e| e.to_string())?;
    if let Some(provider) = providers.iter_mut().find(|p| p.id == provider_id) {
        provider.enabled = enabled;
    }
    store::save_providers(&providers).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_provider(provider_id: String) -> Result<(), String> {
    let mut providers = store::load_providers().map_err(|e| e.to_string())?;
    providers.retain(|p| p.id != provider_id);
    store::save_providers(&providers).map_err(|e| e.to_string())?;

    keychain::delete_credential(&provider_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn get_groups() -> Result<Vec<Group>, String> {
    store::load_groups().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_group(group: Group) -> Result<(), String> {
    let mut groups = store::load_groups().unwrap_or_default();

    if let Some(pos) = groups.iter().position(|g| g.id == group.id) {
        groups[pos] = group;
    } else {
        groups.push(group);
    }

    store::save_groups(&groups).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_group(group_id: String) -> Result<(), String> {
    let mut groups = store::load_groups().map_err(|e| e.to_string())?;
    groups.retain(|g| g.id != group_id);
    store::save_groups(&groups).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_app_config() -> Result<AppConfig, String> {
    store::load_app_config().map_or_else(
        |_| Ok(AppConfig::default()),
        |c| Ok(c),
    )
}

#[tauri::command]
pub async fn save_app_config(config: AppConfig) -> Result<(), String> {
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
    let models_url = match provider.protocol.as_str() {
        "anthropic" => format!("{base_url}/v1/models"),
        _ => format!("{base_url}/v1/models"),
    };

    let request = match provider.protocol.as_str() {
        "anthropic" => client
            .get(&models_url)
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2024-06-01"),
        _ => client
            .get(&models_url)
            .header("Authorization", format!("Bearer {}", api_key)),
    };

    match request.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            if resp.status().is_success() {
                Ok(TestConnectionResult {
                    success: true,
                    status_code: Some(status),
                    message: format!("Connection successful (HTTP {})", status),
                })
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

    let mut all_providers = store::load_providers().map_err(|e| e.to_string())?;
    if let Some(existing) = all_providers.iter_mut().find(|p| p.id == provider_id) {
        existing.models = models;
    }
    store::save_providers(&all_providers).map_err(|e| e.to_string())?;

    Ok(all_providers.iter().map(|p| p.into()).collect())
}

#[tauri::command]
pub fn get_router_status() -> Result<router::RouterStatusResponse, String> {
    let groups = store::load_groups().map_err(|e| e.to_string())?;
    let router_state = router::get_global_router_state()
        .ok_or_else(|| "Router state not initialized".to_string())?;
    let state = router_state.lock().map_err(|e| e.to_string())?;
    Ok(router::get_router_status(&groups, &state))
}

#[tauri::command]
pub fn set_entry_enabled(group_id: String, entry_index: usize, enabled: bool) -> Result<(), String> {
    let groups = store::load_groups().map_err(|e| e.to_string())?;
    let groups_arc = std::sync::Arc::new(groups);
    let router_state = router::get_global_router_state()
        .ok_or_else(|| "Router state not initialized".to_string())?;
    scheduler::set_entry_enabled(&router_state, groups_arc, &group_id, entry_index, enabled)
}

#[tauri::command]
pub fn get_daily_summary(provider_id: String, date: String) -> Result<queries::DailySummary, String> {
    let date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid date format (expected YYYY-MM-DD): {}", e))?;
    with_metrics_db(|conn| {
        queries::get_daily_summary(conn, &provider_id, date)
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
    with_metrics_db(|conn| {
        queries::get_usage_by_day(conn, &provider_id, days)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn get_usage_by_group(days: u32) -> Result<Vec<queries::GroupUsage>, String> {
    with_metrics_db(|conn| {
        queries::get_usage_by_group(conn, days)
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
pub fn inject_opencode_provider(proxy_port: u16) -> Result<(), String> {
    let stored = store::load_app_config().ok().and_then(|c| c.opencode_config_path);
    let config_path = config_writer::resolve_opencode_config_path(stored.as_deref())
        .ok_or_else(|| "Could not determine OpenCode config path".to_string())?;

    let groups = store::load_groups().unwrap_or_default();
    let providers = store::load_providers().unwrap_or_default();

    config_writer::inject_provider(&config_path, &groups, &providers, proxy_port)
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
pub fn preview_opencode_config(proxy_port: u16, mapping: Option<OpenCodeAgentMapping>) -> Result<String, String> {
    let groups = store::load_groups().unwrap_or_default();
    let providers = store::load_providers().unwrap_or_default();

    let agent_mapping = mapping.map(|m| m.into());

    config_writer::preview_opencode_config(&groups, &providers, proxy_port, agent_mapping.as_ref())
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
    with_metrics_db(|conn| {
        queries::get_latency_percentiles(conn, &provider_id, date)
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
    let _ = child.kill();
    let _ = child.wait();
}

pub fn spawn_sidecar() -> Result<Child, String> {
    let sidecar_path = if cfg!(debug_assertions) {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let dev_path = std::path::Path::new(manifest_dir)
            .join("sidecar/coderouter-proxy-x86_64-unknown-linux-gnu");
        if dev_path.exists() {
            dev_path
        } else {
            std::path::PathBuf::from("coderouter-proxy")
        }
    } else {
        // Check multiple possible paths for the sidecar binary:
        // 1. AppImage-extracted path (/tmp/.mount_*/usr/bin/sidecar/)
        // 2. Release path (next to the main binary in sidecar/)
        // 3. Fallback to PATH

        // Check for AppImage mount point
        if let Ok(tmp) = std::env::var("APPIMAGE") {
            if let Some(mount_dir) = std::path::Path::new(&tmp).parent() {
                let appimage_sidecar = mount_dir.join("usr/bin/sidecar/coderouter-proxy");
                if appimage_sidecar.exists() {
                    appimage_sidecar
                } else {
                    find_sidecar_fallback()
                }
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
