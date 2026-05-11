import { invoke } from '@tauri-apps/api/core';
import type { Provider, Group, AppConfig, RouterStatusResponse, DailySummary, RequestRow, GroupUsage, UpdateStatus, ModelUsage, DailyModelUsage, CustomAgent, AgentTemplate } from '../types';

/** Result of a provider connection test. */
export interface TestConnectionResult {
  /** Whether the connection attempt succeeded. */
  success: boolean;
  /** HTTP status code returned, or null if the request did not complete. */
  status_code: number | null;
  /** Human-readable description of the result. */
  message: string;
}

/** Fetch all configured providers. */
export async function getProviders(): Promise<Provider[]> {
  return invoke<Provider[]>('get_providers');
}

/** Persist a provider (create or update) along with its API key. */
export async function saveProvider(provider: Provider, apiKey: string): Promise<void> {
  return invoke<void>('save_provider', { provider, apiKey });
}

/** Enable or disable a provider by toggling its `enabled` flag. */
export async function toggleProviderEnabled(providerId: string, enabled: boolean): Promise<void> {
  return invoke<void>('toggle_provider_enabled', { providerId, enabled });
}

/** Permanently remove a provider and its associated configuration. */
export async function deleteProvider(providerId: string): Promise<void> {
  return invoke<void>('delete_provider', { providerId });
}

/** Re-fetch the model catalog from the provider's remote API and return the updated provider. */
export async function refreshProviderModels(providerId: string): Promise<Provider[]> {
  return invoke<Provider[]>('refresh_provider_models', { providerId });
}

/** Send a lightweight test request to verify the provider's API credentials are valid. */
export async function testProviderConnection(providerId: string): Promise<TestConnectionResult> {
  return invoke<TestConnectionResult>('test_provider_connection', { providerId });
}

/** Fetch all configured routing groups. */
export async function getGroups(): Promise<Group[]> {
  return invoke<Group[]>('get_groups');
}

/** Persist a routing group (create or update). */
export async function saveGroup(group: Group): Promise<void> {
  return invoke<void>('save_group', { group });
}

/** Permanently remove a routing group. */
export async function deleteGroup(groupId: string): Promise<void> {
  return invoke<void>('delete_group', { groupId });
}

/** Fetch the current application configuration. */
export async function getAppConfig(): Promise<AppConfig> {
  return invoke<AppConfig>('get_app_config');
}

/** Persist updated application configuration. */
export async function saveAppConfig(config: AppConfig): Promise<void> {
  return invoke<void>('save_app_config', { config });
}

/** Fetch the live status of all router entries across all groups. */
export async function getRouterStatus(): Promise<RouterStatusResponse> {
  return invoke<RouterStatusResponse>('get_router_status');
}

/** Enable or disable a specific entry within a group. */
export async function setEntryEnabled(groupId: string, entryIndex: number, enabled: boolean): Promise<void> {
  return invoke<void>('set_entry_enabled', { groupId, entryIndex, enabled });
}

/** Fetch aggregated daily usage summary for a specific provider. */
export async function getDailySummary(providerId: string, date: string): Promise<DailySummary> {
  return invoke<DailySummary>('get_daily_summary', { providerId, date });
}

/** Fetch the most recent request rows, limited by count. */
export async function getRecentRequests(limit: number): Promise<RequestRow[]> {
  return invoke<RequestRow[]>('get_recent_requests', { limit });
}

/** Mapping of opencode agent names to group aliases, used to configure which group handles each agent. */
export interface OpenCodeAgentMapping {
  /** Agent that builds/executes code changes. */
  build: string | null;
  /** Agent that creates task plans. */
  plan: string | null;
  /** Agent for general-purpose tasks. */
  general: string | null;
  /** Agent that reads and explores the codebase. */
  explore: string | null;
  /** Agent that compacts conversation history. */
  compaction: string | null;
  /** Agent that generates PR titles. */
  title: string | null;
  /** Agent that generates PR summaries. */
  summary: string | null;
  /** Alias for a cost-effective model used for lightweight tasks. */
  small_model: string | null;
  /** Optional reasoning effort per agent role. */
  reasoning_efforts?: Record<string, string>;
}

/** Get the filesystem path to the opencode configuration file that CodeRouter manages. */
export async function getOpencodeConfigPath(): Promise<string | null> {
  return invoke<string | null>('get_opencode_config_path');
}

/** Set the filesystem path to the opencode configuration file. */
export async function setOpencodeConfigPath(path: string): Promise<void> {
  return invoke<void>('set_opencode_config_path', { path });
}

/** Inject a CodeRouter provider entry into the opencode config so traffic routes through the proxy. */
export async function injectOpencodeProvider(proxyPort: number): Promise<void> {
  return invoke<void>('inject_opencode_provider', { proxyPort });
}

/** Remove the CodeRouter provider entry from the opencode config, restoring the original configuration. */
export async function removeOpencodeProvider(): Promise<void> {
  return invoke<void>('remove_opencode_provider');
}

/** Apply the given agent-to-group mapping into the opencode configuration file. */
export async function setOpencodeAgentModels(mapping: OpenCodeAgentMapping): Promise<void> {
  return invoke<void>('set_opencode_agent_models', { mapping });
}

