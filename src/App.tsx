import { createBrowserRouter, RouterProvider, Outlet, Link } from 'react-router-dom';
import { AppShell } from './components/AppShell';
import { GroupStatusProvider } from './hooks/useGroupStatusPoll';
import Dashboard from './pages/Dashboard';
import Providers from './pages/Providers';
import ModelGroups from './pages/ModelGroups';
import OpenCodeSetup from './pages/OpenCodeSetup';
import UsageMetrics from './pages/UsageMetrics';
import Settings from './pages/Settings';

function NotFound() {
  return (
    <div className="flex flex-col items-center justify-center py-16">
      <h2 className="text-xl font-semibold">Page Not Found</h2>
      <p className="mt-2 text-zinc-400">The page you're looking for doesn't exist.</p>
      <Link to="/" className="mt-4 text-emerald-400 hover:underline">Go to Dashboard</Link>
    </div>
  );
}

function Layout() {
  return (
    <AppShell>
      <GroupStatusProvider>
        <Outlet />
      </GroupStatusProvider>
    </AppShell>
  );
}

const router = createBrowserRouter([
  {
    element: <Layout />,
    children: [
      { path: '/', element: <Dashboard /> },
      { path: '/providers', element: <Providers /> },
      { path: '/groups', element: <ModelGroups /> },
      { path: '/opencode', element: <OpenCodeSetup /> },
      { path: '/metrics', element: <UsageMetrics /> },
      { path: '/settings', element: <Settings /> },
      { path: '*', element: <NotFound /> },
    ],
  },
]);

export default function App() {
  return <RouterProvider router={router} />;
}
