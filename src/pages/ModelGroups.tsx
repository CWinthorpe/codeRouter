import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Plus,
  Edit2,
  Trash2,
  Layers,
  GripVertical,
  ChevronDown,
  ChevronUp,
  AlertTriangle,
  Loader2,
  Clock,
  Zap,
  Ban,
  Settings2,
  Activity,
  CheckCircle2,
  XCircle,
} from 'lucide-react';
import { useStore } from '../store';
import { ActionButton } from '../components/ActionButton';
import { Toast } from '../components/Toast';
import { saveGroup, deleteGroup, getGroups, setEntryEnabled, isGroupReferencedInOpencode } from '../lib/ipc';
import { useGroupStatusPoll } from '../hooks/useGroupStatusPoll';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Progress } from '@/components/ui/progress';
import type { Group, GroupEntry, FailoverConfig, Provider, EntryStatusResponse } from '../types';

/** Default failover configuration applied to new groups. */
const DEFAULT_FAILOVER: FailoverConfig = {
  on429: true,
  onQuotaExhausted: true,
  onConsecutiveErrors: true,
  consecutiveErrorThreshold: 5,
  onLatencyTimeout: true,
  latencyTimeoutMs: 30000,
  latencyTimeoutCooldownMs: 300000,
  consecutiveErrorCooldownMs: 600000,
};

/** Formats a number with locale-aware separators, or '—' for undefined. */
function formatNumber(n?: number): string {
  if (n == null) return '—';
  return n.toLocaleString();
}

/** Formats an ISO timestamp for display, or '—' for falsy values. */
function formatTimestamp(ts?: string): string {
  if (!ts) return '—';
  try {
    return new Date(ts).toLocaleString();
  } catch {
    return ts;
  }
}

/** Returns a Tailwind color class pair for a given entry status. */
function statusBadgeColor(status: string): string {
  switch (status) {
    case 'active':
      return 'bg-green-600/20 text-green-300';
    case 'cooldown':
      return 'bg-yellow-600/20 text-yellow-300';
    case 'quota_exhausted':
      return 'bg-orange-600/20 text-orange-300';
    case 'manually_disabled':
      return 'bg-zinc-600/20 text-zinc-300';
    default:
      return 'bg-zinc-600/20 text-zinc-300';
  }
}

/** Returns a Lucide icon component matching the entry status. */
function statusIcon(status: string) {
  switch (status) {
    case 'active':
      return <CheckCircle2 className="h-3.5 w-3.5" />;
    case 'cooldown':
      return <Clock className="h-3.5 w-3.5" />;
    case 'quota_exhausted':
      return <AlertTriangle className="h-3.5 w-3.5" />;
    case 'manually_disabled':
      return <Ban className="h-3.5 w-3.5" />;
    default:
      return <Activity className="h-3.5 w-3.5" />;
  }
}

/** Returns a human-readable label for an entry status string. */
function statusLabel(status: string): string {
  switch (status) {
    case 'active':
      return 'Active';
    case 'cooldown':
      return 'Cooldown';
    case 'quota_exhausted':
      return 'Quota Exhausted';
    case 'manually_disabled':
      return 'Manually Disabled';
    default:
      return status;
  }
}

const COOLDOWN_REASON_LABELS: Record<string, string> = {
  rate_limited: 'Rate limited (HTTP 429)',
  consecutive_errors: 'Consecutive errors',
  latency_timeout: 'Latency timeout',
  quota_exhausted: 'Daily quota exhausted',
};

function formatCooldownReason(reason?: string): string | null {
  if (!reason) return null;
  return COOLDOWN_REASON_LABELS[reason] ?? reason;
}

/**
 * Returns a human-readable countdown string (e.g., "2m 30s") from a
 * cooldownUntil ISO timestamp. Returns 'Expiring…' when the timestamp
 * is in the past. The _forceUpdateTick param forces re-renders.
 */
function cooldownCountdown(cooldownUntil?: string, _forceUpdateTick?: number): string {
  if (!cooldownUntil) return '';
  try {
    const diff = new Date(cooldownUntil).getTime() - Date.now();
    if (diff <= 0) return 'Expiring…';
    const mins = Math.floor(diff / 60000);
    const secs = Math.floor((diff % 60000) / 1000);
    return `${mins}m ${secs}s`;
  } catch {
    return '';
  }
}

/**
 * Model groups management page. Lists all groups as expandable cards
 * with live status, and provides a modal form for creating/editing groups
 * with drag-and-drop priority reordering and failover configuration.
 */
