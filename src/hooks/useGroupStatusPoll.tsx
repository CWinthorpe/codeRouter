import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { getRouterStatus } from '../lib/ipc';
import type { RouterStatusResponse, EntryStatusResponse } from '../types';

/** Polling interval in milliseconds for group status updates. */
const POLL_INTERVAL_MS = 5000;

/** Shape of the context value exposed by GroupStatusProvider. */
interface GroupStatusContextValue {
  /** The latest router status data for all groups and entries. */
  statusData: RouterStatusResponse;
  /** Whether the initial fetch is still in progress. */
  loading: boolean;
  /** Error message from the most recent failed fetch, or null. */
  error: string | null;
}

const GroupStatusContext = createContext<GroupStatusContextValue>({
  statusData: { entries: [] },
  loading: true,
  error: null,
});

/**
 * Provider that periodically polls the router status endpoint and
 * makes the data available to descendant components via context.
 *
 * Encapsulates the polling lifecycle so multiple consumers share one
 * set of data rather than each starting their own interval.
 */
export function GroupStatusProvider({ children }: { children: ReactNode }) {
  const [statusData, setStatusData] = useState<RouterStatusResponse>({ entries: [] });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    const poll = async () => {
      try {
        const data = await getRouterStatus();
        setStatusData(data);
        setError(null);
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : 'Failed to fetch router status');
      } finally {
        // Mark loading as done even on error so the UI stops showing the skeleton
        setLoading(false);
      }
    };

    // Fetch immediately so the user doesn't stare at a loading state for 5 seconds
    poll();
    intervalRef.current = setInterval(poll, POLL_INTERVAL_MS);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <GroupStatusContext.Provider value={{ statusData, loading, error }}>
      {children}
    </GroupStatusContext.Provider>
  );
}

/** Access the raw group-status context value (status data, loading, error). */
export function useGroupStatusData() {
  return useContext(GroupStatusContext);
}

/**
 * Hook that returns router status data, optionally filtered to a single group.
 *
 * @param groupId - If provided, only entries belonging to this group are returned.
 * @returns The full router status response, or a filtered subset for one group.
 */
export function useGroupStatusPoll(groupId?: string) {
  const { statusData } = useGroupStatusData();

  if (groupId) {
    // Filter to just the entries for the requested group so the consumer
    // doesn't have to iterate the full list on every render
    return {
      entries: statusData.entries.filter((e: EntryStatusResponse) => e.group_id === groupId),
    };
  }

  return statusData;
}