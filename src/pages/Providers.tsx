import { useCallback, useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-shell';
import {
  Plus,
  Edit2,
  Trash2,
  Zap,
  RefreshCw,
  ChevronDown,
  ChevronUp,
  ChevronRight,
  Loader2,
  AlertTriangle,
  CheckCircle2,
  XCircle,
} from 'lucide-react';
import { useStore } from '../store';
import { ActionButton } from '../components/ActionButton';
import { Toast } from '../components/Toast';
import {
  saveProvider,
  saveGroup,
  toggleProviderEnabled,
  deleteProvider,
  testProviderConnection,
  refreshProviderModels,
  getGroups,
  getProviders,
  startCodexDeviceAuth,
  pollCodexDeviceAuth,
} from '../lib/ipc';
import type { Provider, ProviderModel, Group, GroupEntry } from '../types';
import type { CodexDeviceAuthStart, TestConnectionResult } from '../lib/ipc';
import { providerPresets, type ProviderPreset } from '../lib/provider-presets';
import { Card, CardContent } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '@/components/ui/table';

/** Generates a URL-safe, lowercase ID from a provider name. */
function generateId(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
    .slice(0, 40);
}

/** Formats an ISO timestamp string for display, returning 'Never' for falsy values. */
function formatTimestamp(ts?: string): string {
  if (!ts) return 'Never';
  try {
    const d = new Date(ts);
    return d.toLocaleString();
  } catch {
    return ts;
  }
}

/** Formats a number as a USD cost string (e.g., "$1.50"). */
function formatCost(n?: number): string {
  if (n == null) return '—';
  return `$${n.toFixed(2)}`;
}

/** Formats a number with locale-aware separators. */
function formatNumber(n?: number): string {
  if (n == null) return '—';
  return n.toLocaleString();
}

/**
 * Providers page. Lists all configured LLM providers with expand/collapse
 * to browse models, inline actions for testing connections, refreshing
 * models, editing, deleting, and toggling enabled state. New providers
 * are created via the {@link ProviderModal} dialog.
 */
export default function Providers() {
  const providers = useStore((s) => s.providers);
  const setProviders = useStore((s) => s.setProviders);
  const [expandedProviders, setExpandedProviders] = useState<Set<string>>(new Set());
  const [editingProvider, setEditingProvider] = useState<Provider | null>(null);
  const [showAddModal, setShowAddModal] = useState(false);
  const [testingId, setTestingId] = useState<string | null>(null);
  const [refreshingId, setRefreshingId] = useState<string | null>(null);
  const [toasts, setToasts] = useState<{ id: number; type: 'success' | 'error'; message: string }[]>([]);
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

  /** Toggle the expanded state of a provider card to show/hide its model browser. */
  const toggleExpand = useCallback((id: string) => {
    setExpandedProviders((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  /** Sends a test connection request to the provider and shows the result as a toast. */
  const handleTestConnection = useCallback(
    async (provider: Provider) => {
      setTestingId(provider.id);
      try {
        const result: TestConnectionResult = await testProviderConnection(provider.id);
        if (result.success) {
          addToast('success', result.message);
        } else {
          addToast('error', result.message);
        }
      } catch (e: unknown) {
        addToast('error', `Test failed: ${e instanceof Error ? e.message : String(e)}`);
      } finally {
        setTestingId(null);
      }
    },
    [addToast],
  );

  /** Fetches fresh model listings from the provider's API and updates the store. */
  const handleRefreshModels = useCallback(
    async (provider: Provider) => {
      setRefreshingId(provider.id);
      try {
        const updated = await refreshProviderModels(provider.id);
        setProviders(updated);
        addToast('success', `Refreshed models for ${provider.name}`);
      } catch (e: unknown) {
        addToast('error', `Refresh failed: ${e instanceof Error ? e.message : String(e)}`);
      } finally {
        setRefreshingId(null);
      }
    },
    [addToast, setProviders],
  );

  /**
   * Deletes a provider after user confirmation. Warns the user if the
   * provider is currently referenced by any model groups.
   */
  const handleDelete = useCallback(
    async (provider: Provider) => {
      const currentGroups = await getGroups();
      const groupsUsingProvider = currentGroups.filter((g) =>
        g.entries.some((e) => e.providerId === provider.id),
      );

      let message = `Are you sure you want to delete "${provider.name}"?`;
      if (groupsUsingProvider.length > 0) {
        const groupNames = groupsUsingProvider.map((g) => g.displayName || g.alias).join(', ');
        message += `\n\nWarning: This provider is used in the following model groups: ${groupNames}`;
      }

      if (!confirm(message)) return;

      try {
        await deleteProvider(provider.id);
        const updated = await getProviders();
        setProviders(updated);
        addToast('success', `Deleted provider "${provider.name}"`);
      } catch (e: unknown) {
        addToast('error', `Delete failed: ${e instanceof Error ? e.message : String(e)}`);
      }
    },
    [setProviders, addToast],
  );

  /** Persists provider config (with API key) via IPC, then refreshes the store list. */
  const handleSave = useCallback(
    async (provider: Provider, apiKey: string) => {
      await saveProvider(provider, apiKey);
      const updated = await getProviders();
      setProviders(updated);
      setShowAddModal(false);
      setEditingProvider(null);
      addToast('success', `Saved provider "${provider.name}"`);
    },
    [setProviders, addToast],
  );

  /** Toggles a provider's enabled flag and persists the change. */
  const handleToggleEnabled = useCallback(
    async (provider: Provider) => {
      const newEnabled = !provider.enabled;
      await toggleProviderEnabled(provider.id, newEnabled);
      const allProviders = await getProviders();
      setProviders(allProviders);
    },
    [setProviders],
  );

  return (
    <div className="max-w-5xl">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-semibold">Providers</h1>
          <p className="mt-1 text-sm text-zinc-400">
            Manage upstream LLM providers and browse their available models.
          </p>
        </div>
        <Button
          onClick={() => {
            setEditingProvider(null);
            setShowAddModal(true);
          }}
          className="gap-2 bg-emerald-600 hover:bg-emerald-500"
        >
          <Plus className="h-4 w-4" />
          Add Provider
        </Button>
      </div>

      {providers.length === 0 ? (
        <Card className="border-dashed border-zinc-700">
          <CardContent className="flex flex-col items-center justify-center py-16 text-zinc-500">
            <Zap className="mb-3 h-10 w-10" />
            <p className="text-lg font-medium">No providers configured</p>
            <p className="mt-1 text-sm">Add your first upstream provider to get started.</p>
          </CardContent>
        </Card>
      ) : (
        <div className="flex flex-col gap-4">
          {providers.map((provider) => (
            <ProviderCard
              key={provider.id}
              provider={provider}
              isExpanded={expandedProviders.has(provider.id)}
              onToggleExpand={() => toggleExpand(provider.id)}
              onEdit={() => {
                setEditingProvider(provider);
                setShowAddModal(true);
              }}
              onDelete={() => handleDelete(provider)}
              onTestConnection={() => handleTestConnection(provider)}
              onRefreshModels={() => handleRefreshModels(provider)}
              onToggleEnabled={() => handleToggleEnabled(provider)}
              isTesting={testingId === provider.id}
              isRefreshing={refreshingId === provider.id}
            />
          ))}
        </div>
      )}

      {showAddModal && (
        <ProviderModal
          key={editingProvider ? editingProvider.id : 'new'}
          provider={editingProvider}
          onSave={handleSave}
          onClose={() => {
            setShowAddModal(false);
            setEditingProvider(null);
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
 * Renders a single provider as a collapsible card showing name, protocol,
 * enabled state, model count, last refresh time, and action buttons.
 * When expanded, shows the {@link ModelBrowser} table.
 */
function ProviderCard({
  provider,
  isExpanded,
  onToggleExpand,
  onEdit,
  onDelete,
  onTestConnection,
  onRefreshModels,
  onToggleEnabled,
  isTesting,
  isRefreshing,
}: {
  provider: Provider;
  isExpanded: boolean;
  onToggleExpand: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onTestConnection: () => void;
  onRefreshModels: () => void;
  onToggleEnabled: () => void;
  isTesting: boolean;
  isRefreshing: boolean;
}) {
  const protocolLabel =
    provider.protocol === 'anthropic' ? 'Anthropic-compatible'
    : provider.protocol === 'openai-codex' ? 'Codex (ChatGPT)'
    : 'OpenAI-compatible';
  const protocolColor =
    provider.protocol === 'anthropic' ? 'bg-violet-600/20 text-violet-300'
    : provider.protocol === 'openai-codex' ? 'bg-amber-600/20 text-amber-300'
    : 'bg-emerald-600/20 text-emerald-300';

  const lastRefresh = provider.models[0]?.last_refreshed;
  const modelCount = provider.models.length;

  return (
    <Card className={`bg-zinc-900/60 border-zinc-800 transition-opacity ${!provider.enabled ? 'opacity-60' : ''}`}>
      <CardContent className="p-0">
        <div className="flex items-start gap-4 p-5">
          <button onClick={onToggleExpand} className="mt-1 text-zinc-500 transition-colors hover:text-zinc-300">
            {isExpanded ? <ChevronDown className="h-5 w-5" /> : <ChevronRight className="h-5 w-5" />}
          </button>

          <div className="flex-1">
            <div className="flex items-center gap-3">
              <h3 className="text-base font-semibold">{provider.name}</h3>
              <Badge className={`rounded-full text-xs font-medium ${protocolColor}`}>
                {protocolLabel}
              </Badge>
              <Badge
                variant="outline"
                className={`flex items-center gap-1.5 rounded-full text-xs font-medium ${
                  provider.enabled ? 'bg-green-600/20 text-green-300 border-green-600/30' : 'bg-red-600/20 text-red-300 border-red-600/30'
                }`}
              >
                <span className={`h-1.5 w-1.5 rounded-full ${provider.enabled ? 'bg-green-400' : 'bg-red-400'}`} />
                {provider.enabled ? 'Enabled' : 'Disabled'}
              </Badge>
            </div>

            <p className="mt-1 text-sm text-zinc-400 font-mono">{provider.baseUrl}</p>

            <div className="mt-3 flex items-center gap-6 text-sm text-zinc-400">
              <span className="flex items-center gap-1.5">
                <CheckCircle2 className="h-4 w-4 text-zinc-500" />
                {modelCount} model{modelCount !== 1 ? 's' : ''}
              </span>
              <span className="flex items-center gap-1.5">
                <RefreshCw className="h-4 w-4 text-zinc-500" />
                {lastRefresh ? formatTimestamp(lastRefresh) : 'Never refreshed'}
              </span>
              {provider.modelOverrides && provider.modelOverrides.length > 0 && (
                <Badge variant="outline" className="text-xs">
                  Custom models ({provider.modelOverrides.length})
                </Badge>
              )}
            </div>
          </div>

          <div className="flex flex-col items-end gap-2">
            <label className="relative inline-flex cursor-pointer items-center">
              <input
                type="checkbox"
                aria-label={`Toggle ${provider.name}`}
                checked={provider.enabled}
                onChange={onToggleEnabled}
                className="peer sr-only"
              />
              <div className="peer h-5 w-9 rounded-full bg-zinc-700 after:absolute after:start-[2px] after:top-[2px] after:h-4 after:w-4 after:rounded-full after:border after:border-zinc-600 after:bg-zinc-400 after:transition-all peer-checked:bg-emerald-600 peer-checked:after:translate-x-full peer-checked:after:border-white peer-focus:outline-none" />
            </label>
          </div>
        </div>

        <div className="flex items-center gap-2 border-t border-zinc-800 px-5 py-3">
          <ActionButton icon={<Edit2 className="h-3.5 w-3.5" />} label="Edit" onClick={onEdit} />
          <ActionButton
            icon={isTesting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Zap className="h-3.5 w-3.5" />}
            label={isTesting ? 'Testing…' : 'Test Connection'}
            onClick={onTestConnection}
            disabled={isTesting}
          />
          <ActionButton
            icon={isRefreshing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
            label={isRefreshing ? 'Refreshing…' : 'Refresh Models'}
            onClick={onRefreshModels}
            disabled={isRefreshing}
          />
          <div className="flex-1" />
          <Button
            variant="ghost"
            size="sm"
            onClick={onDelete}
            className="gap-1.5 text-xs font-medium text-red-400 hover:bg-red-600/10 hover:text-red-300"
          >
            <Trash2 className="h-3.5 w-3.5" />
            Delete
          </Button>
        </div>

        {isExpanded && <ModelBrowser models={provider.models} providerName={provider.name} providerId={provider.id} />}
      </CardContent>
    </Card>
  );
}

/**
 * Displays a table of models available from a provider. Each row has an
 * "Add to group" action that opens a dropdown of existing groups to
 * which the model can be added as a new entry.
 */
function ModelBrowser({ models, providerName, providerId }: { models: ProviderModel[]; providerName: string; providerId: string }) {
  const [addingModelId, setAddingModelId] = useState<string | null>(null);
  const [groups, setGroups] = useState<Group[]>([]);
  const [loadingGroups, setLoadingGroups] = useState(false);
  const [saving, setSaving] = useState(false);
  const [toasts, setToasts] = useState<{ id: number; type: 'success' | 'error'; message: string }[]>([]);
  const toastCounterRef = useRef(0);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const setStoreGroups = useStore((s) => s.setGroups);

  const addToast = useCallback((type: 'success' | 'error', message: string) => {
    const id = Date.now() * 1000 + (++toastCounterRef.current);
    setToasts((prev) => [...prev, { id, type, message }]);
    setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), 4000);
  }, []);

  useEffect(() => {
    if (!addingModelId) return;
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setAddingModelId(null);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [addingModelId]);

  const handleAddToGroup = async (modelId: string) => {
    if (addingModelId === modelId) {
      setAddingModelId(null);
      return;
    }
    setLoadingGroups(true);
    try {
      const g = await getGroups();
      setGroups(g);
      setAddingModelId(modelId);
    } catch {
      setAddingModelId(null);
    } finally {
      setLoadingGroups(false);
    }
  };

  /** Adds a model to an existing group by appending a new GroupEntry. */
  const handleSelectGroup = async (group: Group, modelId: string) => {
    setSaving(true);
    try {
      const newEntry: GroupEntry = {
        providerId,
        modelId,
        priority: group.entries.length + 1,
        enabled: true,
        status: 'active',
      };
      const updatedGroup: Group = {
        ...group,
        entries: [...group.entries, newEntry],
      };
      await saveGroup(updatedGroup);
      const updatedGroups = await getGroups();
      setStoreGroups(updatedGroups);
      setAddingModelId(null);
    } catch (e: unknown) {
      addToast('error', `Failed to add model to group: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setSaving(false);
    }
  };

  if (models.length === 0) {
    return (
      <div className="border-t border-zinc-800 px-5 py-6 text-center text-sm text-zinc-500">
        No models found. Click "Refresh Models" to fetch available models from {providerName}.
      </div>
    );
  }

  return (
    <div className="border-t border-zinc-800">
      <Table className="text-sm">
        <TableHeader>
          <TableRow className="border-b border-zinc-800 hover:bg-transparent">
            <TableHead className="px-5 py-3 font-medium text-xs uppercase tracking-wider text-zinc-500">Model ID</TableHead>
            <TableHead className="px-5 py-3 font-medium text-xs uppercase tracking-wider text-zinc-500">Context Window</TableHead>
            <TableHead className="px-5 py-3 font-medium text-xs uppercase tracking-wider text-zinc-500">Max Output Tokens</TableHead>
            <TableHead className="px-5 py-3 font-medium text-xs uppercase tracking-wider text-zinc-500">Input Cost/1M</TableHead>
            <TableHead className="px-5 py-3 font-medium text-xs uppercase tracking-wider text-zinc-500">Output Cost/1M</TableHead>
            <TableHead className="px-5 py-3 font-medium text-xs uppercase tracking-wider text-zinc-500">Last Refreshed</TableHead>
            <TableHead className="px-5 py-3 font-medium text-xs uppercase tracking-wider text-zinc-500" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {models.map((model) => (
            <TableRow key={model.id} className="border-b border-zinc-800/50 hover:bg-zinc-800/30">
              <TableCell className="px-5 py-3 font-mono text-xs text-zinc-100">{model.id}</TableCell>
              <TableCell className="px-5 py-3 text-zinc-300">{formatNumber(model.context_window)}</TableCell>
              <TableCell className="px-5 py-3 text-zinc-300">{formatNumber(model.max_output_tokens)}</TableCell>
              <TableCell className="px-5 py-3 text-zinc-300">{formatCost(model.input_cost_per_1m)}</TableCell>
              <TableCell className="px-5 py-3 text-zinc-300">{formatCost(model.output_cost_per_1m)}</TableCell>
              <TableCell className="px-5 py-3 text-zinc-400">{formatTimestamp(model.last_refreshed)}</TableCell>
              <TableCell className="px-5 py-3">
                <div className="relative" ref={addingModelId === model.id ? dropdownRef : undefined}>
                  <button
                    onClick={() => handleAddToGroup(model.id)}
                    disabled={saving || loadingGroups}
                    className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs font-medium text-zinc-300 transition-colors hover:bg-zinc-700 disabled:opacity-50"
                  >
                    {loadingGroups && addingModelId === model.id ? 'Loading…' : saving && addingModelId === model.id ? 'Saving…' : 'Add to group'}
                  </button>
                  {addingModelId === model.id && groups.length > 0 && (
                    <div className="absolute right-0 top-full z-50 mt-1 w-48 rounded-md border border-zinc-700 bg-zinc-800 shadow-xl">
                      {groups.map((g) => (
                        <button
                          key={g.id}
                          type="button"
                          className="w-full px-3 py-2 text-left text-sm text-zinc-200 hover:bg-zinc-700"
                          onClick={() => handleSelectGroup(g, model.id)}
                          disabled={saving}
                        >
                          {g.displayName || g.alias}
                        </button>
                      ))}
                    </div>
                  )}
                  {addingModelId === model.id && groups.length === 0 && (
                    <div className="absolute right-0 top-full z-50 mt-1 w-48 rounded-md border border-zinc-700 bg-zinc-800 p-3 text-xs text-zinc-500 shadow-xl">
                      No groups available
                    </div>
                  )}
                </div>
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
      {toasts.length > 0 && (
        <div className="flex flex-col gap-2 px-5 pt-3">
          {toasts.map((toast) => (
            <Toast key={toast.id} type={toast.type} message={toast.message} />
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * Modal dialog for creating or editing a provider. Handles form validation
 * for name, base URL, API key, quotas, and optional model overrides that
 * replace auto-discovered models with a custom list.
 */
function ProviderModal({
  provider,
  onSave,
  onClose,
}: {
  provider: Provider | null;
  onSave: (provider: Provider, apiKey: string) => Promise<void>;
  onClose: () => void;
}) {
  const isEditing = provider !== null;
  const [selectedPreset, setSelectedPreset] = useState<ProviderPreset | null>(null);
  const [name, setName] = useState(provider?.name ?? '');
  const [baseUrl, setBaseUrl] = useState(provider?.baseUrl ?? '');
  const [protocol, setProtocol] = useState(provider?.protocol ?? 'openai');
  const [apiKey, setApiKey] = useState('');
  const [showApiKey, setShowApiKey] = useState(false);
  const [dailyTokenQuota, setDailyTokenQuota] = useState(
    provider?.dailyTokenQuota != null ? String(provider.dailyTokenQuota) : '',
  );
  const [dailyRequestQuota, setDailyRequestQuota] = useState(
    provider?.dailyRequestQuota != null ? String(provider.dailyRequestQuota) : '',
  );
  const [quotaResetUtcHour, setQuotaResetUtcHour] = useState(
    provider?.quotaResetUtcHour != null ? String(provider.quotaResetUtcHour) : '0',
  );
  const [enabled, setEnabled] = useState(provider?.enabled ?? true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [modelOverrides, setModelOverrides] = useState<ProviderModel[]>(provider?.modelOverrides ?? []);
  const [showOverrides, setShowOverrides] = useState(false);
  const [overrideModelId, setOverrideModelId] = useState('');
  const [overrideContextWindow, setOverrideContextWindow] = useState('');
  const [overrideMaxOutputTokens, setOverrideMaxOutputTokens] = useState('');
  const [overrideInputCost, setOverrideInputCost] = useState('');
  const [overrideOutputCost, setOverrideOutputCost] = useState('');
  const [overrideProtocol, setOverrideProtocol] = useState<string>('');
  const [codexAuth, setCodexAuth] = useState<CodexDeviceAuthStart | null>(null);
  const [codexAuthStatus, setCodexAuthStatus] = useState<string | null>(null);
  const [codexAuthPolling, setCodexAuthPolling] = useState(false);
  const codexPollInFlight = useRef(false);

  /** Builds a ProviderModel from the override form fields and appends it. */
  const handleAddOverride = () => {
    if (!overrideModelId.trim()) return;
    const entry: ProviderModel = { id: overrideModelId.trim() };
    if (overrideContextWindow) entry.context_window = Number(overrideContextWindow);
    if (overrideMaxOutputTokens) entry.max_output_tokens = Number(overrideMaxOutputTokens);
    if (overrideInputCost) entry.input_cost_per_1m = Number(overrideInputCost);
    if (overrideOutputCost) entry.output_cost_per_1m = Number(overrideOutputCost);
    if (overrideProtocol && overrideProtocol !== '__none__') entry.protocol = overrideProtocol as 'openai' | 'anthropic' | 'openai-codex';
    setModelOverrides((prev) => [...prev, entry]);
    setOverrideModelId('');
    setOverrideContextWindow('');
    setOverrideMaxOutputTokens('');
    setOverrideInputCost('');
    setOverrideOutputCost('');
    setOverrideProtocol('');
  };

  const checkCodexAuth = useCallback(
    async (manual: boolean) => {
      if (!codexAuth || codexPollInFlight.current) return;
      codexPollInFlight.current = true;
      setCodexAuthPolling(true);
      try {
        const result = await pollCodexDeviceAuth(codexAuth.deviceAuthId, codexAuth.userCode);
        if (result.status === 'authorized' && result.credential) {
          setApiKey(result.credential);
          setShowApiKey(false);
          setCodexAuth(null);
          setCodexAuthStatus('ChatGPT sign-in complete. Save this provider to store the credential.');
          return;
        }
        if (manual) {
          setCodexAuthStatus(result.message ?? 'Waiting for ChatGPT approval.');
        }
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        codexPollInFlight.current = false;
        setCodexAuthPolling(false);
      }
    },
    [codexAuth],
  );

  const handleStartCodexAuth = async () => {
    setError(null);
    setCodexAuthStatus('Requesting ChatGPT device code...');
    try {
      const auth = await startCodexDeviceAuth();
      setCodexAuth(auth);
      setCodexAuthStatus('Open the link, sign in, and enter the one-time code. CodeRouter will detect approval automatically.');
      try {
        await open(auth.verificationUrl);
      } catch {
        // The URL is displayed below if the OS refuses to open a browser.
      }
    } catch (e: unknown) {
      setCodexAuth(null);
      setCodexAuthStatus(null);
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    if (!codexAuth) return;
    const intervalMs = Math.max(codexAuth.interval, 3) * 1000;
    const timer = window.setInterval(() => {
      void checkCodexAuth(false);
    }, intervalMs);
    return () => window.clearInterval(timer);
  }, [codexAuth, checkCodexAuth]);

  useEffect(() => {
    if (protocol !== 'openai-codex') {
      setCodexAuth(null);
      setCodexAuthStatus(null);
    }
  }, [protocol]);

  /**
   * Validates all form fields (name, URL, API key, quotas), constructs
   * a Provider object, and delegates persistence to the parent's onSave.
   */
  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (!name.trim()) {
      setError('Name is required.');
      return;
    }
    if (!baseUrl.trim()) {
      setError('Base URL is required.');
      return;
    }
    try {
      new URL(baseUrl);
    } catch {
      setError('Base URL must be a valid URL.');
      return;
    }
    if (!isEditing && !apiKey.trim()) {
      setError(protocol === 'openai-codex' ? 'Use Sign in with ChatGPT to generate the Codex credential.' : 'API key is required for new providers.');
      return;
    }
    if (dailyTokenQuota) {
      const quota = Number(dailyTokenQuota);
      if (isNaN(quota) || quota < 0) {
        setError('Daily token quota must be a non-negative number.');
        return;
      }
    }
    if (dailyRequestQuota) {
      const quota = Number(dailyRequestQuota);
      if (isNaN(quota) || quota < 0) {
        setError('Daily request quota must be a non-negative number.');
        return;
      }
    }
    const hour = Number(quotaResetUtcHour);
    if (isNaN(hour) || hour < 0 || hour > 23) {
      setError('Quota reset UTC hour must be between 0 and 23.');
      return;
    }

    setSaving(true);
    try {
      const providerObj: Provider = {
        id: provider?.id ?? generateId(name),
        name: name.trim(),
        protocol,
        baseUrl: baseUrl.trim(),
        credentialKey: provider?.credentialKey ?? generateId(name),
        dailyTokenQuota: dailyTokenQuota ? Number(dailyTokenQuota) : undefined,
        dailyRequestQuota: dailyRequestQuota ? Number(dailyRequestQuota) : undefined,
        quotaResetUtcHour: hour,
        enabled,
        models: provider?.models ?? [],
        modelOverrides: modelOverrides.length > 0 ? modelOverrides : undefined,
      };
      await onSave(providerObj, apiKey);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open onOpenChange={() => onClose()}>
      <DialogContent className="max-h-[90vh] max-w-lg overflow-y-auto bg-zinc-900 border-zinc-800 text-zinc-100">
        <DialogHeader>
          <DialogTitle>{isEditing ? 'Edit Provider' : 'Add Provider'}</DialogTitle>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="flex flex-col gap-4">
          {!isEditing && (
            <div>
              <p className="mb-2 text-sm font-medium text-zinc-300">Quick Add</p>
              <div className="grid grid-cols-2 gap-2">
                {providerPresets.map((preset) => (
                  <button
                    key={preset.id}
                    type="button"
                    onClick={() => {
                      setSelectedPreset(preset);
                      setName(preset.name);
                      setBaseUrl(preset.baseUrl);
                      setProtocol(preset.protocol);
                      setModelOverrides(preset.modelOverrides ?? []);
                      setShowOverrides((preset.modelOverrides?.length ?? 0) > 0);
                    }}
                    className={`rounded-md border px-3 py-2 text-left transition-colors ${
                      selectedPreset?.id === preset.id
                        ? 'border-emerald-500 bg-emerald-600/10'
                        : 'border-zinc-700 bg-zinc-800/50 hover:border-zinc-600 hover:bg-zinc-800'
                    }`}
                  >
                    <span className="block text-sm font-medium text-zinc-100">{preset.name}</span>
                    <span className="block text-xs text-zinc-500">{preset.description}</span>
                  </button>
                ))}
              </div>
            </div>
          )}
          <div>
            <label className="mb-1 block text-sm font-medium text-zinc-300">Name</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
              placeholder="e.g. Z.AI Coding Plan"
              autoFocus
            />
          </div>

          <div>
            <label className="mb-1 block text-sm font-medium text-zinc-300">Base URL</label>
            <input
              type="text"
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
              placeholder="https://api.example.com/v1"
            />
          </div>

          <div>
            <label className="mb-1 block text-sm font-medium text-zinc-300">Protocol Type</label>
            <Select value={protocol} onValueChange={(v) => setProtocol(v as 'openai' | 'anthropic' | 'openai-codex')}>
              <SelectTrigger className="w-full border-zinc-700 bg-zinc-800 text-zinc-100">
                <SelectValue placeholder="Select protocol" />
              </SelectTrigger>
              <SelectContent className="bg-zinc-800 border-zinc-700">
                <SelectItem value="openai" className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">OpenAI-compatible</SelectItem>
                <SelectItem value="anthropic" className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">Anthropic-compatible</SelectItem>
                <SelectItem value="openai-codex" className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">OpenAI Codex (ChatGPT)</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {protocol === 'openai-codex' && (
            <div className="rounded-md border border-amber-700/40 bg-amber-950/20 p-3">
              <div className="flex items-start justify-between gap-3">
                <div>
                  <p className="text-sm font-medium text-amber-200">ChatGPT device login</p>
                  <p className="mt-1 text-xs text-amber-100/70">
                    Generate a one-time code, sign in with ChatGPT in your browser, then save the provider after the credential is filled.
                  </p>
                </div>
                <button
                  type="button"
                  onClick={handleStartCodexAuth}
                  disabled={saving || codexAuthPolling}
                  className="shrink-0 rounded-md bg-amber-600 px-3 py-2 text-xs font-medium text-white transition-colors hover:bg-amber-500 disabled:opacity-50"
                >
                  {codexAuth ? 'New Code' : 'Sign in'}
                </button>
              </div>

              {codexAuth && (
                <div className="mt-3 rounded-md border border-amber-700/40 bg-zinc-950/40 p-3">
                  <div className="flex flex-wrap items-center gap-2 text-xs text-zinc-300">
                    <span>Open</span>
                    <button
                      type="button"
                      onClick={() => void open(codexAuth.verificationUrl)}
                      className="font-medium text-amber-200 underline decoration-amber-500/60 underline-offset-2 hover:text-amber-100"
                    >
                      {codexAuth.verificationUrl}
                    </button>
                    <span>and enter</span>
                    <code className="rounded bg-amber-500/15 px-2 py-1 text-sm font-semibold tracking-widest text-amber-100">
                      {codexAuth.userCode}
                    </code>
                  </div>
                  <div className="mt-3 flex items-center justify-between gap-3">
                    <p className="text-xs text-zinc-500">
                      Expires in {Math.ceil(codexAuth.expiresIn / 60)} minutes. Never share this code with anyone.
                    </p>
                    <button
                      type="button"
                      onClick={() => void checkCodexAuth(true)}
                      disabled={codexAuthPolling}
                      className="rounded-md border border-zinc-700 px-3 py-1.5 text-xs font-medium text-zinc-300 hover:bg-zinc-800 disabled:opacity-50"
                    >
                      {codexAuthPolling ? 'Checking...' : 'Check Now'}
                    </button>
                  </div>
                </div>
              )}

              {codexAuthStatus && (
                <p className="mt-2 text-xs text-amber-100/80">{codexAuthStatus}</p>
              )}
            </div>
          )}

          <div>
            <label className="mb-1 block text-sm font-medium text-zinc-300">
              {protocol === 'openai-codex' ? 'Codex credential' : 'API Key'}
              {isEditing && <span className="ml-2 text-xs text-zinc-500">(leave blank to keep existing)</span>}
            </label>
            {protocol === 'openai-codex' && (
              <p className="mb-2 text-xs text-zinc-500">
                Filled automatically after ChatGPT device login. Manual paste still works for recovery.
              </p>
            )}
            <div className="relative">
              <input
                type={showApiKey ? 'text' : 'password'}
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 pr-16 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                placeholder={protocol === 'openai-codex' ? 'Use Sign in above, or paste auth.json/access token' : isEditing ? 'Leave empty to keep existing key' : 'Enter API key'}
              />
              <button
                type="button"
                onClick={() => setShowApiKey(!showApiKey)}
                className="absolute right-2 top-1/2 -translate-y-1/2 rounded px-2 py-1 text-xs text-zinc-400 hover:text-zinc-200"
              >
                {showApiKey ? 'Hide' : 'Show'}
              </button>
              {isEditing && apiKey && (
                <button
                  type="button"
                  onClick={() => setApiKey('')}
                  className="absolute right-16 top-1/2 -translate-y-1/2 rounded px-2 py-1 text-xs text-zinc-400 hover:text-zinc-200"
                >
                  Clear
                </button>
              )}
            </div>
          </div>

          <div className="grid grid-cols-3 gap-4">
            <div>
              <label className="mb-1 block text-sm font-medium text-zinc-300">
                Daily Token Quota <span className="text-zinc-500">(optional)</span>
              </label>
              <input
                type="number"
                value={dailyTokenQuota}
                onChange={(e) => setDailyTokenQuota(e.target.value)}
                className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                placeholder="Unlimited"
                min="0"
              />
            </div>
            <div>
              <label className="mb-1 block text-sm font-medium text-zinc-300">
                Daily Request Quota <span className="text-zinc-500">(optional)</span>
              </label>
              <input
                type="number"
                value={dailyRequestQuota}
                onChange={(e) => setDailyRequestQuota(e.target.value)}
                className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                placeholder="Unlimited"
                min="0"
              />
            </div>
            <div>
              <label className="mb-1 block text-sm font-medium text-zinc-300">
                Quota Reset UTC Hour <span className="text-zinc-500">(0–23)</span>
              </label>
              <input
                type="number"
                value={quotaResetUtcHour}
                onChange={(e) => setQuotaResetUtcHour(e.target.value)}
                className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                min="0"
                max="23"
              />
            </div>
          </div>

          <div className="flex items-center gap-3">
            <label className="relative inline-flex cursor-pointer items-center">
              <input
                type="checkbox"
                checked={enabled}
                onChange={(e) => setEnabled(e.target.checked)}
                className="peer sr-only"
              />
              <div className="peer h-5 w-9 rounded-full bg-zinc-700 after:absolute after:start-[2px] after:top-[2px] after:h-4 after:w-4 after:rounded-full after:border after:border-zinc-600 after:bg-zinc-400 after:transition-all peer-checked:bg-emerald-600 peer-checked:after:translate-x-full peer-checked:after:border-white peer-focus:outline-none" />
            </label>
            <span className="text-sm text-zinc-300">Enabled</span>
          </div>

          <div className="rounded-md border border-zinc-800">
            <button
              type="button"
              onClick={() => setShowOverrides(!showOverrides)}
              className="flex w-full items-center justify-between px-4 py-3 text-sm font-medium text-zinc-300"
            >
              <span>Model Overrides ({modelOverrides.length})</span>
              {showOverrides ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
            </button>
            {showOverrides && (
              <div className="border-t border-zinc-800 px-4 py-4">
                <p className="mb-3 text-xs text-zinc-500">
                  Override auto-discovered models with a custom list. Leave empty to use auto-discovered models.
                </p>
                {modelOverrides.map((m, i) => (
                  <div key={m.id} className="mb-2 flex items-center gap-2">
                    <code className="text-xs text-zinc-400">{m.id}</code>
                    {m.protocol && (
                      <Badge className={`rounded-full text-xs font-medium ${m.protocol === 'anthropic' ? 'bg-violet-600/20 text-violet-300' : m.protocol === 'openai-codex' ? 'bg-amber-600/20 text-amber-300' : 'bg-emerald-600/20 text-emerald-300'}`}>
                        {m.protocol === 'anthropic' ? 'Anthropic' : m.protocol === 'openai-codex' ? 'Codex' : 'OpenAI'}
                      </Badge>
                    )}
                    <span className="text-xs text-zinc-600">ctx: {m.context_window ?? 'auto'}</span>
                    <span className="text-xs text-zinc-600">out: {m.max_output_tokens ?? 'auto'}</span>
                    {m.input_cost_per_1m != null && (
                      <span className="text-xs text-zinc-600">in: ${m.input_cost_per_1m}/1M</span>
                    )}
                    {m.output_cost_per_1m != null && (
                      <span className="text-xs text-zinc-600">out: ${m.output_cost_per_1m}/1M</span>
                    )}
                    <button
                      type="button"
                      onClick={() => setModelOverrides((prev) => prev.filter((_, j) => j !== i))}
                      className="ml-auto"
                    >
                      <XCircle className="h-3.5 w-3.5 text-zinc-500 hover:text-red-400" />
                    </button>
                  </div>
                ))}
                <div className="mt-3 grid grid-cols-2 gap-2">
                  <input
                    placeholder="Model ID (required)"
                    value={overrideModelId}
                    onChange={(e) => setOverrideModelId(e.target.value)}
                    className="rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                  />
                  <input
                    placeholder="Context window"
                    type="number"
                    value={overrideContextWindow}
                    onChange={(e) => setOverrideContextWindow(e.target.value)}
                    className="rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                  />
                  <input
                    placeholder="Max output tokens"
                    type="number"
                    value={overrideMaxOutputTokens}
                    onChange={(e) => setOverrideMaxOutputTokens(e.target.value)}
                    className="rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                  />
                  <input
                    placeholder="Input cost / 1M tokens"
                    type="number"
                    value={overrideInputCost}
                    onChange={(e) => setOverrideInputCost(e.target.value)}
                    className="rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                  />
                  <input
                    placeholder="Output cost / 1M tokens"
                    type="number"
                    value={overrideOutputCost}
                    onChange={(e) => setOverrideOutputCost(e.target.value)}
                    className="rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                  />
                  <Select value={overrideProtocol || '__none__'} onValueChange={(v) => setOverrideProtocol(v === '__none__' ? '' : v)}>
                    <SelectTrigger className="border-zinc-700 bg-zinc-800 text-zinc-100">
                      <SelectValue placeholder="Provider default" />
                    </SelectTrigger>
                    <SelectContent className="bg-zinc-800 border-zinc-700">
                      <SelectItem value="__none__" className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">Provider default</SelectItem>
                      <SelectItem value="openai" className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">OpenAI-compatible</SelectItem>
                      <SelectItem value="anthropic" className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">Anthropic-compatible</SelectItem>
                      <SelectItem value="openai-codex" className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">OpenAI Codex</SelectItem>
                    </SelectContent>
                  </Select>
                  <button
                    type="button"
                    onClick={handleAddOverride}
                    className="rounded-md bg-zinc-800 px-3 py-2 text-sm font-medium text-zinc-300 transition-colors hover:bg-zinc-700"
                  >
                    Add Model
                  </button>
                </div>
              </div>
            )}
          </div>

          {error && (
            <div className="flex items-center gap-2 rounded-md bg-red-600/10 px-3 py-2 text-sm text-red-400">
              <AlertTriangle className="h-4 w-4 shrink-0" />
              {error}
            </div>
          )}

          <div className="mt-2 flex justify-end gap-3">
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
              {saving ? 'Saving…' : isEditing ? 'Save Changes' : 'Add Provider'}
            </button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
