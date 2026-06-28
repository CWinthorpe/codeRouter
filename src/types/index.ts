/** Supported LLM API protocols. */
export type Protocol = 'openai' | 'anthropic' | 'openai-codex';

/** Status of a group entry, determining whether it can receive traffic. */
export type EntryStatus = 'active' | 'cooldown' | 'manually_disabled' | 'quota_exhausted';

/** Log verbosity levels for the proxy server. */
export type LogVerbosity = 'Error' | 'Info' | 'Debug';

/** Outcome of a proxied request. */
export type RequestStatus = 'success' | 'error' | 'timeout' | 'failover';

/** A single model belonging to a provider, with optional cost and capability metadata. */
export interface ProviderModel {
  /** Unique model identifier, e.g. "gpt-4o". */
  id: string;
  /** Maximum context window in tokens. */
  context_window?: number;
  /** Maximum output tokens the model can produce per request. */
  max_output_tokens?: number;
  /** Cost per 1M input tokens in USD. */
  input_cost_per_1m?: number;
  /** Cost per 1M output tokens in USD. */
  output_cost_per_1m?: number;
  /** ISO timestamp of when model metadata was last refreshed from the provider. */
  last_refreshed?: string;
  /** Protocol override for this model, if different from the provider default. */
  protocol?: Protocol;
}

/** An LLM provider (e.g. OpenAI, Anthropic) with its API credentials and model catalog. */
export interface Provider {
  /** Unique identifier for the provider. */
  id: string;
  /** Human-readable display name. */
  name: string;
  /** API protocol used to communicate with this provider. */
  protocol: Protocol;
  /** Base URL for the provider's API endpoint. */
  baseUrl: string;
  /** API key or credential used for authentication. */
  credentialKey: string;
  /** Optional daily token usage quota. */
  dailyTokenQuota?: number;
  /** Optional daily request count quota. */
  dailyRequestQuota?: number;
  /** UTC hour (0–23) at which daily quotas reset. */
  quotaResetUtcHour: number;
  /** Whether the provider is currently enabled for routing. */
  enabled: boolean;
  /** Models fetched from the provider's remote catalog. */
  models: ProviderModel[];
  /** User-provided model overrides that take precedence over the fetched catalog. */
  modelOverrides?: ProviderModel[];
}

/** A single entry within a group, binding a provider+model pair to a failover priority. */
export interface GroupEntry {
  /** ID of the provider this entry routes to. */
  providerId: string;
  /** ID of the model to use on the provider. */
  modelId: string;
  /** Lower priority number = higher routing precedence. */
  priority: number;
  /** Optional per-entry token quota that overrides the provider-level quota. */
  dailyTokenQuotaOverride?: number;
  /** Whether this entry is enabled for routing. */
  enabled: boolean;
  /** Current operational status of the entry. */
  status: EntryStatus;
  /** ISO timestamp until which the entry is in cooldown. */
  cooldownUntil?: string;
}

/** Configuration controlling automatic failover behaviour within a group. */
export interface FailoverConfig {
  /** Fail over when the upstream returns HTTP 429 (rate-limited). */
  on429: boolean;
  /** Fail over when the entry's daily quota is exhausted. */
  onQuotaExhausted: boolean;
  /** Fail over after a configurable number of consecutive errors. */
  onConsecutiveErrors: boolean;
  /** Number of consecutive errors that triggers failover. */
  consecutiveErrorThreshold: number;
  /** Fail over when request latency exceeds the configured timeout. */
  onLatencyTimeout: boolean;
  /** Request latency threshold in milliseconds. */
  latencyTimeoutMs: number;
  /** Cooldown period in milliseconds after a latency timeout triggers failover. */
  latencyTimeoutCooldownMs: number;
  /** Cooldown period in milliseconds after consecutive errors triggers failover. */
  consecutiveErrorCooldownMs: number;
  /** Maximum total wall-clock duration in milliseconds for a streaming response. */
  maxResponseDurationMs: number;
}

