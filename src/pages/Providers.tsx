import { useCallback, useEffect, useState } from 'react';
import {
  Plus,
  Edit2,
  Trash2,
  Zap,
  RefreshCw,
  ChevronDown,
  ChevronRight,
  Loader2,
  AlertTriangle,
  CheckCircle2,
} from 'lucide-react';
import { useStore } from '../store';
import { ActionButton } from '../components/ActionButton';
import { Toast } from '../components/Toast';
import {
  saveProvider,
  toggleProviderEnabled,
  deleteProvider,
  testProviderConnection,
  refreshProviderModels,
  getGroups,
} from '../lib/ipc';
import type { Provider, ProviderModel, Group } from '../types';
import type { TestConnectionResult } from '../lib/ipc';

function generateId(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
    .slice(0, 40);
}

function formatTimestamp(ts?: string): string {
  if (!ts) return 'Never';
  try {
    const d = new Date(ts);
    return d.toLocaleString();
  } catch {
    return ts;
  }
}

function formatCost(n?: number): string {
  if (n == null) return '—';
  return `$${n.toFixed(2)}`;
}

function formatNumber(n?: number): string {
  if (n == null) return '—';
  return n.toLocaleString();
}

export default function Providers() {
  const providers = useStore((s) => s.providers);
  const setProviders = useStore((s) => s.setProviders);
  const [expandedProviders, setExpandedProviders] = useState<Set<string>>(new Set());
  const [editingProvider, setEditingProvider] = useState<Provider | null>(null);
  const [showAddModal, setShowAddModal] = useState(false);
  const [testingId, setTestingId] = useState<string | null>(null);
  const [refreshingId, setRefreshingId] = useState<string | null>(null);
  const [toasts, setToasts] = useState<{ id: number; type: 'success' | 'error'; message: string }[]>([]);
  const [groups, setGroups] = useState<Group[]>([]);

  useEffect(() => {
    getGroups().then(setGroups).catch(() => {});
  }, []);

  const addToast = useCallback((type: 'success' | 'error', message: string) => {
    const id = Date.now();
    setToasts((prev) => [...prev, { id, type, message }]);
    setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), 4000);
  }, []);

  const toggleExpand = useCallback((id: string) => {
    setExpandedProviders((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

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

  const handleDelete = useCallback(
    async (provider: Provider) => {
      const groupsUsingProvider = groups.filter((g) =>
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
        const updated = providers.filter((p) => p.id !== provider.id);
        setProviders(updated);
        addToast('success', `Deleted provider "${provider.name}"`);
      } catch (e: unknown) {
        addToast('error', `Delete failed: ${e instanceof Error ? e.message : String(e)}`);
      }
    },
    [providers, groups, setProviders, addToast],
  );

  const handleSave = useCallback(
    async (provider: Provider, apiKey: string) => {
      await saveProvider(provider, apiKey);
      const updated = await (await import('../lib/ipc')).getProviders();
      setProviders(updated);
      setShowAddModal(false);
      setEditingProvider(null);
      addToast('success', `Saved provider "${provider.name}"`);
    },
    [setProviders, addToast],
  );

  const handleToggleEnabled = useCallback(
    async (provider: Provider) => {
      const newEnabled = !provider.enabled;
      await toggleProviderEnabled(provider.id, newEnabled);
      const allProviders = await (await import('../lib/ipc')).getProviders();
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
        <button
          onClick={() => {
            setEditingProvider(null);
            setShowAddModal(true);
          }}
          className="flex items-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-emerald-500"
        >
          <Plus className="h-4 w-4" />
          Add Provider
        </button>
      </div>

      {providers.length === 0 ? (
        <div className="flex flex-col items-center justify-center rounded-lg border border-dashed border-zinc-700 py-16 text-zinc-500">
          <Zap className="mb-3 h-10 w-10" />
          <p className="text-lg font-medium">No providers configured</p>
          <p className="mt-1 text-sm">Add your first upstream provider to get started.</p>
        </div>
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
  const protocolLabel = provider.protocol === 'anthropic' ? 'Anthropic-compatible' : 'OpenAI-compatible';
  const protocolColor = provider.protocol === 'anthropic' ? 'bg-violet-600/20 text-violet-300' : 'bg-emerald-600/20 text-emerald-300';

  const lastRefresh = provider.models[0]?.last_refreshed;
  const modelCount = provider.models.length;

  return (
    <div className={`rounded-lg border border-zinc-800 bg-zinc-900/60 transition-opacity ${!provider.enabled ? 'opacity-60' : ''}`}>
      <div className="flex items-start gap-4 p-5">
        <button onClick={onToggleExpand} className="mt-1 text-zinc-500 transition-colors hover:text-zinc-300">
          {isExpanded ? <ChevronDown className="h-5 w-5" /> : <ChevronRight className="h-5 w-5" />}
        </button>

        <div className="flex-1">
          <div className="flex items-center gap-3">
            <h3 className="text-base font-semibold">{provider.name}</h3>
            <span className={`rounded-full px-2.5 py-0.5 text-xs font-medium ${protocolColor}`}>
              {protocolLabel}
            </span>
            <span
              className={`flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ${
                provider.enabled ? 'bg-green-600/20 text-green-300' : 'bg-red-600/20 text-red-300'
              }`}
            >
              <span className={`h-1.5 w-1.5 rounded-full ${provider.enabled ? 'bg-green-400' : 'bg-red-400'}`} />
              {provider.enabled ? 'Enabled' : 'Disabled'}
            </span>
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
          </div>
        </div>

        <div className="flex flex-col items-end gap-2">
          <label className="relative inline-flex cursor-pointer items-center">
            <input
              type="checkbox"
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
        <button
          onClick={onDelete}
          className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium text-red-400 transition-colors hover:bg-red-600/10 hover:text-red-300"
        >
          <Trash2 className="h-3.5 w-3.5" />
          Delete
        </button>
      </div>

      {isExpanded && <ModelBrowser models={provider.models} providerName={provider.name} />}
    </div>
  );
}

function ModelBrowser({ models, providerName }: { models: ProviderModel[]; providerName: string }) {
  if (models.length === 0) {
    return (
      <div className="border-t border-zinc-800 px-5 py-6 text-center text-sm text-zinc-500">
        No models found. Click "Refresh Models" to fetch available models from {providerName}.
      </div>
    );
  }

  return (
    <div className="border-t border-zinc-800">
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-zinc-800 text-left text-xs uppercase tracking-wider text-zinc-500">
              <th className="px-5 py-3 font-medium">Model ID</th>
              <th className="px-5 py-3 font-medium">Context Window</th>
              <th className="px-5 py-3 font-medium">Max Output Tokens</th>
              <th className="px-5 py-3 font-medium">Input Cost/1M</th>
              <th className="px-5 py-3 font-medium">Output Cost/1M</th>
              <th className="px-5 py-3 font-medium">Last Refreshed</th>
              <th className="px-5 py-3 font-medium" />
            </tr>
          </thead>
          <tbody>
            {models.map((model) => (
              <tr key={model.id} className="border-b border-zinc-800/50 transition-colors hover:bg-zinc-800/30">
                <td className="px-5 py-3 font-mono text-xs">{model.id}</td>
                <td className="px-5 py-3 text-zinc-300">{formatNumber(model.context_window)}</td>
                <td className="px-5 py-3 text-zinc-300">{formatNumber(model.max_output_tokens)}</td>
                <td className="px-5 py-3 text-zinc-300">{formatCost(model.input_cost_per_1m)}</td>
                <td className="px-5 py-3 text-zinc-300">{formatCost(model.output_cost_per_1m)}</td>
                <td className="px-5 py-3 text-zinc-400">{formatTimestamp(model.last_refreshed)}</td>
                <td className="px-5 py-3">
                  <button
                    disabled
                    title="Coming soon: add this model to a group"
                    className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs font-medium text-zinc-500 transition-colors cursor-not-allowed"
                  >
                    Add to group
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

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
  const [name, setName] = useState(provider?.name ?? '');
  const [baseUrl, setBaseUrl] = useState(provider?.baseUrl ?? '');
  const [protocol, setProtocol] = useState(provider?.protocol ?? 'openai');
  const [apiKey, setApiKey] = useState('');
  const [showApiKey, setShowApiKey] = useState(false);
  const [dailyTokenQuota, setDailyTokenQuota] = useState(
    provider?.dailyTokenQuota != null ? String(provider.dailyTokenQuota) : '',
  );
  const [quotaResetUtcHour, setQuotaResetUtcHour] = useState(
    provider?.quotaResetUtcHour != null ? String(provider.quotaResetUtcHour) : '0',
  );
  const [enabled, setEnabled] = useState(provider?.enabled ?? true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
      setError('API key is required for new providers.');
      return;
    }
    if (dailyTokenQuota && isNaN(Number(dailyTokenQuota))) {
      setError('Daily token quota must be a number.');
      return;
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
        quotaResetUtcHour: hour,
        enabled,
        models: provider?.models ?? [],
      };
      await onSave(providerObj, apiKey);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        className="w-full max-w-lg rounded-lg border border-zinc-800 bg-zinc-900 p-6 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="mb-5 text-lg font-semibold">{isEditing ? 'Edit Provider' : 'Add Provider'}</h2>

        <form onSubmit={handleSubmit} className="flex flex-col gap-4">
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
              placeholder="https://api.example.com"
            />
          </div>

          <div>
            <label className="mb-1 block text-sm font-medium text-zinc-300">Protocol Type</label>
            <select
              value={protocol}
              onChange={(e) => setProtocol(e.target.value as 'openai' | 'anthropic')}
              className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
            >
              <option value="openai">OpenAI-compatible</option>
              <option value="anthropic">Anthropic-compatible</option>
            </select>
          </div>

          <div>
            <label className="mb-1 block text-sm font-medium text-zinc-300">
              API Key
              {isEditing && <span className="ml-2 text-xs text-zinc-500">(leave blank to keep existing)</span>}
            </label>
            <div className="relative">
              <input
                type={showApiKey ? 'text' : 'password'}
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 pr-16 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                placeholder={isEditing ? '••••••••' : 'sk-...'}
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

          <div className="grid grid-cols-2 gap-4">
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
      </div>
    </div>
  );
}
