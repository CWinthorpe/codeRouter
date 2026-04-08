import { useEffect, useRef } from 'react';
import { useStore } from '../store';

const PROXY_HEALTH_URL = 'http://localhost:4141/health';
const POLL_INTERVAL_MS = 5000;

export function useProxyStatusPoll() {
  const setProxyStatus = useStore((s) => s.setProxyStatus);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    const poll = async () => {
      try {
        const res = await fetch(PROXY_HEALTH_URL, { signal: AbortSignal.timeout(3000) });
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
  }, [setProxyStatus]);
}
