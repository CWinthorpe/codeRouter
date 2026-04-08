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
    const results = await Promise.allSettled([
      getProviders(),
      getGroups(),
      getAppConfig(),
    ]);

    const errors: string[] = [];

    if (results[0].status === 'fulfilled') {
      set({ providers: results[0].value, loadError: null });
    } else {
      errors.push(`providers: ${results[0].reason}`);
    }

    if (results[1].status === 'fulfilled') {
      set({ groups: results[1].value, loadError: null });
    } else {
      errors.push(`groups: ${results[1].reason}`);
    }

    if (results[2].status === 'fulfilled') {
      set({ appConfig: results[2].value, loadError: null });
    } else {
      errors.push(`appConfig: ${results[2].reason}`);
    }

    if (errors.length > 0) {
      console.error('loadInitialData partial failures:', errors);
      set({ loadError: errors.join('; ') });
    }
  },
}));
