use reqwest::Client;
use serde::Deserialize;
use chrono::Utc;

use crate::config::models::{Provider, ProviderModel};
use crate::config::store::{load_app_config, load_providers, save_providers, update_providers_with_lock};
use crate::credentials::keychain::get_credential;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelEntry>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelEntry {
    id: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelDetail {
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    max_output_tokens: Option<u64>,
    #[serde(default)]
    max_completion_tokens: Option<u64>,
    #[serde(default)]
    pricing: Option<PricingInfo>,
    #[serde(default)]
    input_cost_per_token: Option<f64>,
    #[serde(default)]
    output_cost_per_token: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct PricingInfo {
    #[serde(default)]
    prompt: Option<f64>,
    #[serde(default)]
    completion: Option<f64>,
}

pub async fn fetch_models_for_provider(
    provider: &Provider,
    api_key: &str,
    client: &Client,
) -> Result<Vec<ProviderModel>> {
    match provider.protocol.as_str() {
        "anthropic" => Ok(get_anthropic_models(provider)),
        _ => fetch_openai_compatible_models(provider, api_key, client).await,
    }
}

async fn fetch_openai_compatible_models(
    provider: &Provider,
    api_key: &str,
    client: &Client,
) -> Result<Vec<ProviderModel>> {
    let base_url = provider.base_url.trim_end_matches('/');
    let models_url = format!("{base_url}/models");

    let resp = client
        .get(&models_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Failed to fetch models from {}: {} - {}",
            provider.name, status, body
        )
        .into());
    }

    let models_list: OpenAiModelsResponse = resp.json().await?;

    let now = current_iso_timestamp();
    let mut result = Vec::new();

    for entry in models_list.data {
        let detail_url = format!("{base_url}/models/{}", entry.id);

        let mut model = ProviderModel {
            id: entry.id.clone(),
            context_window: None,
            max_output_tokens: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
            last_refreshed: Some(now.clone()),
        };

        if let Ok(detail_resp) = client.get(&detail_url).header("Authorization", format!("Bearer {api_key}")).timeout(std::time::Duration::from_secs(30)).send().await {
            if detail_resp.status().is_success() {
                if let Ok(text) = detail_resp.text().await {
                    if let Ok(detail) = parse_model_detail(&text) {
                        model.context_window = detail
                            .context_window
                            .or(detail.context_length);

                        model.max_output_tokens = detail
                            .max_output_tokens
                            .or(detail.max_completion_tokens);

                        if let Some(pricing) = detail.pricing {
                            if let Some(prompt) = pricing.prompt {
                                model.input_cost_per_1m = Some(prompt * 1_000_000.0);
                            }
                            if let Some(completion) = pricing.completion {
                                model.output_cost_per_1m = Some(completion * 1_000_000.0);
                            }
                        }

                        if let Some(cost) = detail.input_cost_per_token {
                            model.input_cost_per_1m = Some(cost * 1_000_000.0);
                        }
                        if let Some(cost) = detail.output_cost_per_token {
                            model.output_cost_per_1m = Some(cost * 1_000_000.0);
                        }
                    } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                        model = extract_from_raw_json(&value, model, &now);
                    }
                }
            }
        }

        result.push(model);
    }

    Ok(result)
}

fn parse_model_detail(text: &str) -> Result<OpenAiModelDetail> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    let detail = serde_json::from_value::<OpenAiModelDetail>(value).unwrap_or(OpenAiModelDetail {
        context_window: None,
        context_length: None,
        max_output_tokens: None,
        max_completion_tokens: None,
        pricing: None,
        input_cost_per_token: None,
        output_cost_per_token: None,
    });
    Ok(detail)
}

