import React, { useEffect, useState, useCallback, useRef } from 'react';
import { useNavigate } from 'react-router-dom';
import { useStore } from '../store';
import { getDailySummary, getRecentRequests } from '../lib/ipc';
import { useGroupStatusPoll } from '../hooks/useGroupStatusPoll';
import type { DailySummary, RequestRow, Provider, EntryStatusResponse } from '../types';
import { Server, Power, Terminal, ChevronDown, ChevronRight, AlertTriangle, CheckCircle, MinusCircle } from 'lucide-react';
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { StatusBadge } from '@/components/StatusBadge';
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '@/components/ui/table';
import { Progress } from '@/components/ui/progress';

const POLL_INTERVAL_MS = 5000;

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

function getEntryStatusCounts(entries: EntryStatusResponse[], providerId: string): { active: number; cooldown: number; quotaExhausted: number; disabled: number } {
  const filtered = entries.filter((e) => e.provider_id === providerId);
  return {
    active: filtered.filter((e) => e.status === 'active').length,
    cooldown: filtered.filter((e) => e.status === 'cooldown').length,
    quotaExhausted: filtered.filter((e) => e.status === 'quota_exhausted').length,
    disabled: filtered.filter((e) => e.status === 'manually_disabled').length,
  };
}

function getProviderOverallStatus(
  provider: Provider,
  entryCounts: { active: number; cooldown: number; quotaExhausted: number; disabled: number },
): string {
  if (!provider.enabled) return 'Disabled';
  if (entryCounts.active === 0 && entryCounts.cooldown === 0 && entryCounts.quotaExhausted === 0) return 'All Entries Exhausted';
  if (entryCounts.cooldown > 0 || entryCounts.quotaExhausted > 0) return 'Partially Degraded';
  return 'Active';
}

function getProviderCardSortKey(
  provider: Provider,
  entryCounts: { active: number; cooldown: number; quotaExhausted: number; disabled: number },
): number {
  if (!provider.enabled) return 2;
  if (entryCounts.cooldown > 0 || entryCounts.quotaExhausted > 0 || (entryCounts.active === 0 && entryCounts.cooldown === 0 && entryCounts.quotaExhausted === 0)) return 0;
  return 1;
}

function getProviderCardSortSubKey(
  provider: Provider,
  entryCounts: { active: number; cooldown: number; quotaExhausted: number; disabled: number },
): number {
  if (!provider.enabled) return 2;
  if (entryCounts.active === 0 && entryCounts.cooldown === 0 && entryCounts.quotaExhausted === 0) return 0;
  if (entryCounts.cooldown > 0 || entryCounts.quotaExhausted > 0) return 1;
  return 2;
}

function ProxyStatusCard() {
  const proxyStatus = useStore((s) => s.proxyStatus);
  const appConfig = useStore((s) => s.appConfig);
  const healthData = useStore((s) => s.healthData);
  const navigate = useNavigate();

  return (
    <Card className="bg-zinc-900 border-zinc-800">
      <CardHeader>
        <div className="flex items-start justify-between">
          <div>
            <CardTitle className="flex items-center gap-3 text-lg">
              <Server className="h-5 w-5 text-zinc-400" />
              Proxy Status
            </CardTitle>
            <div className="mt-4 flex items-center gap-4">
              <Badge
                variant="outline"
                className={`gap-2 rounded-full px-3 py-1 text-sm font-medium ${
                  proxyStatus === 'running'
                    ? 'bg-green-500/20 text-green-400 border-green-500/30'
                    : 'bg-red-500/20 text-red-400 border-red-500/30'
                }`}
              >
                <Power className="h-4 w-4" />
                {proxyStatus === 'running' ? 'Running' : 'Stopped'}
              </Badge>
            </div>
            <div className="mt-3 space-y-1 text-sm text-zinc-400">
              {appConfig && (
                <>
                  <p>
                    Listening on <span className="text-zinc-200">{appConfig.proxy_host}:{appConfig.proxy_port}</span>
                  </p>
                  {healthData && healthData.uptime_seconds > 0 && (
                    <p>
                      Uptime: <span className="text-zinc-200">{formatUptime(healthData.uptime_seconds)}</span>
                    </p>
                  )}
                </>
              )}
            </div>
          </div>
          <Button
            variant="outline"
            onClick={() => navigate('/opencode')}
            className="gap-2 bg-zinc-800 text-zinc-200 hover:bg-zinc-700"
          >
            <Terminal className="h-4 w-4" />
            Configure OpenCode
          </Button>
        </div>
      </CardHeader>
    </Card>
  );
}

