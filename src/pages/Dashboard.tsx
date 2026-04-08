import React, { useEffect, useState, useRef } from 'react';
import { useNavigate } from 'react-router-dom';
import { useStore } from '../store';
import { getDailySummary, getRecentRequests } from '../lib/ipc';
import { useGroupStatusPoll } from '../hooks/useGroupStatusPoll';
import type { DailySummary, RequestRow, Provider, EntryStatusResponse } from '../types';
import { Server, Power, Terminal, ChevronDown, ChevronRight, AlertTriangle, CheckCircle, MinusCircle } from 'lucide-react';

const POLL_INTERVAL_MS = 5000;

interface HealthData {
  status: string;
  uptime_seconds: number;
}

function useHealthPoll(proxyStatus: string, proxyPort: number, proxyHost?: string) {
  const [health, setHealth] = useState<HealthData | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const healthUrl = `http://${proxyHost ?? 'localhost'}:${proxyPort}/health`;

  useEffect(() => {
    if (proxyStatus !== 'running') {
      setHealth(null);
      return;
    }

    const poll = async () => {
      try {
        const res = await fetch(healthUrl, { signal: AbortSignal.timeout(3000) });
        if (res.ok) {
          const data = await res.json();
          setHealth(data);
        } else {
          setHealth(null);
        }
      } catch {
        setHealth(null);
      }
    };

    poll();
    intervalRef.current = setInterval(poll, POLL_INTERVAL_MS);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [proxyStatus, healthUrl]);

  return health;
}

function formatUptime(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
  return `${Math.floor(seconds / 86400)}d ${Math.floor((seconds % 86400) / 3600)}h`;
}

function formatRelativeTime(ts: number): string {
  const now = Math.floor(Date.now() / 1000);
  const diff = now - ts;
  if (diff < 0) return 'just now';
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

function formatLatency(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return `${n}`;
}

function getEntryStatusCounts(entries: EntryStatusResponse[], providerId: string): { active: number; cooldown: number; disabled: number } {
  const filtered = entries.filter((e) => e.provider_id === providerId);
  return {
    active: filtered.filter((e) => e.status === 'active').length,
    cooldown: filtered.filter((e) => e.status === 'cooldown').length,
    disabled: filtered.filter((e) => e.status === 'manually_disabled').length,
  };
}

function getProviderOverallStatus(
  provider: Provider,
  entryCounts: { active: number; cooldown: number; disabled: number },
): string {
  if (!provider.enabled) return 'Disabled';
  if (entryCounts.active === 0 && entryCounts.cooldown === 0) return 'All Entries Exhausted';
  if (entryCounts.cooldown > 0) return 'Partially Degraded';
  return 'Active';
}

function getProviderCardSortKey(
  provider: Provider,
  entryCounts: { active: number; cooldown: number; disabled: number },
): number {
  if (!provider.enabled) return 2;
  if (entryCounts.cooldown > 0 || (entryCounts.active === 0 && entryCounts.cooldown === 0)) return 0;
  return 1;
}

function getProviderCardSortSubKey(
  provider: Provider,
  entryCounts: { active: number; cooldown: number; disabled: number },
): number {
  if (!provider.enabled) return 2;
  if (entryCounts.active === 0 && entryCounts.cooldown === 0) return 0;
  if (entryCounts.cooldown > 0) return 1;
  return 2;
}

function StatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    success: 'bg-green-500/20 text-green-400 border-green-500/30',
    failover: 'bg-yellow-500/20 text-yellow-400 border-yellow-500/30',
    error: 'bg-red-500/20 text-red-400 border-red-500/30',
    timeout: 'bg-zinc-500/20 text-zinc-400 border-zinc-500/30',
  };
  const color = colors[status] ?? colors.error;
  return (
    <span className={`inline-flex items-center rounded border px-2 py-0.5 text-xs font-medium capitalize ${color}`}>
      {status}
    </span>
  );
}

