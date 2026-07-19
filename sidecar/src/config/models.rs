//! Configuration data models.
//!
//! Defines the serde-based structs that represent the coderouter proxy
//! configuration: providers, model groups, failover rules, and application
//! settings. Every struct derives `Serialize`/`Deserialize` so they map
//! directly to the on-disk JSON files.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const ALLOWED_REASONING_EFFORTS: &[&str] = &["none", "low", "medium", "high", "xhigh", "max"];

/// Describes a single model offered by a provider.
///
/// Contains pricing and capability metadata that the proxy uses for routing
/// decisions and cost tracking.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProviderModel {
    /// The model identifier as recognised by the upstream provider (e.g. `gpt-4o`).
    pub id: String,
    /// Maximum combined input and generated-output tokens in one request context.
    #[serde(default)]
    pub context_window: Option<u64>,
    /// Maximum number of tokens the model can generate in a single response.
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    /// Cost in USD per one million input tokens.
    #[serde(default)]
    pub input_cost_per_1m: Option<f64>,
    /// Cost in USD per one million output tokens.
    #[serde(default)]
    pub output_cost_per_1m: Option<f64>,
    /// ISO-8601 timestamp of when this model's metadata was last refreshed from the provider.
    #[serde(default)]
    pub last_refreshed: Option<String>,
    /// Optional protocol override for this specific model (e.g. `"anthropic"`) when it differs
    /// from the parent provider's default protocol.
    #[serde(default)]
    pub protocol: Option<String>,
}

/// Represents an upstream AI provider (e.g. OpenAI, Anthropic, Google).
///
/// Each provider has a base URL, authentication key reference, optional daily
/// quotas, and a list of available models.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Provider {
    /// Unique identifier for this provider configuration.
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Wire protocol used by this provider (`"openai"`, `"anthropic"`, etc.).
    pub protocol: String,
    /// Base URL for API requests (e.g. `https://api.openai.com/v1`).
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    /// Key used to look up the API credential from the secret store.
    #[serde(rename = "credentialKey")]
    pub credential_key: String,
    /// Optional maximum number of tokens that can be consumed per day.
    #[serde(default, rename = "dailyTokenQuota")]
    pub daily_token_quota: Option<u64>,
    /// Optional maximum number of API requests allowed per day.
    #[serde(default, rename = "dailyRequestQuota")]
    pub daily_request_quota: Option<u64>,
    /// UTC hour (0-23) at which daily quota counters reset. Defaults to midnight UTC.
    #[serde(default = "default_quota_reset_hour", rename = "quotaResetUtcHour")]
    pub quota_reset_utc_hour: u32,
    /// Whether this provider is actively used for routing. Defaults to `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// List of models available from this provider.
    #[serde(default)]
    pub models: Vec<ProviderModel>,
    /// Optional model metadata overrides that take precedence over the entries in `models`.
    /// Used to apply local pricing or capability corrections without modifying the provider list.
    #[serde(default, rename = "modelOverrides")]
    pub model_overrides: Option<Vec<ProviderModel>>,
}

impl Provider {
    pub fn resolve_model_meta(&self, model_id: &str) -> Option<(Option<u64>, Option<u64>)> {
        let base = self.models.iter().find(|m| m.id == model_id);
        let override_entry = self
            .model_overrides
            .as_ref()
            .and_then(|overrides| overrides.iter().find(|m| m.id == model_id));

        match (base, override_entry) {
            (Some(b), Some(o)) => {
                let context = o.context_window.or(b.context_window);
                let max_output = o.max_output_tokens.or(b.max_output_tokens);
                Some((context, max_output))
            }
            (Some(b), None) => Some((b.context_window, b.max_output_tokens)),
            (None, Some(o)) => Some((o.context_window, o.max_output_tokens)),
            (None, None) => None,
        }
    }