fn extract_from_raw_json(value: &serde_json::Value, mut model: ProviderModel, now: &str) -> ProviderModel {
    if let Some(obj) = value.as_object() {
        if let Some(v) = obj.get("context_window").and_then(|v| v.as_u64()) {
            model.context_window = Some(v);
        }
        if let Some(v) = obj.get("context_length").and_then(|v| v.as_u64()) {
            model.context_window = model.context_window.or(Some(v));
        }
        if let Some(v) = obj.get("max_output_tokens").and_then(|v| v.as_u64()) {
            model.max_output_tokens = Some(v);
        }
        if let Some(v) = obj.get("max_completion_tokens").and_then(|v| v.as_u64()) {
            model.max_output_tokens = model.max_output_tokens.or(Some(v));
        }

        if let Some(pricing) = obj.get("pricing").and_then(|v| v.as_object()) {
            if let Some(prompt) = pricing.get("prompt").and_then(|v| v.as_f64()) {
                model.input_cost_per_1m = Some(prompt * 1_000_000.0);
            }
            if let Some(completion) = pricing.get("completion").and_then(|v| v.as_f64()) {
                model.output_cost_per_1m = Some(completion * 1_000_000.0);
            }
        }

        if let Some(v) = obj.get("input_cost_per_token").and_then(|v| v.as_f64()) {
            model.input_cost_per_1m = Some(v * 1_000_000.0);
        }
        if let Some(v) = obj.get("output_cost_per_token").and_then(|v| v.as_f64()) {
            model.output_cost_per_1m = Some(v * 1_000_000.0);
        }
    }

    model.last_refreshed = Some(now.to_string());
    model
}

fn get_anthropic_models(provider: &Provider) -> Vec<ProviderModel> {
    let now = current_iso_timestamp();

    let hardcoded = anthropic_hardcoded_models(&now);

    if let Some(overrides) = &provider.model_overrides {
        if !overrides.is_empty() {
            return overrides
                .iter()
                .map(|m| ProviderModel {
                    last_refreshed: Some(now.clone()),
                    ..m.clone()
                })
                .collect();
        }
    }

    hardcoded
}

fn anthropic_hardcoded_models(now: &str) -> Vec<ProviderModel> {
    vec![
        ProviderModel {
            id: "claude-opus-4-20250514".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(64_000),
            input_cost_per_1m: Some(15.0),
            output_cost_per_1m: Some(75.0),
            last_refreshed: Some(now.to_string()),
        },
        ProviderModel {
            id: "claude-sonnet-4-20250514".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(64_000),
            input_cost_per_1m: Some(3.0),
            output_cost_per_1m: Some(15.0),
            last_refreshed: Some(now.to_string()),
        },
        ProviderModel {
            id: "claude-3-7-sonnet-20250219".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(64_000),
            input_cost_per_1m: Some(3.0),
            output_cost_per_1m: Some(15.0),
            last_refreshed: Some(now.to_string()),
        },
        ProviderModel {
            id: "claude-3-5-sonnet-20241022".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(8_192),
            input_cost_per_1m: Some(3.0),
            output_cost_per_1m: Some(15.0),
            last_refreshed: Some(now.to_string()),
        },
        ProviderModel {
            id: "claude-3-5-haiku-20241022".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(8_192),
            input_cost_per_1m: Some(0.80),
            output_cost_per_1m: Some(4.0),
            last_refreshed: Some(now.to_string()),
        },
        ProviderModel {
            id: "claude-3-opus-20240229".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(4_096),
            input_cost_per_1m: Some(15.0),
            output_cost_per_1m: Some(75.0),
            last_refreshed: Some(now.to_string()),
        },
        ProviderModel {
            id: "claude-3-sonnet-20240229".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(4_096),
            input_cost_per_1m: Some(3.0),
            output_cost_per_1m: Some(15.0),
            last_refreshed: Some(now.to_string()),
        },
        ProviderModel {
            id: "claude-3-haiku-20240307".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(4_096),
            input_cost_per_1m: Some(0.25),
            output_cost_per_1m: Some(1.25),
            last_refreshed: Some(now.to_string()),
        },
    ]
}

fn current_iso_timestamp() -> String {
    Utc::now().to_rfc3339()
}

fn needs_refresh(provider: &Provider, refresh_interval_hours: u32) -> bool {
    if provider.models.is_empty() {
        return true;
    }

    let last_refreshed = match provider.models.first().and_then(|m| m.last_refreshed.as_ref()) {
        Some(ts) => ts,
        None => return true,
    };

    match chrono::DateTime::parse_from_rfc3339(last_refreshed) {
        Ok(refreshed) => {
            let now = Utc::now();
            let refreshed_utc = refreshed.with_timezone(&Utc);
            let elapsed_hours = (now - refreshed_utc).num_seconds() as u64 / 3600;
            elapsed_hours >= refresh_interval_hours as u64
        }
        Err(_) => true,
    }
}

