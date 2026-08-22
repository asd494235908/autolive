import type { AdminPermissionCode, ProductCode } from '../../types/api';

export const FORBIDDEN_RESULT_TEXT = '无权访问当前页面';

export type RoleDraft = {
  code?: string | null;
  name?: string | null;
  product?: ProductCode;
  permissions?: AdminPermissionCode[] | null;
};

type AdminProductAuthorization = {
  isLoading: boolean;
  error: unknown;
  product: ProductCode | null;
  isSuperAdmin: boolean;
};

export function getPermissionKeysFromTreeEvent(
  event: unknown,
  availablePermissions: readonly AdminPermissionCode[]
) {
  const checkedValue =
    Array.isArray(event)
      ? event
      : event && typeof event === 'object' && 'checked' in event
        ? (event as { checked?: unknown }).checked
        : [];
  const checkedKeys = Array.isArray(checkedValue) ? checkedValue : [];
  const allowed = new Set(availablePermissions);

  return checkedKeys.filter(
    (key): key is AdminPermissionCode => typeof key === 'string' && allowed.has(key as AdminPermissionCode)
  );
}

export function resolveAdminProductScope(
  authorization: AdminProductAuthorization,
  currentSelection: ProductCode | null
) {
  if (authorization.isLoading || authorization.error != null || authorization.product === null) {
    return null;
  }

  return authorization.isSuperAdmin ? currentSelection ?? authorization.product : authorization.product;
}

export function validateRoleDraft(values: RoleDraft) {
  const errors: Array<[string, string]> = [];
  const code = values.code?.trim() ?? '';
  const name = values.name?.trim() ?? '';

  if (!/^[a-z0-9_]+$/.test(code)) {
    errors.push(['code', '角色代码只能包含小写字母、数字和下划线']);
  }

  if (!name) {
    errors.push(['name', '请输入角色名称']);
  }

  if (!values.product) {
    errors.push(['product', '请选择产品范围']);
  }

  if (!values.permissions || values.permissions.length === 0) {
    errors.push(['permissions', '至少选择一个权限']);
  }

  return errors;
}

export function createRetryableSubmission(createKey: () => string) {
  let activeKey: string | undefined;

  return {
    current() {
      if (!activeKey) {
        activeKey = createKey();
      }
      return activeKey;
    },
    reset() {
      activeKey = undefined;
    },
  };
}

const DOMAIN_LABELS: Record<string, string> = {
  dashboard: '系统概览',
  users: '用户',
  roles: '角色与授权',
  admin_security: '管理员安全',
  devices: '设备',
  activation_codes: '激活码',
  model_pool: '模型号池',
  model_leases: '模型租约',
  model_usage: '模型用量',
  audit_logs: '审计日志',
  operations: '运维',
  plans: '套餐',
  orders: '订单',
  subscriptions: '订阅',
  payments: '支付',
  password_resets: '密码重置',
  artifacts: '制品',
  public_config: '公共配置',
  error_reports: '错误报告',
  feedback: '反馈',
};

export function groupPermissionsByDomain(permissions: AdminPermissionCode[]) {
  const groups = new Map<string, AdminPermissionCode[]>();

  [...permissions].sort().forEach((permission) => {
    const domain = permission.split('.')[0] ?? 'other';
    const list = groups.get(domain);
    if (list) {
      list.push(permission);
      return;
    }
    groups.set(domain, [permission]);
  });

  return Object.fromEntries(groups);
}

export function buildPermissionTreeData(permissions: AdminPermissionCode[]) {
  const groups = groupPermissionsByDomain(permissions);

  return Object.entries(groups).map(([domain, items]) => ({
    key: domain,
    title: DOMAIN_LABELS[domain] ?? domain,
    selectable: false,
    children: items.map((permission) => ({
      key: permission,
      title: permission,
    })),
  }));
}

export function getDomainLabel(domain: string) {
  return DOMAIN_LABELS[domain] ?? domain;
}
