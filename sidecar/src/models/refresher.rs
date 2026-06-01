//! Model discovery and scheduled refresh for provider model catalogues.
//!
//! Supports two discovery strategies:
//! - **Anthropic**: returns a hardcoded model list because Anthropic has no public
//!   `/v1/models` endpoint.
//! - **OpenAI-compatible**: queries the provider's `/v1/models` list endpoint and
//!   optionally the per-model detail endpoint, handling multiple response formats
//!   including OpenRouter's `top_provider`, `pricing`, and `model_spec` extensions.
//!
//! Pricing fields may be expressed as per-token costs, per-million-token costs, or
//! even quoted strings; this module normalizes everything to USD per million tokens.

use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;

use crate::config::models::{Provider, ProviderModel};
use crate::config::store::{
    load_app_config, load_providers, save_providers, update_providers_with_lock,
};
use crate::credentials::keychain::get_credential;

// Matches the current public @openai/codex release at implementation time; the
// Codex backend gates model visibility by compatible client versions.
const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.135.0";
const DEFAULT_CODEX_MAX_OUTPUT_TOKENS: u64 = 128_000;

/// Result type alias for model refresh operations, boxing errors for flexibility across IO and HTTP failures.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn normalize_pricing(value: f64) -> f64 {
    if value < 0.001 {
        value * 1_000_000.0
    } else {
        value
    }
}

/// Deserializes a JSON value that may be a number, a quoted string, or null into `Option<f64>`.
///
/// Some providers (e.g. OpenRouter) return pricing values as strings like `"0.000003"`
/// instead of native numbers, and may also use `null` as a sentinel for "not applicable".
/// This custom deserializer handles all three cases so that downstream price-per-token
/// fields never fail deserialization regardless of the wire format.
fn deserialize_string_or_float<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = Option::<serde_json::Value>::deserialize(deserializer)?;
    match val {
        None => Ok(None),
        Some(serde_json::Value::Number(n)) => Ok(n.as_f64()),
        Some(serde_json::Value::String(s)) => Ok(s.parse::<f64>().ok()),
        Some(_) => Ok(None),
    }
}

/// Top-level response envelope from OpenAI-compatible `/v1/models` endpoints.
#[derive(Debug, Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelEntry>,
}

/// Top-level response envelope from ChatGPT Codex model discovery.
#[derive(Debug, Deserialize)]
struct CodexModelsResponse {
    models: Vec<CodexModelEntry>,
}

/// A single model entry returned by ChatGPT's Codex backend.
#[derive(Debug, Deserialize)]
struct CodexModelEntry {
    slug: String,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    priority: Option<i64>,
    #[serde(default)]
    context_window: Option<i64>,
    #[serde(default)]
    max_context_window: Option<i64>,
    #[serde(default)]
    max_output_tokens: Option<i64>,
    #[serde(default)]
    max_completion_tokens: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_string_or_float")]
    input_cost_per_1m: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_string_or_float")]
    output_cost_per_1m: Option<f64>,
}

/// A single model entry returned in the list response.
///
/// Fields are optional because different providers expose varying levels of detail
/// in list vs. detail endpoints. `top_provider` and `model_spec` are OpenRouter
/// extensions that carry provider-specific metadata.
#[derive(Debug, Deserialize)]
struct OpenAiModelEntry {
    id: String,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    max_output_tokens: Option<u64>,
    #[serde(default)]
    max_completion_tokens: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_string_or_float")]
    input_cost_per_token: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_string_or_float")]
    output_cost_per_token: Option<f64>,
    #[serde(default)]
    pricing: Option<PricingInfo>,
    #[serde(default)]
    model_spec: Option<ModelSpec>,
    #[serde(default)]
    top_provider: Option<TopProvider>,
}

/// Detailed model information retrieved from a per-model detail endpoint.
///
/// Some providers expose richer metadata (context limits, pricing) only when
/// querying `/v1/models/{id}` rather than in the list response.
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
    #[serde(default, deserialize_with = "deserialize_string_or_float")]
    input_cost_per_token: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_string_or_float")]
    output_cost_per_token: Option<f64>,
}

/// OpenRouter-specific metadata embedded in each model entry, providing
/// provider-level context length and output token limits.
#[derive(Debug, Deserialize)]
struct TopProvider {
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    max_completion_tokens: Option<u64>,
}

/// OpenRouter-style pricing object where `prompt` and `completion` are per-token
/// costs that may appear as either numbers or quoted strings.
#[derive(Debug, Deserialize)]
struct PricingInfo {
    #[serde(default, deserialize_with = "deserialize_string_or_float")]
    prompt: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_string_or_float")]
    completion: Option<f64>,
}

/// OpenRouter `model_spec` extension carrying capability and pricing metadata
/// in a structured format distinct from the flat top-level fields.
#[derive(Debug, Deserialize)]
struct ModelSpec {
    #[serde(default, rename = "availableContextTokens")]
    available_context_tokens: Option<u64>,
    #[serde(default, rename = "maxCompletionTokens")]
    max_completion_tokens: Option<u64>,
    #[serde(default)]
    pricing: Option<ModelSpecPricing>,
}

