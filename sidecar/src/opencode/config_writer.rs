use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use std::os::unix::fs::PermissionsExt;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

use crate::config::models::{Group, Provider};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AgentMapping {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub general: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explore: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "small_model"
    )]
    pub small_model: Option<String>,
}

pub fn detect_opencode_config() -> Option<PathBuf> {
    let mut path = dirs::home_dir()?;
    path.push(".config/opencode/opencode.json");
    Some(path)
}

pub fn opencode_cache_path() -> Option<PathBuf> {
    let mut path = dirs::config_dir()?;
    path.push("coderouter/opencode.json");
    Some(path)
}

pub fn save_opencode_cache(config_path: &Path) -> Result<()> {
    if !config_path.exists() {
        return Ok(());
    }
    let cache_path = match opencode_cache_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    if let Some(parent) = cache_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    let contents = fs::read_to_string(config_path)?;
    fs::write(&cache_path, contents)?;
    Ok(())
}

pub fn load_opencode_cache() -> Option<serde_json::Value> {
    let cache_path = opencode_cache_path()?;
    if !cache_path.exists() {
        return None;
    }
    let contents = fs::read_to_string(&cache_path).ok()?;
    serde_json::from_str(&contents).ok()
}

pub fn resolve_opencode_config_path(stored_path: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = stored_path {
        if !path.is_empty() {
            let p = PathBuf::from(path);
            if p.exists() || p.parent().map(|p| p.exists()).unwrap_or(true) {
                return Some(p);
            }
        }
    }
    detect_opencode_config()
}

pub fn save_opencode_config_path(path: &str) -> Result<()> {
    use crate::config::store::{load_app_config, save_app_config};
    let mut config = load_app_config().unwrap_or_default();
    config.opencode_config_path = Some(path.to_string());
    save_app_config(&config)?;
    Ok(())
}

pub fn inject_provider(
    config_path: &Path,
    groups: &[Group],
    providers: &[Provider],
    proxy_port: u16,
    entry_statuses: &HashMap<String, String>,
) -> Result<()> {
    let mut config = read_config(config_path)?;

    let base_url = format!("http://localhost:{proxy_port}/v1");

    let mut models = serde_json::Map::new();

    for group in groups {
        let highest_active = group
            .entries
            .iter()
            .enumerate()
            .filter(|(idx, e)| {
                if !e.enabled {
                    return false;
                }
                let key = format!("{}:{}", e.provider_id, idx);
                entry_statuses
                    .get(&key)
                    .map(|s| s == "active")
                    .unwrap_or(true)
            })
            .min_by_key(|(_, e)| e.priority)
            .map(|(_, e)| e);

        if let Some(entry) = highest_active {
            let provider = providers.iter().find(|p| p.id == entry.provider_id);

            let mut model_obj = serde_json::Map::new();
            model_obj.insert("name".to_string(), json_str(&group.display_name));

            if let Some(provider) = provider {
                if let Some(model_meta) = provider.models.iter().find(|m| m.id == entry.model_id) {
                    let mut limit = serde_json::Map::new();
                    if let Some(ctx) = model_meta.context_window {
                        limit.insert("context".to_string(), json_num(ctx));
                    }
                    if let Some(out) = model_meta.max_output_tokens {
                        limit.insert("output".to_string(), json_num(out));
                    }
                    if !limit.is_empty() {
                        model_obj.insert("limit".to_string(), serde_json::Value::Object(limit));
                    }
                }
            }

            models.insert(group.alias.clone(), serde_json::Value::Object(model_obj));
        }
    }

    let coderouter_provider = serde_json::json!({
        "npm": "@ai-sdk/openai-compatible",
        "name": "CodeRouter",
        "options": {
            "baseURL": base_url,
            "apiKey": "coderouter"
        },
        "models": serde_json::Value::Object(models)
    });

    {
        let obj = config.as_object_mut().unwrap();
        if let Some(serde_json::Value::Object(provider_obj)) = obj.get_mut("provider") {
            provider_obj.insert("coderouter".to_string(), coderouter_provider);
        } else {
            let mut provider_obj = serde_json::Map::new();
            provider_obj.insert("coderouter".to_string(), coderouter_provider);
            obj.insert(
                "provider".to_string(),
                serde_json::Value::Object(provider_obj),
            );
        }
    }

    write_config(config_path, &config)?;
    let _ = save_opencode_cache(config_path);
    Ok(())
}

