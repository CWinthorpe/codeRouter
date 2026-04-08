export interface ProviderModel {
  id: string;
  context_window?: number;
  max_output_tokens?: number;
  input_cost_per_1m?: number;
  output_cost_per_1m?: number;
  last_refreshed?: string;
}

export interface Provider {
  id: string;
  name: string;
  protocol: string;
  baseUrl: string;
  credentialKey: string;
  daily_token_quota?: number;
  quota_reset_utc_hour: number;
  enabled: boolean;
  models: ProviderModel[];
}

export interface GroupEntry {
  providerId: string;
  modelId: string;
  priority: number;
  daily_token_quota_override?: number;
  enabled: boolean;
  status: string;
  cooldown_until?: string;
}

export interface FailoverConfig {
  on_429: boolean;
  on_quota_exhausted: boolean;
  on_consecutive_errors: boolean;
  consecutive_error_threshold: number;
  on_latency_timeout: boolean;
  latency_timeout_ms: number;
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
  log_verbosity: string;
}

export type ProxyStatus = 'running' | 'stopped' | 'unknown';

export interface EntryStatusResponse {
  group_id: string;
  group_alias: string;
  provider_id: string;
  model_id: string;
  priority: number;
  entry_index: number;
  status: string;
  cooldown_until?: string;
  consecutive_errors: number;
  daily_tokens_used: number;
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
  status: string;
  error_type: string | null;
}