function ProxyStatusCard() {
  const { proxyStatus, appConfig } = useStore((s) => ({ proxyStatus: s.proxyStatus, appConfig: s.appConfig }));
  const health = useHealthPoll(proxyStatus, appConfig?.proxy_port ?? 4141, appConfig?.proxy_host);
  const navigate = useNavigate();

  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900 p-6">
      <div className="flex items-start justify-between">
        <div>
          <div className="flex items-center gap-3">
            <Server className="h-5 w-5 text-zinc-400" />
            <h2 className="text-lg font-semibold">Proxy Status</h2>
          </div>
          <div className="mt-4 flex items-center gap-4">
            <span
              className={`inline-flex items-center gap-2 rounded-full px-3 py-1 text-sm font-medium ${
                proxyStatus === 'running'
                  ? 'bg-green-500/20 text-green-400'
                  : 'bg-red-500/20 text-red-400'
              }`}
            >
              <Power className="h-4 w-4" />
              {proxyStatus === 'running' ? 'Running' : 'Stopped'}
            </span>
          </div>
          <div className="mt-3 space-y-1 text-sm text-zinc-400">
            {appConfig && (
              <>
                <p>
                  Listening on <span className="text-zinc-200">{appConfig.proxy_host}:{appConfig.proxy_port}</span>
                </p>
                {health && health.uptime_seconds > 0 && (
                  <p>
                    Uptime: <span className="text-zinc-200">{formatUptime(health.uptime_seconds)}</span>
                  </p>
                )}
              </>
            )}
          </div>
        </div>
        <button
          onClick={() => navigate('/opencode')}
          className="inline-flex items-center gap-2 rounded-md bg-zinc-800 px-4 py-2 text-sm font-medium text-zinc-200 transition-colors hover:bg-zinc-700"
        >
          <Terminal className="h-4 w-4" />
          Configure OpenCode
        </button>
      </div>
    </div>
  );
}

function ProviderHealthCard({
  provider,
  entryCounts,
  summary,
}: {
  provider: Provider;
  entryCounts: { active: number; cooldown: number; disabled: number };
  summary: DailySummary | null;
}) {
  const overallStatus = getProviderOverallStatus(provider, entryCounts);
  const quota = provider.dailyTokenQuota ?? null;
  const totalTokensToday = summary ? summary.total_prompt_tokens + summary.total_output_tokens : 0;
  const progressPct = quota && quota > 0 ? Math.min((totalTokensToday / quota) * 100, 100) : 0;

  const statusColor =
    overallStatus === 'Active'
      ? 'text-green-400'
      : overallStatus === 'Partially Degraded'
        ? 'text-yellow-400'
        : overallStatus === 'All Entries Exhausted'
          ? 'text-red-400'
          : 'text-zinc-500';

  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900 p-5">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <h3 className="font-medium">{provider.name}</h3>
          <span className="rounded bg-zinc-800 px-2 py-0.5 text-xs text-zinc-400">{provider.protocol}</span>
        </div>
        <span className={`text-sm font-medium ${statusColor}`}>{overallStatus}</span>
      </div>

      <div className="mt-3 flex gap-4 text-xs text-zinc-400">
        <span>
          <CheckCircle className="mr-1 inline h-3 w-3 text-green-500" />
          {entryCounts.active} Active
        </span>
        <span>
          <AlertTriangle className="mr-1 inline h-3 w-3 text-yellow-500" />
          {entryCounts.cooldown} Cooldown
        </span>
        <span>
          <MinusCircle className="mr-1 inline h-3 w-3 text-zinc-500" />
          {entryCounts.disabled} Disabled
        </span>
      </div>

      {quota && quota > 0 && (
        <div className="mt-4">
          <div className="mb-1 flex items-center justify-between text-xs text-zinc-400">
            <span>Tokens today</span>
            <span>
              {formatTokens(totalTokensToday)} / {formatTokens(quota)}
            </span>
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-zinc-800">
            <div
              className={`h-full rounded-full transition-all ${
                progressPct > 90 ? 'bg-red-500' : progressPct > 70 ? 'bg-yellow-500' : 'bg-green-500'
              }`}
              style={{ width: `${progressPct}%` }}
            />
          </div>
        </div>
      )}

      {summary && summary.total_cost > 0 && (
        <p className="mt-2 text-xs text-zinc-400">
          Est. cost today:{' '}
          <span className="text-zinc-200">
            ${summary.total_cost.toFixed(4)}
          </span>
        </p>
      )}
    </div>
  );
}

