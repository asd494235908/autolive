# Admin RBAC Task 5 报告

## 改动文件

- `backend/internal/httpapi/contract_test.go`
- `接口契约/openapi.yaml`
- `.superpowers/sdd/admin-rbac-task-5-report.md`

## 实际执行命令与结果

- `cd backend && go test ./internal/httpapi -run 'TestOpenAPIAdminRequirePermissionRoutesDeclareAdminAuthorizationServiceUnavailable' -count=1`
  - 首次执行失败：`ServiceUnavailable` 响应组件没有文档化 `ADMIN_AUTHORIZATION_UNAVAILABLE` 示例。
  - 补充 OpenAPI 示例后重跑通过。
- `cd backend && gofmt -w internal/httpapi/contract_test.go`
  - 通过。
- `cd admin-web && pnpm api:generate`
  - 通过；重新生成后确认 `admin-web/src/api/openapi.generated.ts` 无差异，无需手改生成产物。
- `cd backend && go test ./internal/httpapi -run 'AdminRBAC|Permission|Contract|Router|ProductScope' -count=1`
  - 通过。
- `cd admin-web && pnpm api:check`
  - 通过。
- `git diff --check`
  - 通过。

## 本次完成内容

- 将 `backend/internal/httpapi/controlplane.go` 与 `admin_rbac.go` 中所有生产 `auth.requirePermission(...)` 管理路由整理为显式契约矩阵，并在 `contract_test.go` 中逐条断言：
  - OpenAPI operation 存在。
  - `responses` 显式包含 `503`。
  - `503` 必须引用 `#/components/responses/ServiceUnavailable`。
- 覆盖的完整矩阵包括：
  - `GET /api/v1/admin/users`
  - `GET /api/v1/admin/users/{user_id}/devices`
  - `GET /api/v1/admin/devices`
  - `GET /api/v1/admin/devices/{device_id}`
  - `POST /api/v1/admin/devices/{device_id}/disable`
  - `POST /api/v1/admin/devices/{device_id}/unbind`
  - `GET /api/v1/admin/activation-codes`
  - `POST /api/v1/admin/activation-codes`
  - `POST /api/v1/admin/activation-codes/{code_id}/revoke`
  - `GET /api/v1/admin/model-pool`
  - `POST /api/v1/admin/model-pool`
  - `POST /api/v1/admin/model-pool/{account_id}/disable`
  - `PATCH /api/v1/admin/model-pool/{account_id}`
  - `POST /api/v1/admin/model-pool/{account_id}/rotate-secret`
  - `POST /api/v1/admin/model-pool/{account_id}/test`
  - `GET /api/v1/admin/model-usage`
  - `GET /api/v1/admin/model-leases`
  - `GET /api/v1/admin/model-leases/{lease_id}`
  - `POST /api/v1/admin/model-leases/{lease_id}/reclaim`
  - `GET /api/v1/admin/audit-logs`
  - `GET /api/v1/admin/permissions`
  - `GET /api/v1/admin/roles`
  - `POST /api/v1/admin/roles`
  - `GET /api/v1/admin/roles/{role_id}`
  - `PATCH /api/v1/admin/roles/{role_id}`
  - `DELETE /api/v1/admin/roles/{role_id}`
  - `GET /api/v1/admin/users/{user_id}/roles`
  - `PUT /api/v1/admin/users/{user_id}/roles`
- 追加契约断言：`ServiceUnavailable` 响应组件必须继续使用 `ErrorResponse` 结构，并至少文档化一个 `ADMIN_AUTHORIZATION_UNAVAILABLE` 示例，避免 fail-closed 语义漂移。
- 确认这批生产 `requirePermission` 管理路径的 OpenAPI `503` 路径声明本身已齐，本次唯一需要补的是 `ServiceUnavailable` 组件示例，不涉及权限实现代码。

## 未验证项

- 未运行 `go test ./...` 全量后端测试。
- 未运行桌面端任何脚本或验证。
- 未运行 `admin-web` 的 `typecheck`、`test`、`build`；本 Task 按要求只执行了 `api:generate` 与 `api:check`。

## 剩余风险

- 当前回归范围聚焦 `internal/httpapi` 与 OpenAPI 契约门禁，更广泛的服务层、存储层或前端页面回归仍依赖其他任务或 CI 门禁。
- `ServiceUnavailable` 组件仍承载多种 503 场景；本次通过新增示例钉住了管理员授权 fail-closed 语义，但未为每一种 503 原因拆分独立响应组件。
