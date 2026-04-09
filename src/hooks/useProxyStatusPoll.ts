import { useEffect, useRef } from 'react';
import { useStore } from '../store';

const POLL_INTERVAL_MS = 5000;

export function useProxyStatusPoll() {
  const setProxyStatus = useStore((s) => s.setProxyStatus);
  const setHealthData = useStore((s) => s.setHealthData);
  const appConfig = useStore((s) => s.appConfig);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const proxyHost = appConfig?.proxy_host ?? '127.0.0.1';
  const proxyPort = appConfig?.proxy_port ?? 4141;
  const healthUrl = `http://${proxyHost}:${proxyPort}/health`;

  useEffect(() => {
    const poll = async () => {
      try {
        const res = await fetch(healthUrl, { signal: AbortSignal.timeout(3000) });
        if (res.ok) {
          setProxyStatus('running');
          try {
            const data = await res.json();
            setHealthData({ status: data.status, uptime_seconds: data.uptime_seconds });
          } catch {
            setHealthData(null);
          }
        } else {
          setProxyStatus('stopped');
          setHealthData(null);
        }
      } catch {
        setProxyStatus('stopped');
        setHealthData(null);
      }
    };

    poll();
    intervalRef.current = setInterval(poll, POLL_INTERVAL_MS);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [setProxyStatus, setHealthData, healthUrl]);
}
