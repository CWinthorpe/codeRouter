import { createBrowserRouter, RouterProvider, Outlet, Link } from 'react-router-dom';
import { AppShell } from './components/AppShell';
import { ErrorBoundary } from './components/ErrorBoundary';
import { GroupStatusProvider } from './hooks/useGroupStatusPoll';
import Dashboard from './pages/Dashboard';
import Providers from './pages/Providers';
import ModelGroups from './pages/ModelGroups';
import OpenCodeSetup from './pages/OpenCodeSetup';
import UsageMetrics from './pages/UsageMetrics';
import Settings from './pages/Settings';

/** 404 page shown for unrecognized routes. */
function NotFound() {
  return (
    <div className="flex flex-col items-center justify-center py-16">
      <h2 className="text-xl font-semibold">Page Not Found</h2>
      <p className="mt-2 text-zinc-400">The page you're looking for doesn't exist.</p>
      <Link to="/" className="mt-4 text-emerald-400 hover:underline">Go to Dashboard</Link>
    </div>
  );
}

/** Wraps all pages in the sidebar shell and group-status polling provider. */
function Layout() {
  return (
    <AppShell>
      <GroupStatusProvider>
        <Outlet />
      </GroupStatusProvider>
    </AppShell>
  );
}

// Route definitions — each path maps to a dedicated page component
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

/**
 * Root application component. Mounts the router inside an ErrorBoundary
 * so that unhandled rendering errors are caught gracefully.
 */
export default function App() {
  return (
    <ErrorBoundary>
      <RouterProvider router={router} />
    </ErrorBoundary>
  );
}