/// Nested pricing inside a `model_spec`, using dollar amounts per million tokens
/// rather than per-token costs.
#[derive(Debug, Deserialize)]
struct ModelSpecPricing {
    #[serde(default)]
    input: Option<ModelSpecPrice>,
    #[serde(default)]
    output: Option<ModelSpecPrice>,
}

/// A single price point (in USD per million tokens) within a `model_spec.pricing` block.
#[derive(Debug, Deserialize)]
struct ModelSpecPrice {
    #[serde(default)]
    usd: Option<f64>,
}

/// Fetches the list of available models for a given provider.
///
/// For Anthropic providers, returns a hardcoded model list because Anthropic has
/// no public model discovery API. For Codex providers, queries ChatGPT's Codex
/// model endpoint and falls back to a local catalogue if discovery fails. For
/// all other providers (OpenAI, OpenRouter, etc.), calls the OpenAI-compatible
/// `/v1/models` endpoint and enriches each entry with per-model detail data when
/// available.
///
/// # Arguments
/// * `provider` - The provider configuration, including protocol and base URL.
/// * `api_key` - The credential used to authenticate with the provider API.
/// * `client` - A reusable HTTP client for making requests.
///
/// # Returns
/// A vector of [`ProviderModel`] entries, or an error if the API call fails.
///
/// # Errors
/// Returns an error if the HTTP request fails, the response is non-success, or
/// the response body cannot be parsed.
pub async fn fetch_models_for_provider(
    provider: &Provider,
    api_key: &str,
    client: &Client,
) -> Result<Vec<ProviderModel>> {
    if provider.protocol == "openai-codex" {
        return match fetch_codex_models(provider, api_key, client).await {
            Ok(models) => Ok(models),
            Err(e) => {
                eprintln!(
                    "[model-refresher] Failed to discover Codex models for {}; using fallback catalogue: {e}",
                    provider.name
                );
                Ok(get_codex_models(provider))
            }
        };
    }

    // If the user has overridden the model list, skip discovery entirely.
    if let Some(overrides) = &provider.model_overrides {
        if !overrides.is_empty() {
            return Ok(overrides.clone());
        }
    }
    match provider.protocol.as_str() {
        "anthropic" => Ok(get_anthropic_models(provider)),
        _ => fetch_openai_compatible_models(provider, api_key, client).await,
    }
}