function ProviderHealthCards() {
  const providers = useStore((s) => s.providers);
  const entryStatusData = useGroupStatusPoll();
  const [summaries, setSummaries] = useState<Record<string, DailySummary | null>>({});

  useEffect(() => {
    const today = new Date().toISOString().slice(0, 10);
    const fetchSummaries = async () => {
      const results: Record<string, DailySummary | null> = {};
      await Promise.all(
        providers.map(async (p) => {
          try {
            results[p.id] = await getDailySummary(p.id, today);
          } catch {
            results[p.id] = null;
          }
        }),
      );
      setSummaries(results);
    };
    fetchSummaries();
  }, [providers]);

  const sortedProviders = [...providers].sort((a, b) => {
    const countsA = getEntryStatusCounts(entryStatusData.entries, a.id);
    const countsB = getEntryStatusCounts(entryStatusData.entries, b.id);
    const keyA = getProviderCardSortKey(a, countsA);
    const keyB = getProviderCardSortKey(b, countsB);
    if (keyA !== keyB) return keyA - keyB;
    const subA = getProviderCardSortSubKey(a, countsA);
    const subB = getProviderCardSortSubKey(b, countsB);
    return subA - subB;
  });

  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
      {sortedProviders.map((provider) => {
        const entryCounts = getEntryStatusCounts(entryStatusData.entries, provider.id);
        return (
          <ProviderHealthCard
            key={provider.id}
            provider={provider}
            entryCounts={entryCounts}
            summary={summaries[provider.id] ?? null}
          />
        );
      })}
    </div>
  );
}

function RequestFeed() {
  const [requests, setRequests] = useState<RequestRow[]>([]);
  const [expandedId, setExpandedId] = useState<number | null>(null);

  useEffect(() => {
    const poll = async () => {
      try {
        const data = await getRecentRequests(20);
        setRequests(data);
      } catch {
        // IPC may fail
      }
    };

    poll();
    const interval = setInterval(poll, POLL_INTERVAL_MS);
    return () => clearInterval(interval);
  }, []);

  const toggleExpand = (id: number) => {
    setExpandedId((prev) => (prev === id ? null : id));
  };

  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900">
      <div className="border-b border-zinc-800 px-6 py-4">
        <h2 className="text-lg font-semibold">Recent Requests</h2>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-zinc-800 text-xs uppercase text-zinc-500">
              <th className="px-4 py-3"></th>
              <th className="px-4 py-3">Time</th>
              <th className="px-4 py-3">Group</th>
              <th className="px-4 py-3">Provider</th>
              <th className="px-4 py-3">Tokens In</th>
              <th className="px-4 py-3">Tokens Out</th>
              <th className="px-4 py-3">Latency</th>
              <th className="px-4 py-3">Status</th>
            </tr>
          </thead>
          <tbody>
            {requests.length === 0 && (
              <tr>
                <td colSpan={8} className="px-4 py-8 text-center text-zinc-500">
                  No requests recorded yet
                </td>
              </tr>
            )}
            {requests.map((req) => {
              const isExpanded = expandedId === req.id;
              const showExpand = req.status === 'error' || req.status === 'timeout' || req.status === 'failover' || req.error_type;
              return (
                <React.Fragment key={req.id}>
                  <tr
                    className={`cursor-pointer border-b border-zinc-800/50 transition-colors hover:bg-zinc-800/30 ${
                      isExpanded ? 'bg-zinc-800/20' : ''
                    }`}
                    onClick={() => showExpand && toggleExpand(req.id)}
                  >
                    <td className="px-4 py-3">
                      {showExpand && (
                        isExpanded ? (
                          <ChevronDown className="h-4 w-4 text-zinc-400" />
                        ) : (
                          <ChevronRight className="h-4 w-4 text-zinc-500" />
                        )
                      )}
                    </td>
                    <td className="px-4 py-3 text-zinc-400">{formatRelativeTime(req.ts)}</td>
                    <td className="px-4 py-3">{req.group_alias}</td>
                    <td className="px-4 py-3 text-zinc-400">{req.provider_id}</td>
                    <td className="px-4 py-3">{formatTokens(req.prompt_tokens)}</td>
                    <td className="px-4 py-3">{formatTokens(req.output_tokens)}</td>
                    <td className="px-4 py-3 text-zinc-400">{formatLatency(req.latency_ms)}</td>
                    <td className="px-4 py-3">
                      <StatusBadge status={req.status} />
                    </td>
                  </tr>
                  {isExpanded && (
                    <tr className="bg-zinc-900/50">
                      <td colSpan={8} className="px-4 py-3">
                        <div className="rounded bg-zinc-950 p-3 font-mono text-xs text-red-400">
                          {req.error_type ? `Error type: ${req.error_type}` : 'No error details available'}
                          {req.status === 'error' && req.error_type && (
                            <div className="mt-1 text-zinc-500">
                              Request failed with status: {req.error_type}
                            </div>
                          )}
                        </div>
                      </td>
                    </tr>
                  )}
                </React.Fragment>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export default function Dashboard() {
  return (
    <div className="mx-auto max-w-7xl space-y-8">
      <ProxyStatusCard />
      <ProviderHealthCards />
      <RequestFeed />
    </div>
  );
}
