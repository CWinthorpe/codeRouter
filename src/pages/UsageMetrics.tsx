import { useState, useEffect, useMemo, useCallback, useRef } from 'react';
import {
  BarChart,
  Bar,
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
} from 'recharts';
import { Calendar, Download, ChevronLeft, ChevronRight, Filter } from 'lucide-react';
import { getRecentRequests } from '../lib/ipc';
import type { RequestRow } from '../types';

type Preset = 'today' | 'last7' | 'last30' | 'custom';

interface DateRange {
  start: Date;
  end: Date;
}

const PAGE_SIZE = 50;
const FETCH_LIMIT = 1000;

function startOfDay(d: Date): Date {
  const copy = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()));
  return copy;
}

function endOfDay(d: Date): Date {
  const copy = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), 23, 59, 59, 999));
  return copy;
}

function formatDate(d: Date): string {
  return d.toISOString().slice(0, 10);
}

function formatUTCDate(d: Date): string {
  const year = d.getUTCFullYear();
  const month = String(d.getUTCMonth() + 1).padStart(2, '0');
  const day = String(d.getUTCDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

function formatDisplayDate(d: Date): string {
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
}

function formatTs(ts: number): string {
  return new Date(ts * 1000).toLocaleString('en-US', {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return `${n}`;
}

function getDaysInRange(start: Date, end: Date): string[] {
  const days: string[] = [];
  const current = startOfDay(start);
  const last = startOfDay(end);
  while (current <= last) {
    days.push(formatUTCDate(current));
    current.setUTCDate(current.getUTCDate() + 1);
  }
  return days;
}

function generateColors(n: number): string[] {
  const palette = [
    '#3b82f6', '#ef4444', '#10b981', '#f59e0b', '#8b5cf6',
    '#ec4899', '#06b6d4', '#f97316', '#6366f1', '#14b8a6',
    '#e11d48', '#84cc16', '#0ea5e9', '#d946ef', '#22c55e',
  ];
  return Array.from({ length: n }, (_, i) => palette[i % palette.length]);
}

function SortHeader({ column, children, sortColumn, sortDirection, onSort }: { column: keyof RequestRow; children: React.ReactNode; sortColumn: keyof RequestRow; sortDirection: 'asc' | 'desc'; onSort: (column: keyof RequestRow) => void }) {
  return (
    <th
      className="cursor-pointer select-none px-4 py-3 text-xs uppercase text-zinc-500 hover:text-zinc-300"
      onClick={() => onSort(column)}
    >
      <span className="inline-flex items-center gap-1">
        {children}
        {sortColumn === column && (
          <span className="text-zinc-300">{sortDirection === 'asc' ? '↑' : '↓'}</span>
        )}
      </span>
    </th>
  );
}

function FilterDropdown({
  label,
  options,
  selected,
  show,
  onToggle,
  onToggleOption,
}: {
  label: string;
  options: string[];
  selected: string[];
  show: boolean;
  onToggle: () => void;
  onToggleOption: (value: string) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const onToggleRef = useRef(onToggle);
  onToggleRef.current = onToggle;

  useEffect(() => {
    if (!show) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        onToggleRef.current();
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [show]);

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={onToggle}
        className={`flex items-center gap-2 rounded-md border px-3 py-1.5 text-sm transition-colors ${
          selected.length > 0
            ? 'border-blue-500/50 bg-blue-500/10 text-blue-400'
            : 'border-zinc-700 bg-zinc-800 text-zinc-300 hover:border-zinc-600'
        }`}
      >
        <Filter className="h-3.5 w-3.5" />
        {label}
        {selected.length > 0 && (
          <span className="rounded-full bg-blue-500/30 px-1.5 text-xs">{selected.length}</span>
        )}
      </button>
      {show && (
        <div className="absolute left-0 top-full z-50 mt-1 w-56 rounded-md border border-zinc-700 bg-zinc-800 p-2 shadow-xl">
          {options.length === 0 && (
            <p className="px-2 py-1 text-xs text-zinc-500">No options available</p>
          )}
          {options.map((opt) => (
            <label key={opt} className="flex cursor-pointer items-center gap-2 rounded px-2 py-1 text-sm hover:bg-zinc-700">
              <input
                type="checkbox"
                checked={selected.includes(opt)}
                onChange={() => onToggleOption(opt)}
                className="accent-blue-500"
              />
              <span className="truncate">{opt}</span>
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

export default function UsageMetrics() {
  const [preset, setPreset] = useState<Preset>('last7');
  const [customStart, setCustomStart] = useState(formatDate(new Date(Date.now() - 7 * 86400000)));
  const [customEnd, setCustomEnd] = useState(formatDate(new Date()));
  const [allRequests, setAllRequests] = useState<RequestRow[]>([]);
  const [loading, setLoading] = useState(true);

  const [filterProviders, setFilterProviders] = useState<string[]>([]);
  const [filterGroups, setFilterGroups] = useState<string[]>([]);
  const [filterStatuses, setFilterStatuses] = useState<string[]>([]);

  const [sortColumn, setSortColumn] = useState<keyof RequestRow>('ts');
  const [sortDirection, setSortDirection] = useState<'asc' | 'desc'>('desc');

  const [page, setPage] = useState(1);

  const [showFilterProviders, setShowFilterProviders] = useState(false);
  const [showFilterGroups, setShowFilterGroups] = useState(false);
  const [showFilterStatuses, setShowFilterStatuses] = useState(false);

  const dateRange: DateRange = useMemo(() => {
    const now = new Date();
    switch (preset) {
      case 'today':
        return { start: startOfDay(now), end: endOfDay(now) };
      case 'last7':
        return { start: startOfDay(new Date(now.getTime() - 6 * 86400000)), end: endOfDay(now) };
      case 'last30':
        return { start: startOfDay(new Date(now.getTime() - 29 * 86400000)), end: endOfDay(now) };
      case 'custom':
        return { start: startOfDay(new Date(customStart + 'T00:00:00')), end: endOfDay(new Date(customEnd + 'T23:59:59')) };
    }
  }, [preset, customStart, customEnd]);

  useEffect(() => {
    const loadData = async () => {
      setLoading(true);
      try {
        const requests = await getRecentRequests(FETCH_LIMIT);
        setAllRequests(requests);
      } catch {
        // IPC may fail outside Tauri
      } finally {
        setLoading(false);
      }
    };
    loadData();
    const interval = setInterval(loadData, 30000);
    return () => clearInterval(interval);
  }, []);

  const filteredRequests = useMemo(() => {
    const startTs = dateRange.start.getTime() / 1000;
    const endTs = dateRange.end.getTime() / 1000;

    return allRequests.filter((r) => {
      if (r.ts < startTs || r.ts > endTs) return false;
      if (filterProviders.length > 0 && !filterProviders.includes(r.provider_id)) return false;
      if (filterGroups.length > 0 && !filterGroups.includes(r.group_alias)) return false;
      if (filterStatuses.length > 0 && !filterStatuses.includes(r.status)) return false;
      return true;
    });
  }, [allRequests, dateRange, filterProviders, filterGroups, filterStatuses]);

  const sortedRequests = useMemo(() => {
    return [...filteredRequests].sort((a, b) => {
      const aVal = a[sortColumn];
      const bVal = b[sortColumn];
      if (aVal == null && bVal == null) return 0;
      if (aVal == null) return sortDirection === 'asc' ? 1 : -1;
      if (bVal == null) return sortDirection === 'asc' ? -1 : 1;
      let cmp = 0;
      if (typeof aVal === 'number' && typeof bVal === 'number') {
        cmp = aVal - bVal;
      } else if (typeof aVal === 'string' && typeof bVal === 'string') {
        cmp = aVal.localeCompare(bVal);
      }
      return sortDirection === 'asc' ? cmp : -cmp;
    });
  }, [filteredRequests, sortColumn, sortDirection]);

  const totalPages = Math.max(1, Math.ceil(sortedRequests.length / PAGE_SIZE));
  const pagedRequests = useMemo(() => {
    const startIdx = (page - 1) * PAGE_SIZE;
    return sortedRequests.slice(startIdx, startIdx + PAGE_SIZE);
  }, [sortedRequests, page]);

  useEffect(() => {
    setPage(1);
  }, [dateRange, filterProviders, filterGroups, filterStatuses, sortColumn, sortDirection]);

  const handleSort = (column: keyof RequestRow) => {
    if (sortColumn === column) {
      setSortDirection((d) => (d === 'asc' ? 'desc' : 'asc'));
    } else {
      setSortColumn(column);
      setSortDirection(column === 'ts' ? 'desc' : 'asc');
    }
  };

  const toggleFilter = (list: string[], value: string, setter: (v: string[]) => void) => {
    setter(list.includes(value) ? list.filter((v) => v !== value) : [...list, value]);
  };

  const summary = useMemo(() => {
    const totalRequests = filteredRequests.length;
    const totalTokens = filteredRequests.reduce((s, r) => s + r.prompt_tokens + r.output_tokens, 0);
    const totalCost = filteredRequests.reduce((s, r) => s + r.cost_usd, 0);
    const latencies = filteredRequests.map((r) => r.latency_ms).sort((a, b) => a - b);
    const p50 = latencies.length > 0 ? latencies[Math.floor(latencies.length / 2)] : 0;
    return { totalRequests, totalTokens, totalCost, p50 };
  }, [filteredRequests]);

  const chartData = useMemo(() => {
    const days = getDaysInRange(dateRange.start, dateRange.end);
    const providerColors: Record<string, string> = {};
    const groupColors: Record<string, string> = {};

    const providerNames = [...new Set(allRequests.map((r) => r.provider_id))];
    const groupNames = [...new Set(allRequests.map((r) => r.group_alias))];

    providerNames.forEach((name, i) => { providerColors[name] = generateColors(providerNames.length)[i]; });
    groupNames.forEach((name, i) => { groupColors[name] = generateColors(groupNames.length)[i]; });

    const costByProvider: Record<string, Record<string, number>> = {};
    const tokensByProvider: Record<string, Record<string, number>> = {};
    const volumeByGroup: Record<string, Record<string, number>> = {};

    for (const day of days) {
      costByProvider[day] = {};
      tokensByProvider[day] = {};
      volumeByGroup[day] = {};
      for (const p of providerNames) {
        costByProvider[day][p] = 0;
        tokensByProvider[day][p] = 0;
      }
      for (const g of groupNames) {
        volumeByGroup[day][g] = 0;
      }
    }

    for (const r of filteredRequests) {
      const day = new Date(r.ts * 1000).toISOString().slice(0, 10);
      if (!costByProvider[day]) continue;
      if (costByProvider[day][r.provider_id] !== undefined) {
        costByProvider[day][r.provider_id] += r.cost_usd;
        tokensByProvider[day][r.provider_id] += r.prompt_tokens + r.output_tokens;
      }
      if (volumeByGroup[day][r.group_alias] !== undefined) {
        volumeByGroup[day][r.group_alias] += 1;
      }
    }

    const costChartData = days.map((day) => ({ day, ...costByProvider[day] }));
    const tokensChartData = days.map((day) => ({ day, ...tokensByProvider[day] }));
    const volumeChartData = days.map((day) => ({ day, ...volumeByGroup[day] }));

    return { costChartData, tokensChartData, volumeChartData, providerNames, groupNames, providerColors, groupColors };
  }, [filteredRequests, allRequests, dateRange]);

  const exportCSV = useCallback(() => {
    const headers = ['Timestamp', 'Group', 'Provider', 'Model', 'Prompt Tokens', 'Output Tokens', 'Cost', 'Latency (ms)', 'Status', 'Error'];
    const rows = filteredRequests.map((r) => [
      new Date(r.ts * 1000).toISOString(),
      r.group_alias,
      r.provider_id,
      r.model_id,
      r.prompt_tokens,
      r.output_tokens,
      r.cost_usd.toFixed(6),
      r.latency_ms,
      r.status,
      r.error_type ?? '',
    ]);

    const csvContent = [headers, ...rows].map((row) => row.map((cell) => `"${String(cell).replace(/"/g, '""')}"`).join(',')).join('\n');
    const blob = new Blob([csvContent], { type: 'text/csv;charset=utf-8;' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = `coderouter-requests-${formatDate(dateRange.start)}-${formatDate(dateRange.end)}.csv`;
    link.click();
    URL.revokeObjectURL(url);
  }, [filteredRequests, dateRange]);

  const uniqueProviders = useMemo(() => [...new Set(allRequests.map((r) => r.provider_id))].sort(), [allRequests]);
  const uniqueGroups = useMemo(() => [...new Set(allRequests.map((r) => r.group_alias))].sort(), [allRequests]);
  const uniqueStatuses = useMemo(() => [...new Set(allRequests.map((r) => r.status))].sort(), [allRequests]);

  return (
    <div className="mx-auto max-w-7xl space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold">Usage & Metrics</h1>
      </div>

      {/* Date Range Picker */}
      <div className="flex flex-wrap items-center gap-3 rounded-lg border border-zinc-800 bg-zinc-900 p-4">
        <Calendar className="h-4 w-4 text-zinc-400" />
        <div className="flex gap-2">
          {(['today', 'last7', 'last30'] as Preset[]).map((p) => (
            <button
              key={p}
              onClick={() => setPreset(p)}
              className={`rounded-md px-3 py-1.5 text-sm transition-colors ${
                preset === p
                  ? 'bg-blue-600 text-white'
                  : 'bg-zinc-800 text-zinc-300 hover:bg-zinc-700'
              }`}
            >
              {p === 'today' ? 'Today' : p === 'last7' ? 'Last 7 days' : 'Last 30 days'}
            </button>
          ))}
          <button
            onClick={() => setPreset('custom')}
            className={`rounded-md px-3 py-1.5 text-sm transition-colors ${
              preset === 'custom'
                ? 'bg-blue-600 text-white'
                : 'bg-zinc-800 text-zinc-300 hover:bg-zinc-700'
            }`}
          >
            Custom
          </button>
        </div>
        {preset === 'custom' && (
          <div className="flex items-center gap-2">
            <input
              type="date"
              value={customStart}
              onChange={(e) => setCustomStart(e.target.value)}
              className="rounded-md border border-zinc-700 bg-zinc-800 px-2 py-1.5 text-sm text-zinc-200"
            />
            <span className="text-zinc-500">→</span>
            <input
              type="date"
              value={customEnd}
              onChange={(e) => setCustomEnd(e.target.value)}
              className="rounded-md border border-zinc-700 bg-zinc-800 px-2 py-1.5 text-sm text-zinc-200"
            />
          </div>
        )}
        <span className="ml-auto text-sm text-zinc-400">
          {formatDisplayDate(dateRange.start)} — {formatDisplayDate(dateRange.end)}
        </span>
      </div>

      {/* Summary Cards */}
      <div className="grid grid-cols-2 gap-4 md:grid-cols-4">
        <SummaryCard label="Total Requests" value={summary.totalRequests.toLocaleString()} />
        <SummaryCard label="Total Tokens" value={formatTokens(summary.totalTokens)} />
        <SummaryCard label="Total Est. Cost" value={`$${summary.totalCost.toFixed(4)}`} />
        <SummaryCard label="Avg Latency (p50)" value={`${summary.p50}ms`} />
      </div>

      {/* Charts */}
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <ChartCard title="Cost by Provider">
          {chartData.costChartData.length > 0 ? (
            <ResponsiveContainer width="100%" height={280}>
              <BarChart data={chartData.costChartData}>
                <CartesianGrid strokeDasharray="3 3" stroke="#27272a" />
                <XAxis dataKey="day" tick={{ fill: '#71717a', fontSize: 11 }} tickFormatter={(v: string) => v.slice(5)} />
                <YAxis tick={{ fill: '#71717a', fontSize: 11 }} tickFormatter={(v: number) => `$${v.toFixed(2)}`} />
                <Tooltip
                  contentStyle={{ backgroundColor: '#18181b', border: '1px solid #27272a', borderRadius: '6px' }}
                  labelStyle={{ color: '#a1a1aa' }}
                  formatter={(value: unknown) => [`$${Number(value).toFixed(4)}`, '']}
                />
                <Legend wrapperStyle={{ fontSize: '12px' }} />
                {chartData.providerNames.map((name) => (
                  <Bar key={name} dataKey={name} stackId="a" fill={chartData.providerColors[name]} name={name} />
                ))}
              </BarChart>
            </ResponsiveContainer>
          ) : (
            <EmptyChart />
          )}
        </ChartCard>

        <ChartCard title="Tokens by Provider">
          {chartData.tokensChartData.length > 0 ? (
            <ResponsiveContainer width="100%" height={280}>
              <BarChart data={chartData.tokensChartData}>
                <CartesianGrid strokeDasharray="3 3" stroke="#27272a" />
                <XAxis dataKey="day" tick={{ fill: '#71717a', fontSize: 11 }} tickFormatter={(v: string) => v.slice(5)} />
                <YAxis tick={{ fill: '#71717a', fontSize: 11 }} tickFormatter={(v: number) => formatTokens(v)} />
                <Tooltip
                  contentStyle={{ backgroundColor: '#18181b', border: '1px solid #27272a', borderRadius: '6px' }}
                  labelStyle={{ color: '#a1a1aa' }}
                  formatter={(value: unknown) => [formatTokens(Number(value)), '']}
                />
                <Legend wrapperStyle={{ fontSize: '12px' }} />
                {chartData.providerNames.map((name) => (
                  <Bar key={name} dataKey={name} stackId="a" fill={chartData.providerColors[name]} name={name} />
                ))}
              </BarChart>
            </ResponsiveContainer>
          ) : (
            <EmptyChart />
          )}
        </ChartCard>

        <ChartCard title="Request Volume by Group" className="lg:col-span-2">
          {chartData.volumeChartData.length > 0 ? (
            <ResponsiveContainer width="100%" height={280}>
              <LineChart data={chartData.volumeChartData}>
                <CartesianGrid strokeDasharray="3 3" stroke="#27272a" />
                <XAxis dataKey="day" tick={{ fill: '#71717a', fontSize: 11 }} tickFormatter={(v: string) => v.slice(5)} />
                <YAxis tick={{ fill: '#71717a', fontSize: 11 }} allowDecimals={false} />
                <Tooltip
                  contentStyle={{ backgroundColor: '#18181b', border: '1px solid #27272a', borderRadius: '6px' }}
                  labelStyle={{ color: '#a1a1aa' }}
                />
                <Legend wrapperStyle={{ fontSize: '12px' }} />
                {chartData.groupNames.map((name) => (
                  <Line
                    key={name}
                    type="monotone"
                    dataKey={name}
                    stroke={chartData.groupColors[name]}
                    strokeWidth={2}
                    dot={{ r: 3 }}
                    name={name}
                  />
                ))}
              </LineChart>
            </ResponsiveContainer>
          ) : (
            <EmptyChart />
          )}
        </ChartCard>
      </div>

      {/* Request Log Table */}
      <div className="rounded-lg border border-zinc-800 bg-zinc-900">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-zinc-800 px-6 py-4">
          <h2 className="text-lg font-semibold">Request Log</h2>
          <div className="flex items-center gap-3">
            <FilterDropdown
              label="Provider"
              options={uniqueProviders}
              selected={filterProviders}
              show={showFilterProviders}
              onToggle={() => { setShowFilterProviders(!showFilterProviders); setShowFilterGroups(false); setShowFilterStatuses(false); }}
              onToggleOption={(v) => toggleFilter(filterProviders, v, setFilterProviders)}
            />
            <FilterDropdown
              label="Group"
              options={uniqueGroups}
              selected={filterGroups}
              show={showFilterGroups}
              onToggle={() => { setShowFilterGroups(!showFilterGroups); setShowFilterProviders(false); setShowFilterStatuses(false); }}
              onToggleOption={(v) => toggleFilter(filterGroups, v, setFilterGroups)}
            />
            <FilterDropdown
              label="Status"
              options={uniqueStatuses}
              selected={filterStatuses}
              show={showFilterStatuses}
              onToggle={() => { setShowFilterStatuses(!showFilterStatuses); setShowFilterProviders(false); setShowFilterGroups(false); }}
              onToggleOption={(v) => toggleFilter(filterStatuses, v, setFilterStatuses)}
            />
            <button
              onClick={exportCSV}
              className="flex items-center gap-2 rounded-md bg-zinc-800 px-3 py-1.5 text-sm text-zinc-300 transition-colors hover:bg-zinc-700"
            >
              <Download className="h-3.5 w-3.5" />
              Export CSV
            </button>
          </div>
        </div>

        {loading ? (
          <div className="px-6 py-12 text-center text-zinc-500">Loading request data...</div>
        ) : (
          <>
            <div className="overflow-x-auto">
              <table className="w-full text-left text-sm">
                <thead>
                  <tr className="border-b border-zinc-800">
                    <SortHeader column="ts" sortColumn={sortColumn} sortDirection={sortDirection} onSort={handleSort}>Timestamp</SortHeader>
                    <SortHeader column="group_alias" sortColumn={sortColumn} sortDirection={sortDirection} onSort={handleSort}>Group</SortHeader>
                    <SortHeader column="provider_id" sortColumn={sortColumn} sortDirection={sortDirection} onSort={handleSort}>Provider</SortHeader>
                    <SortHeader column="model_id" sortColumn={sortColumn} sortDirection={sortDirection} onSort={handleSort}>Model</SortHeader>
                    <SortHeader column="prompt_tokens" sortColumn={sortColumn} sortDirection={sortDirection} onSort={handleSort}>Prompt Tokens</SortHeader>
                    <SortHeader column="output_tokens" sortColumn={sortColumn} sortDirection={sortDirection} onSort={handleSort}>Output Tokens</SortHeader>
                    <SortHeader column="cost_usd" sortColumn={sortColumn} sortDirection={sortDirection} onSort={handleSort}>Cost</SortHeader>
                    <SortHeader column="latency_ms" sortColumn={sortColumn} sortDirection={sortDirection} onSort={handleSort}>Latency (ms)</SortHeader>
                    <SortHeader column="status" sortColumn={sortColumn} sortDirection={sortDirection} onSort={handleSort}>Status</SortHeader>
                    <SortHeader column="error_type" sortColumn={sortColumn} sortDirection={sortDirection} onSort={handleSort}>Error</SortHeader>
                  </tr>
                </thead>
                <tbody>
                  {pagedRequests.length === 0 && (
                    <tr>
                      <td colSpan={10} className="px-4 py-8 text-center text-zinc-500">
                        No requests match the selected filters and date range
                      </td>
                    </tr>
                  )}
                  {pagedRequests.map((req) => (
                    <tr key={req.id} className="border-b border-zinc-800/50 transition-colors hover:bg-zinc-800/30">
                      <td className="px-4 py-2.5 text-zinc-400">{formatTs(req.ts)}</td>
                      <td className="px-4 py-2.5">{req.group_alias}</td>
                      <td className="px-4 py-2.5 text-zinc-400">{req.provider_id}</td>
                      <td className="px-4 py-2.5 text-zinc-400">{req.model_id}</td>
                      <td className="px-4 py-2.5">{formatTokens(req.prompt_tokens)}</td>
                      <td className="px-4 py-2.5">{formatTokens(req.output_tokens)}</td>
                      <td className="px-4 py-2.5 text-zinc-300">${req.cost_usd.toFixed(4)}</td>
                      <td className="px-4 py-2.5 text-zinc-400">{req.latency_ms}</td>
                      <td className="px-4 py-2.5">
                        <StatusBadge status={req.status} />
                      </td>
                      <td className="max-w-[200px] truncate px-4 py-2.5 text-xs text-zinc-500">
                        {req.error_type ?? '—'}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>

            {/* Pagination */}
            <div className="flex items-center justify-between border-t border-zinc-800 px-6 py-3">
              <span className="text-sm text-zinc-400">
                {sortedRequests.length} result{sortedRequests.length !== 1 ? 's' : ''} · Page {page} of {totalPages}
              </span>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                  disabled={page <= 1}
                  className="rounded-md border border-zinc-700 px-2 py-1 text-sm text-zinc-400 transition-colors hover:bg-zinc-800 disabled:opacity-30 disabled:cursor-not-allowed"
                >
                  <ChevronLeft className="h-4 w-4" />
                </button>
                <button
                  onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                  disabled={page >= totalPages}
                  className="rounded-md border border-zinc-700 px-2 py-1 text-sm text-zinc-400 transition-colors hover:bg-zinc-800 disabled:opacity-30 disabled:cursor-not-allowed"
                >
                  <ChevronRight className="h-4 w-4" />
                </button>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function SummaryCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900 p-4">
      <p className="text-xs uppercase text-zinc-500">{label}</p>
      <p className="mt-1 text-2xl font-semibold text-zinc-100">{value}</p>
    </div>
  );
}

function ChartCard({ title, children, className = '' }: { title: string; children: React.ReactNode; className?: string }) {
  return (
    <div className={`rounded-lg border border-zinc-800 bg-zinc-900 p-5 ${className}`}>
      <h3 className="mb-4 text-sm font-medium text-zinc-300">{title}</h3>
      {children}
    </div>
  );
}

function EmptyChart() {
  return (
    <div className="flex h-[280px] items-center justify-center text-sm text-zinc-500">
      No data for selected range
    </div>
  );
}

import { StatusBadge } from '@/components/StatusBadge';