pub fn remove_provider(config_path: &Path) -> Result<()> {
    let mut config = read_config(config_path)?;

    {
        let obj = config.as_object_mut().unwrap();
        if let Some(serde_json::Value::Object(provider_obj)) = obj.get_mut("provider") {
            provider_obj.remove("coderouter");
            if provider_obj.is_empty() {
                obj.remove("provider");
            }
        }
    }

    write_config(config_path, &config)?;
    let _ = save_opencode_cache(config_path);
    Ok(())
}

pub fn set_agent_models(config_path: &Path, mapping: &AgentMapping) -> Result<()> {
    let mut config = read_config(config_path)?;

    let agent_map = [
        ("build", &mapping.build),
        ("plan", &mapping.plan),
        ("general", &mapping.general),
        ("explore", &mapping.explore),
        ("compaction", &mapping.compaction),
        ("title", &mapping.title),
        ("summary", &mapping.summary),
    ];

    for (agent_name, model_alias) in &agent_map {
        if let Some(alias) = model_alias {
            let obj = config.as_object_mut().unwrap();
            let agents = obj
                .entry("agent".to_string())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

            if let serde_json::Value::Object(agents_map) = agents {
                let agent_config = agents_map
                    .entry(agent_name.to_string())
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

                if let serde_json::Value::Object(config_map) = agent_config {
                    config_map.insert(
                        "model".to_string(),
                        json_str(&format!("coderouter/{}", alias)),
                    );
                }
            }
        }
    }

    if let Some(ref small) = mapping.small_model {
        let obj = config.as_object_mut().unwrap();
        obj.insert(
            "small_model".to_string(),
            json_str(&format!("coderouter/{}", small)),
        );
    }

    write_config(config_path, &config)?;
    let _ = save_opencode_cache(config_path);
    Ok(())
}

pub fn remove_agent_models(config_path: &Path) -> Result<()> {
    let mut config = read_config(config_path)?;

    {
        let obj = config.as_object_mut().unwrap();
        if let Some(serde_json::Value::Object(agents)) = obj.get_mut("agent") {
            let keys_to_remove: Vec<String> = agents
                .iter()
                .filter_map(|(key, value)| {
                    if let serde_json::Value::Object(agent_config) = value {
                        if let Some(serde_json::Value::String(model)) = agent_config.get("model") {
                            if model.starts_with("coderouter/") {
                                return Some(key.clone());
                            }
                        }
                    }
                    None
                })
                .collect();

            for key in keys_to_remove {
                if let Some(serde_json::Value::Object(agent_config)) = agents.get_mut(&key) {
                    agent_config.remove("model");
                    if agent_config.is_empty() {
                        agents.remove(&key);
                    }
                }
            }

            if agents.is_empty() {
                obj.remove("agent");
            }
        }

        if let Some(serde_json::Value::String(small)) = obj.get("small_model") {
            if small.starts_with("coderouter/") {
                obj.remove("small_model");
            }
        }
    }

    write_config(config_path, &config)
}