export default function ModelGroups() {
  const groups = useStore((s) => s.groups);
  const setGroups = useStore((s) => s.setGroups);
  const providers = useStore((s) => s.providers);
  const [editingGroup, setEditingGroup] = useState<Group | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [toasts, setToasts] = useState<{ id: number; type: 'success' | 'error'; message: string }[]>([]);
  const [expandedCards, setExpandedCards] = useState<Set<string>>(new Set());
  const toastCounterRef = useRef(0);

  /**
   * Enqueue a toast notification that auto-dismisses after 4 seconds.
   * Uses a counter ref to guarantee unique IDs even for rapid calls.
   */
  const addToast = useCallback((type: 'success' | 'error', message: string) => {
    const id = Date.now() * 1000 + (++toastCounterRef.current);
    setToasts((prev) => [...prev, { id, type, message }]);
    setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), 4000);
  }, []);

  /** Refreshes the group list from the backend and updates the store. */
  const refreshGroups = useCallback(async () => {
    try {
      const data = await getGroups();
      setGroups(data);
    } catch {
      // IPC may fail
    }
  }, [setGroups]);

  /** Opens the edit form pre-populated with the given group. */
  const handleEdit = useCallback((group: Group) => {
    setEditingGroup(group);
    setShowForm(true);
  }, []);

  /** Opens the create form with a blank group. */
  const handleCreate = useCallback(() => {
    setEditingGroup(null);
    setShowForm(true);
  }, []);

  /**
   * Deletes a group after user confirmation. Checks whether the group
   * is referenced in the OpenCode config and warns the user if so.
   */
  const handleDelete = useCallback(
    async (group: Group) => {
      let openCodeRef = false;
      try {
        openCodeRef = await isGroupReferencedInOpencode(group.alias);
      } catch {
        // IPC may fail
      }
      let message = `Are you sure you want to delete "${group.displayName || group.alias}"?`;
      if (openCodeRef) {
        message += '\n\nWarning: This group is referenced in your OpenCode configuration.';
      }

      if (!confirm(message)) return;

      try {
        await deleteGroup(group.id);
        await refreshGroups();
        addToast('success', `Deleted group "${group.displayName || group.alias}"`);
      } catch (e: unknown) {
        addToast('error', `Delete failed: ${e instanceof Error ? e.message : String(e)}`);
      }
    },
    [refreshGroups, addToast],
  );

  /** Toggles the expanded/collapsed state of a group card. */
  const toggleExpand = useCallback((id: string) => {
    setExpandedCards((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  return (
    <div className="max-w-5xl">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-semibold">Model Groups</h1>
          <p className="mt-1 text-sm text-zinc-400">
            Group models across providers with priority-based failover.
          </p>
        </div>
        <button
          onClick={handleCreate}
          className="flex items-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-emerald-500"
        >
          <Plus className="h-4 w-4" />
          Create Group
        </button>
      </div>

      {groups.length === 0 ? (
        <div className="flex flex-col items-center justify-center rounded-lg border border-dashed border-zinc-700 py-16 text-zinc-500">
          <Layers className="mb-3 h-10 w-10" />
          <p className="text-lg font-medium">No model groups configured</p>
          <p className="mt-1 text-sm">Create your first model group to enable failover routing.</p>
        </div>
      ) : (
        <div className="flex flex-col gap-4">
          {groups.map((group) => (
            <GroupCard
              key={group.id}
              group={group}
              providers={providers}
              isExpanded={expandedCards.has(group.id)}
              onToggleExpand={() => toggleExpand(group.id)}
              onEdit={() => handleEdit(group)}
              onDelete={() => handleDelete(group)}
            />
          ))}
        </div>
      )}

      {showForm && (
        <GroupForm
          group={editingGroup}
          providers={providers}
          onSave={async (group) => {
            await saveGroup(group);
            await refreshGroups();
            setShowForm(false);
            setEditingGroup(null);
            addToast('success', `Saved group "${group.displayName || group.alias}"`);
          }}
          onClose={() => {
            setShowForm(false);
            setEditingGroup(null);
          }}
        />
      )}

      <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2">
        {toasts.map((toast) => (
          <Toast key={toast.id} type={toast.type} message={toast.message} />
        ))}
      </div>
    </div>
  );
}

/**
 * Renders a single model group as a card showing display name, alias,
 * entry count, active entry info, health summary badges, and action buttons.
 * When expanded, shows the {@link LiveStatusPanel}.
 */
function GroupCard({
  group,
  providers,
  isExpanded,
  onToggleExpand,
  onEdit,
  onDelete,
}: {
  group: Group;
  providers: Provider[];
  isExpanded: boolean;
  onToggleExpand: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const statusData = useGroupStatusPoll(group.id);

  const healthSummary = useMemo(() => {
    const summary = { active: 0, cooldown: 0, quotaExhausted: 0, manuallyDisabled: 0 };
    statusData.entries.forEach((e) => {
      switch (e.status) {
        case 'active':
          summary.active++;
          break;
        case 'cooldown':
          summary.cooldown++;
          break;
        case 'quota_exhausted':
          summary.quotaExhausted++;
          break;
        case 'manually_disabled':
          summary.manuallyDisabled++;
          break;
      }
    });
    return summary;
  }, [statusData.entries]);

  const activeEntry = useMemo(() => {
    const activeEntries = statusData.entries.filter((e) => e.status === 'active');
    if (activeEntries.length === 0) return undefined;
    const indexMap = new Map<number, EntryStatusResponse>();
    statusData.entries.forEach((e) => indexMap.set(e.entry_index, e));
    const sortedByPriority = [...group.entries]
      .map((entry, idx) => ({ entry, idx }))
      .filter(({ idx }) => indexMap.get(idx)?.status === 'active')
      .sort((a, b) => a.entry.priority - b.entry.priority);
    if (sortedByPriority.length === 0) return activeEntries[0];
    return indexMap.get(sortedByPriority[0].idx);
  }, [statusData.entries, group.entries]);

  const providerNameForEntry = useCallback(
    (providerId: string) => {
      return providers.find((p) => p.id === providerId)?.name ?? providerId;
    },
    [providers],
  );

  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900/60">
      <div className="flex items-start gap-4 p-5">
        <div className="flex-1">
          <div className="flex items-center gap-3">
            <h3 className="text-base font-semibold">{group.displayName}</h3>
            <code className="rounded bg-zinc-800 px-2 py-0.5 text-xs font-mono text-zinc-400">
              {group.alias}
            </code>
          </div>

          <div className="mt-3 flex items-center gap-6 text-sm text-zinc-400">
            <span>{group.entries.length} entries</span>
            {activeEntry && (
              <span className="flex items-center gap-1.5">
                <Zap className="h-3.5 w-3.5 text-emerald-400" />
                Active: {providerNameForEntry(activeEntry.provider_id)} / {activeEntry.model_id}
              </span>
            )}
          </div>

          <div className="mt-2 flex items-center gap-3 text-xs">
            <HealthBadge label="Active" count={healthSummary.active} color="text-green-400" />
            <HealthBadge label="Cooldown" count={healthSummary.cooldown} color="text-yellow-400" />
            <HealthBadge label="Quota Exhausted" count={healthSummary.quotaExhausted} color="text-orange-400" />
            <HealthBadge label="Disabled" count={healthSummary.manuallyDisabled} color="text-zinc-400" />
          </div>
        </div>
      </div>

      <div className="flex items-center gap-2 border-t border-zinc-800 px-5 py-3">
        <button
          onClick={onToggleExpand}
          className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium text-zinc-300 transition-colors hover:bg-zinc-800 hover:text-zinc-100"
        >
          {isExpanded ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
          {isExpanded ? 'Hide' : 'Show'} Status
        </button>
        <ActionButton icon={<Edit2 className="h-3.5 w-3.5" />} label="Edit" onClick={onEdit} />
        <div className="flex-1" />
        <button
          onClick={onDelete}
          className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium text-red-400 transition-colors hover:bg-red-600/10 hover:text-red-300"
        >
          <Trash2 className="h-3.5 w-3.5" />
          Delete
        </button>
      </div>

      {isExpanded && (
        <LiveStatusPanel groupId={group.id} entries={group.entries} providers={providers} />
      )}
    </div>
  );
}

/** Small badge displaying a count and label with a colored text class. Hidden when count is 0. */
function HealthBadge({ label, count, color }: { label: string; count: number; color: string }) {
  if (count === 0) return null;
  return (
    <span className={`flex items-center gap-1 rounded bg-zinc-800/60 px-2 py-0.5 ${color}`}>
      {count} {label}
    </span>
  );
}

/**
 * Displays live entry status information inside an expanded group card.
 * Shows each entry's provider, model, status, cooldown countdown, quota
 * progress bar, and an enable/disable toggle that calls the IPC layer.
 */
function LiveStatusPanel({
  groupId,
  entries,
  providers,
}: {
  groupId: string;
  entries: GroupEntry[];
  providers: Provider[];
}) {
  const statusData = useGroupStatusPoll(groupId);
  const [toggling, setToggling] = useState<string | null>(null);
  const [tick, setTick] = useState(0);

  // Tick every second to refresh cooldown countdowns
  useEffect(() => {
    const interval = setInterval(() => setTick((t) => t + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  // Build a lookup map from entry index to its live status for O(1) access.
  const statusByIndex = useMemo(() => {
    const map = new Map<number, EntryStatusResponse>();
    statusData.entries.forEach((e) => map.set(e.entry_index, e));
    return map;
  }, [statusData.entries]);

  /** Toggles the enabled/disabled state of an entry via IPC. */
  const handleToggle = useCallback(
    async (entryIndex: number, currentEnabled: boolean) => {
      const key = `${groupId}-${entryIndex}`;
      setToggling(key);
      try {
        await setEntryEnabled(groupId, entryIndex, !currentEnabled);
      } catch {
        // IPC may fail
      } finally {
        setToggling(null);
      }
    },
    [groupId],
  );

  const providerNameForId = useCallback(
    (id: string) => providers.find((p) => p.id === id)?.name ?? id,
    [providers],
  );

  return (
    <div className="border-t border-zinc-800 px-5 py-4">
      <h4 className="mb-3 flex items-center gap-2 text-sm font-medium text-zinc-300">
        <Activity className="h-4 w-4" />
        Live Status
      </h4>
      <div className="flex flex-col gap-3">
        {entries.map((entry, idx) => {
          const st = statusByIndex.get(idx);
          const status = st?.status ?? (entry.enabled ? 'active' : 'manually_disabled');
          const toggleKey = `${groupId}-${idx}`;
          const quota = entry.dailyTokenQuotaOverride;
          const tokensUsed = st?.daily_tokens_used ?? 0;
          const entryKey = `${entry.providerId}-${entry.modelId}-${idx}-${entry.priority}`;

          return (
            <div
              key={entryKey}
              className="flex items-center gap-4 rounded-md border border-zinc-800 bg-zinc-900 px-4 py-3"
            >
              <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-zinc-800 text-xs font-mono text-zinc-400">
                {entry.priority}
              </span>

              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="truncate text-sm font-medium">
                    {providerNameForId(entry.providerId)}
                  </span>
                  <span className="font-mono text-xs text-zinc-500">{entry.modelId}</span>
                </div>

                <div className="mt-1.5 flex items-center gap-3">
                  <span className={`flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium ${statusBadgeColor(status)}`}>
                    {statusIcon(status)}
                    {statusLabel(status)}
                    {status === 'cooldown' && st?.cooldown_until && (
                      <span className="ml-1 font-mono">{cooldownCountdown(st.cooldown_until, tick)}</span>
                    )}
                    {status === 'quota_exhausted' && st?.daily_reset_at && (
                      <span className="ml-1 text-zinc-400">resets {formatTimestamp(st.daily_reset_at)}</span>
                    )}
                  </span>
                </div>

                {(status === 'cooldown' || status === 'quota_exhausted') && st?.cooldown_reason && (
                  <p className="mt-0.5 text-xs text-yellow-400">
                    {formatCooldownReason(st.cooldown_reason)}
                  </p>
                )}

                {quota != null && quota > 0 && (
                  <div className="mt-2 flex items-center gap-2">
                    <Progress
                      value={Math.min((tokensUsed / quota) * 100, 100)}
                      className="h-1.5 flex-1 bg-zinc-800 [&>div]:bg-emerald-500"
                    />
                    <span className="text-xs text-zinc-500">
                      {formatNumber(tokensUsed)} / {formatNumber(quota)}
                    </span>
                  </div>
                )}
              </div>

              <label className="relative inline-flex shrink-0 cursor-pointer items-center">
                <input
                  type="checkbox"
                  aria-label={`Toggle ${providerNameForId(entry.providerId)} ${entry.modelId}`}
                  checked={entry.enabled}
                  onChange={() => handleToggle(idx, entry.enabled)}
                  className="peer sr-only"
                  disabled={toggling === toggleKey}
                />
                <div className="peer h-5 w-9 rounded-full bg-zinc-700 after:absolute after:start-[2px] after:top-[2px] after:h-4 after:w-4 after:rounded-full after:border after:border-zinc-600 after:bg-zinc-400 after:transition-all peer-checked:bg-emerald-600 peer-checked:after:translate-x-full peer-checked:after:border-white peer-focus:outline-none peer-disabled:opacity-50" />
              </label>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/**
 * Reusable searchable dropdown component. Shows a text input that
 * filters options on type, with click-outside-to-close behavior.
 */
function SearchableSelect({ options, value, onChange, placeholder, disabled }: {
  options: { value: string; label: string }[];
  value: string;
  onChange: (v: string) => void;
  placeholder: string;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const ref = useRef<HTMLDivElement>(null);

  // Close the dropdown when clicking outside the component.
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  const filtered = options.filter(o => o.label.toLowerCase().includes(search.toLowerCase()));
  const selectedLabel = options.find(o => o.value === value)?.label ?? '';

  return (
    <div ref={ref} className="relative">
      <input
        type="text"
        disabled={disabled}
        value={open ? search : selectedLabel}
        onChange={(e) => { setSearch(e.target.value); if (!open) setOpen(true); }}
        onFocus={() => { setOpen(true); setSearch(selectedLabel); }}
        placeholder={placeholder}
        className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500 disabled:opacity-50"
      />
      {open && !disabled && (
        <div className="absolute z-50 mt-1 max-h-48 w-full overflow-auto rounded-md border border-zinc-700 bg-zinc-800 shadow-xl">
          {filtered.length === 0 && <p className="px-3 py-2 text-xs text-zinc-500">No matches</p>}
          {filtered.map(o => (
            <button key={o.value} type="button"
              className={`w-full px-3 py-2 text-left text-sm hover:bg-zinc-700 ${o.value === value ? 'bg-zinc-700 text-emerald-400' : 'text-zinc-200'}`}
              onClick={() => { onChange(o.value); setOpen(false); setSearch(''); }}
            >{o.label}</button>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * Modal form for creating or editing a model group. Supports drag-and-drop
 * priority reordering of entries, per-entry quota overrides, enable/disable
 * toggles, and a collapsible failover configuration panel.
 */
function GroupForm({
  group,
  providers,
  onSave,
  onClose,
}: {
  group: Group | null;
  providers: Provider[];
  onSave: (group: Group) => Promise<void>;
  onClose: () => void;
}) {
  const isEditing = group !== null;
  const [alias, setAlias] = useState(group?.alias ?? '');
  const [displayName, setDisplayName] = useState(group?.displayName ?? '');
  const [entries, setEntries] = useState<GroupEntry[]>(group?.entries ?? []);
  const [failoverConfig, setFailoverConfig] = useState<FailoverConfig>(
    group?.failoverConfig ?? { ...DEFAULT_FAILOVER },
  );
  const [aliasError, setAliasError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [showAddEntry, setShowAddEntry] = useState(false);
  const [showFailover, setShowFailover] = useState(false);
  const [dragIdx, setDragIdx] = useState<number | null>(null);
  const dragOverThrottleRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const entryIdCounter = useRef(0);
  // Track unique React keys for each entry row so that drag-and-drop
  // reordering doesn't lose component state incorrectly.
  const [entryKeys, setEntryKeys] = useState<string[]>(
    group?.entries.map(() => `e-${++entryIdCounter.current}`) ?? []
  );

  // Reset the failover panel when switching between groups.
  // Reset the failover panel when switching between groups.
  useEffect(() => {
    setShowFailover(false);
  }, [group?.id]);

  // Add entry sub-form state
  const [addProviderId, setAddProviderId] = useState('');
  const [addModelId, setAddModelId] = useState('');
  const [addQuotaOverride, setAddQuotaOverride] = useState('');

  const selectedProvider = useMemo(
    () => providers.find((p) => p.id === addProviderId),
    [providers, addProviderId],
  );

  /** Validates the group alias: must be non-empty and contain only lowercase alphanumerics and hyphens. */
  const validateAlias = useCallback((value: string) => {
    if (!value.trim()) {
      setAliasError('Alias is required.');
      return false;
    }
    if (!/^[a-z0-9-]+$/.test(value)) {
      setAliasError('Alias must contain only lowercase letters, numbers, and hyphens.');
      return false;
    }
    setAliasError(null);
    return true;
  }, []);

  /** Normalizes alias input by lowercasing and replacing spaces/special chars with hyphens. */
  const handleAliasChange = useCallback(
    (value: string) => {
      const normalized = value.toLowerCase().replace(/\s+/g, '-').replace(/[^a-z0-9-]/g, '');
      setAlias(normalized);
      if (aliasError) validateAlias(normalized);
    },
    [aliasError, validateAlias],
  );

  /** Appends a new provider+model entry to the group with the next priority. */
  const handleAddEntry = useCallback(() => {
    if (!addProviderId || !addModelId) return;
    const newEntry: GroupEntry = {
      providerId: addProviderId,
      modelId: addModelId,
      priority: entries.length + 1,
      dailyTokenQuotaOverride: addQuotaOverride ? Number(addQuotaOverride) : undefined,
      enabled: true,
      status: 'active',
    };
    setEntries((prev) => [...prev, newEntry]);
    setEntryKeys((prev) => [...prev, `e-${++entryIdCounter.current}`]);
    setAddProviderId('');
    setAddModelId('');
    setAddQuotaOverride('');
    setShowAddEntry(false);
  }, [addProviderId, addModelId, addQuotaOverride, entries.length]);

  /** Removes an entry by index and re-numbers priorities to stay contiguous. */
  const handleRemoveEntry = useCallback((idx: number) => {
    setEntries((prev) => prev.filter((_, i) => i !== idx).map((e, i) => ({ ...e, priority: i + 1 })));
    setEntryKeys((prev) => prev.filter((_, i) => i !== idx));
  }, []);

  /** Toggles the enabled flag on a single entry. */
  const handleEntryToggle = useCallback((idx: number) => {
    setEntries((prev) =>
      prev.map((e, i) => (i === idx ? { ...e, enabled: !e.enabled } : e)),
    );
  }, []);

  /** Updates the per-entry daily token quota override. */
  const handleQuotaChange = useCallback((idx: number, value: string) => {
    setEntries((prev) =>
      prev.map((e, i) =>
        i === idx
          ? { ...e, dailyTokenQuotaOverride: value ? Number(value) : undefined }
          : e,
      ),
    );
  }, []);

  /** Initiates a drag: stores the source index in dataTransfer and state. */
const handleDragStart = useCallback((e: React.DragEvent, idx: number) => {
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', String(idx));
    setDragIdx(idx);
  }, []);

  /**
   * Handles dragging over another entry. Throttled to 50ms to avoid
   * excessive state updates. Swaps entries in the list and re-numbers
   * all priorities to stay contiguous.
   */
  const handleDragOver = useCallback((e: React.DragEvent, idx: number) => {
    e.preventDefault();
    if (dragIdx === null || dragIdx === idx) return;
    if (dragOverThrottleRef.current) return;
    dragOverThrottleRef.current = setTimeout(() => {
      dragOverThrottleRef.current = null;
    }, 50);
    setEntries((prev) => {
      const next = [...prev];
      const [moved] = next.splice(dragIdx, 1);
      next.splice(idx, 0, moved);
      return next.map((e, i) => ({ ...e, priority: i + 1 }));
    });
    setEntryKeys((prev) => {
      const next = [...prev];
      const [moved] = next.splice(dragIdx, 1);
      next.splice(idx, 0, moved);
      return next;
    });
    setDragIdx(idx);
  }, [dragIdx]);

  /** Resets drag state when the drag operation ends. */
  const handleDragEnd = useCallback(() => {
    setDragIdx(null);
  }, []);

  /** Prevents default browser behavior on drop to avoid navigation. */
  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragIdx(null);
  }, []);

  /**
   * Validates alias, display name, and entries, then constructs a Group
   * object with renumbered priorities and delegates to the parent's onSave.
   */
  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSaveError(null);

    if (!validateAlias(alias)) return;
    if (!displayName.trim()) {
      setSaveError('Display name is required.');
      return;
    }
    if (entries.length === 0) {
      setSaveError('At least one provider entry is required.');
      return;
    }

    setSaving(true);
    try {
      const groupObj: Group = {
        id: isEditing ? group!.id : alias,
        alias: alias.trim(),
        displayName: displayName.trim(),
        entries: entries.map((entry, idx) => ({ ...entry, priority: idx + 1 })),
        failoverConfig,
      };
      await onSave(groupObj);
    } catch (err: unknown) {
      setSaveError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  /** Resolves a provider ID to its display name. */
  const providerNameForId = useCallback(
    (id: string) => providers.find((p) => p.id === id)?.name ?? id,
    [providers],
  );

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="max-w-2xl bg-zinc-900 border-zinc-800 text-zinc-100">
        <DialogHeader>
          <DialogTitle>{isEditing ? 'Edit Group' : 'Create Group'}</DialogTitle>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="flex flex-col gap-6">
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="mb-1 block text-sm font-medium text-zinc-300">
                Alias <span className="text-zinc-500">(model ID)</span>
              </label>
              <input
                type="text"
                value={alias}
                onChange={(e) => handleAliasChange(e.target.value)}
                className={`w-full rounded-md border bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:outline-none focus:ring-1 ${
                  aliasError
                    ? 'border-red-600 focus:border-red-500 focus:ring-red-500'
                    : 'border-zinc-700 focus:border-emerald-500 focus:ring-emerald-500'
                }`}
                placeholder="e.g. glm-5-router"
                autoFocus
              />
              {aliasError && <p className="mt-1 text-xs text-red-400">{aliasError}</p>}
            </div>
            <div>
              <label className="mb-1 block text-sm font-medium text-zinc-300">Display Name</label>
              <input
                type="text"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                placeholder="e.g. GLM-5 (Multi-Account)"
              />
            </div>
          </div>

          {/* Provider entries */}
          <div>
            <div className="mb-2 flex items-center justify-between">
              <label className="text-sm font-medium text-zinc-300">Provider Entries</label>
              <button
                type="button"
                onClick={() => setShowAddEntry(true)}
                className="flex items-center gap-1 rounded-md bg-zinc-800 px-2.5 py-1 text-xs font-medium text-zinc-300 transition-colors hover:bg-zinc-700"
              >
                <Plus className="h-3.5 w-3.5" />
                Add Entry
              </button>
            </div>

            {entries.length === 0 ? (
              <p className="rounded-md border border-dashed border-zinc-700 py-6 text-center text-sm text-zinc-500">
                No entries yet. Click "Add Entry" to add a provider+model.
              </p>
            ) : (
              <div className="flex flex-col gap-2">
                {entries.map((entry, idx) => (
                  <div
                    key={entryKeys[idx]}
                    draggable
                    onDragStart={(e) => handleDragStart(e, idx)}
                    onDragOver={(e) => handleDragOver(e, idx)}
                    onDrop={handleDrop}
                    onDragEnd={handleDragEnd}
                    className={`flex items-center gap-3 rounded-md border border-zinc-800 bg-zinc-900 px-3 py-2.5 transition-opacity ${
                      dragIdx === idx ? 'opacity-50' : ''
                    }`}
                  >
                    <div className="cursor-grab text-zinc-600 hover:text-zinc-400 active:cursor-grabbing">
                      <GripVertical className="h-4 w-4" />
                    </div>
                    <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-zinc-800 text-xs font-mono text-zinc-400">
                      {idx + 1}
                    </span>
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <span className="truncate text-sm font-medium">
                          {providerNameForId(entry.providerId)}
                        </span>
                        <span className="font-mono text-xs text-zinc-500">{entry.modelId}</span>
                      </div>
                    </div>
                    <div className="flex items-center gap-2">
                      <input
                        type="number"
                        value={entry.dailyTokenQuotaOverride ?? ''}
                        onChange={(e) => handleQuotaChange(idx, e.target.value)}
                        placeholder="Quota override"
                        className="w-28 rounded-md border border-zinc-700 bg-zinc-800 px-2 py-1 text-xs text-zinc-100 placeholder-zinc-600 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                      />
                      <label className="relative inline-flex cursor-pointer items-center">
                        <input
                          type="checkbox"
                          checked={entry.enabled}
                          onChange={() => handleEntryToggle(idx)}
                          className="peer sr-only"
                        />
                        <div className="peer h-4 w-7 rounded-full bg-zinc-700 after:absolute after:start-[1px] after:top-[1px] after:h-3 after:w-3 after:rounded-full after:border after:border-zinc-600 after:bg-zinc-400 after:transition-all peer-checked:bg-emerald-600 peer-checked:after:translate-x-full peer-checked:after:border-white" />
                      </label>
                      <button
                        type="button"
                        onClick={() => handleRemoveEntry(idx)}
                        className="rounded p-1 text-zinc-500 transition-colors hover:bg-red-600/10 hover:text-red-400"
                      >
                        <XCircle className="h-4 w-4" />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Add Entry sub-form */}
          {showAddEntry && (
            <div className="rounded-md border border-zinc-700 bg-zinc-800/50 p-4">
              <h4 className="mb-3 text-sm font-medium text-zinc-300">Add Provider Entry</h4>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="mb-1 block text-xs font-medium text-zinc-400">Provider</label>
                  <SearchableSelect
                    value={addProviderId}
                    onChange={(v) => {
                      setAddProviderId(v);
                      setAddModelId('');
                    }}
                    placeholder="Select provider…"
                    options={providers.map(p => ({ value: p.id, label: p.name }))}
                  />
                </div>
                <div>
                  <label className="mb-1 block text-xs font-medium text-zinc-400">Model</label>
                  <SearchableSelect
                    value={addModelId}
                    onChange={setAddModelId}
                    disabled={!selectedProvider}
                    placeholder="Select model…"
                    options={selectedProvider?.models.map(m => ({ value: m.id, label: m.id })) ?? []}
                  />
                </div>
              </div>
              <div className="mt-3">
                <label className="mb-1 block text-xs font-medium text-zinc-400">
                  Daily Token Quota Override <span className="text-zinc-600">(optional)</span>
                </label>
                <input
                  type="number"
                  value={addQuotaOverride}
                  onChange={(e) => setAddQuotaOverride(e.target.value)}
                  placeholder="Uses provider quota if empty"
                  className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-600 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                  min="0"
                />
              </div>
              <div className="mt-3 flex justify-end gap-2">
                <button
                  type="button"
                  onClick={() => setShowAddEntry(false)}
                  className="rounded-md px-3 py-1.5 text-xs font-medium text-zinc-300 transition-colors hover:bg-zinc-700"
                >
                  Cancel
                </button>
                <button
                  type="button"
                  onClick={handleAddEntry}
                  disabled={!addProviderId || !addModelId}
                  className="rounded-md bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-emerald-500 disabled:opacity-50"
                >
                  Add Entry
                </button>
              </div>
            </div>
          )}

          {/* Failover settings */}
          <div className="rounded-md border border-zinc-800">
            <button
              type="button"
              onClick={() => setShowFailover(!showFailover)}
              className="flex w-full items-center justify-between px-4 py-3 text-sm font-medium text-zinc-300 transition-colors hover:bg-zinc-800/50"
            >
              <span className="flex items-center gap-2">
                <Settings2 className="h-4 w-4" />
                Failover Settings
              </span>
              {showFailover ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
            </button>

            {showFailover && (
              <div className="border-t border-zinc-800 px-4 py-4">
                <div className="flex flex-col gap-4">
                  <ToggleRow
                    label="Failover on 429 / rate limit"
                    checked={failoverConfig.on429}
                    onChange={(v) => setFailoverConfig((c) => ({ ...c, on429: v }))}
                  />
                  <ToggleRow
                    label="Failover on daily quota exhausted"
                    checked={failoverConfig.onQuotaExhausted}
                    onChange={(v) => setFailoverConfig((c) => ({ ...c, onQuotaExhausted: v }))}
                  />
                  <ToggleRow
                    label="Failover on consecutive errors"
                    checked={failoverConfig.onConsecutiveErrors}
                    onChange={(v) => setFailoverConfig((c) => ({ ...c, onConsecutiveErrors: v }))}
                  />
                  {failoverConfig.onConsecutiveErrors && (
                    <div className="ml-7">
                      <label className="mb-1 block text-xs font-medium text-zinc-400">Error threshold</label>
                      <input
                        type="number"
                        value={failoverConfig.consecutiveErrorThreshold}
                        onChange={(e) =>
                          setFailoverConfig((c) => ({
                            ...c,
                            consecutiveErrorThreshold: Math.max(1, Number(e.target.value)),
                          }))
                        }
                        className="w-24 rounded-md border border-zinc-700 bg-zinc-800 px-3 py-1.5 text-sm text-zinc-100 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                        min="1"
                      />
                      <label className="mb-1 mt-3 block text-xs font-medium text-zinc-400">Cooldown after errors (seconds)</label>
                      <input
                        type="number"
                        value={failoverConfig.consecutiveErrorCooldownMs / 1000}
                        onChange={(e) =>
                          setFailoverConfig((c) => ({
                            ...c,
                            consecutiveErrorCooldownMs: Math.max(60000, Number(e.target.value) * 1000),
                          }))
                        }
                        className="w-32 rounded-md border border-zinc-700 bg-zinc-800 px-3 py-1.5 text-sm text-zinc-100 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                        min="60"
                      />
                    </div>
                  )}
                  <ToggleRow
                    label="Failover on latency timeout"
                    checked={failoverConfig.onLatencyTimeout}
                    onChange={(v) => setFailoverConfig((c) => ({ ...c, onLatencyTimeout: v }))}
                  />
                  {failoverConfig.onLatencyTimeout && (
                    <div className="ml-7">
                      <label className="mb-1 block text-xs font-medium text-zinc-400">Timeout (seconds)</label>
                      <input
                        type="number"
                        value={failoverConfig.latencyTimeoutMs / 1000}
                        onChange={(e) =>
                          setFailoverConfig((c) => ({
                            ...c,
                            latencyTimeoutMs: Math.max(1000, Number(e.target.value) * 1000),
                          }))
                        }
                        className="w-32 rounded-md border border-zinc-700 bg-zinc-800 px-3 py-1.5 text-sm text-zinc-100 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                        min="1"
                      />
                      <label className="mb-1 mt-3 block text-xs font-medium text-zinc-400">Cooldown after timeout (seconds)</label>
                      <input
                        type="number"
                        value={failoverConfig.latencyTimeoutCooldownMs / 1000}
                        onChange={(e) =>
                          setFailoverConfig((c) => ({
                            ...c,
                            latencyTimeoutCooldownMs: Math.max(60000, Number(e.target.value) * 1000),
                          }))
                        }
                        className="w-32 rounded-md border border-zinc-700 bg-zinc-800 px-3 py-1.5 text-sm text-zinc-100 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                        min="60"
                      />
                    </div>
                  )}
                </div>
              </div>
            )}
          </div>

          {saveError && (
            <div className="flex items-center gap-2 rounded-md bg-red-600/10 px-3 py-2 text-sm text-red-400">
              <AlertTriangle className="h-4 w-4 shrink-0" />
              {saveError}
            </div>
          )}

          <div className="flex justify-end gap-3">
            <button
              type="button"
              onClick={onClose}
              className="rounded-md px-4 py-2 text-sm font-medium text-zinc-300 transition-colors hover:bg-zinc-800"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={saving}
              className="flex items-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-emerald-500 disabled:opacity-50"
            >
              {saving && <Loader2 className="h-4 w-4 animate-spin" />}
              {saving ? 'Saving…' : isEditing ? 'Save Changes' : 'Create Group'}
            </button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Simple toggle switch row used in the failover settings panel. */
function ToggleRow({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-sm text-zinc-300">{label}</span>
      <label className="relative inline-flex cursor-pointer items-center">
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
          className="peer sr-only"
        />
        <div className="peer h-5 w-9 rounded-full bg-zinc-700 after:absolute after:start-[2px] after:top-[2px] after:h-4 after:w-4 after:rounded-full after:border after:border-zinc-600 after:bg-zinc-400 after:transition-all peer-checked:bg-emerald-600 peer-checked:after:translate-x-full peer-checked:after:border-white peer-focus:outline-none" />
      </label>
    </div>
  );
}