/** Optional group-level Mixture of Agents routing configuration. */
export interface AggregationConfig {
  /** Whether this group routes via Mixture of Agents instead of direct failover. */
  enabled: boolean;
  /** Group IDs used for advisory reference calls. */
  referenceGroupIds: string[];
  /** Group ID used for the final aggregator call. */
  aggregatorGroupId: string | null;
  /** Optional temperature override for reference calls. */
  referenceTemperature?: number | null;
  /** Optional temperature override for the aggregator call. */
  aggregatorTemperature?: number | null;
  /** Fail the request if any reference group fails. */
  requireAllReferences: boolean;
}

/** A routing group that maps an alias to an ordered list of provider entries with failover rules. */
export interface Group {
  /** Unique identifier for the group. */
  id: string;
  /** Short alias used as the routing key (e.g. "code-large"). */
  alias: string;
  /** Human-readable display name. */
  displayName: string;
  /** Ordered list of provider+model entries for this group. */
  entries: GroupEntry[];
  /** Failover rules applied when an entry becomes unavailable. */
  failoverConfig: FailoverConfig;
  /** Optional Mixture of Agents routing configuration. */
  aggregationConfig?: AggregationConfig;
}

/** Application-level configuration persisted to disk. */
export interface AppConfig {
  /** Port the proxy server listens on. */
  proxy_port: number;
  /** Host interface the proxy server binds to. */
  proxy_host: string;
  /** How often (in hours) the provider model catalogs are refreshed. */
  refresh_interval_hours: number;
  /** Minimum log verbosity level. */
  log_verbosity: LogVerbosity;
  /** Path to the opencode configuration file, if managed by CodeRouter. */
  opencode_config_path?: string;
  /** Whether the onboarding flow has been dismissed by the user. */
  onboarding_dismissed?: boolean;
}

/** Runtime status of the proxy server process. */
export type ProxyStatus = 'running' | 'stopped' | 'unknown';

/** Status of a single group entry, as reported by the backend. */
export interface EntryStatusResponse {
  /** ID of the group this entry belongs to. */
  group_id: string;
  /** Alias of the group. */
  group_alias: string;
  /** ID of the provider for this entry. */
  provider_id: string;
  /** ID of the model for this entry. */
  model_id: string;
  /** Priority number within the group. */
  priority: number;
  /** Zero-based index of this entry in the group's entries array. */
  entry_index: number;
  /** Current operational status. */
  status: EntryStatus;
  /** ISO timestamp until which the entry remains in cooldown, if applicable. */
  cooldown_until?: string;
  /** Number of consecutive errors seen for this entry. */
  consecutive_errors: number;
  /** Tokens consumed today by this entry. */
  daily_tokens_used: number;
  /** Requests made today by this entry. */
  daily_requests_used: number;
  /** ISO timestamp when daily counters reset. */
  daily_reset_at: string;
  /** Remaining cooldown duration in seconds, if applicable. */
  cooldown_duration_seconds?: number;
  /** Reason for cooldown or quota exhaustion, if applicable. */
  cooldown_reason?: 'rate_limited' | 'consecutive_errors' | 'latency_timeout' | 'quota_exhausted';
}

/** Full router status containing an array of all entry statuses. */
export interface RouterStatusResponse {
  /** All entry statuses across all groups. */
  entries: EntryStatusResponse[];
}

/** Aggregated daily usage summary. */
export interface DailySummary {
  /** Total number of requests. */
  total_requests: number;
  /** Total prompt tokens consumed. */
  total_prompt_tokens: number;
  /** Total output tokens produced. */
  total_output_tokens: number;
  /** Total cost in USD. */
  total_cost: number;
  /** Number of requests that ended in an error. */
  error_count: number;
}

/** A single logged request row for display in the metrics table. */
export interface RequestRow {
  /** Auto-incremented row ID. */
  id: number;
  /** Unix timestamp of the request. */
  ts: number;
  /** Group alias the request was routed through. */
  group_alias: string;
  /** Provider ID that handled the request. */
  provider_id: string;
  /** Model ID that handled the request. */
  model_id: string;
  /** Number of prompt tokens consumed. */
  prompt_tokens: number;
  /** Number of output tokens produced. */
  output_tokens: number;
  /** Cost of this request in USD. */
  cost_usd: number;
  /** Request latency in milliseconds. */
  latency_ms: number;
  /** Outcome status of the request. */
  status: RequestStatus;
  /** Error type string, or null if the request succeeded. */
  error_type: string | null;
}

