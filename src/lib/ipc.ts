import { invoke } from '@tauri-apps/api/core';
import type { Provider, Group, AppConfig, RouterStatusResponse, DailySummary, RequestRow, GroupUsage } from '../types';

export interface TestConnectionResult {
  success: boolean;
  status_code: number | null;
  message: string;
}

export async function getProviders(): Promise<Provider[]> {
  return invoke<Provider[]>('get_providers');
}

export async function saveProvider(provider: Provider, apiKey: string): Promise<void> {
  return invoke<void>('save_provider', { provider, apiKey });
}

export async function toggleProviderEnabled(providerId: string, enabled: boolean): Promise<void> {
  return invoke<void>('toggle_provider_enabled', { providerId, enabled });
}

export async function deleteProvider(providerId: string): Promise<void> {
  return invoke<void>('delete_provider', { providerId });
}

export async function refreshProviderModels(providerId: string): Promise<Provider[]> {
  return invoke<Provider[]>('refresh_provider_models', { providerId });
}

export async function testProviderConnection(providerId: string): Promise<TestConnectionResult> {
  return invoke<TestConnectionResult>('test_provider_connection', { providerId });
}

export async function getGroups(): Promise<Group[]> {
  return invoke<Group[]>('get_groups');
}

export async function saveGroup(group: Group): Promise<void> {
  return invoke<void>('save_group', { group });
}

export async function deleteGroup(groupId: string): Promise<void> {
  return invoke<void>('delete_group', { groupId });
}

export async function getAppConfig(): Promise<AppConfig> {
  return invoke<AppConfig>('get_app_config');
}

export async function saveAppConfig(config: AppConfig): Promise<void> {
  return invoke<void>('save_app_config', { config });
}

export async function getRouterStatus(): Promise<RouterStatusResponse> {
  return invoke<RouterStatusResponse>('get_router_status');
}

export async function setEntryEnabled(groupId: string, entryIndex: number, enabled: boolean): Promise<void> {
  return invoke<void>('set_entry_enabled', { groupId, entryIndex, enabled });
}

export async function getDailySummary(providerId: string, date: string): Promise<DailySummary> {
  return invoke<DailySummary>('get_daily_summary', { providerId, date });
}

export async function getRecentRequests(limit: number): Promise<RequestRow[]> {
  return invoke<RequestRow[]>('get_recent_requests', { limit });
}

export interface OpenCodeAgentMapping {
  build: string | null;
  plan: string | null;
  general: string | null;
  explore: string | null;
  compaction: string | null;
  title: string | null;
  summary: string | null;
  small_model: string | null;
}

export async function getOpencodeConfigPath(): Promise<string | null> {
  return invoke<string | null>('get_opencode_config_path');
}

export async function setOpencodeConfigPath(path: string): Promise<void> {
  return invoke<void>('set_opencode_config_path', { path });
}

export async function injectOpencodeProvider(proxyPort: number): Promise<void> {
  return invoke<void>('inject_opencode_provider', { proxyPort });
}

export async function removeOpencodeProvider(): Promise<void> {
  return invoke<void>('remove_opencode_provider');
}

export async function setOpencodeAgentModels(mapping: OpenCodeAgentMapping): Promise<void> {
  return invoke<void>('set_opencode_agent_models', { mapping });
}

export async function removeOpencodeAgentModels(): Promise<void> {
  return invoke<void>('remove_opencode_agent_models');
}

export async function previewOpencodeConfig(proxyPort: number, mapping: OpenCodeAgentMapping | null): Promise<string> {
  return invoke<string>('preview_opencode_config', { proxyPort, mapping });
}

export async function clearMetricsData(): Promise<void> {
  return invoke<void>('clear_metrics_data');
}

export async function resetAllConfig(): Promise<void> {
  return invoke<void>('reset_all_config');
}

export async function restartProxy(): Promise<void> {
  return invoke<void>('restart_proxy');
}

export async function isGroupReferencedInOpencode(groupAlias: string): Promise<boolean> {
  return invoke<boolean>('is_group_referenced_in_opencode', { groupAlias: groupAlias });
}

export async function removeCoderouterFromOpencode(): Promise<void> {
  return invoke<void>('remove_coderouter_from_opencode');
}

export async function dismissOnboarding(): Promise<void> {
  return invoke<void>('dismiss_onboarding');
}

export async function getUsageByGroup(days: number, providerId?: string): Promise<GroupUsage[]> {
  return invoke<GroupUsage[]>('get_usage_by_group', { days, providerId: providerId ?? null });
}


