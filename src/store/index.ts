import { create } from 'zustand';
import type { Provider, Group, AppConfig, ProxyStatus, RequestRow } from '../types';
import { getProviders, getGroups, getAppConfig } from '../lib/ipc';

export interface HealthData {
  status: string;
  uptime_seconds: number;
}

interface AppState {
  providers: Provider[];
  groups: Group[];
  appConfig: AppConfig | null;
  proxyStatus: ProxyStatus;
  healthData: HealthData | null;
  loadError: string | null;
  recentStreamRequests: RequestRow[];
  setProviders: (providers: Provider[]) => void;
  setGroups: (groups: Group[]) => void;
  setAppConfig: (config: AppConfig) => void;
  setProxyStatus: (status: ProxyStatus) => void;
  setHealthData: (data: HealthData | null) => void;
  addRecentRequest: (request: RequestRow) => void;
  resetAll: () => void;
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

    if (errors.length > 0) {
      console.error('loadInitialData partial failures:', errors);
      set({ ...updates, loadError: errors.join('; ') });
    } else {
      set({ ...updates, loadError: null });
    }
  },
}));
