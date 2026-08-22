# Task 1 实现报告

## 状态

已完成当前任务要求的 RBAC Phase 2 第一个实现切片，并保留当前工作区中的 `admin_rbac.go`、`admin_rbac_test.go` 与稳定错误码改动。未创建 worktree，未触碰 `desktop/` 或 `desktop/ui/`。

## 改动文件

- `backend/internal/controlplane/admin_rbac.go`
  - 新增固定权限目录 `PermissionCatalog()`。
  - 新增 `PermissionCode`、`AdminRole`、`AdminRoleAssignment`、`AdminAuthorization`。
  - 新增 `BuiltinAdminRoleSuperAdmin` 与 `NormalizePermissionCodes([]string)`。
- `backend/internal/controlplane/admin_rbac_test.go`
  - 新增权限目录无重复、目录防御性副本、权限规范化、未知权限错误、全局 `super_admin` 表达和稳定错误码测试。
- `backend/internal/controlplane/errors.go`
  - 新增 RBAC 稳定错误码：
    - `ADMIN_PERMISSION_UNKNOWN`
    - `ADMIN_PERMISSION_DENIED`
    - `ADMIN_ROLE_NOT_FOUND`
    - `ADMIN_BUILTIN_ROLE_IMMUTABLE`
    - `ADMIN_ROLE_ASSIGNED`
    - `ADMIN_ROLE_DELEGATION_FORBIDDEN`
    - `ADMIN_LAST_SUPER_ADMIN_PROTECTED`
    - `ADMIN_PRODUCT_SCOPE_MISMATCH`
- `接口契约/错误码.md`
  - 同步新增 RBAC 稳定错误码文档。
  - 该文件本次保留，原因是新增稳定错误码后需要同步错误码契约文档，且已通过错误码契约测试验证。

## TDD 记录

### RED

命令：

```bash
cd backend && go test ./internal/controlplane -run 'TestPermissionCatalog|TestNormalizePermissionCodes' -count=1
```

输出：

```text
# autoLive/backend/internal/controlplane [autoLive/backend/internal/controlplane.test]
internal/controlplane/admin_rbac_test.go:9:13: undefined: PermissionCatalog
internal/controlplane/admin_rbac_test.go:14:19: undefined: PermissionCode
internal/controlplane/admin_rbac_test.go:15:18: undefined: PermissionCode
internal/controlplane/admin_rbac_test.go:36:11: undefined: PermissionCatalog
internal/controlplane/admin_rbac_test.go:43:14: undefined: NormalizePermissionCodes
internal/controlplane/admin_rbac_test.go:48:15: undefined: NormalizePermissionCodes
internal/controlplane/admin_rbac_test.go:54:10: undefined: AdminAuthorization
internal/controlplane/admin_rbac_test.go:57:30: undefined: BuiltinAdminRoleSuperAdmin
internal/controlplane/admin_rbac_test.go:58:21: undefined: PermissionCatalog
internal/controlplane/admin_rbac_test.go:67:38: undefined: BuiltinAdminRoleSuperAdmin
internal/controlplane/admin_rbac_test.go:67:38: too many errors
FAIL	autoLive/backend/internal/controlplane [build failed]
FAIL
```

结果：失败，且失败原因符合预期，证明测试先于实现落地。

### GREEN / 验证

实际执行命令与结果：

```bash
gofmt -w /Users/mac/work/gepin/autoLive/backend/internal/controlplane/admin_rbac.go /Users/mac/work/gepin/autoLive/backend/internal/controlplane/admin_rbac_test.go /Users/mac/work/gepin/autoLive/backend/internal/controlplane/errors.go
```

```text
无输出
```

```bash
cd backend && go test ./internal/controlplane -run 'TestPermissionCatalog|TestNormalizePermissionCodes|TestAdminAuthorizationAllowsGlobalSuperAdminScope|TestAdminRBACStableErrors' -count=1
```

```text
ok  	autoLive/backend/internal/controlplane	1.127s
```

```bash
cd backend && go test ./internal/controlplane -run 'TestPermissionCatalog|TestNormalizePermissionCodes' -count=1
```

```text
ok  	autoLive/backend/internal/controlplane	0.420s
```

```bash
cd backend && go test ./internal/httpapi -run TestGoErrorCodesMatchErrorCodeDocument -count=1
```

```text
ok  	autoLive/backend/internal/httpapi	1.060s
```

```bash
gofmt -l /Users/mac/work/gepin/autoLive/backend/internal/controlplane/admin_rbac.go /Users/mac/work/gepin/autoLive/backend/internal/controlplane/admin_rbac_test.go /Users/mac/work/gepin/autoLive/backend/internal/controlplane/errors.go
```

```text
无输出
```

## 自审结论

- 本次只交付“固定权限目录 + 领域类型 + 稳定错误码”，没有提前把服务层、路由或数据库切到 RBAC，边界与 `task-1-brief.md` 一致。
- `NormalizePermissionCodes` 使用去重后稳定排序，满足测试要求和后续持久化稳定性。
- `PermissionCatalog()` 返回防御性副本，避免调用方误改全局目录。
- 错误码文档同步保留，因为这是当前仓库下稳定错误码的契约事实源之一；若不更新，错误码契约测试会漂移。

## 提交

- 代码提交 SHA：`b0f5993`
- 提交信息：`feat: define admin RBAC domain catalog`

## 风险 / 疑问

- 当前任务尚未把旧 `permissionsForRole(user.Role)` 接口替换为 RBAC 鉴权，这属于后续任务范围，不在本次实现内。
- `PermissionCode` 采用 `string` 别名以兼容现有测试和后续字符串型 DTO/契约；如果后续需要更强类型约束，再单独评估切换成本。
- 本次仅运行了任务要求的简短测试与错误码契约测试，未运行 `go test ./...` 全量验证。
