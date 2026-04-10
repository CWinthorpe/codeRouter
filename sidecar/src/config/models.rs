//! Configuration data models.
//!
//! Defines the serde-based structs that represent the coderouter proxy
//! configuration: providers, model groups, failover rules, and application
//! settings. Every struct derives `Serialize`/`Deserialize` so they map
//! directly to the on-disk JSON files.

use serde::{Deserialize, Serialize};

/// Describes a single model offered by a provider.
///
/// Contains pricing and capability metadata that the proxy uses for routing
/// decisions and cost tracking.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProviderModel {
    /// The model identifier as recognised by the upstream provider (e.g. `gpt-4o`).
    pub id: String,
    /// Maximum number of tokens the model can accept in a single request context.
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
    /// Milliseconds after which a request is considered timed out for failover purposes. Defaults to 30 000 (30 s).
    #[serde(default = "default_latency_timeout_ms", rename = "latencyTimeoutMs")]
    pub latency_timeout_ms: u64,
    /// Cooldown in milliseconds before a latency-timed-out entry is retried. Defaults to 300 000 (5 min).
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
}

fn default_error_threshold() -> u32 {
    5
}

fn default_latency_timeout_ms() -> u64 {
    30000
}

fn default_latency_timeout_cooldown_ms() -> u64 {
    300000
}

fn default_consecutive_error_cooldown_ms() -> u64 {
    600000
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
