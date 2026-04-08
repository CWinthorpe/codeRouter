import { describe, it, expect } from 'vitest';

describe('Provider type', () => {
  it('should accept modelOverrides field', () => {
    const provider = {
      id: 'test',
      name: 'Test',
      protocol: 'openai' as const,
      baseUrl: 'https://test.com',
      credentialKey: 'test',
      quotaResetUtcHour: 0,
      enabled: true,
      models: [],
      modelOverrides: [{ id: 'model-1', contextWindow: 128000 }],
    };
    expect(provider.modelOverrides).toBeDefined();
    expect(provider.modelOverrides).toHaveLength(1);
  });
});

describe('AppConfig type', () => {
  it('should accept opencode_config_path field', () => {
    const config = {
      proxy_port: 4141,
      proxy_host: '127.0.0.1',
      refresh_interval_hours: 24,
      log_verbosity: 'Info' as const,
      opencode_config_path: '/home/user/.config/opencode/opencode.json',
    };
    expect(config.opencode_config_path).toBe('/home/user/.config/opencode/opencode.json');
  });

  it('should work without opencode_config_path', () => {
    const config: {
      proxy_port: number;
      proxy_host: string;
      refresh_interval_hours: number;
      log_verbosity: 'Debug';
      opencode_config_path?: string;
    } = {
      proxy_port: 4141,
      proxy_host: '127.0.0.1',
      refresh_interval_hours: 24,
      log_verbosity: 'Debug',
    };
    expect(config.opencode_config_path).toBeUndefined();
  });
});
