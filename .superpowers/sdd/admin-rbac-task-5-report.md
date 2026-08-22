# Admin RBAC Task 5 报告

## 改动文件

- `backend/internal/httpapi/admin_rbac.go`
- `backend/internal/httpapi/admin_rbac_test.go`
- `backend/internal/httpapi/auth.go`
- `backend/internal/httpapi/controlplane.go`
- `backend/internal/httpapi/router.go`
- `接口契约/openapi.yaml`
- `admin-web/src/api/openapi.generated.ts`

## 实际执行命令与结果

- `cd backend && go test ./internal/httpapi -run 'AdminRBAC|Permission|Contract|Router' -count=1`
  - 初次执行失败，确认了 `/api/v1/admin/me`、RBAC API 和 `requirePermission` 路由矩阵缺失。
  - 最终执行通过。
- `cd backend && gofmt -w internal/httpapi/admin_rbac.go internal/httpapi/admin_rbac_test.go internal/httpapi/auth.go internal/httpapi/controlplane.go internal/httpapi/router.go`
  - 通过。
- `cd admin-web && pnpm api:generate`
  - 通过，已更新 `src/api/openapi.generated.ts`。
- `cd admin-web && pnpm api:check`
  - 通过。
- `git diff --check`
  - 通过。
- `rg -n 'auth\\.requireAdmin\\(' backend/internal/httpapi`
  - 未命中，说明生产路由里已没有直接 `auth.requireAdmin(...)` 调用。

## 本次完成内容

- 新增 `/api/v1/admin/me`、`/api/v1/admin/permissions`、角色 CRUD、用户角色读取/替换 HTTP handler。
- 新增 `requirePermission`，从 Bearer Session 读取 actor，并按会话产品做即时权限判定；读取失败返回 503 fail-closed。
- 将现有管理路由从 `requireAdmin` 切换到固定权限点：用户、设备、激活码、模型池、模型用量、模型租约、审计、安全。
- 补充 Task 5 聚焦 HTTP/OpenAPI/Router 测试，覆盖：
  - `/admin/me` 的 401、普通用户空权限、即时权限返回、fail-closed。
  - 权限目录、角色 CRUD、用户角色绑定 API。
  - `user.role != admin` 但拥有 RBAC 权限时的真实 Router 放行。
- 更新 OpenAPI 契约并重新生成 `admin-web` API 类型。

## 未验证项

- 未运行 `go test ./...`。
- 未运行桌面端任何脚本或验证。
- 未运行 `admin-web` 的 `typecheck`、`test`、`build`；本 Task 按要求只执行了 `api:generate` 与 `api:check`。

## 剩余风险

- Task 5 只补了管理 RBAC 的最小闭环；更细的 OpenAPI 字段约束、前端消费细节和页面权限壳由 Task 6 继续承接。
- 现有更广范围的历史测试未全量回归，因此跨模块回归风险仍依赖后续任务或整体验证门禁发现。

## 后续建议

- Task 6 直接消费新的 `/api/v1/admin/me` 和 RBAC API，不再手写权限快照。
- 后续若执行更大范围回归，优先跑服务端全量 HTTP 相关测试与 `admin-web` typecheck/test。
