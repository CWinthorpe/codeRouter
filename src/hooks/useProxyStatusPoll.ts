import { useEffect, useRef } from 'react';
import { useStore } from '../store';

const POLL_INTERVAL_MS = 5000;

export function useProxyStatusPoll() {
  const setProxyStatus = useStore((s) => s.setProxyStatus);
  const appConfig = useStore((s) => s.appConfig);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const proxyPort = appConfig?.proxy_port ?? 4141;
  const healthUrl = `http://localhost:${proxyPort}/health`;

  useEffect(() => {
    const poll = async () => {
      try {
        const res = await fetch(healthUrl, { signal: AbortSignal.timeout(3000) });
        if (res.ok) {
          setProxyStatus('running');
        } else {
          setProxyStatus('stopped');
        }
      } catch {
        setProxyStatus('stopped');
      }
    };

    poll();
    intervalRef.current = setInterval(poll, POLL_INTERVAL_MS);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [setProxyStatus, healthUrl]);
}
