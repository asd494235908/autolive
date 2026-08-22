import test from 'node:test';
import assert from 'node:assert/strict';
import { QueryClient } from '@tanstack/react-query';

import {
  ADMIN_AUTHORIZATION_QUERY_KEY,
  buildAdminAuthorizationState,
  createAdminAuthorizationQueryOptions,
} from './useAdminAuthorization.ts';
import {
  adminRouteMeta,
  buildVisibleNavItems,
  getPermissionGateState,
} from './adminRouteMeta.ts';
import {
  FORBIDDEN_RESULT_TEXT,
  createRetryableSubmission,
  getPermissionKeysFromTreeEvent,
  groupPermissionsByDomain,
  resolveAdminProductScope,
  validateRoleDraft,
} from './adminRbacModel.ts';
import {
  AUTHORIZATION_ERROR_TITLE,
} from './useAdminAuthorization.ts';

test('权限集合缺失、加载中和错误都必须 fail-closed', () => {
  const missing = buildAdminAuthorizationState({
    isLoading: false,
    error: null,
    data: null,
    refresh: async () => undefined,
  });
  assert.equal(missing.can('users.read'), false);
  assert.deepEqual(missing.permissions, []);
  assert.deepEqual(missing.roles, []);
  assert.equal(missing.isSuperAdmin, false);

  const loading = buildAdminAuthorizationState({
    isLoading: true,
    error: null,
    data: {
      product: 'autolive',
      permissions: ['users.read'],
      role_codes: ['viewer'],
      global_super_admin: false,
    },
    refresh: async () => undefined,
  });
  assert.equal(loading.can('users.read'), false);

  const failed = buildAdminAuthorizationState({
    isLoading: false,
    error: new Error('boom'),
    data: {
      product: 'autolive',
      permissions: ['users.read'],
      role_codes: ['viewer'],
      global_super_admin: false,
    },
    refresh: async () => undefined,
  });
  assert.equal(failed.can('users.read'), false);
});

test('菜单过滤和直接 URL 守卫复用同一份权限元数据', () => {
  const usersMeta = adminRouteMeta.find((route) => route.path === '/users');
  const rbacMeta = adminRouteMeta.find((route) => route.path === '/roles');
  assert.equal(usersMeta?.requiredPermission, 'users.read');
  assert.equal(rbacMeta?.requiredPermission, 'roles.read');

  const visible = buildVisibleNavItems(adminRouteMeta, (permission) =>
    permission === 'dashboard.read' || permission === 'users.read'
  );
  assert.deepEqual(
    visible.map((item) => item.path),
    ['/', '/users']
  );

  assert.equal(
    getPermissionGateState(
      {
        isLoading: false,
        error: null,
        can: (permission) => permission === 'users.read',
      },
      usersMeta?.requiredPermission
    ),
    'allowed'
  );
  assert.equal(
    getPermissionGateState(
      {
        isLoading: false,
        error: null,
        can: () => false,
      },
      usersMeta?.requiredPermission
    ),
    'forbidden'
  );
});

test('403 fixture 显示无权限结果页', () => {
  assert.equal(FORBIDDEN_RESULT_TEXT, '无权访问当前页面');
  assert.equal(AUTHORIZATION_ERROR_TITLE, '管理员授权加载失败');
});

test('权限变更后 refresh 会重新取数', async () => {
  let current = {
    request_id: 'req_1',
    user: { id: 'usr_1', username: 'alice', role: 'admin', status: 'active', created_at: '2026-08-22T00:00:00Z' },
    product: 'autolive',
    global_super_admin: false,
    role_codes: ['viewer'],
    permissions: ['users.read'],
  };
  const calls = [];
  const options = createAdminAuthorizationQueryOptions(async () => {
    calls.push(current.request_id);
    return current;
  });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });

  const first = await client.fetchQuery(options);
  assert.deepEqual(first.permissions, ['users.read']);

  current = {
    ...current,
    request_id: 'req_2',
    role_codes: ['viewer', 'manager'],
    permissions: ['users.read', 'users.manage'],
  };
  await client.invalidateQueries({ queryKey: ADMIN_AUTHORIZATION_QUERY_KEY });
  const second = await client.fetchQuery(options);
  assert.deepEqual(second.permissions, ['users.manage', 'users.read']);
  assert.deepEqual(calls, ['req_1', 'req_2']);
});

test('角色表单校验和提交重试辅助符合预期', () => {
  assert.deepEqual(
    validateRoleDraft({
      code: 'bad code',
      name: ' ',
      product: undefined,
      permissions: [],
    }),
    [
      ['code', '角色代码只能包含小写字母、数字和下划线'],
      ['name', '请输入角色名称'],
      ['product', '请选择产品范围'],
      ['permissions', '至少选择一个权限'],
    ]
  );

  const submission = createRetryableSubmission(() => 'req_fixed');
  assert.equal(submission.current(), 'req_fixed');
  assert.equal(submission.current(), 'req_fixed');
  submission.reset();
  assert.equal(submission.current(), 'req_fixed');

  const groups = groupPermissionsByDomain([
    'users.read',
    'users.manage',
    'model_pool.test',
    'roles.assign',
  ]);
  assert.deepEqual(Object.keys(groups), ['model_pool', 'roles', 'users']);
  assert.deepEqual(groups.users, ['users.manage', 'users.read']);
});

test('权限 Tree 的数组和 checked 对象事件都会回写为权限集合', () => {
  const availablePermissions = ['users.read', 'users.manage', 'roles.assign'];

  assert.deepEqual(
    getPermissionKeysFromTreeEvent(['users', 'users.read'], availablePermissions),
    ['users.read']
  );
  assert.deepEqual(
    getPermissionKeysFromTreeEvent(
      { checked: ['roles', 'roles.assign'], halfChecked: ['users'] },
      availablePermissions
    ),
    ['roles.assign']
  );
});

test('产品范围授权未就绪时不产生查询范围，非全局管理员遵循授权产品', () => {
  assert.equal(
    resolveAdminProductScope(
      { isLoading: true, error: null, product: null, isSuperAdmin: false },
      'autolive'
    ),
    null
  );
  assert.equal(
    resolveAdminProductScope(
      { isLoading: false, error: new Error('authorization unavailable'), product: null, isSuperAdmin: false },
      'autolive'
    ),
    null
  );
  assert.equal(
    resolveAdminProductScope(
      { isLoading: false, error: null, product: 'douyin_desktop', isSuperAdmin: false },
      'autolive'
    ),
    'douyin_desktop'
  );
  assert.equal(
    resolveAdminProductScope(
      { isLoading: false, error: null, product: 'autolive', isSuperAdmin: true },
      'douyin_desktop'
    ),
    'douyin_desktop'
  );
});