/** Remove all agent model overrides from the opencode configuration. */
export async function removeOpencodeAgentModels(): Promise<void> {
  return invoke<void>('remove_opencode_agent_models');
}

/** Read the current agent-to-group mapping from the opencode configuration. */
export async function getOpencodeAgentModels(): Promise<OpenCodeAgentMapping> {
  return invoke<OpenCodeAgentMapping>('get_opencode_agent_models');
}

/** Render a preview of what the opencode config would look like with the given mapping applied. */
export async function previewOpencodeConfig(proxyPort: number, mapping: OpenCodeAgentMapping | null): Promise<string> {
  return invoke<string>('preview_opencode_config', { proxyPort, mapping });
}

/** Clear all stored metrics data from the database. */
export async function clearMetricsData(): Promise<void> {
  return invoke<void>('clear_metrics_data');
}

/** Reset all configuration (providers, groups, app config) to factory defaults. */
export async function resetAllConfig(): Promise<void> {
  return invoke<void>('reset_all_config');
}

/** Restart the proxy server process. */
export async function restartProxy(): Promise<void> {
  return invoke<void>('restart_proxy');
}

/** Check whether a group alias is currently referenced in the opencode configuration. */
export async function isGroupReferencedInOpencode(groupAlias: string): Promise<boolean> {
  return invoke<boolean>('is_group_referenced_in_opencode', { groupAlias: groupAlias });
}

/** Completely remove CodeRouter's integration from the opencode configuration file. */
export async function removeCoderouterFromOpencode(): Promise<void> {
  return invoke<void>('remove_coderouter_from_opencode');
}

/** Mark the onboarding flow as dismissed so it is not shown again. */
export async function dismissOnboarding(): Promise<void> {
  return invoke<void>('dismiss_onboarding');
}

/** Fetch usage metrics aggregated by group over a number of days, optionally filtered by provider. */
export async function getUsageByGroup(days: number, providerId?: string): Promise<GroupUsage[]> {
  // Send null when providerId is undefined so the backend returns unfiltered data
  return invoke<GroupUsage[]>('get_usage_by_group', { days, providerId: providerId ?? null });
}

/** Fetch usage metrics aggregated by model over a number of days. */
export async function getUsageByModel(days: number): Promise<ModelUsage[]> {
  return invoke<ModelUsage[]>('get_usage_by_model', { days });
}

/** Fetch daily cost breakdown per model over a number of days. */
export async function getDailyUsageByModel(days: number): Promise<DailyModelUsage[]> {
  return invoke<DailyModelUsage[]>('get_daily_usage_by_model', { days });
}

/** Result of a proxy health check, indicating whether it is running and for how long. */
export interface HealthCheckResult {
  /** Whether the proxy process is currently running. */
  running: boolean;
  /** Human-readable status string, or null if not running. */
  status: string | null;
  /** How long the proxy has been running, in seconds, or null if not running. */
  uptime_seconds: number | null;
}

/** Check whether the proxy process is alive and retrieve its uptime. */
export async function checkProxyHealth(): Promise<HealthCheckResult> {
  return invoke<HealthCheckResult>('check_proxy_health');
}

/** Calculate the total cost (USD) for a provider over a given number of days. */
export async function getCostSummary(providerId: string, days: number): Promise<number> {
  return invoke<number>('get_cost_summary', { providerId, days });
}

/** Get the application version string. */
export async function getAppVersion(): Promise<string> {
  return invoke<string>('get_app_version');
}

/** Check for available application updates. */
export async function checkForUpdates(): Promise<UpdateStatus> {
  return invoke<UpdateStatus>('check_for_updates');
}

/** Download and install the latest update, then restart the app. */
export async function installUpdate(): Promise<void> {
  return invoke<void>('install_update');
}

/** List all custom agents stored as markdown files. */
export async function listCustomAgents(): Promise<CustomAgent[]> {
  return invoke<CustomAgent[]>('list_custom_agents');
}

/** Create a new custom agent from the given configuration. */
export async function createCustomAgent(agent: CustomAgent): Promise<CustomAgent> {
  return invoke<CustomAgent>('create_custom_agent', { agent });
}

/** Update an existing custom agent by name. */
export async function updateCustomAgent(name: string, agent: CustomAgent): Promise<CustomAgent> {
  return invoke<CustomAgent>('update_custom_agent', { name, agent });
}

/** Delete a custom agent by name. */
export async function deleteCustomAgent(name: string): Promise<void> {
  return invoke<void>('delete_custom_agent', { name });
}

/** Get the built-in agent templates available for selection. */
export async function getAgentTemplates(): Promise<AgentTemplate[]> {
  return invoke<AgentTemplate[]>('get_agent_templates');
}

/** Request body for AI enhancement. */
export interface AgentEnhanceRequest {
  text: string;
  enhanceType: 'description' | 'prompt' | 'suggestions';
  modelGroup: string;
}

/** Result of an AI enhancement request. */
export interface AgentEnhanceResponse {
  result: string;
}

/** Use the proxy's chat completion to enhance agent text or suggest settings. */
export async function enhanceAgentText(request: AgentEnhanceRequest): Promise<AgentEnhanceResponse> {
  return invoke<AgentEnhanceResponse>('enhance_agent_text', { request });
}