    /// Resolves the wire protocol for a model, with local overrides taking precedence.
    pub fn resolve_model_protocol<'a>(&'a self, model_id: &str) -> &'a str {
        self.model_overrides
            .as_ref()
            .and_then(|overrides| overrides.iter().find(|model| model.id == model_id))
            .and_then(|model| model.protocol.as_deref())
            .or_else(|| {
                self.models
                    .iter()
                    .find(|model| model.id == model_id)
                    .and_then(|model| model.protocol.as_deref())
            })
            .unwrap_or(&self.protocol)
    }
}

fn default_quota_reset_hour() -> u32 {
    0
}

fn default_true() -> bool {
    true
}

/// A single entry within a [`Group`], binding a provider + model pair at a
/// given priority level.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GroupEntry {
    /// The [`Provider::id`] this entry targets.
    #[serde(rename = "providerId")]
    pub provider_id: String,
    /// The [`ProviderModel::id`] this entry targets.
    #[serde(rename = "modelId")]
    pub model_id: String,
    /// Routing priority — lower values are tried first during failover.
    pub priority: u32,
    /// Optional per-entry override for the provider's daily token quota.
    #[serde(default, rename = "dailyTokenQuotaOverride")]
    pub daily_token_quota_override: Option<u64>,
    /// Whether this entry is considered during routing. Defaults to `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Current operational status: `"active"`, `"cooldown"`, etc. Defaults to `"active"`.
    #[serde(default = "default_active_status")]
    pub status: String,
    /// ISO-8601 timestamp after which a cooldown entry becomes active again.
    #[serde(default, rename = "cooldownUntil")]
    pub cooldown_until: Option<String>,
}

fn default_active_status() -> String {
    "active".to_string()
}

/// Controls how often MoA reference advisors are refreshed during an agentic turn.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MoaFanoutMode {
    /// Run advisors once for each user turn and reuse their guidance across tool iterations.
    UserTurn,
    /// Refresh advisors whenever their advisory view of the conversation changes.
    #[default]
    PerIteration,
}

fn default_moa_reference_max_tokens() -> Option<u32> {
    Some(600)
}

const MOA_REFERENCE_INPUT_OVERHEAD_TOKENS: u64 = 1_024;
const MOA_SUMMARY_BASE_OVERHEAD_TOKENS: u64 = 128;
const MOA_SUMMARY_PER_REFERENCE_OVERHEAD_TOKENS: u64 = 64;

/// Optional group-level Mixture of Agents configuration.
///
/// When enabled, a group fans out non-streaming advisory requests to
/// `reference_group_ids`, then routes a final aggregator request through
/// `aggregator_group_id`. The referenced and aggregator groups still use the
/// normal priority/failover routing path.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AggregationConfig {
    /// Enables MoA routing for the parent group. Missing config defaults to normal failover.
    #[serde(default)]
    pub enabled: bool,
    /// Group IDs used for advisory reference calls.
    #[serde(default, rename = "referenceGroupIds")]
    pub reference_group_ids: Vec<String>,
    /// Group ID used for the final aggregator call.
    #[serde(default, rename = "aggregatorGroupId")]
    pub aggregator_group_id: Option<String>,
    /// Controls whether references run once per user turn or after every tool iteration.
    #[serde(default)]
    pub fanout: MoaFanoutMode,
    /// Optional output-token ceiling for advisory calls on providers that support one.
    #[serde(
        default = "default_moa_reference_max_tokens",
        rename = "referenceMaxTokens"
    )]
    pub reference_max_tokens: Option<u32>,
    /// Optional temperature override for reference calls.
    #[serde(default, rename = "referenceTemperature")]
    pub reference_temperature: Option<f64>,
    /// Optional temperature override for the aggregator call.
    #[serde(default, rename = "aggregatorTemperature")]
    pub aggregator_temperature: Option<f64>,
    /// Optional reasoning effort override per reference group ID.
    #[serde(
        default,
        rename = "referenceReasoningEfforts",
        skip_serializing_if = "HashMap::is_empty"
    )]
    pub reference_reasoning_efforts: HashMap<String, String>,
    /// Optional reasoning effort override for the final aggregator call.
    #[serde(
        default,
        rename = "aggregatorReasoningEffort",
        skip_serializing_if = "Option::is_none"
    )]
    pub aggregator_reasoning_effort: Option<String>,
    /// If true, any reference failure fails the full MoA request.
    #[serde(default, rename = "requireAllReferences")]
    pub require_all_references: bool,
}

