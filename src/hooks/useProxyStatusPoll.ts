import { useEffect, useRef } from 'react';
import { useStore } from '../store';
import { checkProxyHealth } from '../lib/ipc';

const POLL_INTERVAL_MS = 5000;

export function useProxyStatusPoll() {
  const setProxyStatus = useStore((s) => s.setProxyStatus);
  const setHealthData = useStore((s) => s.setHealthData);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    const poll = async () => {
      try {
        const result = await checkProxyHealth();
        if (result.running) {
          setProxyStatus('running');
          setHealthData(result.uptime_seconds != null
            ? { status: result.status ?? 'ok', uptime_seconds: result.uptime_seconds }
            : null);
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
  }, [setProxyStatus, setHealthData]);
}
