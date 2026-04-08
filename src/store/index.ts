import { create } from 'zustand';
import type { Provider, Group, AppConfig, ProxyStatus } from '../types';
import { getProviders, getGroups, getAppConfig } from '../lib/ipc';

interface AppState {
  providers: Provider[];
  groups: Group[];
  appConfig: AppConfig | null;
  proxyStatus: ProxyStatus;
  setProviders: (providers: Provider[]) => void;
  setGroups: (groups: Group[]) => void;
  setAppConfig: (config: AppConfig) => void;
  setProxyStatus: (status: ProxyStatus) => void;
  loadInitialData: () => Promise<void>;
}

export const useStore = create<AppState>((set) => ({
  providers: [],
  groups: [],
  appConfig: null,
  proxyStatus: 'unknown',

  setProviders: (providers) => set({ providers }),
  setGroups: (groups) => set({ groups }),
  setAppConfig: (appConfig) => set({ appConfig }),
  setProxyStatus: (proxyStatus) => set({ proxyStatus }),

  loadInitialData: async () => {
    try {
      const [providers, groups, appConfig] = await Promise.all([
        getProviders(),
        getGroups(),
        getAppConfig(),
      ]);
      set({ providers, groups, appConfig });
    } catch {
      // IPC calls may fail when running outside Tauri (e.g. vite dev)
    }
  },
}));
