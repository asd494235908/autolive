import { useQuery, useQueryClient } from '@tanstack/react-query';
import { apiClient } from '../../api/client';
import type {
  AdminMeResponse,
  AdminPermissionCode,
  ProductCode,
  UserSummary,
} from '../../types/api';

export const ADMIN_AUTHORIZATION_QUERY_KEY = ['admin-me'] as const;
export const AUTHORIZATION_ERROR_TITLE = '管理员授权加载失败';

export type AdminAuthorizationData = {
  request_id?: string;
  user?: UserSummary;
  product: ProductCode;
  global_super_admin: boolean;
  role_codes: string[];
  permissions: AdminPermissionCode[];
};

type AdminAuthorizationStateInput = {
  isLoading: boolean;
  error: unknown;
  data: AdminAuthorizationData | null | undefined;
  refresh: () => Promise<unknown>;
};

const VALID_PRODUCTS = new Set<ProductCode>(['autolive', 'douyin_desktop']);

function isAdminPermissionCode(value: string): value is AdminPermissionCode {
  return value.includes('.');
}

export function normalizeAdminAuthorization(
  payload: AdminMeResponse | null | undefined
): AdminAuthorizationData {
  if (!payload || typeof payload !== 'object') {
    throw new Error('管理员授权响应缺失');
  }

  if (!VALID_PRODUCTS.has(payload.product)) {
    throw new Error('管理员授权响应缺少产品范围');
  }

  if (!Array.isArray(payload.role_codes) || !payload.role_codes.every((value) => typeof value === 'string')) {
    throw new Error('管理员授权响应缺少角色编码');
  }

  if (
    !Array.isArray(payload.permissions) ||
    !payload.permissions.every((value) => typeof value === 'string' && isAdminPermissionCode(value))
  ) {
    throw new Error('管理员授权响应缺少权限集合');
  }

  return {
    request_id: payload.request_id,
    user: payload.user,
    product: payload.product,
    global_super_admin: payload.global_super_admin === true,
    role_codes: [...payload.role_codes],
    permissions: [...payload.permissions].sort(),
  };
}

export function createAdminAuthorizationQueryOptions(
  fetcher: () => Promise<AdminMeResponse> = () =>
    apiClient.get<AdminMeResponse>('/api/v1/admin/me')
) {
  return {
    queryKey: ADMIN_AUTHORIZATION_QUERY_KEY,
    queryFn: async () => normalizeAdminAuthorization(await fetcher()),
  };
}

export function buildAdminAuthorizationState({
  isLoading,
  error,
  data,
  refresh,
}: AdminAuthorizationStateInput) {
  const allowed = !isLoading && error == null && data !== null && data !== undefined;
  const permissions = allowed ? data.permissions : [];
  const roles = allowed ? data.role_codes : [];
  const isSuperAdmin = allowed ? data.global_super_admin : false;

  return {
    isLoading,
    error,
    product: allowed ? data.product : null,
    user: allowed ? data.user ?? null : null,
    requestId: allowed ? data.request_id ?? null : null,
    permissions,
    roles,
    isSuperAdmin,
    can(permission: AdminPermissionCode) {
      return allowed && permissions.includes(permission);
    },
    refresh,
  };
}

export function useAdminAuthorization() {
  const queryClient = useQueryClient();
  const query = useQuery(createAdminAuthorizationQueryOptions());

  return buildAdminAuthorizationState({
    isLoading: query.isLoading,
    error: query.error,
    data: query.data,
    refresh: async () => {
      await queryClient.invalidateQueries({ queryKey: ADMIN_AUTHORIZATION_QUERY_KEY });
      return query.refetch();
    },
  });
}
