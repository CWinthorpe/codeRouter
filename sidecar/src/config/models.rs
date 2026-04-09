use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProviderModel {
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub protocol: String,
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    #[serde(rename = "credentialKey")]
    pub credential_key: String,
    #[serde(default, rename = "dailyTokenQuota")]
    pub daily_token_quota: Option<u64>,
    #[serde(default, rename = "dailyRequestQuota")]
    pub daily_request_quota: Option<u64>,
    #[serde(default = "default_quota_reset_hour", rename = "quotaResetUtcHour")]
    pub quota_reset_utc_hour: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub models: Vec<ProviderModel>,
    #[serde(default)]
    pub model_overrides: Option<Vec<ProviderModel>>,
}

fn default_quota_reset_hour() -> u32 {
    0
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GroupEntry {
    #[serde(rename = "providerId")]
    pub provider_id: String,
    #[serde(rename = "modelId")]
    pub model_id: String,
    pub priority: u32,
    #[serde(default, rename = "dailyTokenQuotaOverride")]
    pub daily_token_quota_override: Option<u64>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_active_status")]
    pub status: String,
    #[serde(default, rename = "cooldownUntil")]
    pub cooldown_until: Option<String>,
}

fn default_active_status() -> String {
    "active".to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FailoverConfig {
    #[serde(default = "default_true", rename = "on429")]
    pub on_429: bool,
    #[serde(default = "default_true", rename = "onQuotaExhausted")]
    pub on_quota_exhausted: bool,
    #[serde(default = "default_true", rename = "onConsecutiveErrors")]
    pub on_consecutive_errors: bool,
    #[serde(
        default = "default_error_threshold",
        rename = "consecutiveErrorThreshold"
    )]
    pub consecutive_error_threshold: u32,
    #[serde(default = "default_true", rename = "onLatencyTimeout")]
    pub on_latency_timeout: bool,
    #[serde(default = "default_latency_timeout_ms", rename = "latencyTimeoutMs")]
    pub latency_timeout_ms: u64,
    #[serde(
        default = "default_latency_timeout_cooldown_ms",
        rename = "latencyTimeoutCooldownMs"
    )]
    pub latency_timeout_cooldown_ms: u64,
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Group {
    pub id: String,
    pub alias: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub entries: Vec<GroupEntry>,
    #[serde(rename = "failoverConfig")]
    pub failover_config: FailoverConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppConfig {
    #[serde(default = "default_proxy_port")]
    pub proxy_port: u16,
    #[serde(default = "default_proxy_host")]
    pub proxy_host: String,
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_hours: u32,
    #[serde(default = "default_log_verbosity")]
    pub log_verbosity: String,
    #[serde(default)]
    pub opencode_config_path: Option<String>,
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
