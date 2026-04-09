export type Protocol = 'openai' | 'anthropic';

export type EntryStatus = 'active' | 'cooldown' | 'manually_disabled' | 'quota_exhausted';

export type LogVerbosity = 'Error' | 'Info' | 'Debug';

export type RequestStatus = 'success' | 'error' | 'timeout' | 'failover';

export interface ProviderModel {
  id: string;
  context_window?: number;
  max_output_tokens?: number;
  input_cost_per_1m?: number;
  output_cost_per_1m?: number;
  last_refreshed?: string;
  protocol?: Protocol;
}

export interface Provider {
  id: string;
  name: string;
  protocol: Protocol;
  baseUrl: string;
  credentialKey: string;
  dailyTokenQuota?: number;
  dailyRequestQuota?: number;
  quotaResetUtcHour: number;
  enabled: boolean;
  models: ProviderModel[];
  modelOverrides?: ProviderModel[];
}

export interface GroupEntry {
  providerId: string;
  modelId: string;
  priority: number;
  dailyTokenQuotaOverride?: number;
  enabled: boolean;
  status: EntryStatus;
  cooldownUntil?: string;
}

export interface FailoverConfig {
  on429: boolean;
  onQuotaExhausted: boolean;
  onConsecutiveErrors: boolean;
  consecutiveErrorThreshold: number;
  onLatencyTimeout: boolean;
  latencyTimeoutMs: number;
  latencyTimeoutCooldownMs: number;
  consecutiveErrorCooldownMs: number;
}

export interface Group {
  id: string;
  alias: string;
  displayName: string;
  entries: GroupEntry[];
  failoverConfig: FailoverConfig;
}

export interface AppConfig {
  proxy_port: number;
  proxy_host: string;
  refresh_interval_hours: number;
  log_verbosity: LogVerbosity;
  opencode_config_path?: string;
  onboarding_dismissed?: boolean;
}

export type ProxyStatus = 'running' | 'stopped' | 'unknown';

export interface EntryStatusResponse {
  group_id: string;
  group_alias: string;
  provider_id: string;
  model_id: string;
  priority: number;
  entry_index: number;
  status: EntryStatus;
  cooldown_until?: string;
  consecutive_errors: number;
  daily_tokens_used: number;
  daily_requests_used: number;
  daily_reset_at: string;
  cooldown_duration_seconds?: number;
}

export interface RouterStatusResponse {
  entries: EntryStatusResponse[];
}

export interface DailySummary {
  total_requests: number;
  total_prompt_tokens: number;
  total_output_tokens: number;
  total_cost: number;
  error_count: number;
}

export interface RequestRow {
  id: number;
  ts: number;
  group_alias: string;
  provider_id: string;
  model_id: string;
  prompt_tokens: number;
  output_tokens: number;
  cost_usd: number;
  latency_ms: number;
  status: RequestStatus;
  error_type: string | null;
}

export interface GroupUsage {
  group_alias: string;
  total_requests: number;
  total_prompt_tokens: number;
  total_output_tokens: number;
  total_cost: number;
}
