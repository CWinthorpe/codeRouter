import { describe, it, expect, vi, beforeEach } from 'vitest';
import { useStore } from '../store';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

describe('useStore', () => {
  beforeEach(() => {
    const { setProviders, setGroups, setAppConfig } = useStore.getState();
    setProviders([]);
    setGroups([]);
    setAppConfig({
      proxy_port: 4141,
      proxy_host: '127.0.0.1',
      refresh_interval_hours: 24,
      log_verbosity: 'Info',
    });
  });

  it('should have default initial state', () => {
    const state = useStore.getState();
    expect(state.providers).toEqual([]);
    expect(state.groups).toEqual([]);
    expect(state.proxyStatus).toBe('unknown');
    expect(state.loadError).toBeNull();
  });

  it('should update providers via setProviders', () => {
    const { setProviders } = useStore.getState();
    const mockProviders = [{ id: 'p1', name: 'Test', protocol: 'openai', baseUrl: 'https://test.com', credentialKey: 'p1', quotaResetUtcHour: 0, enabled: true, models: [] }];
    setProviders(mockProviders as any);
    expect(useStore.getState().providers).toHaveLength(1);
    expect(useStore.getState().providers[0].id).toBe('p1');
  });

  it('should update groups via setGroups', () => {
    const { setGroups } = useStore.getState();
    const mockGroups = [{ id: 'g1', alias: 'test', displayName: 'Test', entries: [], failoverConfig: { on429: true, onQuotaExhausted: true, onConsecutiveErrors: true, consecutiveErrorThreshold: 5, onLatencyTimeout: true, latencyTimeoutMs: 30000 } }];
    setGroups(mockGroups as any);
    expect(useStore.getState().groups).toHaveLength(1);
    expect(useStore.getState().groups[0].alias).toBe('test');
  });

  it('should update proxy status', () => {
    const { setProxyStatus } = useStore.getState();
    setProxyStatus('running');
    expect(useStore.getState().proxyStatus).toBe('running');
    setProxyStatus('stopped');
    expect(useStore.getState().proxyStatus).toBe('stopped');
  });
});
