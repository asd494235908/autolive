import {
  createBrowserRouter,
  Navigate,
  Outlet,
  useLocation
} from 'react-router-dom';
import { Alert, Space, Typography } from 'antd';
import { AppLayout } from '../components/AppLayout';
import { DashboardPage } from '../features/dashboard/DashboardPage';
import { LoginPage } from '../features/auth/LoginPage';
import { UserManagementPage } from '../features/users/UserManagementPage';
import { DeviceManagementPage } from '../features/devices/DeviceManagementPage';
import { ActivationCodesPage } from '../features/activation-codes/ActivationCodesPage';
import { ModelPoolPage } from '../features/model-pool/ModelPoolPage';
import { ModelLeasesPage } from '../features/model-leases/ModelLeasesPage';
import { AuditLogsPage } from '../features/audit-logs/AuditLogsPage';
import { AdminSecurityPage } from '../features/security/AdminSecurityPage';
import { readSession } from '../features/auth/session';

type NavItem = {
  key: string;
  label: string;
  path: string;
};

export const navItems: NavItem[] = [
  { key: 'dashboard', label: '系统概览', path: '/' },
  { key: 'users', label: '用户管理', path: '/users' },
  { key: 'devices', label: '设备管理', path: '/devices' },
  { key: 'activation-codes', label: '激活码', path: '/activation-codes' },
  { key: 'model-pools', label: '号池管理', path: '/model-pools' },
  { key: 'model-leases', label: '模型租约', path: '/model-leases' },
  { key: 'audit-logs', label: '审计日志', path: '/audit-logs' },
  { key: 'security', label: '管理员安全', path: '/security' }
];

function PlaceholderPage({ title, description }: { title: string; description: string }) {
  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <Typography.Title level={2} style={{ margin: 0 }}>
        {title}
      </Typography.Title>
      <Alert
        type="info"
        showIcon
        message="功能建设中"
        description={description}
      />
    </Space>
  );
}

function NotFoundPage() {
  return (
    <PlaceholderPage
      title="页面不存在"
      description="功能建设中：当前路由不存在，请从左侧导航返回已规划页面。"
    />
  );
}

function AppShell() {
  const location = useLocation();

  return (
    <AppLayout currentPath={location.pathname} navItems={navItems}>
      <Outlet />
    </AppLayout>
  );
}

function RequireSession() {
  const location = useLocation();
  if (!readSession()) {
    return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  }

  return <AppShell />;
}

export const router = createBrowserRouter([
  {
    path: '/login',
    element: <LoginPage />
  },
  {
    path: '/',
    element: <RequireSession />,
    children: [
      {
        index: true,
        element: <DashboardPage />
      },
      {
        path: 'users',
        element: <UserManagementPage />
      },
      {
        path: 'devices',
        element: <DeviceManagementPage />
      },
      {
        path: 'activation-codes',
        element: <ActivationCodesPage />
      },
      {
        path: 'model-pools',
        element: <ModelPoolPage />
      },
      {
        path: 'model-leases',
        element: <ModelLeasesPage />
      },
      {
        path: 'audit-logs',
        element: <AuditLogsPage />
      },
      {
        path: 'security',
        element: <AdminSecurityPage />
      },
      {
        path: '404',
        element: <NotFoundPage />
      },
      {
        path: '*',
        element: <Navigate to="/404" replace />
      }
    ]
  }
]);
