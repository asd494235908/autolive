import type { AdminPermissionCode } from '../../types/api';

export type AdminRouteMetaItem = {
  key: string;
  label: string;
  path: string;
  nav: boolean;
  requiredPermission?: AdminPermissionCode;
};

type PermissionGateAuth = {
  isLoading: boolean;
  error: unknown;
  can: (permission: AdminPermissionCode) => boolean;
};

export const adminRouteMeta: AdminRouteMetaItem[] = [
  { key: 'dashboard', label: '系统概览', path: '/', nav: true, requiredPermission: 'dashboard.read' },
  { key: 'users', label: '用户管理', path: '/users', nav: true, requiredPermission: 'users.read' },
  { key: 'devices', label: '设备管理', path: '/devices', nav: true, requiredPermission: 'devices.read' },
  { key: 'activation-codes', label: '激活码', path: '/activation-codes', nav: true, requiredPermission: 'activation_codes.read' },
  { key: 'model-pools', label: '号池管理', path: '/model-pools', nav: true, requiredPermission: 'model_pool.read' },
  { key: 'model-leases', label: '模型租约', path: '/model-leases', nav: true, requiredPermission: 'model_leases.read' },
  { key: 'audit-logs', label: '审计日志', path: '/audit-logs', nav: true, requiredPermission: 'audit_logs.read' },
  { key: 'security', label: '管理员安全', path: '/security', nav: true, requiredPermission: 'admin_security.manage' },
  { key: 'roles', label: 'RBAC', path: '/roles', nav: true, requiredPermission: 'roles.read' },
];

export function buildVisibleNavItems(
  items: AdminRouteMetaItem[],
  can: (permission: AdminPermissionCode) => boolean
) {
  return items.filter((item) => item.nav && item.requiredPermission && can(item.requiredPermission));
}

export function getPermissionGateState(
  authorization: PermissionGateAuth,
  requiredPermission?: AdminPermissionCode
) {
  if (!requiredPermission) {
    return 'allowed';
  }

  if (authorization.isLoading) {
    return 'loading';
  }

  if (authorization.error) {
    return 'error';
  }

  return authorization.can(requiredPermission) ? 'allowed' : 'forbidden';
}