pub async fn refresh_all_providers(client: &Client) {
    let providers = match load_providers() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[model-refresher] Failed to load providers: {e}");
            return;
        }
    };

    let refresh_interval = match load_app_config() {
        Ok(cfg) => cfg.refresh_interval_hours,
        Err(_) => 24,
    };

    let providers_needing_refresh: Vec<_> = providers
        .iter()
        .filter(|p| p.enabled && needs_refresh(p, refresh_interval))
        .cloned()
        .collect();

    if providers_needing_refresh.is_empty() {
        return;
    }

    eprintln!(
        "[model-refresher] Refreshing models for {} providers",
        providers_needing_refresh.len()
    );

    let mut handles = Vec::new();

    for provider in providers_needing_refresh {
        let client = client.clone();
        let handle = tokio::spawn(async move {
            refresh_single_provider(&provider, &client).await;
        });
        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }
}

async fn refresh_single_provider(provider: &Provider, client: &Client) {
    let api_key = match get_credential(&provider.credential_key).await {
        Ok(key) => key,
        Err(e) => {
            eprintln!(
                "[model-refresher] Failed to get credential for {}: {e}",
                provider.name
            );
            return;
        }
    };

    match fetch_models_for_provider(provider, &api_key, client).await {
        Ok(models) => {
            eprintln!(
                "[model-refresher] Successfully fetched {} models for {}",
                models.len(),
                provider.name
            );

            let provider_id = provider.id.clone();
            if let Err(e) = update_providers_with_lock(|all_providers| {
                if let Some(existing) = all_providers.iter_mut().find(|p| p.id == provider_id) {
                    existing.models = models;
                }
            }) {
                eprintln!(
                    "[model-refresher] Failed to save providers after refresh: {e}"
                );
            }
        }
        Err(e) => {
            eprintln!(
                "[model-refresher] Failed to refresh models for {}: {e}",
                provider.name
            );
        }
    }
}