/// Fetches account-entitled models from ChatGPT's Codex backend.
async fn fetch_codex_models(
    provider: &Provider,
    api_key: &str,
    client: &Client,
) -> Result<Vec<ProviderModel>> {
    let (access_token, account_id, id_token) =
        crate::proxy::codex::resolve_codex_credential(client, api_key, &provider.credential_key)
            .await?;
    let headers = crate::proxy::codex::build_codex_auth_headers(
        &access_token,
        account_id.as_deref(),
        id_token.as_deref(),
    );

    let base_url = provider.base_url.trim_end_matches('/');
    let models_url = format!("{base_url}/models");
    let client_version = std::env::var("CODEROUTER_CODEX_CLIENT_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_CODEX_CLIENT_VERSION.to_string());

    let mut request = client
        .get(&models_url)
        .query(&[("client_version", client_version.as_str())]);
    for (name, value) in headers {
        request = request.header(name, value);
    }

    let resp = request.send().await?;
    if !resp.status().is_success() {
        return Err(format!(
            "Failed to fetch Codex models from {}: HTTP {}",
            provider.name,
            resp.status()
        )
        .into());
    }

    let text = resp.text().await?;
    parse_codex_models_response(&text, &current_iso_timestamp())
}

fn positive_i64_to_u64(value: Option<i64>) -> Option<u64> {
    value.and_then(|v| u64::try_from(v).ok()).filter(|v| *v > 0)
}

fn parse_codex_models_response(text: &str, now: &str) -> Result<Vec<ProviderModel>> {
    let mut entries: Vec<CodexModelEntry> =
        serde_json::from_str::<CodexModelsResponse>(text)?.models;
    entries.sort_by_key(|entry| entry.priority.unwrap_or(i64::MAX));

    let listed_count = entries
        .iter()
        .filter(|entry| entry.visibility.as_deref() == Some("list"))
        .count();
    let prefer_listed = listed_count > 0;

    let models = entries
        .into_iter()
        .filter(|entry| !prefer_listed || entry.visibility.as_deref() == Some("list"))
        .map(|entry| ProviderModel {
            id: entry.slug,
            context_window: positive_i64_to_u64(entry.max_context_window)
                .or_else(|| positive_i64_to_u64(entry.context_window)),
            max_output_tokens: positive_i64_to_u64(entry.max_output_tokens)
                .or_else(|| positive_i64_to_u64(entry.max_completion_tokens))
                .or(Some(DEFAULT_CODEX_MAX_OUTPUT_TOKENS)),
            input_cost_per_1m: Some(entry.input_cost_per_1m.unwrap_or(0.0)),
            output_cost_per_1m: Some(entry.output_cost_per_1m.unwrap_or(0.0)),
            last_refreshed: Some(now.to_string()),
            protocol: None,
        })
        .collect();

    Ok(models)
}

/// Fetches models from any OpenAI-compatible provider by querying the `/v1/models`
/// endpoint and then attempting to GET per-model details for richer metadata.
///
/// The function first collects basic model info from the list endpoint, then for
/// each model it tries to fetch detailed info (context window, pricing, output limits).
/// Detail endpoint failures are silently ignored so partial data still flows through.
///
/// # Arguments
/// * `provider` - Provider configuration with `base_url`.
/// * `api_key` - Bearer token for the provider API.
/// * `client` - Shared HTTP client.
///
/// # Errors
/// Returns an error if the initial list endpoint returns a non-success status or
/// the response body cannot be deserialized.
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

        // Context window is reported under different field names by different providers.
        // Prefer the most specific field available, falling back through OpenRouter's
        // top_provider.context_length and model_spec.availableContextTokens.
        let entry_ctx = entry
            .context_window
            .or(entry.context_length)
            .or_else(|| entry.top_provider.as_ref().and_then(|tp| tp.context_length))
            .or_else(|| {
                entry
                    .model_spec
                    .as_ref()
                    .and_then(|ms| ms.available_context_tokens)
            });

        // Similarly for max output tokens, multiple field names are used.
        let entry_max_out = entry
            .max_output_tokens
            .or(entry.max_completion_tokens)
            .or_else(|| {
                entry
                    .top_provider
                    .as_ref()
                    .and_then(|tp| tp.max_completion_tokens)
            })
            .or_else(|| {
                entry
                    .model_spec
                    .as_ref()
                    .and_then(|ms| ms.max_completion_tokens)
            });

        // Per-token costs from the list endpoint are multiplied by 1M to produce
        // per-million-token costs. model_spec pricing is already expressed in USD
        // per million tokens, so it is used as-is.
        let entry_input_cost = entry
            .input_cost_per_token
            .map(|c| c * 1_000_000.0)
            .or_else(|| {
                entry
                    .pricing
                    .as_ref()
                    .and_then(|p| p.prompt)
                    .map(|c| normalize_pricing(c))
            })
            .or_else(|| {
                entry
                    .model_spec
                    .as_ref()
                    .and_then(|ms| ms.pricing.as_ref())
                    .and_then(|p| p.input.as_ref())
                    .and_then(|i| i.usd)
            });

        let entry_output_cost = entry
            .output_cost_per_token
            .map(|c| c * 1_000_000.0)
            .or_else(|| {
                entry
                    .pricing
                    .as_ref()
                    .and_then(|p| p.completion)
                    .map(|c| normalize_pricing(c))
            })
            .or_else(|| {
                entry
                    .model_spec
                    .as_ref()
                    .and_then(|ms| ms.pricing.as_ref())
                    .and_then(|p| p.output.as_ref())
                    .and_then(|o| o.usd)
            });

        let mut model = ProviderModel {
            id: entry.id.clone(),
            context_window: entry_ctx,
            max_output_tokens: entry_max_out,
            input_cost_per_1m: entry_input_cost,
            output_cost_per_1m: entry_output_cost,
            last_refreshed: Some(now.clone()),
            protocol: None,
        };

        // Attempt to fetch per-model detail for richer metadata. A failure here
        // is non-fatal; we keep the basic data from the list endpoint.
        if let Ok(detail_resp) = client
            .get(&detail_url)
            .header("Authorization", format!("Bearer {api_key}"))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
        {
            if detail_resp.status().is_success() {
                if let Ok(text) = detail_resp.text().await {
                    if let Ok(detail) = parse_model_detail(&text) {
                        // Detail endpoint data overrides list data for context and output limits
                        // only when the detail endpoint actually provides those fields.
                        let detail_ctx = detail.context_window.or(detail.context_length);
                        if detail_ctx.is_some() {
                            model.context_window = detail_ctx;
                        }

                        let detail_max = detail.max_output_tokens.or(detail.max_completion_tokens);
                        if detail_max.is_some() {
                            model.max_output_tokens = detail_max;
                        }

                        // Detail endpoint pricing overrides list pricing when present.
                        // PricingInfo fields are per-token costs, so they need the 1M multiplier.
                        if let Some(pricing) = detail.pricing {
                            if let Some(prompt) = pricing.prompt {
                                model.input_cost_per_1m = Some(normalize_pricing(prompt));
                            }
                            if let Some(completion) = pricing.completion {
                                model.output_cost_per_1m = Some(normalize_pricing(completion));
                            }
                        }

                        // Top-level per-token fields from the detail endpoint override
                        // everything else, as they are the most authoritative source.
                        if let Some(cost) = detail.input_cost_per_token {
                            model.input_cost_per_1m = Some(cost * 1_000_000.0);
                        }
                        if let Some(cost) = detail.output_cost_per_token {
                            model.output_cost_per_1m = Some(cost * 1_000_000.0);
                        }
                    } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                        // The detail endpoint returned valid JSON but it didn't match
                        // OpenAiModelDetail — fall back to manual field extraction.
                        model = extract_from_raw_json(&value, model, &now);
                    }
                }
            }
        }

        result.push(model);
    }

    Ok(result)
}

/// Attempts to parse a model detail response body into [`OpenAiModelDetail`].
///
/// Falls back to an all-`None` struct on deserialization failure so callers can
/// still use partial data from the list response.
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