impl Default for AggregationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            reference_group_ids: Vec::new(),
            aggregator_group_id: None,
            fanout: MoaFanoutMode::PerIteration,
            reference_max_tokens: default_moa_reference_max_tokens(),
            reference_temperature: None,
            aggregator_temperature: None,
            reference_reasoning_efforts: HashMap::new(),
            aggregator_reasoning_effort: None,
            require_all_references: false,
        }
    }
}

fn validate_reasoning_effort(effort: &str) -> bool {
    ALLOWED_REASONING_EFFORTS.contains(&effort)
}

fn allowed_reasoning_efforts_label() -> &'static str {
    "none, low, medium, high, xhigh, max"
}

/// Controls when and how the proxy fails over to the next [`GroupEntry`].
///
/// Each trigger has an independent flag and associated cooldown/timeout
/// parameter so operators can fine-tune failover behaviour.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FailoverConfig {
    /// Fail over when the upstream returns HTTP 429 (Too Many Requests). Defaults to `true`.
    #[serde(default = "default_true", rename = "on429")]
    pub on_429: bool,
    /// Fail over when the daily quota for the current provider/model is exhausted. Defaults to `true`.
    #[serde(default = "default_true", rename = "onQuotaExhausted")]
    pub on_quota_exhausted: bool,
    /// Fail over after a streak of consecutive errors. Defaults to `true`.
    #[serde(default = "default_true", rename = "onConsecutiveErrors")]
    pub on_consecutive_errors: bool,
    /// Number of consecutive errors that triggers failover. Defaults to `5`.
    #[serde(
        default = "default_error_threshold",
        rename = "consecutiveErrorThreshold"
    )]
    pub consecutive_error_threshold: u32,
    /// Fail over when a request exceeds the latency timeout. Defaults to `true`.
    #[serde(default = "default_true", rename = "onLatencyTimeout")]
    pub on_latency_timeout: bool,
    /// Milliseconds after which a request is considered timed out for failover purposes. Defaults to 90 000 (90 s).
    #[serde(default = "default_latency_timeout_ms", rename = "latencyTimeoutMs")]
    pub latency_timeout_ms: u64,
    /// Cooldown in milliseconds before a latency-timed-out entry is retried. Defaults to 60 000 (60 s).
    #[serde(
        default = "default_latency_timeout_cooldown_ms",
        rename = "latencyTimeoutCooldownMs"
    )]
    pub latency_timeout_cooldown_ms: u64,
    /// Cooldown in milliseconds before a consecutively-errored entry is retried. Defaults to 600 000 (10 min).
    #[serde(
        default = "default_consecutive_error_cooldown_ms",
        rename = "consecutiveErrorCooldownMs"
    )]
    pub consecutive_error_cooldown_ms: u64,
    /// Maximum total wall-clock duration (ms) for a streaming response.
    /// If the response stream exceeds this duration from first byte received,
    /// the stream is terminated and failover is triggered.
    /// Defaults to 1_200_000 (20 minutes).
    #[serde(
        default = "default_max_response_duration_ms",
        rename = "maxResponseDurationMs"
    )]
    pub max_response_duration_ms: u64,
}

fn default_error_threshold() -> u32 {
    5
}

fn default_latency_timeout_ms() -> u64 {
    90000
}

fn default_latency_timeout_cooldown_ms() -> u64 {
    60000
}

fn default_consecutive_error_cooldown_ms() -> u64 {
    600000
}

fn default_max_response_duration_ms() -> u64 {
    1_200_000
}