pub async fn refresh_provider_models(provider_id: String, client: &Client) -> Result<Vec<ProviderModel>> {
    let providers = load_providers()?;

    let provider = providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    if !provider.enabled {
        return Err(format!("Provider '{}' is disabled", provider_id).into());
    }

    let api_key = get_credential(&provider.credential_key)
        .await
        .map_err(|e| format!("Failed to get credential for {}: {}", provider.name, e))?;

    let models = fetch_models_for_provider(provider, &api_key, client).await?;

    let mut all_providers = providers;
    if let Some(existing) = all_providers.iter_mut().find(|p| p.id == provider_id) {
        existing.models = models.clone();
    }
    save_providers(&all_providers)?;

    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_iso_timestamp_format() {
        let ts = current_iso_timestamp();
        assert!(ts.ends_with("+00:00") || ts.ends_with('Z'));
        assert!(ts.contains('T'));
    }

    #[test]
    fn test_parse_iso_timestamp() {
        let ts = "2026-04-07T00:00:00Z";
        let parsed = chrono::DateTime::parse_from_rfc3339(ts);
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_needs_refresh_empty_models() {
        let provider = Provider {
            id: "test".to_string(),
            name: "Test".to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.test.com".to_string(),
            credential_key: "test".to_string(),
            daily_token_quota: None,
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![],
            model_overrides: None,
        };

        assert!(needs_refresh(&provider, 24));
    }

    #[test]
    fn test_needs_refresh_old_timestamp() {
        let model = ProviderModel {
            id: "test-model".to_string(),
            context_window: None,
            max_output_tokens: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
            last_refreshed: Some("2020-01-01T00:00:00Z".to_string()),
        };

        let provider = Provider {
            id: "test".to_string(),
            name: "Test".to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.test.com".to_string(),
            credential_key: "test".to_string(),
            daily_token_quota: None,
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![model],
            model_overrides: None,
        };

        assert!(needs_refresh(&provider, 24));
    }

    #[test]
    fn test_needs_refresh_recent_timestamp() {
        let now = current_iso_timestamp();
        let model = ProviderModel {
            id: "test-model".to_string(),
            context_window: None,
            max_output_tokens: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
            last_refreshed: Some(now),
        };

        let provider = Provider {
            id: "test".to_string(),
            name: "Test".to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.test.com".to_string(),
            credential_key: "test".to_string(),
            daily_token_quota: None,
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![model],
            model_overrides: None,
        };

        assert!(!needs_refresh(&provider, 24));
    }

    #[test]
    fn test_anthropic_hardcoded_models() {
        let now = current_iso_timestamp();
        let models = anthropic_hardcoded_models(&now);

        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id.starts_with("claude-opus-4")));
        assert!(models.iter().any(|m| m.id.starts_with("claude-sonnet-4")));
        assert!(models.iter().any(|m| m.id.starts_with("claude-3-7-sonnet")));
        assert!(models.iter().any(|m| m.id.starts_with("claude-3-5-sonnet")));
        assert!(models.iter().any(|m| m.id.starts_with("claude-3-5-haiku")));
        assert!(models.iter().any(|m| m.id.starts_with("claude-3-opus")));
        assert!(models.iter().any(|m| m.id.starts_with("claude-3-sonnet")));
        assert!(models.iter().any(|m| m.id.starts_with("claude-3-haiku")));

        for model in &models {
            assert!(model.context_window.is_some());
            assert!(model.max_output_tokens.is_some());
            assert!(model.input_cost_per_1m.is_some());
            assert!(model.output_cost_per_1m.is_some());
        }
    }

    #[test]
    fn test_get_anthropic_models_with_overrides() {
        let now = current_iso_timestamp();
        let override_model = ProviderModel {
            id: "custom-claude-model".to_string(),
            context_window: Some(100_000),
            max_output_tokens: Some(4_096),
            input_cost_per_1m: Some(5.0),
            output_cost_per_1m: Some(25.0),
            last_refreshed: None,
        };

        let provider = Provider {
            id: "test".to_string(),
            name: "Test Anthropic".to_string(),
            protocol: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            credential_key: "test".to_string(),
            daily_token_quota: None,
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![],
            model_overrides: Some(vec![override_model]),
        };

        let models = get_anthropic_models(&provider);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "custom-claude-model");
        assert!(models[0].last_refreshed.is_some());
        assert!(models[0].last_refreshed.as_ref().unwrap().starts_with("2026-"));
    }

    #[test]
    fn test_get_anthropic_models_without_overrides() {
        let provider = Provider {
            id: "test".to_string(),
            name: "Test Anthropic".to_string(),
            protocol: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            credential_key: "test".to_string(),
            daily_token_quota: None,
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![],
            model_overrides: None,
        };

        let models = get_anthropic_models(&provider);
        assert_eq!(models.len(), 8);
    }

    #[test]
    fn test_get_anthropic_models_with_empty_overrides() {
        let provider = Provider {
            id: "test".to_string(),
            name: "Test Anthropic".to_string(),
            protocol: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            credential_key: "test".to_string(),
            daily_token_quota: None,
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![],
            model_overrides: Some(vec![]),
        };

        let models = get_anthropic_models(&provider);
        assert_eq!(models.len(), 8);
    }

    #[test]
    fn test_extract_from_raw_json_with_pricing() {
        let json = serde_json::json!({
            "id": "test-model",
            "context_window": 128000,
            "max_output_tokens": 8192,
            "pricing": {
                "prompt": 0.000003,
                "completion": 0.000015
            }
        });

        let model = ProviderModel {
            id: "test-model".to_string(),
            context_window: None,
            max_output_tokens: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
            last_refreshed: None,
        };

        let now = current_iso_timestamp();
        let result = extract_from_raw_json(&json, model, &now);

        assert_eq!(result.context_window, Some(128000));
        assert_eq!(result.max_output_tokens, Some(8192));
        assert_eq!(result.input_cost_per_1m, Some(3.0));
        assert_eq!(result.output_cost_per_1m, Some(15.0));
    }

    #[test]
    fn test_extract_from_raw_json_with_cost_per_token() {
        let json = serde_json::json!({
            "id": "test-model",
            "context_length": 200000,
            "max_completion_tokens": 64000,
            "input_cost_per_token": 0.000003,
            "output_cost_per_token": 0.000015
        });

        let model = ProviderModel {
            id: "test-model".to_string(),
            context_window: None,
            max_output_tokens: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
            last_refreshed: None,
        };

        let now = current_iso_timestamp();
        let result = extract_from_raw_json(&json, model, &now);

        assert_eq!(result.context_window, Some(200000));
        assert_eq!(result.max_output_tokens, Some(64000));
        assert_eq!(result.input_cost_per_1m, Some(3.0));
        assert_eq!(result.output_cost_per_1m, Some(15.0));
    }
}