/** Result of checking for application updates. */
export interface UpdateStatus {
  available: boolean;
  currentVersion: string;
  latestVersion: string | null;
  releaseNotes: string | null;
}

/** Aggregated usage metrics for a single group over a time range. */
export interface GroupUsage {
  /** Group alias. */
  group_alias: string;
  /** Total number of requests. */
  total_requests: number;
  /** Total prompt tokens consumed. */
  total_prompt_tokens: number;
  /** Total output tokens produced. */
  total_output_tokens: number;
  /** Total cost in USD. */
  total_cost: number;
}

/** Per-model usage aggregation over a time range. */
export interface ModelUsage {
  /** Model identifier. */
  model_id: string;
  /** Total number of requests. */
  total_requests: number;
  /** Total cost in USD. */
  total_cost_usd: number;
  /** Total prompt tokens consumed. */
  total_prompt_tokens: number;
  /** Total output tokens produced. */
  total_output_tokens: number;
  /** Average latency in milliseconds. */
  avg_latency_ms: number;
}

/** Daily cost breakdown per model for chart rendering. */
export interface DailyModelUsage {
  /** Date string in YYYY-MM-DD format. */
  day: string;
  /** Model identifier. */
  model_id: string;
  /** Total cost in USD for this model on this day. */
  total_cost_usd: number;
}

/** Permission level for a specific tool or command pattern. */
export type PermissionLevel = 'allow' | 'deny' | 'ask';

/** Bash permission config: either a simple level or a map of command patterns to levels. */
export type BashPermission = PermissionLevel | Record<string, PermissionLevel>;

/** Configurable permissions for a custom agent's tool access. */
export interface AgentPermissions {
  /** File edit permission. */
  edit?: PermissionLevel;
  /** Bash command permission (simple level or command-pattern map). */
  bash?: BashPermission;
  /** Web fetch permission. */
  webfetch?: PermissionLevel;
  /** Task/subagent invocation permissions (agent-name pattern to level). */
  task?: Record<string, PermissionLevel>;
}

/** Agent mode determining how the agent can be invoked. */
export type AgentMode = 'primary' | 'subagent' | 'all';

/** A custom OpenCode agent defined via markdown file. */
export interface CustomAgent {
  /** Agent identifier (derived from filename, without .md extension). */
  name: string;
  /** Brief description of what the agent does (required in frontmatter). */
  description: string;
  /** How the agent can be used: primary, subagent, or both. */
  mode: AgentMode;
  /** CodeRouter model group alias (written as coderouter/<alias> in the file). */
  model: string | undefined;
  /** System prompt / instructions (markdown body, after frontmatter). */
  prompt: string;
  /** Temperature for response generation (0.0–1.0). */
  temperature?: number;
  /** Maximum number of agentic iterations before forced text-only response. */
  steps?: number;
  /** Whether the agent is disabled. */
  disable?: boolean;
  /** Hide subagent from @ autocomplete menu. */
  hidden?: boolean;
  /** Visual color for the agent in the UI (hex or theme color name). */
  color?: string;
  /** Top P for response diversity (0.0–1.0). */
  topP?: number;
  /** Reasoning effort for models that support it (none, low, medium, high, xhigh, max). */
  reasoningEffort?: string;
  /** Tool access permissions. */
  permissions?: AgentPermissions;
  /** Additional provider-specific options passed through as-is. */
  additional?: Record<string, unknown>;
}

/** Built-in template for creating a new custom agent. */
export interface AgentTemplate {
  /** Unique template identifier. */
  id: string;
  /** Display name shown in the template picker. */
  name: string;
  /** Short description of the template's purpose. */
  description: string;
  /** Icon emoji or lucide icon name for the template card. */
  icon: string;
  /** Pre-filled agent configuration. */
  agent: Omit<CustomAgent, 'name'>;
}