/// A named routing group that maps an alias to an ordered list of
/// [`GroupEntry`] values with shared failover settings.
///
/// When a request arrives for a group alias the proxy tries entries in
/// priority order, failing over according to the [`FailoverConfig`].
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Group {
    /// Unique identifier for this group.
    pub id: String,
    /// Short alias used in API requests to select this group.
    pub alias: String,
    /// Human-readable name shown in the UI.
    #[serde(rename = "displayName")]
    pub display_name: String,
    /// Ordered list of provider/model entries for this group.
    pub entries: Vec<GroupEntry>,
    /// Failover rules shared by all entries in this group.
    #[serde(rename = "failoverConfig")]
    pub failover_config: FailoverConfig,
    /// Optional Mixture of Agents routing configuration.
    #[serde(
        default,
        rename = "aggregationConfig",
        skip_serializing_if = "Option::is_none"
    )]
    pub aggregation_config: Option<AggregationConfig>,
}

impl Group {
    /// Returns true only when an aggregation config exists and is enabled.
    pub fn aggregation_enabled(&self) -> bool {
        self.aggregation_config
            .as_ref()
            .map(|c| c.enabled)
            .unwrap_or(false)
    }
}

/// Validates all enabled group-level MoA configs in a group list.
///
/// This is intentionally small and strict for the MVP: referenced and
/// aggregator groups must exist and must be normal failover groups.
pub fn validate_group_aggregation(groups: &[Group]) -> Result<(), String> {
    for group in groups {
        let Some(config) = group.aggregation_config.as_ref().filter(|c| c.enabled) else {
            continue;
        };

        if let Some(effort) = config.aggregator_reasoning_effort.as_deref() {
            if !validate_reasoning_effort(effort) {
                return Err(format!(
                    "MoA group '{}' has invalid aggregator reasoning effort '{}' (allowed: {})",
                    group.alias,
                    effort,
                    allowed_reasoning_efforts_label()
                ));
            }
        }

        if config.reference_max_tokens == Some(0) {
            return Err(format!(
                "MoA group '{}' reference max tokens must be greater than zero",
                group.alias
            ));
        }

        for (reference_group_id, effort) in &config.reference_reasoning_efforts {
            if !validate_reasoning_effort(effort) {
                return Err(format!(
                    "MoA group '{}' has invalid reasoning effort '{}' for reference group '{}' (allowed: {})",
                    group.alias,
                    effort,
                    reference_group_id,
                    allowed_reasoning_efforts_label()
                ));
            }
        }

        if config.reference_group_ids.is_empty() {
            return Err(format!(
                "MoA group '{}' must reference at least one group",
                group.alias
            ));
        }

        let aggregator_group_id = config
            .aggregator_group_id
            .as_deref()
            .ok_or_else(|| format!("MoA group '{}' must have an aggregator group", group.alias))?;

        if aggregator_group_id == group.id {
            return Err(format!(
                "MoA group '{}' cannot use itself as the aggregator",
                group.alias
            ));
        }

        for reference_group_id in &config.reference_group_ids {
            if reference_group_id == &group.id {
                return Err(format!(
                    "MoA group '{}' cannot reference itself",
                    group.alias
                ));
            }

            let reference_group = groups
                .iter()
                .find(|g| g.id == *reference_group_id)
                .ok_or_else(|| {
                    format!(
                        "MoA group '{}' references missing group id '{}'",
                        group.alias, reference_group_id
                    )
                })?;

            if reference_group.aggregation_enabled() {
                return Err(format!(
                    "MoA group '{}' cannot reference MoA-enabled group '{}'",
                    group.alias, reference_group.alias
                ));
            }
        }

        let aggregator_group = groups
            .iter()
            .find(|g| g.id == aggregator_group_id)
            .ok_or_else(|| {
                format!(
                    "MoA group '{}' uses missing aggregator group id '{}'",
                    group.alias, aggregator_group_id
                )
            })?;

        if aggregator_group.aggregation_enabled() {
            return Err(format!(
                "MoA group '{}' cannot use MoA-enabled aggregator group '{}'",
                group.alias, aggregator_group.alias
            ));
        }
    }

    Ok(())
}