/// Extracts model metadata from an arbitrary JSON object when structured
/// deserialization fails. Handles multiple provider-specific formats:
///
/// - OpenAI-style: `context_window`, `max_output_tokens`, `input_cost_per_token`
/// - OpenRouter-style: `context_length`, `max_completion_tokens`, `pricing.prompt`/`completion`
/// - OpenRouter `top_provider`: nested `context_length` and `max_completion_tokens`
/// - OpenRouter `model_spec`: `availableContextTokens`, `maxCompletionTokens`, structured pricing
///
/// For context/token values, top-level keys take precedence; `model_spec` fields
/// are used only as fallbacks (via `.or()`) so that explicit data is not overwritten.
/// For pricing, per-token costs and PricingInfo are applied unconditionally since
/// they represent the most authoritative source available at that extraction stage.
fn extract_from_raw_json(
    value: &serde_json::Value,
    mut model: ProviderModel,
    now: &str,
) -> ProviderModel {
    if let Some(obj) = value.as_object() {
        // Top-level context fields: unconditional assignment overrides list data.
        if let Some(v) = obj.get("context_window").and_then(|v| v.as_u64()) {
            model.context_window = Some(v);
        }
        // context_length is a fallback only if context_window hasn't been set.
        if let Some(v) = obj.get("context_length").and_then(|v| v.as_u64()) {
            model.context_window = model.context_window.or(Some(v));
        }
        if let Some(v) = obj.get("max_output_tokens").and_then(|v| v.as_u64()) {
            model.max_output_tokens = Some(v);
        }
        if let Some(v) = obj.get("max_completion_tokens").and_then(|v| v.as_u64()) {
            model.max_output_tokens = model.max_output_tokens.or(Some(v));
        }

        // OpenRouter-style pricing object with per-token prompt/completion costs.
        if let Some(pricing) = obj.get("pricing").and_then(|v| v.as_object()) {
            if let Some(prompt) = pricing.get("prompt") {
                let parsed = prompt
                    .as_f64()
                    .or_else(|| prompt.as_str().and_then(|s| s.parse::<f64>().ok()));
                if let Some(p) = parsed {
                    model.input_cost_per_1m = Some(normalize_pricing(p));
                }
            }
            if let Some(completion) = pricing.get("completion") {
                let parsed = completion
                    .as_f64()
                    .or_else(|| completion.as_str().and_then(|s| s.parse::<f64>().ok()));
                if let Some(p) = parsed {
                    model.output_cost_per_1m = Some(normalize_pricing(p));
                }
            }
        }

        // Top-level per-token cost fields override pricing-object values.
        if let Some(v) = obj.get("input_cost_per_token") {
            let parsed = v
                .as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()));
            if let Some(p) = parsed {
                model.input_cost_per_1m = Some(p * 1_000_000.0);
            }
        }
        if let Some(v) = obj.get("output_cost_per_token") {
            let parsed = v
                .as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()));
            if let Some(p) = parsed {
                model.output_cost_per_1m = Some(p * 1_000_000.0);
            }
        }

        // model_spec fields are used as fallback when top-level values are missing.
        if let Some(spec) = obj.get("model_spec").and_then(|v| v.as_object()) {
            if let Some(v) = spec.get("availableContextTokens").and_then(|v| v.as_u64()) {
                model.context_window = model.context_window.or(Some(v));
            }
            if let Some(v) = spec.get("maxCompletionTokens").and_then(|v| v.as_u64()) {
                model.max_output_tokens = model.max_output_tokens.or(Some(v));
            }
            if let Some(spec_pricing) = spec.get("pricing").and_then(|v| v.as_object()) {
                // model_spec pricing is in USD per million tokens, so no multiplier needed.
                if let Some(usd) = spec_pricing
                    .get("input")
                    .and_then(|v| v.as_object())
                    .and_then(|o| o.get("usd"))
                    .and_then(|v| v.as_f64())
                {
                    model.input_cost_per_1m = model.input_cost_per_1m.or(Some(usd));
                }
                if let Some(usd) = spec_pricing
                    .get("output")
                    .and_then(|v| v.as_object())
                    .and_then(|o| o.get("usd"))
                    .and_then(|v| v.as_f64())
                {
                    model.output_cost_per_1m = model.output_cost_per_1m.or(Some(usd));
                }
            }
        }
    }

    model.last_refreshed = Some(now.to_string());
    model
}

/// Returns the hardcoded list of Anthropic models with their specifications.
///
/// Anthropic does not expose a public model discovery API, so the available
/// models with their context windows, output limits, and per-million-token
/// pricing are maintained here. If `model_overrides` are set on the provider
/// config, those are returned instead (with updated `last_refreshed` timestamps).
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