pub fn preview_opencode_config(
    groups: &[Group],
    providers: &[Provider],
    proxy_port: u16,
    mapping: Option<&AgentMapping>,
    entry_statuses: &HashMap<String, String>,
) -> Result<String> {
    let mut config = read_config_or_empty()?;

    let base_url = format!("http://localhost:{proxy_port}/v1");

    let mut models = serde_json::Map::new();

    for group in groups {
        let highest_active = group
            .entries
            .iter()
            .enumerate()
            .filter(|(idx, e)| {
                if !e.enabled {
                    return false;
                }
                let key = format!("{}:{}", e.provider_id, idx);
                entry_statuses
                    .get(&key)
                    .map(|s| s == "active")
                    .unwrap_or(true)
            })
            .min_by_key(|(_, e)| e.priority)
            .map(|(_, e)| e);

        if let Some(entry) = highest_active {
            let provider = providers.iter().find(|p| p.id == entry.provider_id);

            let mut model_obj = serde_json::Map::new();
            model_obj.insert("name".to_string(), json_str(&group.display_name));

            if let Some(provider) = provider {
                if let Some(model_meta) = provider.models.iter().find(|m| m.id == entry.model_id) {
                    let mut limit = serde_json::Map::new();
                    if let Some(ctx) = model_meta.context_window {
                        limit.insert("context".to_string(), json_num(ctx));
                    }
                    if let Some(out) = model_meta.max_output_tokens {
                        limit.insert("output".to_string(), json_num(out));
                    }
                    if !limit.is_empty() {
                        model_obj.insert("limit".to_string(), serde_json::Value::Object(limit));
                    }
                }
            }

            models.insert(group.alias.clone(), serde_json::Value::Object(model_obj));
        }
    }

    let coderouter_provider = serde_json::json!({
        "npm": "@ai-sdk/openai-compatible",
        "name": "CodeRouter",
        "options": {
            "baseURL": base_url,
            "apiKey": "coderouter"
        },
        "models": serde_json::Value::Object(models)
    });

    {
        let obj = config.as_object_mut().unwrap();
        if let Some(serde_json::Value::Object(provider_obj)) = obj.get_mut("provider") {
            provider_obj.insert("coderouter".to_string(), coderouter_provider);
        } else {
            let mut provider_obj = serde_json::Map::new();
            provider_obj.insert("coderouter".to_string(), coderouter_provider);
            obj.insert(
                "provider".to_string(),
                serde_json::Value::Object(provider_obj),
            );
        }
    }

    if let Some(mapping) = mapping {
        let agent_map = [
            ("build", &mapping.build),
            ("plan", &mapping.plan),
            ("general", &mapping.general),
            ("explore", &mapping.explore),
            ("compaction", &mapping.compaction),
            ("title", &mapping.title),
            ("summary", &mapping.summary),
        ];

        for (agent_name, model_alias) in &agent_map {
            if let Some(alias) = model_alias {
                let obj = config.as_object_mut().unwrap();
                let agents = obj
                    .entry("agent".to_string())
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

                if let serde_json::Value::Object(agents_map) = agents {
                    let agent_config = agents_map
                        .entry(agent_name.to_string())
                        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

                    if let serde_json::Value::Object(config_map) = agent_config {
                        config_map.insert(
                            "model".to_string(),
                            json_str(&format!("coderouter/{}", alias)),
                        );
                    }
                }
            }
        }

        if let Some(ref small) = mapping.small_model {
            let obj = config.as_object_mut().unwrap();
            obj.insert(
                "small_model".to_string(),
                json_str(&format!("coderouter/{}", small)),
            );
        }
    }

    serde_json::to_string_pretty(&config).map_err(|e| e.into())
}

fn read_config(config_path: &Path) -> Result<serde_json::Value> {
    if config_path.exists() {
        let contents = fs::read_to_string(config_path)?;
        let value: serde_json::Value = serde_json::from_str(&contents)?;
        Ok(value)
    } else {
        Ok(serde_json::Value::Object(serde_json::Map::new()))
    }
}

fn read_config_or_empty() -> Result<serde_json::Value> {
    let path = detect_opencode_config();
    match path {
        Some(p) if p.exists() => read_config(&p),
        Some(_) => match load_opencode_cache() {
            Some(cache) => Ok(cache),
            None => Ok(serde_json::Value::Object(serde_json::Map::new())),
        },
        None => Ok(serde_json::Value::Object(serde_json::Map::new())),
    }
}

fn write_config(config_path: &Path, config: &serde_json::Value) -> Result<()> {
    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    let content = serde_json::to_string_pretty(config)?;
    let tmp_path = config_path.with_extension(format!("tmp.{}", std::process::id()));

    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&tmp_path)?;

    file.lock_exclusive()?;
    file.write_all(content.as_bytes())?;
    file.flush()?;
    file.sync_all()?;
    file.unlock()?;

    let mut perms = file.metadata()?.permissions();
    perms.set_mode(0o600);
    file.set_permissions(perms)?;

    fs::rename(&tmp_path, config_path)?;

    if let Ok(metadata) = fs::metadata(config_path) {
        let mut perms = metadata.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(config_path, perms)?;
    }

    Ok(())
}