/// Returns a conservative input-context limit for an enabled MoA group.
///
/// The client-visible request must fit every configured reference after the
/// advisory framing is added, and it must fit the aggregator after all
/// reference outputs are appended. Enabled failover entries are treated as
/// possible routes, so the smallest per-route input capacity and largest
/// possible reference output reserves are used.
pub fn effective_moa_input_tokens(
    group: &Group,
    groups: &[Group],
    providers: &[Provider],
) -> Option<u64> {
    let config = group
        .aggregation_config
        .as_ref()
        .filter(|config| config.enabled)?;
    let aggregator_id = config.aggregator_group_id.as_deref()?;
    let aggregator = groups
        .iter()
        .find(|candidate| candidate.id == aggregator_id)?;
    let (aggregator_input_capacity, _) =
        conservative_group_input_capacity(aggregator, providers, None)?;

    let mut summary_reserve = MOA_SUMMARY_BASE_OVERHEAD_TOKENS;
    let mut reference_budget: Option<u64> = None;
    for reference_id in &config.reference_group_ids {
        let reference = groups
            .iter()
            .find(|candidate| candidate.id == *reference_id)?;
        let (input_capacity, output_reserve) = conservative_group_input_capacity(
            reference,
            providers,
            config.reference_max_tokens.map(u64::from),
        )?;
        let input_budget = input_capacity.checked_sub(MOA_REFERENCE_INPUT_OVERHEAD_TOKENS)?;
        reference_budget = Some(
            reference_budget
                .map(|current| current.min(input_budget))
                .unwrap_or(input_budget),
        );

        let label_reserve = MOA_SUMMARY_PER_REFERENCE_OVERHEAD_TOKENS
            .checked_add(reference.alias.len() as u64)?
            .checked_add(reference.display_name.len() as u64)?;
        summary_reserve = summary_reserve
            .checked_add(output_reserve)?
            .checked_add(label_reserve)?;
    }

    let aggregator_budget = aggregator_input_capacity.checked_sub(summary_reserve)?;
    Some(aggregator_budget.min(reference_budget?))
}

fn conservative_group_input_capacity(
    group: &Group,
    providers: &[Provider],
    configured_cap: Option<u64>,
) -> Option<(u64, u64)> {
    let mut input_capacity: Option<u64> = None;
    let mut max_output: Option<u64> = None;
    for entry in group.entries.iter().filter(|entry| entry.enabled) {
        let provider = providers
            .iter()
            .find(|provider| provider.id == entry.provider_id)?;
        let (context, model_max_output) = provider.resolve_model_meta(&entry.model_id)?;
        let context = context?;
        let protocol = provider.resolve_model_protocol(&entry.model_id);
        let output = if protocol == "openai-codex" || configured_cap.is_none() {
            model_max_output?
        } else {
            let cap = configured_cap?;
            model_max_output
                .map(|model_max| cap.min(model_max))
                .unwrap_or(cap)
        };
        let entry_input_capacity = context.checked_sub(output)?;
        input_capacity = Some(
            input_capacity
                .map(|current| current.min(entry_input_capacity))
                .unwrap_or(entry_input_capacity),
        );
        max_output = Some(
            max_output
                .map(|current| current.max(output))
                .unwrap_or(output),
        );
    }
    Some((input_capacity?, max_output?))
}

/// Top-level application configuration stored in `~/.config/coderouter/config.json`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppConfig {
    /// TCP port the proxy listens on. Defaults to `4141`.
    #[serde(default = "default_proxy_port")]
    pub proxy_port: u16,
    /// Bind address for the proxy. Defaults to `"127.0.0.1"`.
    #[serde(default = "default_proxy_host")]
    pub proxy_host: String,
    /// How often (in hours) the proxy refreshes model metadata from providers. Defaults to `24`.
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_hours: u32,
    /// Log verbosity level (`"Trace"`, `"Debug"`, `"Info"`, `"Warn"`, `"Error"`). Defaults to `"Info"`.
    #[serde(default = "default_log_verbosity")]
    pub log_verbosity: String,
    /// Optional absolute path to an opencode configuration file to import settings from.
    #[serde(default)]
    pub opencode_config_path: Option<String>,
    /// Whether the onboarding wizard has been dismissed by the user.
    #[serde(default)]
    pub onboarding_dismissed: bool,
}

