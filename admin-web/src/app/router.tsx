import {
  createBrowserRouter,
  Navigate,
  Outlet,
  useLocation,
} from 'react-router-dom';
import { Alert, Button, Result, Space, Spin, Typography } from 'antd';
import type { ReactElement } from 'react';
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
import { AdminForbiddenPage } from '../features/admin-rbac/AdminForbiddenPage';
import { AdminRbacPage } from '../features/admin-rbac/AdminRbacPage';
import { adminRouteMeta, buildVisibleNavItems, getPermissionGateState } from '../features/admin-rbac/adminRouteMeta';
import { AUTHORIZATION_ERROR_TITLE, useAdminAuthorization } from '../features/admin-rbac/useAdminAuthorization';

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

function PermissionBoundary({
  requiredPermission,
  children,
}: {
  requiredPermission?: (typeof adminRouteMeta)[number]['requiredPermission'];
  children: ReactElement;
}) {
  const authorization = useAdminAuthorization();
  const gateState = getPermissionGateState(authorization, requiredPermission);

  if (gateState === 'loading') {
    return (
      <Space direction="vertical" align="center" size="middle" style={{ width: '100%', paddingBlock: 96 }}>
        <Spin size="large" />
        <Typography.Text type="secondary">正在验证管理员权限…</Typography.Text>
      </Space>
    );
  }

  if (gateState === 'error') {
    return (
      <Result
        status="warning"
        title={AUTHORIZATION_ERROR_TITLE}
        subTitle="无法确认当前会话的权限范围，系统已按最小权限策略阻止访问。"
        extra={
          <Button type="primary" onClick={() => void authorization.refresh()}>
            重新获取权限
          </Button>
        }
      />
    );
  }

  if (gateState === 'forbidden') {
    return (
      <AdminForbiddenPage
        title="无权访问该管理页面"
        description={requiredPermission ? `当前页面需要 ${requiredPermission} 权限。` : undefined}
      />
    );
  }

  return children;
}

function AppShell() {
  const location = useLocation();
  const authorization = useAdminAuthorization();

  return (
    <AppLayout
      currentPath={location.pathname}
      navItems={buildVisibleNavItems(adminRouteMeta, authorization.can)}
    >
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

function withPermission(
  path: string,
  element: ReactElement
) {
  const meta = adminRouteMeta.find((item) => item.path === path);
  return (
    <PermissionBoundary requiredPermission={meta?.requiredPermission}>
      {element}
    </PermissionBoundary>
  );
}

export function createAppRouter() {
  return createBrowserRouter([
    {
      path: '/login',
      element: <LoginPage />,
    },
    {
      path: '/',
      element: <RequireSession />,
      children: [
        {
          index: true,
          element: withPermission('/', <DashboardPage />),
        },
        {
          path: 'users',
          element: withPermission('/users', <UserManagementPage />),
        },
        {
          path: 'devices',
          element: withPermission('/devices', <DeviceManagementPage />),
        },
        {
          path: 'activation-codes',
          element: withPermission('/activation-codes', <ActivationCodesPage />),
        },
        {
          path: 'model-pools',
          element: withPermission('/model-pools', <ModelPoolPage />),
        },
        {
          path: 'model-leases',
          element: withPermission('/model-leases', <ModelLeasesPage />),
        },
        {
          path: 'audit-logs',
          element: withPermission('/audit-logs', <AuditLogsPage />),
        },
        {
          path: 'security',
          element: withPermission('/security', <AdminSecurityPage />),
        },
        {
          path: 'roles',
          element: withPermission('/roles', <AdminRbacPage />),
        },
        {
          path: '404',
          element: <NotFoundPage />,
        },
        {
          path: '*',
          element: <Navigate to="/404" replace />,
        },
      ],
    },
  ]);
}

export const router = createAppRouter();
