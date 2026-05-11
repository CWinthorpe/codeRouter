import { useCallback, useEffect, useRef, useState } from 'react';
import {
  ChevronDown,
  ChevronUp,
  CheckCircle2,
  AlertTriangle,
  XCircle,
  Loader2,
  RefreshCw,
  Eye,
  Code2,
} from 'lucide-react';
import { useStore } from '../store';
import { Toast } from '../components/Toast';
import CustomAgentsManager from '../components/CustomAgentsManager';
import {
  getOpencodeConfigPath,
  setOpencodeConfigPath,
  injectOpencodeProvider,
  removeOpencodeProvider,
  setOpencodeAgentModels,
  removeOpencodeAgentModels,
  removeCoderouterFromOpencode,
  previewOpencodeConfig,
  getOpencodeAgentModels,
  type OpenCodeAgentMapping,
} from '../lib/ipc';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';

/** Maps internal agent keys to human-readable labels for the UI. */
const AGENT_LABELS: Record<string, string> = {
  build: 'Build agent',
  plan: 'Plan agent',
  general: 'General subagent',
  explore: 'Explore subagent',
  compaction: 'Compaction (system)',
  title: 'Title (system)',
  summary: 'Summary (system)',
};

/** User-facing agent keys that should have dropdowns in the UI. */
const AGENT_KEYS = ['build', 'plan', 'general', 'explore'] as const;

const REASONING_OPTIONS = [
  { value: '__none__', label: '— default —' },
  { value: 'none', label: 'None' },
  { value: 'low', label: 'Low' },
  { value: 'medium', label: 'Medium' },
  { value: 'high', label: 'High' },
  { value: 'xhigh', label: 'Extra High' },
  { value: 'max', label: 'Max' },
] as const;

/** Creates a blank agent mapping object with all fields set to null. */
function emptyMapping(): OpenCodeAgentMapping {
  return { build: null, plan: null, general: null, explore: null, compaction: null, title: null, summary: null, small_model: null, reasoning_efforts: undefined };
}

/**
 * OpenCode setup page. Allows users to:
 * - Detect or manually set the OpenCode config path
 * - Toggle CodeRouter as a provider in OpenCode config
 * - Assign model groups to OpenCode agents (build, plan, etc.)
 * - Preview and apply the resulting JSON configuration
 */
