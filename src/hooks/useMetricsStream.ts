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

    const host = appConfig?.proxy_host ?? 'localhost';
    const port = appConfig?.proxy_port ?? 4141;
    const url = `http://${host}:${port}/internal/metrics/stream`;

    const es = new EventSource(url);
    esRef.current = es;

    es.onmessage = (e) => {
      try {
        const event = JSON.parse(e.data);
        addRecentRequest(event);
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
