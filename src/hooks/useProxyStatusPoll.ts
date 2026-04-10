import { useEffect, useRef } from 'react';
import { useStore } from '../store';
import { checkProxyHealth } from '../lib/ipc';

/** Polling interval in milliseconds for proxy health checks. */
const POLL_INTERVAL_MS = 5000;

/**
 * Hook that periodically polls the proxy health endpoint and updates
 * the store with the current proxy status and health data.
 *
 * Automatically starts polling on mount and cleans up on unmount.
 * If the health check fails, the proxy is assumed to be stopped.
 */
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
          // Only surface health data when uptime is available; otherwise clear it
          setHealthData(result.uptime_seconds != null
            ? { status: result.status ?? 'ok', uptime_seconds: result.uptime_seconds }
            : null);
        } else {
          setProxyStatus('stopped');
          setHealthData(null);
        }
      } catch {
        // Network error means the proxy process is not reachable
        setProxyStatus('stopped');
        setHealthData(null);
      }
    };

    // Run the first check immediately so the UI doesn't show "unknown" for 5 seconds
    poll();
    intervalRef.current = setInterval(poll, POLL_INTERVAL_MS);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [setProxyStatus, setHealthData]);
}