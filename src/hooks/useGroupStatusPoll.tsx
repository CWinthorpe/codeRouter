import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { getRouterStatus } from '../lib/ipc';
import type { RouterStatusResponse, EntryStatusResponse } from '../types';

const POLL_INTERVAL_MS = 5000;

interface GroupStatusContextValue {
  statusData: RouterStatusResponse;
}

const GroupStatusContext = createContext<GroupStatusContextValue>({
  statusData: { entries: [] },
});

export function GroupStatusProvider({ children }: { children: ReactNode }) {
  const [statusData, setStatusData] = useState<RouterStatusResponse>({ entries: [] });
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    const poll = async () => {
      try {
        const data = await getRouterStatus();
        setStatusData(data);
      } catch {
        // IPC may fail
      }
    };

    poll();
    intervalRef.current = setInterval(poll, POLL_INTERVAL_MS);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, []);

  return (
    <GroupStatusContext.Provider value={{ statusData }}>
      {children}
    </GroupStatusContext.Provider>
  );
}

export function useGroupStatusData() {
  return useContext(GroupStatusContext);
}

export function useGroupStatusPoll(groupId?: string) {
  const { statusData } = useGroupStatusData();

  if (groupId) {
    return {
      entries: statusData.entries.filter((e: EntryStatusResponse) => e.group_id === groupId),
    };
  }

  return statusData;
}