function ProviderHealthCard({
  provider,
  entryCounts,
  summary,
}: {
  provider: Provider;
  entryCounts: { active: number; cooldown: number; quotaExhausted: number; disabled: number };
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
    <Card className="bg-zinc-900 border-zinc-800">
      <CardContent className="p-5">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <h3 className="font-medium">{provider.name}</h3>
            <Badge variant="secondary" className="bg-zinc-800 text-zinc-400">{provider.protocol}</Badge>
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
          {entryCounts.quotaExhausted > 0 && (
            <span>
              <AlertTriangle className="mr-1 inline h-3 w-3 text-orange-500" />
              {entryCounts.quotaExhausted} Quota Exhausted
            </span>
          )}
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
            <Progress
              value={progressPct}
              className={`h-2 bg-zinc-800 ${
                progressPct > 90
                  ? '[&>div]:bg-red-500'
                  : progressPct > 70
                    ? '[&>div]:bg-yellow-500'
                    : '[&>div]:bg-green-500'
              }`}
            />
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
      </CardContent>
    </Card>
  );
}

function ProviderHealthCards() {
  const providers = useStore((s) => s.providers);
  const entryStatusData = useGroupStatusPoll();
  const [summaries, setSummaries] = useState<Record<string, DailySummary | null>>({});

  const fetchSummaries = useCallback(async () => {
    const today = new Date().toISOString().slice(0, 10);
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
  }, [providers]);

  useEffect(() => {
    fetchSummaries();
    const interval = setInterval(fetchSummaries, 60000);
    return () => clearInterval(interval);
  }, [fetchSummaries]);

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
  const recentStreamRequests = useStore((s) => s.recentStreamRequests);
  const [sseApplied, setSseApplied] = useState(0);
  const lastSseTimeRef = useRef(0);
  const [isLive, setIsLive] = useState(false);

  useEffect(() => {
    if (recentStreamRequests.length > sseApplied) {
      lastSseTimeRef.current = Date.now();
    }
  }, [recentStreamRequests, sseApplied]);

  useEffect(() => {
    const interval = setInterval(() => {
      setIsLive(Date.now() - lastSseTimeRef.current < 5000);
    }, 500);
    return () => clearInterval(interval);
  }, []);

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

  useEffect(() => {
    if (recentStreamRequests.length === sseApplied) return;
    const pending = recentStreamRequests.slice(0, recentStreamRequests.length - sseApplied);
    setSseApplied(recentStreamRequests.length);
    setRequests((prev) => {
      const existingIds = new Set(prev.map((r) => r.id));
      const newItems = pending.filter((r) => !existingIds.has(r.id));
      if (newItems.length === 0) return prev;
      return [...newItems, ...prev].slice(0, 50);
    });
  }, [recentStreamRequests, sseApplied]);

  const toggleExpand = (id: number) => {
    setExpandedId((prev) => (prev === id ? null : id));
  };

  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900">
      <div className="border-b border-zinc-800 px-6 py-4">
        <h2 className="text-lg font-semibold flex items-center gap-2">
          Recent Requests
          {isLive && (
            <span className="flex items-center gap-1.5 rounded-full bg-green-500/15 px-2.5 py-0.5 text-xs font-medium text-green-400">
              <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-green-400" />
              Live
            </span>
          )}
        </h2>
      </div>
      <Table className="text-left text-sm">
        <TableHeader>
          <TableRow className="border-b border-zinc-800 text-xs uppercase text-zinc-500 hover:bg-transparent">
            <TableHead className="px-4 py-3"></TableHead>
            <TableHead className="px-4 py-3">Time</TableHead>
            <TableHead className="px-4 py-3">Group</TableHead>
            <TableHead className="px-4 py-3">Provider</TableHead>
            <TableHead className="px-4 py-3">Tokens In</TableHead>
            <TableHead className="px-4 py-3">Tokens Out</TableHead>
            <TableHead className="px-4 py-3">Latency</TableHead>
            <TableHead className="px-4 py-3">Status</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {requests.length === 0 && (
            <TableRow>
              <TableCell colSpan={8} className="px-4 py-8 text-center text-zinc-500">
                No requests recorded yet
              </TableCell>
            </TableRow>
          )}
          {requests.map((req) => {
            const isExpanded = expandedId === req.id;
            const showExpand = req.status === 'error' || req.status === 'timeout' || req.status === 'failover' || req.error_type;
            return (
              <React.Fragment key={req.id}>
                <TableRow
                  className={`cursor-pointer border-b border-zinc-800/50 hover:bg-zinc-800/30 ${
                    isExpanded ? 'bg-zinc-800/20' : ''
                  }`}
                  onClick={() => showExpand && toggleExpand(req.id)}
                >
                  <TableCell className="px-4 py-3">
                    {showExpand && (
                      isExpanded ? (
                        <ChevronDown className="h-4 w-4 text-zinc-400" />
                      ) : (
                        <ChevronRight className="h-4 w-4 text-zinc-500" />
                      )
                    )}
                  </TableCell>
                  <TableCell className="px-4 py-3 text-zinc-400">{formatRelativeTime(req.ts)}</TableCell>
                  <TableCell className="px-4 py-3">{req.group_alias}</TableCell>
                  <TableCell className="px-4 py-3 text-zinc-400">{req.provider_id}</TableCell>
                  <TableCell className="px-4 py-3">{formatTokens(req.prompt_tokens)}</TableCell>
                  <TableCell className="px-4 py-3">{formatTokens(req.output_tokens)}</TableCell>
                  <TableCell className="px-4 py-3 text-zinc-400">{formatLatency(req.latency_ms)}</TableCell>
                  <TableCell className="px-4 py-3">
                    <StatusBadge status={req.status} />
                  </TableCell>
                </TableRow>
                {isExpanded && (
                  <TableRow className="bg-zinc-900/50 hover:bg-zinc-900/50">
                    <TableCell colSpan={8} className="px-4 py-3">
                      <div className="rounded bg-zinc-950 p-3 font-mono text-xs text-red-400">
                        {req.error_type ? `Error type: ${req.error_type}` : 'No error details available'}
                        {req.status === 'error' && req.error_type && (
                          <div className="mt-1 text-zinc-500">
                            Request failed with status: {req.error_type}
                          </div>
                        )}
                      </div>
                    </TableCell>
                  </TableRow>
                )}
              </React.Fragment>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}

function LiveMetricsCard() {
  const recentStreamRequests = useStore((s) => s.recentStreamRequests);
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    const interval = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(interval);
  }, []);

  const thirtySecondsAgo = (now / 1000) - 30;
  const fiveSecondsAgo = (now / 1000) - 5;

  const recent30s = recentStreamRequests.filter((r) => r.ts >= thirtySecondsAgo);
  const recent5s = recentStreamRequests.filter((r) => r.ts >= fiveSecondsAgo);

  const totalTokens30s = recent30s.reduce((sum, r) => sum + (r.prompt_tokens ?? 0) + (r.output_tokens ?? 0), 0);
  const elapsed30s = recent30s.length > 0
    ? Math.max(now / 1000 - Math.min(...recent30s.map((r) => r.ts)), 1)
    : 30;
  const tokensPerSecond = totalTokens30s / Math.min(elapsed30s, 30);
  const activeStreams = recent5s.length;
  const isActive = activeStreams > 0;

  return (
    <Card className="bg-zinc-900 border-zinc-800">
      <CardContent className="flex items-center gap-6 p-4">
        <div className="flex items-center gap-2">
          <span
            className={`inline-block h-2 w-2 rounded-full ${
              isActive ? 'animate-pulse bg-green-400' : 'bg-zinc-600'
            }`}
          />
          <span className="text-sm font-medium text-zinc-300">Live Metrics</span>
        </div>
        <div className="flex items-center gap-1 text-sm">
          <span className="text-zinc-400">Throughput:</span>
          <span className="font-mono text-zinc-100">{tokensPerSecond.toFixed(1)} tokens/s</span>
        </div>
        <div className="flex items-center gap-1 text-sm">
          <span className="text-zinc-400">Active streams:</span>
          <span className={`font-mono ${isActive ? 'text-green-400' : 'text-zinc-100'}`}>{activeStreams}</span>
        </div>
      </CardContent>
    </Card>
  );
}

export default function Dashboard() {
  return (
    <div className="mx-auto max-w-7xl space-y-8">
      <ProxyStatusCard />
      <ProviderHealthCards />
      <LiveMetricsCard />
      <RequestFeed />
    </div>
  );
}
