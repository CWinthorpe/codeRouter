import { useEffect, useRef, useState } from 'react';
import { getRouterStatus } from '../lib/ipc';
import type { RouterStatusResponse } from '../types';

const POLL_INTERVAL_MS = 5000;

export function useGroupStatusPoll(groupId?: string) {
  const [statusData, setStatusData] = useState<RouterStatusResponse>({ entries: [] });
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    const poll = async () => {
      try {
        const data = await getRouterStatus();
        if (groupId) {
          setStatusData({
            entries: data.entries.filter((e) => e.group_id === groupId),
          });
        } else {
          setStatusData(data);
        }
      } catch {
        // IPC may fail
      }
    };

    poll();
    intervalRef.current = setInterval(poll, POLL_INTERVAL_MS);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [groupId]);

  return statusData;
}
