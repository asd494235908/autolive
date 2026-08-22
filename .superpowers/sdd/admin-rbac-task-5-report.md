# Admin RBAC Task 5 报告

## 改动文件

- `backend/internal/httpapi/contract_test.go`
- `backend/internal/httpapi/product_scope_integration_test.go`
- `接口契约/openapi.yaml`
- `admin-web/src/api/openapi.generated.ts`
- `.superpowers/sdd/admin-rbac-task-5-report.md`

## 实际执行命令与结果

- `cd backend && go test ./internal/httpapi -run 'TestOpenAPIAdminPermissionGuardRoutesDeclareServiceUnavailable|TestAdminUserListProductScopeReturnsActualItemsAndTotals|TestOrdinaryUsersReadCannotWidenUserListAcrossProducts' -count=1`
  - 初次执行失败：
    - OpenAPI 缺少多条 `requirePermission` 管理路由的 `503 ServiceUnavailable` 响应声明。
    - `TestAdminUserListProductScopeReturnsActualItemsAndTotals` 返回 `403 ADMIN_PERMISSION_DENIED`，根因是 `usr_product_admin` 只有旧 `role=admin`，没有真实 RBAC `users.read` 绑定。
  - 修改后重跑通过。
- `cd backend && gofmt -w internal/httpapi/contract_test.go internal/httpapi/product_scope_integration_test.go`
  - 通过。
- `cd backend && go test ./internal/httpapi -run 'AdminRBAC|Permission|Contract|Router|ProductScope' -count=1`
  - 首次在更新 fixture 后失败，暴露 `product_scope_integration_test.go` 中多条“旧 role=admin 自动具备设备/激活码/模型/审计读取能力”的过时断言。
  - 按显式 RBAC 语义收敛测试后通过。
- `cd admin-web && pnpm api:generate`
  - 通过；`src/api/openapi.generated.ts` 已同步新增多个 `503: ServiceUnavailable` 响应类型。
- `cd admin-web && pnpm api:check`
  - 通过。
- `git diff --check`
  - 通过。

## 本次完成内容

- 在 `contract_test.go` 新增 fail-closed 契约测试，明确要求所有关键 `auth.requirePermission` 管理路由声明 `503`，并复用现有 `#/components/responses/ServiceUnavailable`。
- 为以下管理 path 补全实际可能的 `503 ADMIN_AUTHORIZATION_UNAVAILABLE` OpenAPI 响应声明：
  - `GET /api/v1/admin/users/{user_id}/devices`
  - `GET /api/v1/admin/devices/{device_id}`
  - `POST /api/v1/admin/activation-codes/{code_id}/revoke`
  - `GET /api/v1/admin/model-pool`
  - `POST /api/v1/admin/model-pool/{account_id}/disable`
  - `PATCH /api/v1/admin/model-pool/{account_id}`
  - `POST /api/v1/admin/model-pool/{account_id}/test`
  - `GET /api/v1/admin/model-usage`
  - `GET /api/v1/admin/model-leases`
  - `GET /api/v1/admin/model-leases/{lease_id}`
  - `POST /api/v1/admin/model-leases/{lease_id}/reclaim`
  - `GET /api/v1/admin/audit-logs`
  - 同时确认 `GET /api/v1/admin/users` 与角色相关 RBAC 路由保留该声明。
- 在 `newProductScopeIntegrationRouter` 中通过真实 `MemoryStore` RBAC API 创建 `autolive` 角色并给 `usr_product_admin` 绑定 `users.read`，不恢复任意 `role=admin` 自动授权。
- 补充普通 `users.read` 场景测试：
  - 用户列表默认产品与 `product=autolive` 正向返回 200。
  - 用户列表跨产品 widening 返回 403。
  - `users.read` 允许访问 `/api/v1/admin/users/{user_id}/devices`，但不会隐式获得设备/激活码/模型/审计等其它管理页权限。

## 未验证项

- 未运行 `go test ./...`。
- 未运行桌面端任何脚本或验证。
- 未运行 `admin-web` 的 `typecheck`、`test`、`build`；本 Task 按要求只执行了 `api:generate` 与 `api:check`。

## 剩余风险

- 当前只回归了 `internal/httpapi` 的 Task 5 聚焦集合，没有跑服务端全量测试，所以更广泛的 HTTP/Service/Store 回归仍要依赖后续门禁。
- 本次 `api:generate` 已把新增 `503` 响应反映到前端类型；若后续前端开始显式消费这些分支，还需要页面侧单独补交互处理。

## 后续建议

- Task 6/后续前端权限壳开发时，把 `ADMIN_AUTHORIZATION_UNAVAILABLE` 视为独立故障态，不要和普通 403 合并处理。
- 如果后续继续清理产品范围测试，建议把“具备某权限才可读某页面”的断言拆成按权限点分组的 fixture，避免再次混入旧 `role=admin` 语义。
