import { useEffect, useState } from 'react';
import { useLocation, Link } from 'react-router-dom';
import { LayoutDashboard, Server, Layers, Terminal, BarChart3, Settings } from 'lucide-react';
import { useStore } from '../store';
import { useProxyStatusPoll } from '../hooks/useProxyStatusPoll';
import { useMetricsStream } from '../hooks/useMetricsStream';
import { Onboarding } from './Onboarding';
import { dismissOnboarding } from '../lib/ipc';

export function AppShell({ children }: { children: React.ReactNode }) {
  const loadInitialData = useStore((s) => s.loadInitialData);
  const providers = useStore((s) => s.providers);
  const groups = useStore((s) => s.groups);
  const appConfig = useStore((s) => s.appConfig);
  const [showOnboarding, setShowOnboarding] = useState(false);
  useProxyStatusPoll();
  useMetricsStream();

  useEffect(() => {
    loadInitialData();
  }, [loadInitialData]);

  useEffect(() => {
    if (appConfig) {
      setShowOnboarding(!appConfig.onboarding_dismissed && (providers.length === 0 || groups.length === 0));
    }
  }, [appConfig, providers.length, groups.length]);

  const handleDismissOnboarding = async () => {
    try {
      await dismissOnboarding();
    } catch {
      // IPC may fail
    }
    setShowOnboarding(false);
  };

  return (
    <div className="flex h-screen w-screen bg-zinc-950 text-zinc-100">
      <Sidebar />
      <main className="flex-1 overflow-auto p-8">{children}</main>
      {showOnboarding && (
        <Onboarding
          providersCount={providers.length}
          groupsCount={groups.length}
          onDismiss={handleDismissOnboarding}
        />
      )}
    </div>
  );
}

function Sidebar() {
  return (
    <aside className="flex w-56 flex-col border-r border-zinc-800 bg-zinc-900 p-3">
      <div className="mb-4 flex items-center gap-2 px-2 py-2">
        <StatusDot />
        <span className="text-sm font-semibold">CodeRouter</span>
      </div>
      <nav className="flex flex-col gap-1">
        <SidebarItem to="/" icon={LayoutDashboard} label="Dashboard" />
        <SidebarItem to="/providers" icon={Server} label="Providers" />
        <SidebarItem to="/groups" icon={Layers} label="Model Groups" />
        <SidebarItem to="/opencode" icon={Terminal} label="OpenCode Setup" />
        <SidebarItem to="/metrics" icon={BarChart3} label="Usage & Metrics" />
        <SidebarItem to="/settings" icon={Settings} label="Settings" />
      </nav>
    </aside>
  );
}

function StatusDot() {
  const status = useStore((s) => s.proxyStatus);
  const color = status === 'running' ? 'bg-green-500' : 'bg-red-500';
  return <div className={`h-2.5 w-2.5 rounded-full ${color}`} />;
}

function SidebarItem({
  to,
  icon: Icon,
  label,
}: {
  to: string;
  icon: React.FC<React.SVGProps<SVGSVGElement>>;
  label: string;
}) {
  const location = useLocation();
  const isActive = location.pathname === to;

  return (
    <Link
      to={to}
      className={`flex items-center gap-3 rounded-md px-3 py-2 text-sm transition-colors ${
        isActive ? 'bg-zinc-800 text-white' : 'text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200'
      }`}
    >
      <Icon className="h-4 w-4 shrink-0" />
      <span>{label}</span>
    </Link>
  );
}