export default function OpenCodeSetup() {
  const groups = useStore((s) => s.groups);
  const appConfig = useStore((s) => s.appConfig);
  const proxyPort = appConfig?.proxy_port ?? 4141;

  const [configPath, setConfigPath] = useState<string | null>(null);
  const [manualPath, setManualPath] = useState('');
  const [pathDetected, setPathDetected] = useState(false);

  const [providerEnabled, setProviderEnabled] = useState(false);
  const [togglingProvider, setTogglingProvider] = useState(false);

  const [mapping, setMapping] = useState<OpenCodeAgentMapping>(emptyMapping());
  const [applyingMapping, setApplyingMapping] = useState(false);
  const [clearingMapping, setClearingMapping] = useState(false);

  const [previewOpen, setPreviewOpen] = useState(false);
  const [previewJson, setPreviewJson] = useState<string>('');
  const [previewLoading, setPreviewLoading] = useState(false);

  const [toasts, setToasts] = useState<{ id: number; type: 'success' | 'error'; message: string }[]>([]);
  const toastCounterRef = useRef(0);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [removing, setRemoving] = useState(false);

  const addToast = useCallback((type: 'success' | 'error', message: string) => {
    const id = Date.now() * 1000 + (++toastCounterRef.current);
    setToasts((prev) => [...prev, { id, type, message }]);
    setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), 4000);
  }, []);

  // Detect config path on mount
  useEffect(() => {
    detectPath();
  }, []);

  /** Tries to auto-detect the OpenCode config path via IPC and updates state. */
  const detectPath = useCallback(async () => {
    try {
      const path = await getOpencodeConfigPath();
      setConfigPath(path);
      setPathDetected(!!path);
      if (path) setManualPath(path);
    } catch {
      setConfigPath(null);
      setPathDetected(false);
    }
  }, []);

  // Build mapping object for preview (convert empty strings to null)
  // so the preview reflects what will actually be written to config.
  const mappingForPreview = useCallback((): OpenCodeAgentMapping => {
    return {
      build: mapping.build || null,
      plan: mapping.plan || null,
      general: mapping.general || null,
      explore: mapping.explore || null,
      compaction: mapping.compaction || null,
      title: mapping.title || null,
      summary: mapping.summary || null,
      small_model: mapping.small_model || null,
      reasoning_efforts: mapping.reasoning_efforts,
    };
  }, [mapping]);

  /** Fetches a JSON preview of the OpenCode config with current settings applied. */
  const fetchPreview = useCallback(async () => {
    setPreviewLoading(true);
    try {
      const m = mappingForPreview();
      const hasAnyMapping = m.build || m.plan || m.general || m.explore || m.compaction || m.title || m.summary || m.small_model;
      const json = await previewOpencodeConfig(proxyPort, hasAnyMapping ? m : null);
      setPreviewJson(json);
    } catch {
      setPreviewJson('Failed to load preview');
    } finally {
      setPreviewLoading(false);
    }
  }, [mappingForPreview, proxyPort]);

  // Debounced preview update: waits 500ms after the last mapping/port
  // change before fetching the preview to avoid excessive IPC calls.
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      fetchPreview();
    }, 500);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [mapping, proxyPort, fetchPreview]);

  // Check if CodeRouter is already configured as a provider in the
  // OpenCode config by parsing the preview JSON on mount.
  useEffect(() => {
    const checkProviderStatus = async () => {
      try {
        const json = await previewOpencodeConfig(proxyPort, null);
        const parsed = JSON.parse(json);
        setProviderEnabled(!!parsed?.provider?.coderouter);
      } catch {
        // IPC may fail outside Tauri
      }
    };
    checkProviderStatus();
  }, [proxyPort]);

  // Load the current agent→model assignments from OpenCode config on mount
  // so the dropdowns show the persisted values.
  useEffect(() => {
    const loadAgentModels = async () => {
      try {
        const current = await getOpencodeAgentModels();
        setMapping((prev) => ({
          build: current.build ?? prev.build,
          plan: current.plan ?? prev.plan,
          general: current.general ?? prev.general,
          explore: current.explore ?? prev.explore,
          compaction: current.compaction ?? prev.compaction,
          title: current.title ?? prev.title,
          summary: current.summary ?? prev.summary,
          small_model: current.small_model ?? prev.small_model,
          reasoning_efforts: current.reasoning_efforts ?? prev.reasoning_efforts,
        }));
      } catch {
        // IPC may fail outside Tauri or config may not exist yet
      }
    };
    loadAgentModels();
  }, []);

  /** Toggles CodeRouter as a provider in the OpenCode config (inject or remove). */
  const handleToggleProvider = useCallback(async () => {
    setTogglingProvider(true);
    try {
      if (providerEnabled) {
        await removeOpencodeProvider();
        setProviderEnabled(false);
        addToast('success', 'CodeRouter removed from OpenCode config');
      } else {
        await injectOpencodeProvider(proxyPort);
        setProviderEnabled(true);
        addToast('success', 'CodeRouter added to OpenCode config');
      }
    } catch (e: unknown) {
      addToast('error', e instanceof Error ? e.message : String(e));
    } finally {
      setTogglingProvider(false);
    }
  }, [providerEnabled, proxyPort, addToast]);

  /** Updates a single agent→group mapping key, converting empty strings to null. */
  const handleMappingChange = useCallback((key: keyof OpenCodeAgentMapping, value: string) => {
    setMapping((prev) => ({ ...prev, [key]: value || null }));
  }, []);

  const handleReasoningChange = useCallback((agentKey: string, value: string) => {
    setMapping((prev) => {
      const efforts = { ...(prev.reasoning_efforts ?? {}) };
      if (value === '__none__') {
        delete efforts[agentKey];
      } else {
        efforts[agentKey] = value;
      }
      return { ...prev, reasoning_efforts: Object.keys(efforts).length > 0 ? efforts : undefined };
    });
  }, []);

  /** Persists the current agent mapping to the OpenCode config file via IPC. */
  const handleApplyMapping = useCallback(async () => {
    setApplyingMapping(true);
    try {
      const m = mappingForPreview();
      await setOpencodeAgentModels(m);
      addToast('success', 'Agent mapping applied');
    } catch (e: unknown) {
      addToast('error', e instanceof Error ? e.message : String(e));
    } finally {
      setApplyingMapping(false);
    }
  }, [mappingForPreview, addToast]);

  /** Removes all agent model assignments from the OpenCode config and resets the form. */
  const handleClearMapping = useCallback(async () => {
    setClearingMapping(true);
    try {
      await removeOpencodeAgentModels();
      setMapping(emptyMapping());
      addToast('success', 'Agent mapping cleared');
    } catch (e: unknown) {
      addToast('error', e instanceof Error ? e.message : String(e));
    } finally {
      setClearingMapping(false);
    }
  }, [addToast]);

  const groupAliases = groups.map((g) => g.alias);

  return (
    <div className="max-w-3xl">
      <div className="mb-6">
        <h1 className="text-2xl font-semibold">OpenCode Setup</h1>
        <p className="mt-1 text-sm text-zinc-400">
          Configure CodeRouter as a provider in OpenCode and assign model groups to agents.
        </p>
      </div>

      <div className="flex flex-col gap-6">
        {/* Config path section */}
        <SectionCard>
          <h2 className="text-base font-semibold text-zinc-100">OpenCode Config Path</h2>
          <div className="mt-3 flex flex-col gap-3">
            {pathDetected && configPath ? (
              <div className="flex items-center gap-2 text-sm text-zinc-300">
                <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-400" />
                <code className="rounded bg-zinc-800 px-2 py-1 text-xs">{configPath}</code>
              </div>
            ) : (
              <div className="flex items-center gap-2 rounded-md bg-amber-600/10 px-3 py-2 text-sm text-amber-300">
                <AlertTriangle className="h-4 w-4 shrink-0" />
                OpenCode config not found at the default location.
              </div>
            )}

            <div className="flex items-center gap-2">
              <input
                type="text"
                value={manualPath}
                onChange={(e) => setManualPath(e.target.value)}
                onBlur={async () => {
                  if (manualPath && manualPath !== configPath) {
                    try {
                      await setOpencodeConfigPath(manualPath);
                      setConfigPath(manualPath);
                      addToast('success', 'Config path saved');
                    } catch {
                      addToast('error', 'Failed to save config path');
                    }
                  }
                }}
                placeholder="~/.config/opencode/opencode.json"
                className="flex-1 rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
              />
              <button
                onClick={detectPath}
                className="flex items-center gap-1.5 rounded-md bg-zinc-800 px-3 py-2 text-sm font-medium text-zinc-300 transition-colors hover:bg-zinc-700"
              >
                <RefreshCw className="h-4 w-4" />
                Detect Again
              </button>
            </div>
          </div>
        </SectionCard>

        {/* Provider setup section */}
        <SectionCard>
          <h2 className="text-base font-semibold text-zinc-100">CodeRouter as OpenCode Provider</h2>
          <div className="mt-3 flex items-center justify-between">
            <div className="flex items-center gap-3">
              <label className="relative inline-flex cursor-pointer items-center">
                <input
                  type="checkbox"
                  checked={providerEnabled}
                  onChange={handleToggleProvider}
                  disabled={togglingProvider}
                  className="peer sr-only"
                />
                <div className="peer h-5 w-9 rounded-full bg-zinc-700 after:absolute after:start-[2px] after:top-[2px] after:h-4 after:w-4 after:rounded-full after:border after:border-zinc-600 after:bg-zinc-400 after:transition-all peer-checked:bg-emerald-600 peer-checked:after:translate-x-full peer-checked:after:border-white peer-focus:outline-none peer-disabled:opacity-50" />
              </label>
              <span className="text-sm text-zinc-300">Enable CodeRouter in OpenCode config</span>
              {togglingProvider && <Loader2 className="h-4 w-4 animate-spin text-zinc-400" />}
            </div>
            <span
              className={`flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ${
                providerEnabled
                  ? 'bg-green-600/20 text-green-300'
                  : 'bg-zinc-700/60 text-zinc-400'
              }`}
            >
              {providerEnabled ? (
                <CheckCircle2 className="h-3.5 w-3.5" />
              ) : (
                <XCircle className="h-3.5 w-3.5" />
              )}
              {providerEnabled ? 'Configured' : 'Not configured'}
            </span>
          </div>

          <div className="mt-4 flex items-center gap-3">
            <button
              onClick={async () => {
                setRemoving(true);
                try {
                  await removeCoderouterFromOpencode();
                  setProviderEnabled(false);
                  addToast('success', 'CodeRouter removed from OpenCode config');
                } catch (e: unknown) {
                  addToast('error', e instanceof Error ? e.message : String(e));
                } finally {
                  setRemoving(false);
                }
              }}
              disabled={removing}
              className="flex items-center gap-2 rounded-md border border-red-800 bg-red-900/30 px-4 py-2 text-sm font-medium text-red-300 transition-colors hover:bg-red-900/50 disabled:opacity-50"
            >
              {removing && <Loader2 className="h-4 w-4 animate-spin" />}
              Remove CodeRouter from OpenCode config
            </button>
          </div>
        </SectionCard>

        {/* Agent mapping section */}
        <SectionCard>
          <h2 className="text-base font-semibold text-zinc-100">Agent Model Assignments (Optional)</h2>
          <p className="mt-1 text-sm text-zinc-400">
            Assign specific model groups to OpenCode agents. Leave blank to use OpenCode&apos;s default.
          </p>

          <div className="mt-4 flex flex-col gap-4">
            {AGENT_KEYS.map((key) => (
              <div key={key} className="flex items-center gap-3">
                <label className="w-40 shrink-0 text-sm text-zinc-300">{AGENT_LABELS[key]}</label>
                <Select value={mapping[key] ?? ''} onValueChange={(v) => handleMappingChange(key, v === '__none__' ? '' : v)}>
                  <SelectTrigger className="flex-1 border-zinc-700 bg-zinc-800 text-zinc-100">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent className="bg-zinc-800 border-zinc-700">
                    <SelectItem value="__none__" className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">— use default —</SelectItem>
                    {groupAliases.map((alias) => (
                      <SelectItem key={alias} value={alias} className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">
                        {alias}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Select value={mapping.reasoning_efforts?.[key] ?? '__none__'} onValueChange={(v) => handleReasoningChange(key, v)}>
                  <SelectTrigger className="w-36 border-zinc-700 bg-zinc-800 text-zinc-100">
                    <SelectValue placeholder="Reasoning" />
                  </SelectTrigger>
                  <SelectContent className="bg-zinc-800 border-zinc-700">
                    {REASONING_OPTIONS.map((opt) => (
                      <SelectItem key={opt.value} value={opt.value} className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">
                        {opt.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            ))}

            <div className="flex items-center gap-3">
              <label className="w-40 shrink-0 text-sm text-zinc-300">
                Small/Fast model
                <span className="ml-1 text-zinc-500">(titles, summaries)</span>
              </label>
              <Select value={mapping.small_model ?? ''} onValueChange={(v) => handleMappingChange('small_model', v === '__none__' ? '' : v)}>
                <SelectTrigger className="flex-1 border-zinc-700 bg-zinc-800 text-zinc-100">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent className="bg-zinc-800 border-zinc-700">
                  <SelectItem value="__none__" className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">— use default —</SelectItem>
                  {groupAliases.map((alias) => (
                    <SelectItem key={alias} value={alias} className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">
                      {alias}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          <div className="mt-5 flex items-center gap-3">
            <button
              onClick={handleApplyMapping}
              disabled={applyingMapping || groupAliases.length === 0}
              className="flex items-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-emerald-500 disabled:opacity-50"
            >
              {applyingMapping && <Loader2 className="h-4 w-4 animate-spin" />}
              Apply Agent Mapping
            </button>
            <button
              onClick={handleClearMapping}
              disabled={clearingMapping}
              className="flex items-center gap-2 rounded-md bg-zinc-800 px-4 py-2 text-sm font-medium text-zinc-300 transition-colors hover:bg-zinc-700 disabled:opacity-50"
            >
              Clear Agent Mapping
            </button>
          </div>
        </SectionCard>

        {/* Custom Agents section */}
        <SectionCard>
          <CustomAgentsManager />
        </SectionCard>

        {/* Config preview panel */}
        <SectionCard>
          <button
            onClick={() => setPreviewOpen(!previewOpen)}
            className="flex w-full items-center justify-between text-base font-semibold text-zinc-100 transition-colors hover:text-zinc-50"
          >
            <span className="flex items-center gap-2">
              <Eye className="h-4 w-4" />
              Preview changes
            </span>
            {previewOpen ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
          </button>

          {previewOpen && (
            <div className="mt-3">
              {previewLoading ? (
                <div className="flex items-center gap-2 rounded-md bg-zinc-800/50 px-4 py-6 text-sm text-zinc-400">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  Loading preview…
                </div>
              ) : (
                <div className="relative rounded-md border border-zinc-800 bg-zinc-950">
                  <div className="absolute right-3 top-3 flex items-center gap-1.5 text-xs text-zinc-500">
                    <Code2 className="h-3.5 w-3.5" />
                    JSON
                  </div>
                  <pre className="overflow-auto p-4 pr-16 text-xs leading-relaxed text-zinc-300">
                    {previewJson}
                  </pre>
                </div>
              )}
            </div>
          )}
        </SectionCard>
      </div>

      {/* Toasts */}
      <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2">
        {toasts.map((toast) => (
          <Toast key={toast.id} type={toast.type} message={toast.message} />
        ))}
      </div>
    </div>
  );
}

/** Simple section card wrapper providing consistent styling for each config section. */
function SectionCard({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900/60 p-5">{children}</div>
  );
}
