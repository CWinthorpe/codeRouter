import { createBrowserRouter, RouterProvider, Outlet } from 'react-router-dom';
import { AppShell } from './components/AppShell';
import Dashboard from './pages/Dashboard';
import Providers from './pages/Providers';
import ModelGroups from './pages/ModelGroups';
import OpenCodeSetup from './pages/OpenCodeSetup';
import UsageMetrics from './pages/UsageMetrics';
import Settings from './pages/Settings';

function Layout() {
  return (
    <AppShell>
      <Outlet />
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
    ],
  },
]);

export default function App() {
  return <RouterProvider router={router} />;
}
