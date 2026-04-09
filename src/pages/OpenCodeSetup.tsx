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
import {
  getOpencodeConfigPath,
  setOpencodeConfigPath,
  injectOpencodeProvider,
  removeOpencodeProvider,
  setOpencodeAgentModels,
  removeOpencodeAgentModels,
  removeCoderouterFromOpencode,
  previewOpencodeConfig,
  type OpenCodeAgentMapping,
} from '../lib/ipc';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';

const AGENT_LABELS: Record<string, string> = {
  build: 'Build agent',
  plan: 'Plan agent',
  general: 'General subagent',
  explore: 'Explore subagent',
  compaction: 'Compaction (system)',
  title: 'Title (system)',
  summary: 'Summary (system)',
};

const AGENT_KEYS = ['build', 'plan', 'general', 'explore'] as const;

function emptyMapping(): OpenCodeAgentMapping {
  return { build: null, plan: null, general: null, explore: null, compaction: null, title: null, summary: null, small_model: null };
}

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
    };
  }, [mapping]);

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

  // Debounced preview update
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      fetchPreview();
    }, 500);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [mapping, proxyPort, fetchPreview]);

  // Check if provider is already enabled on mount
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

  const handleMappingChange = useCallback((key: keyof OpenCodeAgentMapping, value: string) => {
    setMapping((prev) => ({ ...prev, [key]: value || null }));
  }, []);

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

function SectionCard({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900/60 p-5">{children}</div>
  );
}
