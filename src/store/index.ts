import { create } from 'zustand';
import type { Provider, Group, AppConfig, ProxyStatus } from '../types';
import { getProviders, getGroups, getAppConfig } from '../lib/ipc';

interface AppState {
  providers: Provider[];
  groups: Group[];
  appConfig: AppConfig | null;
  proxyStatus: ProxyStatus;
  loadError: string | null;
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
  loadError: null,

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
      set({ providers, groups, appConfig, loadError: null });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('loadInitialData failed:', message);
      set({ loadError: message });
    }
  },
}));