/// Returns the hardcoded fallback list of Codex models with their specifications.
///
/// If remote discovery fails, model overrides are still honored as a local
/// fallback. Successful remote discovery does not use overrides.
fn get_codex_models(provider: &Provider) -> Vec<ProviderModel> {
    let now = current_iso_timestamp();

    let hardcoded = codex_hardcoded_models(&now);

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

/// Known Codex model catalogue.
fn codex_hardcoded_models(now: &str) -> Vec<ProviderModel> {
    let codex_model = |id: &str| ProviderModel {
        id: id.to_string(),
        context_window: Some(400_000),
        max_output_tokens: Some(DEFAULT_CODEX_MAX_OUTPUT_TOKENS),
        input_cost_per_1m: Some(0.0),
        output_cost_per_1m: Some(0.0),
        last_refreshed: Some(now.to_string()),
        protocol: None,
    };

    vec![
        codex_model("gpt-5.5"),
        codex_model("gpt-5.4"),
        codex_model("gpt-5.4-mini"),
        codex_model("gpt-5.3-codex"),
        codex_model("gpt-5.3-codex-spark"),
        codex_model("gpt-5.2"),
        codex_model("gpt-5.2-codex"),
        codex_model("gpt-5.1-codex"),
        codex_model("gpt-5.1-codex-mini"),
        codex_model("gpt-5.1-codex-max"),
        codex_model("gpt-5-codex"),
    ]
}

/// Known Anthropic model catalogue with context windows, output token limits,
/// and pricing (USD per million tokens). Updated manually when Anthropic
/// releases new models or changes pricing.
fn anthropic_hardcoded_models(now: &str) -> Vec<ProviderModel> {
    vec![
        ProviderModel {
            id: "claude-opus-4-20250514".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(64_000),
            input_cost_per_1m: Some(15.0),
            output_cost_per_1m: Some(75.0),
            last_refreshed: Some(now.to_string()),
            protocol: None,
        },
        ProviderModel {
            id: "claude-sonnet-4-20250514".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(64_000),
            input_cost_per_1m: Some(3.0),
            output_cost_per_1m: Some(15.0),
            last_refreshed: Some(now.to_string()),
            protocol: None,
        },
        ProviderModel {
            id: "claude-3-7-sonnet-20250219".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(64_000),
            input_cost_per_1m: Some(3.0),
            output_cost_per_1m: Some(15.0),
            last_refreshed: Some(now.to_string()),
            protocol: None,
        },
        ProviderModel {
            id: "claude-3-5-sonnet-20241022".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(8_192),
            input_cost_per_1m: Some(3.0),
            output_cost_per_1m: Some(15.0),
            last_refreshed: Some(now.to_string()),
            protocol: None,
        },
        ProviderModel {
            id: "claude-3-5-haiku-20241022".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(8_192),
            input_cost_per_1m: Some(0.80),
            output_cost_per_1m: Some(4.0),
            last_refreshed: Some(now.to_string()),
            protocol: None,
        },
        ProviderModel {
            id: "claude-3-opus-20240229".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(4_096),
            input_cost_per_1m: Some(15.0),
            output_cost_per_1m: Some(75.0),
            last_refreshed: Some(now.to_string()),
            protocol: None,
        },
        ProviderModel {
            id: "claude-3-sonnet-20240229".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(4_096),
            input_cost_per_1m: Some(3.0),
            output_cost_per_1m: Some(15.0),
            last_refreshed: Some(now.to_string()),
            protocol: None,
        },
        ProviderModel {
            id: "claude-3-haiku-20240307".to_string(),
            context_window: Some(200_000),
            max_output_tokens: Some(4_096),
            input_cost_per_1m: Some(0.25),
            output_cost_per_1m: Some(1.25),
            last_refreshed: Some(now.to_string()),
            protocol: None,
        },
    ]
}

/// Returns the current UTC timestamp in RFC 3339 / ISO 8601 format.
fn current_iso_timestamp() -> String {
    Utc::now().to_rfc3339()
}

/// Determines whether a provider's model list should be refreshed based on age.
///
/// A provider needs refresh if its model list is empty, if no `last_refreshed`
/// timestamp exists, if the timestamp cannot be parsed, or if more than
/// `refresh_interval_hours` have elapsed since the last successful refresh.
fn needs_refresh(provider: &Provider, refresh_interval_hours: u32) -> bool {
    if provider.models.is_empty() {
        return true;
    }

    let last_refreshed = match provider
        .models
        .first()
        .and_then(|m| m.last_refreshed.as_ref())
    {
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

/// Refreshes model lists for all enabled providers that have stale data.
///
/// Loads the provider list and app config to determine the refresh interval,
/// then spawns concurrent tasks for each provider that needs updating.
/// Errors are logged but do not short-circuit other providers.
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

/// Refreshes a single provider's model list by fetching current data and
/// persisting it via the lock-protected store update.
///
/// Credential lookup failures and API errors are logged; the provider is
/// silently skipped rather than causing a panic.
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
                eprintln!("[model-refresher] Failed to save providers after refresh: {e}");
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

/// Refreshes model metadata for a specific provider by ID and returns the updated list.
///
/// Unlike [`refresh_all_providers`], this function is request-scoped: it validates
/// the provider exists and is enabled, fetches the models, and writes back to the
/// store. It uses `save_providers` rather than the lock-based update because it
/// runs in a single-user context (the TUI).
///
/// # Arguments
/// * `provider_id` - The unique identifier of the provider to refresh.
/// * `client` - A reusable HTTP client for the API request.
///
/// # Returns
/// The refreshed model list on success.
///
/// # Errors
/// Returns an error if the provider is not found, disabled, credential lookup
/// fails, the API call fails, or the store cannot be updated.
pub async fn refresh_provider_models(
    provider_id: String,
    client: &Client,
) -> Result<Vec<ProviderModel>> {
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
    fn test_parse_codex_models_prefers_list_visibility_and_priority() {
        let json = r#"{
            "models": [
                {
                    "slug": "hidden-model",
                    "visibility": "hidden",
                    "priority": 0,
                    "max_context_window": 800000
                },
                {
                    "slug": "gpt-5.5",
                    "visibility": "list",
                    "priority": 2,
                    "context_window": 400000,
                    "max_completion_tokens": 64000
                },
                {
                    "slug": "gpt-5.4",
                    "visibility": "list",
                    "priority": 1,
                    "max_context_window": 500000,
                    "context_window": 400000,
                    "max_output_tokens": 128000
                }
            ]
        }"#;

        let models = parse_codex_models_response(json, "2026-01-01T00:00:00Z").unwrap();

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5.4");
        assert_eq!(models[0].context_window, Some(500000));
        assert_eq!(models[0].max_output_tokens, Some(128000));
        assert_eq!(models[0].input_cost_per_1m, Some(0.0));
        assert_eq!(models[0].output_cost_per_1m, Some(0.0));
        assert_eq!(models[1].id, "gpt-5.5");
        assert_eq!(models[1].max_output_tokens, Some(64000));
        assert!(models.iter().all(|m| m.last_refreshed.is_some()));
    }

    #[test]
    fn test_parse_codex_models_keeps_all_when_no_list_visibility() {
        let json = r#"{
            "models": [
                {
                    "slug": "alpha",
                    "visibility": "hidden",
                    "priority": 2,
                    "context_window": -1,
                    "max_output_tokens": -1
                },
                {
                    "slug": "beta",
                    "visibility": "internal",
                    "priority": 1,
                    "context_window": 200000
                }
            ]
        }"#;

        let models = parse_codex_models_response(json, "2026-01-01T00:00:00Z").unwrap();

        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["beta", "alpha"]
        );
        assert_eq!(models[0].context_window, Some(200000));
        assert_eq!(
            models[0].max_output_tokens,
            Some(DEFAULT_CODEX_MAX_OUTPUT_TOKENS)
        );
        assert_eq!(models[1].context_window, None);
        assert_eq!(
            models[1].max_output_tokens,
            Some(DEFAULT_CODEX_MAX_OUTPUT_TOKENS)
        );
    }

    #[test]
    fn test_codex_fallback_models_include_gpt_5_5_with_zero_cost() {
        let models = codex_hardcoded_models("2026-01-01T00:00:00Z");
        let model = models.iter().find(|m| m.id == "gpt-5.5").unwrap();

        assert_eq!(model.input_cost_per_1m, Some(0.0));
        assert_eq!(model.output_cost_per_1m, Some(0.0));
        assert_eq!(
            model.max_output_tokens,
            Some(DEFAULT_CODEX_MAX_OUTPUT_TOKENS)
        );
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
            protocol: None,
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
            protocol: None,
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
        let _now = current_iso_timestamp();
        let override_model = ProviderModel {
            id: "custom-claude-model".to_string(),
            context_window: Some(100_000),
            max_output_tokens: Some(4_096),
            input_cost_per_1m: Some(5.0),
            output_cost_per_1m: Some(25.0),
            last_refreshed: None,
            protocol: None,
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
        assert!(models[0]
            .last_refreshed
            .as_ref()
            .unwrap()
            .starts_with("2026-"));
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
            protocol: None,
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
            protocol: None,
        };

        let now = current_iso_timestamp();
        let result = extract_from_raw_json(&json, model, &now);

        assert_eq!(result.context_window, Some(200000));
        assert_eq!(result.max_output_tokens, Some(64000));
        assert_eq!(result.input_cost_per_1m, Some(3.0));
        assert_eq!(result.output_cost_per_1m, Some(15.0));
    }

    #[test]
    fn test_openai_model_entry_with_model_spec() {
        let json = serde_json::json!({
            "id": "qwen3-235b-a22b",
            "object": "model",
            "model_spec": {
                "availableContextTokens": 131072,
                "maxCompletionTokens": 24000,
                "pricing": {
                    "input": { "usd": 0.15 },
                    "output": { "usd": 0.60 }
                }
            }
        });

        let entry: OpenAiModelEntry = serde_json::from_value(json).unwrap();
        assert_eq!(entry.id, "qwen3-235b-a22b");
        let spec = entry.model_spec.unwrap();
        assert_eq!(spec.available_context_tokens, Some(131072));
        assert_eq!(spec.max_completion_tokens, Some(24000));
        let spec_pricing = spec.pricing.unwrap();
        assert_eq!(spec_pricing.input.unwrap().usd, Some(0.15));
        assert_eq!(spec_pricing.output.unwrap().usd, Some(0.60));
    }

    #[test]
    fn test_openai_model_entry_top_level_fields() {
        let json = serde_json::json!({
            "id": "test-model",
            "context_window": 128000,
            "max_output_tokens": 8192,
            "input_cost_per_token": 0.000003,
            "output_cost_per_token": 0.000015
        });

        let entry: OpenAiModelEntry = serde_json::from_value(json).unwrap();
        assert_eq!(entry.id, "test-model");
        assert_eq!(entry.context_window, Some(128000));
        assert_eq!(entry.max_output_tokens, Some(8192));
        assert_eq!(entry.input_cost_per_token, Some(0.000003));
        assert_eq!(entry.output_cost_per_token, Some(0.000015));
    }

    #[test]
    fn test_extract_from_raw_json_with_model_spec() {
        let json = serde_json::json!({
            "id": "qwen3-235b-a22b",
            "model_spec": {
                "availableContextTokens": 131072,
                "maxCompletionTokens": 24000,
                "pricing": {
                    "input": { "usd": 0.15 },
                    "output": { "usd": 0.60 }
                }
            }
        });

        let model = ProviderModel {
            id: "qwen3-235b-a22b".to_string(),
            context_window: None,
            max_output_tokens: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
            last_refreshed: None,
            protocol: None,
        };

        let now = current_iso_timestamp();
        let result = extract_from_raw_json(&json, model, &now);

        assert_eq!(result.context_window, Some(131072));
        assert_eq!(result.max_output_tokens, Some(24000));
        assert_eq!(result.input_cost_per_1m, Some(0.15));
        assert_eq!(result.output_cost_per_1m, Some(0.60));
    }

    #[test]
    fn test_extract_from_raw_json_model_spec_preserves_existing() {
        let json = serde_json::json!({
            "id": "test",
            "context_window": 64000,
            "model_spec": {
                "availableContextTokens": 131072
            }
        });

        let model = ProviderModel {
            id: "test".to_string(),
            context_window: Some(64000),
            max_output_tokens: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
            last_refreshed: None,
            protocol: None,
        };

        let now = current_iso_timestamp();
        let result = extract_from_raw_json(&json, model, &now);

        assert_eq!(result.context_window, Some(64000));
    }

    #[test]
    fn test_extract_from_raw_json_model_spec_fills_missing() {
        let json = serde_json::json!({
            "id": "test",
            "model_spec": {
                "availableContextTokens": 131072,
                "maxCompletionTokens": 24000,
                "pricing": {
                    "input": { "usd": 0.15 },
                    "output": { "usd": 0.60 }
                }
            }
        });

        let model = ProviderModel {
            id: "test".to_string(),
            context_window: None,
            max_output_tokens: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
            last_refreshed: None,
            protocol: None,
        };

        let now = current_iso_timestamp();
        let result = extract_from_raw_json(&json, model, &now);

        assert_eq!(result.context_window, Some(131072));
        assert_eq!(result.max_output_tokens, Some(24000));
        assert_eq!(result.input_cost_per_1m, Some(0.15));
        assert_eq!(result.output_cost_per_1m, Some(0.60));
    }

    #[test]
    fn test_openrouter_model_entry_string_pricing() {
        let json = serde_json::json!({
            "id": "anthropic/claude-sonnet-4",
            "context_length": 200000,
            "pricing": {
                "prompt": "0.000003",
                "completion": "0.000015"
            },
            "top_provider": {
                "context_length": 200000,
                "max_completion_tokens": 64000
            }
        });

        let entry: OpenAiModelEntry = serde_json::from_value(json).unwrap();
        assert_eq!(entry.id, "anthropic/claude-sonnet-4");
        assert_eq!(entry.context_length, Some(200000));
        let pricing = entry.pricing.unwrap();
        assert_eq!(pricing.prompt, Some(0.000003));
        assert_eq!(pricing.completion, Some(0.000015));

        let tp = entry.top_provider.unwrap();
        assert_eq!(tp.context_length, Some(200000));
        assert_eq!(tp.max_completion_tokens, Some(64000));
    }

    #[test]
    fn test_openrouter_model_entry_numeric_pricing() {
        let json = serde_json::json!({
            "id": "test-model",
            "pricing": {
                "prompt": 0.000003,
                "completion": 0.000015
            }
        });

        let entry: OpenAiModelEntry = serde_json::from_value(json).unwrap();
        let pricing = entry.pricing.unwrap();
        assert_eq!(pricing.prompt, Some(0.000003));
        assert_eq!(pricing.completion, Some(0.000015));
    }

    #[test]
    fn test_pricing_info_null_values() {
        let json = serde_json::json!({
            "prompt": null,
            "completion": null
        });
        let pricing: PricingInfo = serde_json::from_value(json).unwrap();
        assert_eq!(pricing.prompt, None);
        assert_eq!(pricing.completion, None);
    }

    #[test]
    fn test_pricing_info_missing_fields() {
        let json = serde_json::json!({});
        let pricing: PricingInfo = serde_json::from_value(json).unwrap();
        assert_eq!(pricing.prompt, None);
        assert_eq!(pricing.completion, None);
    }

    #[test]
    fn test_pricing_info_sentinel_string() {
        let json = serde_json::json!({
            "prompt": "-1",
            "completion": "free"
        });
        let pricing: PricingInfo = serde_json::from_value(json).unwrap();
        assert_eq!(pricing.prompt, Some(-1.0));
        assert_eq!(pricing.completion, None);
    }

    #[test]
    fn test_top_provider_fallback_context_length() {
        let json = serde_json::json!({
            "id": "test-model",
            "top_provider": {
                "context_length": 128000,
                "max_completion_tokens": 4096
            }
        });

        let entry: OpenAiModelEntry = serde_json::from_value(json).unwrap();
        assert_eq!(entry.context_window, None);
        assert_eq!(entry.context_length, None);

        let tp = entry.top_provider.as_ref().unwrap();
        assert_eq!(tp.context_length, Some(128000));
        assert_eq!(tp.max_completion_tokens, Some(4096));
    }

    #[test]
    fn test_extract_from_raw_json_string_pricing() {
        let json = serde_json::json!({
            "id": "test-model",
            "context_window": 128000,
            "pricing": {
                "prompt": "0.000003",
                "completion": "0.000015"
            }
        });

        let model = ProviderModel {
            id: "test-model".to_string(),
            context_window: None,
            max_output_tokens: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
            last_refreshed: None,
            protocol: None,
        };

        let now = current_iso_timestamp();
        let result = extract_from_raw_json(&json, model, &now);

        assert_eq!(result.context_window, Some(128000));
        assert_eq!(result.input_cost_per_1m, Some(3.0));
        assert_eq!(result.output_cost_per_1m, Some(15.0));
    }

    #[test]
    fn test_extract_from_raw_json_string_cost_per_token() {
        let json = serde_json::json!({
            "id": "test-model",
            "input_cost_per_token": "0.000003",
            "output_cost_per_token": "0.000015"
        });

        let model = ProviderModel {
            id: "test-model".to_string(),
            context_window: None,
            max_output_tokens: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
            last_refreshed: None,
            protocol: None,
        };

        let now = current_iso_timestamp();
        let result = extract_from_raw_json(&json, model, &now);

        assert_eq!(result.input_cost_per_1m, Some(3.0));
        assert_eq!(result.output_cost_per_1m, Some(15.0));
    }

    #[test]
    fn test_openai_model_entry_string_cost_per_token() {
        let json = serde_json::json!({
            "id": "test-model",
            "input_cost_per_token": "0.000003",
            "output_cost_per_token": "0.000015"
        });

        let entry: OpenAiModelEntry = serde_json::from_value(json).unwrap();
        assert_eq!(entry.input_cost_per_token, Some(0.000003));
        assert_eq!(entry.output_cost_per_token, Some(0.000015));
    }

    #[test]
    fn test_openai_model_detail_string_cost_per_token() {
        let json = serde_json::json!({
            "input_cost_per_token": "0.000003",
            "output_cost_per_token": "0.000015"
        });

        let detail: OpenAiModelDetail = serde_json::from_value(json).unwrap();
        assert_eq!(detail.input_cost_per_token, Some(0.000003));
        assert_eq!(detail.output_cost_per_token, Some(0.000015));
    }

    #[test]
    fn test_openrouter_full_response_string_pricing() {
        let json = serde_json::json!({
            "data": [
                {
                    "id": "anthropic/claude-sonnet-4",
                    "context_length": 200000,
                    "pricing": {
                        "prompt": "0.000003",
                        "completion": "0.000015"
                    },
                    "top_provider": {
                        "context_length": 200000,
                        "max_completion_tokens": 64000
                    }
                },
                {
                    "id": "openai/gpt-4o",
                    "pricing": {
                        "prompt": "0.000005",
                        "completion": "0.000015"
                    }
                }
            ]
        });

        let resp: OpenAiModelsResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.data.len(), 2);

        let claude = &resp.data[0];
        assert_eq!(claude.id, "anthropic/claude-sonnet-4");
        let pricing = claude.pricing.as_ref().unwrap();
        assert_eq!(pricing.prompt, Some(0.000003));
        assert_eq!(pricing.completion, Some(0.000015));
        let tp = claude.top_provider.as_ref().unwrap();
        assert_eq!(tp.context_length, Some(200000));
        assert_eq!(tp.max_completion_tokens, Some(64000));

        let gpt = &resp.data[1];
        assert_eq!(gpt.id, "openai/gpt-4o");
        let pricing = gpt.pricing.as_ref().unwrap();
        assert_eq!(pricing.prompt, Some(0.000005));
        assert_eq!(pricing.completion, Some(0.000015));
        assert!(gpt.top_provider.is_none());
    }

    #[test]
    fn test_normalize_per_token_pricing() {
        assert!((normalize_pricing(0.000003) - 3.0).abs() < 0.001);
    }

    #[test]
    fn test_normalize_per_million_pricing() {
        assert!((normalize_pricing(0.40) - 0.40).abs() < 0.001);
    }

    #[test]
    fn test_normalize_boundary_pricing() {
        assert!((normalize_pricing(0.001) - 0.001).abs() < 0.0001);
    }

    #[test]
    fn test_crofai_style_pricing_in_full_response() {
        let json = serde_json::json!({
            "data": [
                {
                    "id": "deepseek-v4-pro",
                    "context_length": 1000000,
                    "max_completion_tokens": 131072,
                    "pricing": {
                        "prompt": "0.40",
                        "completion": "0.85"
                    }
                }
            ]
        });

        let resp: OpenAiModelsResponse = serde_json::from_value(json).unwrap();
        let entry = &resp.data[0];
        let pricing = entry.pricing.as_ref().unwrap();
        assert!((pricing.prompt.unwrap() - 0.40).abs() < 0.001);
        assert!((pricing.completion.unwrap() - 0.85).abs() < 0.001);
    }
}
