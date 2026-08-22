# Task 3 修复报告

日期：2026-08-22

## 修复范围

- 只修改 `backend/internal/store/postgres_admin_rbac_repository.go`
- 只修改 `backend/internal/store/postgres_admin_rbac_repository_test.go`
- 新增本报告 `.superpowers/sdd/admin-rbac-task-3-fix-report.md`
- 未修改桌面端、migration 正文、计划文档

## 修复内容

1. 修复 `ListUserAdminRoles` 与 Memory 的兼容超级管理员行为不一致
   - PostgreSQL 实现原先只返回 `user_admin_roles` 表中的绑定。
   - 现在会先读取用户；当用户满足 `compatibilityLocalSuperAdmin`（`usr_local_admin`、`role=admin`、`status=active`）时，若结果里没有全局 `super_admin`，则补出一条全局 `super_admin` 绑定。
   - 这样 `ListUserAdminRoles` 与 Memory 实现、`GetAdminAuthorization` 的兼容行为保持一致。

2. 为三个列表查询增加固定上限
   - 在 store 层新增私有常量 `adminRBACListLimit = 200`。
   - 进一步新增 `adminRBACScopedRoleLimit = 199`，专门用于 `ListAdminRoles` 的普通角色窗口。
   - `ListAdminPermissions` 改为显式 `LIMIT $1`。
   - `ListAdminRoles` 最初改为显式 `LIMIT $2`，但 reviewer 复核发现该 `LIMIT` 直接作用在角色×权限 JOIN 结果上，可能截断后续角色或某个角色的权限。
   - 现已进一步修正为：先在 `admin_roles` 子查询/CTE 中按产品过滤、全局 `super_admin` 可见和稳定排序选出固定上限的角色集合，再对该角色集合 `LEFT JOIN admin_role_permissions` 聚合完整权限。
   - 最终版本继续收紧为：`global_role` 单独取全局 `super_admin`，`scoped_roles` 只取普通角色并固定 `LIMIT 199`，之后 `UNION ALL` 成 `limited_roles` 再聚合权限。这样总上限仍是 200，但无论普通角色有多少，都会为全局 `super_admin` 保留一个槽位；`product=''` 的全局列表也保持同样语义。
   - `ListUserAdminRoles` 改为显式 `LIMIT $3`。
   - `ReplaceUserAdminRoles` 的幂等回放查询也同步使用带 `LIMIT` 的列表 SQL，避免回放路径绕开有界约束；同时幂等回放现在会复用已锁定用户的 `withCompatibilityLocalSuperAdmin` 兼容补偿，不再绕过本地超级管理员列表一致性。

3. 稳定同文件现有 PostgreSQL SQLMock 聚焦测试
   - `GetAdminAuthorization` 全量权限断言改为与仓库实际排序语义一致的 `normalizedCatalogPermissions()`。
   - `loadAssignableRoles` 读前对角色码排序，消除 `ANY($1)` 参数顺序抖动。

## 回归测试

- 新增 `TestPostgresRepositoryListAdminPermissionsUsesBoundedQuery`
  - 钉住 `ListAdminPermissions` 显式 `LIMIT`
- 新增 `TestPostgresRepositoryListUserAdminRolesAddsCompatibilityLocalSuperAdminAndUsesLimit`
  - 钉住 `usr_local_admin` 自动补全局 `super_admin`
  - 钉住 `ListUserAdminRoles` 显式 `LIMIT`
- 更新现有 SQLMock 断言
  - `ListAdminRoles` 显式 `LIMIT` 且 `LIMIT` 位于 role 子查询/CTE，不再直接截断 join 行
  - `ReplaceUserAdminRoles` 幂等回放列表查询显式 `LIMIT`
  - 现有 `ListUserAdminRoles` 过滤测试显式 `LIMIT`
- 新增 `TestPostgresRepositoryListAdminRolesLimitsRoleSetBeforeJoiningPermissions`
  - 钉住 `ListAdminRoles` 采用“先限角色集合、后连权限”的 SQL 结构
  - 使用 203 条返回行验证聚合逻辑不会因 join 行数量超过 200 而丢失后续角色或其完整权限
- 新增 `TestPostgresRepositoryListAdminRolesReservesSlotForGlobalSuperAdmin`
  - 钉住 `ListAdminRoles` 采用 `global_role + scoped_roles + UNION ALL` 的 SQL 结构
  - 钉住普通角色窗口参数为 `199`，确保总上限 200 下仍为全局 `super_admin` 保留槽位
  - 验证返回的 199 个普通角色和 `super_admin` 都能完整聚合权限
- 新增 `TestPostgresRepositoryReplaceUserAdminRolesReplayAddsCompatibilityLocalSuperAdmin`
  - 钉住 `ReplaceUserAdminRoles` 的幂等回放不会绕过 `usr_local_admin` 的兼容 `super_admin` 补偿

## 执行命令

1. `cd backend && go test ./internal/store -run 'TestPostgresRepository(ListAdminPermissionsUsesBoundedQuery|ListUserAdminRolesAddsCompatibilityLocalSuperAdminAndUsesLimit|AdminRoleCRUDUsesNormalizedTransactions)$' -count=1`
   - 结果：通过

2. `gofmt -w backend/internal/store/postgres_admin_rbac_repository.go backend/internal/store/postgres_admin_rbac_repository_test.go`
   - 结果：通过

3. `cd backend && go test ./internal/store -run 'AdminRBAC|RBAC' -count=1`
   - 结果：通过

4. `git diff --check`
   - 结果：通过

5. `cd backend && go test ./internal/store -run 'AdminRBAC|RBAC' -count=1`
   - 最终边界修复后再次通过

## 未验证项

- 未运行 `cd backend && go test ./...`
- 未连接真实 PostgreSQL 做集成验证；本次按任务要求只做 store 层聚焦 SQLMock 测试
