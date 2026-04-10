import { create } from 'zustand';
import type { Provider, Group, AppConfig, ProxyStatus, RequestRow } from '../types';
import { getProviders, getGroups, getAppConfig } from '../lib/ipc';

/** Health data reported by the proxy process. */
export interface HealthData {
  /** Current operational status string (e.g. "ok"). */
  status: string;
  /** How long the proxy has been running, in seconds. */
  uptime_seconds: number;
}

interface AppState {
  /** All configured LLM providers. */
  providers: Provider[];
  /** All configured routing groups. */
  groups: Group[];
  /** Application configuration, or null before initial load. */
  appConfig: AppConfig | null;
  /** Runtime status of the proxy process. */
  proxyStatus: ProxyStatus;
  /** Health data from the last proxy health check, or null if not running. */
  healthData: HealthData | null;
  /** Error message if initial data loading partially failed, or null. */
  loadError: string | null;
  /** Ring buffer of the most recent streaming request rows (max 50). */
  recentStreamRequests: RequestRow[];
  /** Replace the entire providers list. */
  setProviders: (providers: Provider[]) => void;
  /** Replace the entire groups list. */
  setGroups: (groups: Group[]) => void;
  /** Replace the application configuration. */
  setAppConfig: (config: AppConfig) => void;
  /** Update the proxy status indicator. */
  setProxyStatus: (status: ProxyStatus) => void;
  /** Update the proxy health data. */
  setHealthData: (data: HealthData | null) => void;
  /** Append a request row to the front of the recent-streams buffer (capped at 50). */
  addRecentRequest: (request: RequestRow) => void;
  /** Reset all state back to initial defaults (used after app reset). */
  resetAll: () => void;
  /** Fetch providers, groups, and app config in parallel and hydrate the store. */
  loadInitialData: () => Promise<void>;
}

export const useStore = create<AppState>((set) => ({
  providers: [],
  groups: [],
  appConfig: null,
  proxyStatus: 'unknown',
  healthData: null,
  loadError: null,
  recentStreamRequests: [],

  setProviders: (providers) => set({ providers }),
  setGroups: (groups) => set({ groups }),
  setAppConfig: (appConfig) => set({ appConfig }),
  setProxyStatus: (proxyStatus) => set({ proxyStatus }),
  setHealthData: (healthData) => set({ healthData }),

  addRecentRequest: (request) =>
    // Prepend the new request and keep only the 50 most recent entries
    set((state) => ({
      recentStreamRequests: [request, ...state.recentStreamRequests].slice(0, 50),
    })),

  resetAll: () => set({
    providers: [],
    groups: [],
    appConfig: null,
    proxyStatus: 'unknown',
    healthData: null,
    loadError: null,
    recentStreamRequests: [],
  }),

  loadInitialData: async () => {
    // Fetch all three data sources concurrently so a slow provider doesn't block groups/config
    const results = await Promise.allSettled([
      getProviders(),
      getGroups(),
      getAppConfig(),
    ]);

    const errors: string[] = [];
    const updates: Partial<Pick<AppState, 'providers' | 'groups' | 'appConfig' | 'loadError'>> = {};

    if (results[0].status === 'fulfilled') {
      updates.providers = results[0].value;
    } else {
      errors.push(`providers: ${results[0].reason}`);
    }

    if (results[1].status === 'fulfilled') {
      updates.groups = results[1].value;
    } else {
      errors.push(`groups: ${results[1].reason}`);
    }

    if (results[2].status === 'fulfilled') {
      updates.appConfig = results[2].value;
    } else {
      errors.push(`appConfig: ${results[2].reason}`);
    }

    // Partial failures are stored but don't prevent the successful data from loading
    if (errors.length > 0) {
      console.error('loadInitialData partial failures:', errors);
      set({ ...updates, loadError: errors.join('; ') });
    } else {
      set({ ...updates, loadError: null });
    }
  },
}));