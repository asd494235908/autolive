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
   - `ListAdminPermissions` 改为显式 `LIMIT $1`。
   - `ListAdminRoles` 改为显式 `LIMIT $2`。
   - `ListUserAdminRoles` 改为显式 `LIMIT $3`。
   - `ReplaceUserAdminRoles` 的幂等回放查询也同步使用带 `LIMIT` 的列表 SQL，避免回放路径绕开有界约束。

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
  - `ListAdminRoles` 显式 `LIMIT`
  - `ReplaceUserAdminRoles` 幂等回放列表查询显式 `LIMIT`
  - 现有 `ListUserAdminRoles` 过滤测试显式 `LIMIT`

## 执行命令

1. `cd backend && go test ./internal/store -run 'TestPostgresRepository(ListAdminPermissionsUsesBoundedQuery|ListUserAdminRolesAddsCompatibilityLocalSuperAdminAndUsesLimit|AdminRoleCRUDUsesNormalizedTransactions)$' -count=1`
   - 结果：通过

2. `gofmt -w backend/internal/store/postgres_admin_rbac_repository.go backend/internal/store/postgres_admin_rbac_repository_test.go`
   - 结果：通过

3. `cd backend && go test ./internal/store -run 'AdminRBAC|RBAC' -count=1`
   - 结果：通过

## 未验证项

- 未运行 `cd backend && go test ./...`
- 未连接真实 PostgreSQL 做集成验证；本次按任务要求只做 store 层聚焦 SQLMock 测试