fn default_proxy_port() -> u16 {
    4141
}

fn default_proxy_host() -> String {
    "127.0.0.1".to_string()
}

fn default_refresh_interval() -> u32 {
    24
}

fn default_log_verbosity() -> String {
    "Info".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            proxy_port: default_proxy_port(),
            proxy_host: default_proxy_host(),
            refresh_interval_hours: default_refresh_interval(),
            log_verbosity: default_log_verbosity(),
            opencode_config_path: None,
            onboarding_dismissed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failover_config() -> FailoverConfig {
        FailoverConfig {
            on_429: true,
            on_quota_exhausted: true,
            on_consecutive_errors: true,
            consecutive_error_threshold: 5,
            on_latency_timeout: true,
            latency_timeout_ms: 90000,
            latency_timeout_cooldown_ms: 60000,
            consecutive_error_cooldown_ms: 600000,
            max_response_duration_ms: 1_200_000,
        }
    }

    fn group(id: &str, aggregation_config: Option<AggregationConfig>) -> Group {
        Group {
            id: id.to_string(),
            alias: id.to_string(),
            display_name: id.to_string(),
            entries: vec![],
            failover_config: failover_config(),
            aggregation_config,
        }
    }

    fn routed_group(id: &str, provider_id: &str, model_id: &str) -> Group {
        let mut group = group(id, None);
        group.entries.push(GroupEntry {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            priority: 1,
            daily_token_quota_override: None,
            enabled: true,
            status: "active".to_string(),
            cooldown_until: None,
        });
        group
    }

    fn provider(id: &str, model_id: &str, context: u64, output: u64, protocol: &str) -> Provider {
        Provider {
            id: id.to_string(),
            name: id.to_string(),
            protocol: protocol.to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            credential_key: id.to_string(),
            daily_token_quota: None,
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![ProviderModel {
                id: model_id.to_string(),
                context_window: Some(context),
                max_output_tokens: Some(output),
                input_cost_per_1m: None,
                output_cost_per_1m: None,
                last_refreshed: None,
                protocol: None,
            }],
            model_overrides: None,
        }
    }

    #[test]
    fn test_group_deserializes_without_aggregation_config() {
        let json = serde_json::json!({
            "id": "plain",
            "alias": "plain",
            "displayName": "Plain",
            "entries": [],
            "failoverConfig": {
                "on429": true,
                "onQuotaExhausted": true,
                "onConsecutiveErrors": true,
                "consecutiveErrorThreshold": 5,
                "onLatencyTimeout": true,
                "latencyTimeoutMs": 90000,
                "latencyTimeoutCooldownMs": 60000,
                "consecutiveErrorCooldownMs": 600000,
                "maxResponseDurationMs": 1200000
            }
        });

        let group: Group = serde_json::from_value(json).unwrap();
        assert!(group.aggregation_config.is_none());
        assert!(!group.aggregation_enabled());
    }

    #[test]
    fn test_aggregation_config_nested_defaults() {
        let json = serde_json::json!({ "enabled": true });
        let config: AggregationConfig = serde_json::from_value(json).unwrap();

        assert!(config.enabled);
        assert!(config.reference_group_ids.is_empty());
        assert!(config.aggregator_group_id.is_none());
        assert_eq!(config.fanout, MoaFanoutMode::PerIteration);
        assert_eq!(config.reference_max_tokens, Some(600));
        assert!(config.reference_temperature.is_none());
        assert!(config.aggregator_temperature.is_none());
        assert!(config.reference_reasoning_efforts.is_empty());
        assert!(config.aggregator_reasoning_effort.is_none());
        assert!(!config.require_all_references);
    }

    #[test]
    fn test_aggregation_config_allows_uncapped_per_iteration_references() {
        let config: AggregationConfig = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "fanout": "per_iteration",
            "referenceMaxTokens": null
        }))
        .unwrap();

        assert_eq!(config.fanout, MoaFanoutMode::PerIteration);
        assert_eq!(config.reference_max_tokens, None);
    }

    #[test]
    fn test_aggregation_config_allows_explicit_user_turn_fanout() {
        let config: AggregationConfig = serde_json::from_value(serde_json::json!({
            "fanout": "user_turn"
        }))
        .unwrap();

        assert_eq!(config.fanout, MoaFanoutMode::UserTurn);
    }

    #[test]
    fn test_model_protocol_override_takes_precedence() {
        let mut provider = provider("provider", "model", 100_000, 10_000, "openai");
        provider.models[0].protocol = Some("anthropic".to_string());
        provider.model_overrides = Some(vec![ProviderModel {
            id: "model".to_string(),
            context_window: None,
            max_output_tokens: None,
            input_cost_per_1m: None,
            output_cost_per_1m: None,
            last_refreshed: None,
            protocol: Some("openai-codex".to_string()),
        }]);

        assert_eq!(provider.resolve_model_protocol("model"), "openai-codex");
        assert_eq!(provider.resolve_model_protocol("unknown"), "openai");
    }

    #[test]
    fn test_effective_moa_input_reserves_reference_summary() {
        let moa = group(
            "moa",
            Some(AggregationConfig {
                enabled: true,
                reference_group_ids: vec!["ref".to_string()],
                aggregator_group_id: Some("agg".to_string()),
                ..AggregationConfig::default()
            }),
        );
        let groups = vec![
            moa,
            routed_group("agg", "agg-provider", "agg-model"),
            routed_group("ref", "ref-provider", "ref-model"),
        ];
        let providers = vec![
            provider("agg-provider", "agg-model", 100_000, 10_000, "openai"),
            provider("ref-provider", "ref-model", 200_000, 4_000, "openai"),
        ];

        // 100,000 total - 10,000 output - 798 tokens of reference summary framing.
        assert_eq!(
            effective_moa_input_tokens(&groups[0], &groups, &providers),
            Some(89_202)
        );
    }

    #[test]
    fn test_effective_moa_input_honors_reference_context() {
        let moa = group(
            "moa",
            Some(AggregationConfig {
                enabled: true,
                reference_group_ids: vec!["ref".to_string()],
                aggregator_group_id: Some("agg".to_string()),
                ..AggregationConfig::default()
            }),
        );
        let groups = vec![
            moa,
            routed_group("agg", "agg-provider", "agg-model"),
            routed_group("ref", "ref-provider", "ref-model"),
        ];
        let providers = vec![
            provider("agg-provider", "agg-model", 100_000, 10_000, "openai"),
            provider("ref-provider", "ref-model", 20_000, 4_000, "openai"),
        ];

        assert_eq!(
            effective_moa_input_tokens(&groups[0], &groups, &providers),
            Some(18_376)
        );
    }

    #[test]
    fn test_effective_moa_input_pairs_context_and_output_per_route() {
        let moa = group(
            "moa",
            Some(AggregationConfig {
                enabled: true,
                reference_group_ids: vec!["ref".to_string()],
                aggregator_group_id: Some("agg".to_string()),
                ..AggregationConfig::default()
            }),
        );
        let mut aggregator = routed_group("agg", "small-provider", "small-model");
        aggregator.entries.push(GroupEntry {
            provider_id: "large-provider".to_string(),
            model_id: "large-model".to_string(),
            priority: 2,
            daily_token_quota_override: None,
            enabled: true,
            status: "active".to_string(),
            cooldown_until: None,
        });
        let groups = vec![
            moa,
            aggregator,
            routed_group("ref", "ref-provider", "ref-model"),
        ];
        let providers = vec![
            provider("small-provider", "small-model", 128_000, 8_000, "openai"),
            provider(
                "large-provider",
                "large-model",
                1_000_000,
                128_000,
                "openai",
            ),
            provider("ref-provider", "ref-model", 500_000, 4_000, "openai"),
        ];

        // The two aggregator routes have 120k and 872k input capacity respectively.
        assert_eq!(
            effective_moa_input_tokens(&groups[0], &groups, &providers),
            Some(119_202)
        );
    }

    #[test]
    fn test_effective_moa_input_uses_model_output_when_cap_is_unenforceable() {
        let moa = group(
            "moa",
            Some(AggregationConfig {
                enabled: true,
                reference_group_ids: vec!["ref".to_string()],
                aggregator_group_id: Some("agg".to_string()),
                reference_max_tokens: Some(600),
                ..AggregationConfig::default()
            }),
        );
        let groups = vec![
            moa,
            routed_group("agg", "agg-provider", "agg-model"),
            routed_group("ref", "ref-provider", "ref-model"),
        ];
        let providers = vec![
            provider("agg-provider", "agg-model", 100_000, 10_000, "openai"),
            provider("ref-provider", "ref-model", 200_000, 4_000, "openai-codex"),
        ];

        // Codex cannot enforce the 600-token cap, so reserve its 4,000-token maximum.
        assert_eq!(
            effective_moa_input_tokens(&groups[0], &groups, &providers),
            Some(85_802)
        );
    }

    #[test]
    fn test_validate_group_aggregation_rejects_invalid_reference_reasoning_effort() {
        let moa = group(
            "moa",
            Some(AggregationConfig {
                enabled: true,
                reference_group_ids: vec!["ref".to_string()],
                aggregator_group_id: Some("ref".to_string()),
                reference_reasoning_efforts: HashMap::from([(
                    "ref".to_string(),
                    "extreme".to_string(),
                )]),
                ..AggregationConfig::default()
            }),
        );
        let groups = vec![moa, group("ref", None)];

        assert!(validate_group_aggregation(&groups)
            .unwrap_err()
            .contains("invalid reasoning effort"));
    }

    #[test]
    fn test_validate_group_aggregation_rejects_invalid_aggregator_reasoning_effort() {
        let moa = group(
            "moa",
            Some(AggregationConfig {
                enabled: true,
                reference_group_ids: vec!["ref".to_string()],
                aggregator_group_id: Some("ref".to_string()),
                aggregator_reasoning_effort: Some("extreme".to_string()),
                ..AggregationConfig::default()
            }),
        );
        let groups = vec![moa, group("ref", None)];

        assert!(validate_group_aggregation(&groups)
            .unwrap_err()
            .contains("invalid aggregator reasoning effort"));
    }

    #[test]
    fn test_validate_group_aggregation_rejects_self_reference() {
        let moa = group(
            "moa",
            Some(AggregationConfig {
                enabled: true,
                reference_group_ids: vec!["moa".to_string()],
                aggregator_group_id: Some("ref".to_string()),
                ..AggregationConfig::default()
            }),
        );
        let groups = vec![moa, group("ref", None)];

        assert!(validate_group_aggregation(&groups)
            .unwrap_err()
            .contains("cannot reference itself"));
    }

    #[test]
    fn test_validate_group_aggregation_rejects_missing_group() {
        let moa = group(
            "moa",
            Some(AggregationConfig {
                enabled: true,
                reference_group_ids: vec!["missing".to_string()],
                aggregator_group_id: Some("ref".to_string()),
                ..AggregationConfig::default()
            }),
        );
        let groups = vec![moa, group("ref", None)];

        assert!(validate_group_aggregation(&groups)
            .unwrap_err()
            .contains("missing group"));
    }

    #[test]
    fn test_validate_group_aggregation_rejects_nested_moa() {
        let nested = group(
            "nested",
            Some(AggregationConfig {
                enabled: true,
                reference_group_ids: vec!["ref".to_string()],
                aggregator_group_id: Some("ref".to_string()),
                ..AggregationConfig::default()
            }),
        );
        let moa = group(
            "moa",
            Some(AggregationConfig {
                enabled: true,
                reference_group_ids: vec!["nested".to_string()],
                aggregator_group_id: Some("ref".to_string()),
                ..AggregationConfig::default()
            }),
        );
        let groups = vec![moa, nested, group("ref", None)];

        assert!(validate_group_aggregation(&groups)
            .unwrap_err()
            .contains("MoA-enabled group"));
    }
}