pub fn is_group_alias_referenced(group_alias: &str) -> bool {
    let path = match detect_opencode_config() {
        Some(p) => p,
        None => return false,
    };
    if !path.exists() {
        return false;
    }
    let Ok(contents) = fs::read_to_string(&path) else {
        return false;
    };
    let Ok(config) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return false;
    };
    let Some(obj) = config.as_object() else {
        return false;
    };
    let coderouter_model = format!("coderouter/{}", group_alias);
    if let Some(serde_json::Value::Object(agents)) = obj.get("agent") {
        for (_key, value) in agents {
            if let serde_json::Value::Object(agent_config) = value {
                if let Some(serde_json::Value::String(model)) = agent_config.get("model") {
                    if model == &coderouter_model {
                        return true;
                    }
                }
            }
        }
    }
    if let Some(serde_json::Value::String(small)) = obj.get("small_model") {
        if small == &coderouter_model {
            return true;
        }
    }
    false
}

fn json_str(s: &str) -> serde_json::Value {
    serde_json::Value::String(s.to_string())
}

fn json_num(n: u64) -> serde_json::Value {
    serde_json::Value::Number(serde_json::Number::from(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::models::{FailoverConfig, GroupEntry, ProviderModel};
    use std::fs;

    fn test_dir() -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "coderouter_opencode_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }

    fn setup_test_dir() -> PathBuf {
        let dir = test_dir();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup_test_dir(dir: &Path) {
        if dir.exists() {
            fs::remove_dir_all(dir).ok();
        }
    }

    fn test_group() -> Group {
        Group {
            id: "glm-5-router".to_string(),
            alias: "glm-5-router".to_string(),
            display_name: "GLM-5 (Multi-Account)".to_string(),
            entries: vec![GroupEntry {
                provider_id: "test-provider".to_string(),
                model_id: "glm-4.5".to_string(),
                priority: 1,
                daily_token_quota_override: None,
                enabled: true,
                status: "active".to_string(),
                cooldown_until: None,
            }],
            failover_config: FailoverConfig {
                on_429: true,
                on_quota_exhausted: true,
                on_consecutive_errors: true,
                consecutive_error_threshold: 5,
                on_latency_timeout: true,
                latency_timeout_ms: 30000,
                latency_timeout_cooldown_ms: 300000,
                consecutive_error_cooldown_ms: 600000,
            },
        }
    }

    fn test_provider() -> Provider {
        Provider {
            id: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.test.com/v1".to_string(),
            credential_key: "test-provider".to_string(),
            daily_token_quota: Some(1_000_000),
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![ProviderModel {
                id: "glm-4.5".to_string(),
                context_window: Some(128000),
                max_output_tokens: Some(8192),
                input_cost_per_1m: Some(1.0),
                output_cost_per_1m: Some(2.0),
                last_refreshed: Some("2026-04-07T00:00:00Z".to_string()),
            }],
            model_overrides: None,
        }
    }

    #[test]
    fn test_detect_opencode_config_returns_path() {
        let result = detect_opencode_config();
        assert!(result.is_some());
        let path = result.unwrap();
        assert!(path.ends_with(".config/opencode/opencode.json"));
    }

    #[test]
    fn test_inject_provider_creates_new_config() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");
        let groups = vec![test_group()];
        let providers = vec![test_provider()];

        inject_provider(&config_path, &groups, &providers, 4141, &HashMap::new()).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        let provider = config.get("provider").unwrap().get("coderouter").unwrap();
        assert_eq!(provider.get("npm").unwrap(), "@ai-sdk/openai-compatible");
        assert_eq!(provider.get("name").unwrap(), "CodeRouter");

        let options = provider.get("options").unwrap();
        assert_eq!(options.get("baseURL").unwrap(), "http://localhost:4141/v1");
        assert_eq!(options.get("apiKey").unwrap(), "coderouter");

        let models = provider.get("models").unwrap().as_object().unwrap();
        assert!(models.contains_key("glm-5-router"));

        let glm_model = models.get("glm-5-router").unwrap();
        assert_eq!(glm_model.get("name").unwrap(), "GLM-5 (Multi-Account)");

        let limit = glm_model.get("limit").unwrap().as_object().unwrap();
        assert_eq!(limit.get("context").unwrap().as_u64().unwrap(), 128000);
        assert_eq!(limit.get("output").unwrap().as_u64().unwrap(), 8192);

        cleanup_test_dir(&test_dir);
    }

    #[test]
    fn test_inject_provider_preserves_existing_keys() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");

        let existing_config = serde_json::json!({
            "model": "anthropic/claude-sonnet-4-5",
            "autoupdate": true,
            "provider": {
                "anthropic": {
                    "options": {
                        "apiKey": "sk-test"
                    }
                }
            }
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&existing_config).unwrap(),
        )
        .unwrap();

        let groups = vec![test_group()];
        let providers = vec![test_provider()];

        inject_provider(&config_path, &groups, &providers, 4141, &HashMap::new()).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        assert_eq!(config.get("model").unwrap(), "anthropic/claude-sonnet-4-5");
        assert_eq!(config.get("autoupdate").unwrap(), true);

        let provider = config.get("provider").unwrap().as_object().unwrap();
        assert!(provider.contains_key("anthropic"));
        assert!(provider.contains_key("coderouter"));

        cleanup_test_dir(&test_dir);
    }

    #[test]
    fn test_inject_provider_overwrites_existing_coderouter() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");

        let existing_config = serde_json::json!({
            "provider": {
                "coderouter": {
                    "npm": "old-value",
                    "name": "Old Router",
                    "options": {
                        "baseURL": "http://localhost:9999/v1",
                        "apiKey": "old-key"
                    },
                    "models": {}
                }
            }
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&existing_config).unwrap(),
        )
        .unwrap();

        let groups = vec![test_group()];
        let providers = vec![test_provider()];

        inject_provider(&config_path, &groups, &providers, 4141, &HashMap::new()).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        let coderouter = config.get("provider").unwrap().get("coderouter").unwrap();
        assert_eq!(coderouter.get("name").unwrap(), "CodeRouter");

        let options = coderouter.get("options").unwrap();
        assert_eq!(options.get("baseURL").unwrap(), "http://localhost:4141/v1");

        cleanup_test_dir(&test_dir);
    }

    #[test]
    fn test_remove_provider_removes_coderouter() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");

        let existing_config = serde_json::json!({
            "model": "anthropic/claude-sonnet-4-5",
            "provider": {
                "anthropic": {
                    "options": {
                        "apiKey": "sk-test"
                    }
                },
                "coderouter": {
                    "npm": "@ai-sdk/openai-compatible",
                    "name": "CodeRouter",
                    "options": {
                        "baseURL": "http://localhost:4141/v1",
                        "apiKey": "coderouter"
                    },
                    "models": {}
                }
            }
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&existing_config).unwrap(),
        )
        .unwrap();

        remove_provider(&config_path).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        let provider = config.get("provider").unwrap().as_object().unwrap();
        assert!(!provider.contains_key("coderouter"));
        assert!(provider.contains_key("anthropic"));
        assert_eq!(config.get("model").unwrap(), "anthropic/claude-sonnet-4-5");

        cleanup_test_dir(&test_dir);
    }

    #[test]
    fn test_remove_provider_cleans_up_empty_provider_object() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");

        let existing_config = serde_json::json!({
            "model": "anthropic/claude-sonnet-4-5",
            "provider": {
                "coderouter": {
                    "npm": "@ai-sdk/openai-compatible",
                    "name": "CodeRouter",
                    "options": {
                        "baseURL": "http://localhost:4141/v1",
                        "apiKey": "coderouter"
                    },
                    "models": {}
                }
            }
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&existing_config).unwrap(),
        )
        .unwrap();

        remove_provider(&config_path).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        assert!(config.get("provider").is_none());
        assert_eq!(config.get("model").unwrap(), "anthropic/claude-sonnet-4-5");

        cleanup_test_dir(&test_dir);
    }

    #[test]
    fn test_set_agent_models_sets_all_agents() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");

        let existing_config = serde_json::json!({
            "model": "anthropic/claude-sonnet-4-5"
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&existing_config).unwrap(),
        )
        .unwrap();

        let mapping = AgentMapping {
            build: Some("glm-5-router".to_string()),
            plan: Some("fast-model-router".to_string()),
            general: Some("glm-5-router".to_string()),
            explore: Some("fast-model-router".to_string()),
            compaction: None,
            title: None,
            summary: None,
            small_model: Some("fast-model-router".to_string()),
        };

        set_agent_models(&config_path, &mapping).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        let agent = config.get("agent").unwrap().as_object().unwrap();

        assert_eq!(
            agent.get("build").unwrap().get("model").unwrap(),
            "coderouter/glm-5-router"
        );
        assert_eq!(
            agent.get("plan").unwrap().get("model").unwrap(),
            "coderouter/fast-model-router"
        );
        assert_eq!(
            agent.get("general").unwrap().get("model").unwrap(),
            "coderouter/glm-5-router"
        );
        assert_eq!(
            agent.get("explore").unwrap().get("model").unwrap(),
            "coderouter/fast-model-router"
        );
        assert_eq!(
            config.get("small_model").unwrap(),
            "coderouter/fast-model-router"
        );
        assert_eq!(config.get("model").unwrap(), "anthropic/claude-sonnet-4-5");

        cleanup_test_dir(&test_dir);
    }

    #[test]
    fn test_set_agent_models_partial_mapping() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");

        let existing_config = serde_json::json!({
            "agent": {
                "build": {
                    "model": "openai/gpt-4",
                    "tools": {
                        "write": true
                    }
                }
            }
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&existing_config).unwrap(),
        )
        .unwrap();

        let mapping = AgentMapping {
            build: Some("glm-5-router".to_string()),
            plan: None,
            general: None,
            explore: None,
            compaction: None,
            title: None,
            summary: None,
            small_model: Some("fast-model-router".to_string()),
        };

        set_agent_models(&config_path, &mapping).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        let agent = config.get("agent").unwrap().as_object().unwrap();
        let build_agent = agent.get("build").unwrap().as_object().unwrap();

        assert_eq!(build_agent.get("model").unwrap(), "coderouter/glm-5-router");
        assert!(build_agent.contains_key("tools"));
        assert_eq!(
            config.get("small_model").unwrap(),
            "coderouter/fast-model-router"
        );

        cleanup_test_dir(&test_dir);
    }

    #[test]
    fn test_remove_agent_models_removes_coderouter_models() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");

        let existing_config = serde_json::json!({
            "model": "anthropic/claude-sonnet-4-5",
            "small_model": "coderouter/fast-model-router",
            "agent": {
                "build": {
                    "model": "coderouter/glm-5-router",
                    "tools": {
                        "write": true
                    }
                },
                "plan": {
                    "model": "coderouter/fast-model-router"
                },
                "general": {
                    "model": "openai/gpt-4"
                }
            }
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&existing_config).unwrap(),
        )
        .unwrap();

        remove_agent_models(&config_path).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        let agent = config.get("agent").unwrap().as_object().unwrap();

        assert!(agent
            .get("build")
            .unwrap()
            .as_object()
            .unwrap()
            .get("model")
            .is_none());
        assert!(agent
            .get("build")
            .unwrap()
            .as_object()
            .unwrap()
            .get("tools")
            .is_some());

        assert!(agent.get("plan").is_none());

        assert_eq!(
            agent.get("general").unwrap().get("model").unwrap(),
            "openai/gpt-4"
        );

        assert!(config.get("small_model").is_none());

        cleanup_test_dir(&test_dir);
    }

    #[test]
    fn test_remove_agent_models_cleans_up_empty_agent_object() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");

        let existing_config = serde_json::json!({
            "model": "anthropic/claude-sonnet-4-5",
            "agent": {
                "build": {
                    "model": "coderouter/glm-5-router"
                }
            }
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&existing_config).unwrap(),
        )
        .unwrap();

        remove_agent_models(&config_path).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        assert!(config.get("agent").is_none());
        assert_eq!(config.get("model").unwrap(), "anthropic/claude-sonnet-4-5");

        cleanup_test_dir(&test_dir);
    }

    #[test]
    fn test_inject_provider_omits_limit_when_null() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");

        let provider = Provider {
            id: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.test.com/v1".to_string(),
            credential_key: "test-provider".to_string(),
            daily_token_quota: None,
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![ProviderModel {
                id: "unknown-model".to_string(),
                context_window: None,
                max_output_tokens: None,
                input_cost_per_1m: None,
                output_cost_per_1m: None,
                last_refreshed: None,
            }],
            model_overrides: None,
        };

        let groups = vec![test_group()];
        let providers = vec![provider];

        inject_provider(&config_path, &groups, &providers, 4141, &HashMap::new()).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        let models = config
            .get("provider")
            .unwrap()
            .get("coderouter")
            .unwrap()
            .get("models")
            .unwrap()
            .as_object()
            .unwrap();

        let glm_model = models.get("glm-5-router").unwrap();
        assert!(glm_model.get("limit").is_none());
        assert_eq!(glm_model.get("name").unwrap(), "GLM-5 (Multi-Account)");

        cleanup_test_dir(&test_dir);
    }

    #[test]
    fn test_preview_opencode_config_returns_json_string() {
        let groups = vec![test_group()];
        let providers = vec![test_provider()];

        let result =
            preview_opencode_config(&groups, &providers, 4141, None, &HashMap::new()).unwrap();

        let config: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(config.get("provider").unwrap().get("coderouter").is_some());
    }

    #[test]
    fn test_preview_opencode_config_with_agent_mapping() {
        let groups = vec![test_group()];
        let providers = vec![test_provider()];

        let mapping = AgentMapping {
            build: Some("glm-5-router".to_string()),
            plan: None,
            general: None,
            explore: None,
            compaction: None,
            title: None,
            summary: None,
            small_model: Some("fast-model-router".to_string()),
        };

        let result =
            preview_opencode_config(&groups, &providers, 4141, Some(&mapping), &HashMap::new())
                .unwrap();

        let config: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            config
                .get("agent")
                .unwrap()
                .get("build")
                .unwrap()
                .get("model")
                .unwrap(),
            "coderouter/glm-5-router"
        );
        assert_eq!(
            config.get("small_model").unwrap(),
            "coderouter/fast-model-router"
        );
    }

    #[test]
    fn test_inject_provider_uses_2_space_indent() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");

        let groups = vec![test_group()];
        let providers = vec![test_provider()];

        inject_provider(&config_path, &groups, &providers, 4141, &HashMap::new()).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();

        assert!(lines.iter().any(|l| l.starts_with("  \"provider\"")));
        assert!(lines.iter().any(|l| l.starts_with("    \"coderouter\"")));

        cleanup_test_dir(&test_dir);
    }

    #[test]
    fn test_inject_provider_skips_disabled_entries() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");

        let mut group = test_group();
        group.entries[0].enabled = false;
        group.entries.push(GroupEntry {
            provider_id: "test-provider-2".to_string(),
            model_id: "glm-4.5-v2".to_string(),
            priority: 2,
            daily_token_quota_override: None,
            enabled: true,
            status: "active".to_string(),
            cooldown_until: None,
        });

        let provider1 = test_provider();
        let provider2 = Provider {
            id: "test-provider-2".to_string(),
            name: "Test Provider 2".to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.test2.com/v1".to_string(),
            credential_key: "test-provider-2".to_string(),
            daily_token_quota: None,
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![ProviderModel {
                id: "glm-4.5-v2".to_string(),
                context_window: Some(64000),
                max_output_tokens: Some(4096),
                input_cost_per_1m: Some(0.5),
                output_cost_per_1m: Some(1.0),
                last_refreshed: Some("2026-04-07T00:00:00Z".to_string()),
            }],
            model_overrides: None,
        };

        let groups = vec![group];
        let providers = vec![provider1, provider2];

        inject_provider(&config_path, &groups, &providers, 4141, &HashMap::new()).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        let models = config
            .get("provider")
            .unwrap()
            .get("coderouter")
            .unwrap()
            .get("models")
            .unwrap()
            .as_object()
            .unwrap();

        let glm_model = models.get("glm-5-router").unwrap();
        let limit = glm_model.get("limit").unwrap().as_object().unwrap();
        assert_eq!(limit.get("context").unwrap().as_u64().unwrap(), 64000);
        assert_eq!(limit.get("output").unwrap().as_u64().unwrap(), 4096);

        cleanup_test_dir(&test_dir);
    }

    #[test]
    fn test_remove_agent_models_preserves_non_coderouter_agents() {
        let test_dir = setup_test_dir();
        let config_path = test_dir.join("opencode.json");

        let existing_config = serde_json::json!({
            "agent": {
                "build": {
                    "model": "coderouter/glm-5-router"
                },
                "custom-reviewer": {
                    "model": "anthropic/claude-sonnet-4-5",
                    "prompt": "Review code"
                }
            }
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&existing_config).unwrap(),
        )
        .unwrap();

        remove_agent_models(&config_path).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        let agent = config.get("agent").unwrap().as_object().unwrap();
        assert!(agent.get("build").is_none());

        let reviewer = agent.get("custom-reviewer").unwrap().as_object().unwrap();
        assert_eq!(
            reviewer.get("model").unwrap(),
            "anthropic/claude-sonnet-4-5"
        );
        assert_eq!(reviewer.get("prompt").unwrap(), "Review code");

        cleanup_test_dir(&test_dir);
    }
}
