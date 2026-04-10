import { useCallback, useEffect, useRef, useState } from 'react';
import { AlertTriangle, RotateCcw, Trash2, Eye, RotateCw } from 'lucide-react';
import { useStore } from '../store';
import { Toast } from '../components/Toast';
import { Card, CardHeader, CardTitle, CardContent, CardDescription } from '../components/ui/card';
import { Button } from '../components/ui/button';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';
import { getAppConfig, saveAppConfig, clearMetricsData, resetAllConfig, restartProxy, getAppVersion } from '../lib/ipc';
import type { AppConfig } from '../types';

const REFRESH_INTERVAL_OPTIONS: { label: string; value: number }[] = [
  { label: 'Every 12 hours', value: 12 },
  { label: 'Every 24 hours', value: 24 },
  { label: 'Every 7 days', value: 168 },
  { label: 'Manual only', value: 0 },
];

const LOG_VERBOSITY_OPTIONS = ['Error', 'Info', 'Debug'];

export default function Settings() {
  const setAppConfig = useStore((s) => s.setAppConfig);
  const [form, setForm] = useState<AppConfig | null>(null);
  const [showRestartBanner, setShowRestartBanner] = useState(false);
  const [originalPort, setOriginalPort] = useState<number | null>(null);
  const [originalHost, setOriginalHost] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [toasts, setToasts] = useState<{ id: number; type: 'success' | 'error'; message: string }[]>([]);
  const toastCounterRef = useRef(0);
  const [loading, setLoading] = useState(true);
  const [appVersion, setAppVersion] = useState<string>('');

  const addToast = useCallback((type: 'success' | 'error', message: string) => {
    const id = Date.now() * 1000 + (++toastCounterRef.current);
    setToasts((prev) => [...prev, { id, type, message }]);
    setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), 4000);
  }, []);

  useEffect(() => {
    const load = async () => {
      try {
        const config = await getAppConfig();
        setForm(config);
        setOriginalPort(config.proxy_port);
        setOriginalHost(config.proxy_host);
      } catch {
        addToast('error', 'Failed to load settings');
      } finally {
        setLoading(false);
      }
      try {
        const version = await getAppVersion();
        setAppVersion(version);
      } catch {}
    };
    load();
  }, [addToast]);

  const updateField = <K extends keyof AppConfig>(key: K, value: AppConfig[K]) => {
    if (!form) return;
    setForm({ ...form, [key]: value });
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!form) return;

    const port = parseInt(String(form.proxy_port), 10);
    if (isNaN(port) || port < 1024 || port > 65535) {
      addToast('error', 'Port must be an integer between 1024 and 65535');
      return;
    }

    setSaving(true);
    try {
      await saveAppConfig(form);
      setAppConfig(form);

      const portChanged = originalPort !== form.proxy_port;
      const hostChanged = originalHost !== form.proxy_host;

      if (portChanged || hostChanged) {
        setOriginalPort(form.proxy_port);
        setOriginalHost(form.proxy_host);
        setShowRestartBanner(true);
      }

      addToast('success', 'Settings saved');
    } catch {
      addToast('error', 'Failed to save settings');
    } finally {
      setSaving(false);
    }
  };

  const handleViewLogs = async () => {
    const { open } = await import('@tauri-apps/plugin-shell');
    const { homeDir, join } = await import('@tauri-apps/api/path');
    const home = await homeDir();
    const logPath = await join(home, '.local', 'share', 'coderouter', 'proxy.log');
    try {
      await open(logPath);
    } catch {
      addToast('error', 'Failed to open log file');
    }
  };

  const handleResetUsageData = async () => {
    if (!confirm('Are you sure you want to reset all usage data? This will permanently delete all request metrics.')) {
      return;
    }
    try {
      await clearMetricsData();
      addToast('success', 'Usage data cleared');
    } catch {
      addToast('error', 'Failed to clear usage data');
    }
  };

  const handleResetSettings = async () => {
    if (!confirm('Are you sure you want to reset all settings? This will clear providers, groups, and app config. OpenCode config will NOT be affected.')) {
      return;
    }
    try {
      await resetAllConfig();
      const store = useStore.getState();
      store.resetAll();
      const config = await getAppConfig();
      setForm(config);
      setAppConfig(config);
      setOriginalPort(config.proxy_port);
      setOriginalHost(config.proxy_host);
      setShowRestartBanner(false);
      addToast('success', 'Settings reset to defaults');
    } catch {
      addToast('error', 'Failed to reset settings');
    }
  };

  const handleRestartProxy = async () => {
    setRestarting(true);
    try {
      await restartProxy();
      setShowRestartBanner(false);
      addToast('success', 'Proxy restarted');
    } catch {
      addToast('error', 'Failed to restart proxy');
    } finally {
      setRestarting(false);
    }
  };

  if (loading || !form) {
    return (
      <div className="mx-auto max-w-3xl">
        <div className="flex items-center justify-center py-12">
          <p className="text-zinc-400">Loading settings...</p>
        </div>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-3xl space-y-8">
      {toasts.length > 0 && (
        <div className="fixed right-4 top-4 z-50 space-y-2">
          {toasts.map((t) => (
            <Toast key={t.id} type={t.type} message={t.message} />
          ))}
        </div>
      )}

      {showRestartBanner && (
        <Card className="border-yellow-600/30 bg-yellow-600/10">
          <CardContent className="flex items-center justify-between pt-6">
            <div className="flex items-center gap-3">
              <AlertTriangle className="h-5 w-5 text-yellow-400" />
              <p className="text-sm text-yellow-200">Proxy restart required. Restart now?</p>
            </div>
            <Button variant="outline" onClick={handleRestartProxy} disabled={restarting}>
              <RotateCw className={`h-4 w-4 ${restarting ? 'animate-spin' : ''}`} />
              {restarting ? 'Restarting...' : 'Restart Proxy'}
            </Button>
          </CardContent>
        </Card>
      )}

      <form onSubmit={handleSave} className="space-y-8">
        <Card>
          <CardHeader>
            <CardTitle>Proxy Settings</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div>
              <label className="mb-1 block text-sm font-medium text-zinc-300">Port</label>
              <input
                type="number"
                min={1024}
                max={65535}
                value={form.proxy_port}
                onChange={(e) => updateField('proxy_port', parseInt(e.target.value, 10) || 0)}
                className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 focus:border-zinc-500 focus:outline-none"
              />
              <p className="mt-1 text-xs text-zinc-500">Range: 1024–65535 (default: 4141)</p>
            </div>
            <div>
              <label className="mb-1 block text-sm font-medium text-zinc-300">Listen Address</label>
              <input
                type="text"
                value={form.proxy_host}
                onChange={(e) => updateField('proxy_host', e.target.value)}
                className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 focus:border-zinc-500 focus:outline-none"
              />
              {form.proxy_host === '0.0.0.0' && (
                <div className="mt-1 flex items-center gap-1 text-xs text-yellow-400">
                  <AlertTriangle className="h-3 w-3" />
                  Listening on all interfaces exposes the proxy to your local network.
                </div>
              )}
              <p className="mt-1 text-xs text-zinc-500">Default: 127.0.0.1</p>
            </div>
            <p className="text-xs text-zinc-500">Restart the proxy after changing port or address for changes to take effect.</p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Model Refresh</CardTitle>
          </CardHeader>
          <CardContent>
            <label className="mb-1 block text-sm font-medium text-zinc-300">Auto-refresh interval</label>
            <Select value={String(form.refresh_interval_hours)} onValueChange={(v) => updateField('refresh_interval_hours', Number(v))}>
              <SelectTrigger className="w-full border-zinc-700 bg-zinc-800 text-zinc-100">
                <SelectValue />
              </SelectTrigger>
              <SelectContent className="bg-zinc-800 border-zinc-700">
                {REFRESH_INTERVAL_OPTIONS.map((opt) => (
                  <SelectItem key={opt.value} value={String(opt.value)} className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">
                    {opt.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Logging</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div>
              <label className="mb-1 block text-sm font-medium text-zinc-300">Log verbosity</label>
              <Select value={form.log_verbosity} onValueChange={(v) => updateField('log_verbosity', v as 'Error' | 'Info' | 'Debug')}>
                <SelectTrigger className="w-full border-zinc-700 bg-zinc-800 text-zinc-100">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent className="bg-zinc-800 border-zinc-700">
                  {LOG_VERBOSITY_OPTIONS.map((v) => (
                    <SelectItem key={v} value={v} className="text-zinc-100 focus:bg-zinc-700 focus:text-zinc-100">
                      {v}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <Button variant="outline" type="button" onClick={handleViewLogs}>
              <Eye className="h-4 w-4" />
              View Logs
            </Button>
          </CardContent>
        </Card>

        <div className="flex justify-end">
          <Button type="submit" disabled={saving}>
            {saving ? 'Saving...' : 'Save Settings'}
          </Button>
        </div>

        <Card className="border-red-800/50">
          <CardHeader>
            <CardTitle className="text-red-400">Danger Zone</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex items-center justify-between rounded-md border border-zinc-800 bg-zinc-900 px-4 py-3">
              <div>
                <p className="text-sm font-medium text-zinc-200">Reset All Usage Data</p>
                <CardDescription>Permanently delete all request metrics and usage data.</CardDescription>
              </div>
              <Button variant="destructive" type="button" onClick={handleResetUsageData}>
                <Trash2 className="h-4 w-4" />
                Reset Usage Data
              </Button>
            </div>
            <div className="flex items-center justify-between rounded-md border border-zinc-800 bg-zinc-900 px-4 py-3">
              <div>
                <p className="text-sm font-medium text-zinc-200">Reset All Settings</p>
                <CardDescription>Restore config.json, providers.json, and groups.json to empty defaults. Does NOT affect OpenCode config.</CardDescription>
              </div>
              <Button variant="destructive" type="button" onClick={handleResetSettings}>
                <RotateCcw className="h-4 w-4" />
                Reset Settings
              </Button>
            </div>
          </CardContent>
        </Card>
      </form>

      {appVersion && (
        <p className="pt-4 text-center text-xs text-zinc-500">
          CodeRouter v{appVersion}
        </p>
      )}
    </div>
  );
}
