import { useEffect, useRef } from 'react';
import { useStore } from '../store';

export function useMetricsStream() {
  const appConfig = useStore((s) => s.appConfig);
  const addRecentRequest = useStore((s) => s.addRecentRequest);
  const proxyStatus = useStore((s) => s.proxyStatus);
  const esRef = useRef<EventSource | null>(null);

  useEffect(() => {
    if (proxyStatus !== 'running') {
      if (esRef.current) {
        esRef.current.close();
        esRef.current = null;
      }
      return;
    }

    const host = appConfig?.proxy_host ?? '127.0.0.1';
    const port = appConfig?.proxy_port ?? 4141;
    const url = `http://${host}:${port}/internal/metrics/stream`;

    const es = new EventSource(url);
    esRef.current = es;

    es.onmessage = (e) => {
      try {
        const raw = JSON.parse(e.data);
        const inputCost = typeof raw.input_cost_per_1m === 'number' && typeof raw.prompt_tokens === 'number'
          ? (raw.prompt_tokens * raw.input_cost_per_1m) / 1_000_000
          : 0;
        const outputCost = typeof raw.output_cost_per_1m === 'number' && typeof raw.output_tokens === 'number'
          ? (raw.output_tokens * raw.output_cost_per_1m) / 1_000_000
          : 0;
        const row = {
          id: Date.now() * 1000 + Math.floor(Math.random() * 1000),
          ts: raw.ts ?? Math.floor(Date.now() / 1000),
          group_alias: raw.group_alias ?? '',
          provider_id: raw.provider_id ?? '',
          model_id: raw.model_id ?? '',
          prompt_tokens: raw.prompt_tokens ?? 0,
          output_tokens: raw.output_tokens ?? 0,
          cost_usd: inputCost + outputCost,
          latency_ms: raw.latency_ms ?? 0,
          status: raw.status ?? 'success',
          error_type: raw.error_type ?? null,
        };
        addRecentRequest(row);
      } catch {
        // ignore parse errors
      }
    };

    es.onerror = () => {
      es.close();
      esRef.current = null;
    };

    return () => {
      es.close();
      esRef.current = null;
    };
  }, [appConfig?.proxy_host, appConfig?.proxy_port, proxyStatus, addRecentRequest]);
}
