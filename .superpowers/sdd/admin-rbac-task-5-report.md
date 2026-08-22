# Admin RBAC Task 5 报告

## 改动文件

- `backend/internal/httpapi/admin_rbac.go`
- `backend/internal/httpapi/admin_rbac_test.go`
- `backend/internal/httpapi/controlplane.go`
- `接口契约/openapi.yaml`
- `admin-web/src/api/openapi.generated.ts`

## 实际执行命令与结果

- `cd backend && go test ./internal/httpapi -run 'AdminRBAC|Permission|Contract|Router' -count=1`
  - 初次执行失败，先后确认了 `/api/v1/admin/me`/RBAC 契约缺口、内建本地管理员兼容边界回退、`user_id` 契约校验以及 OpenAPI 错误响应缺失。
  - 最终执行通过。
- `cd backend && gofmt -w internal/httpapi/admin_rbac.go internal/httpapi/admin_rbac_test.go internal/httpapi/controlplane.go`
  - 通过。
- `cd admin-web && pnpm api:generate`
  - 通过，已更新 `src/api/openapi.generated.ts`。
- `cd admin-web && pnpm api:check`
  - 通过。
- `git diff --check`
  - 通过。

## 本次完成内容

- 新增 `/api/v1/admin/me`、`/api/v1/admin/permissions`、角色 CRUD、用户角色读取/替换 HTTP handler。
- 新增 `requirePermission`，从 Bearer Session 读取 actor，并按会话产品做即时权限判定；读取失败返回 503 fail-closed。
- 将产品范围内的管理路由切换到固定权限点：用户列表/设备列表、激活码、模型池、模型用量、模型租约、审计、安全。
- 按审查要求保留内建本地管理员兼容边界：
  - `POST /api/v1/admin/auth/change-password`
  - `GET /api/v1/admin/users/{user_id}/authorization-summary`
  - `PATCH /api/v1/admin/users/{user_id}/authorization`
  - `POST /api/v1/admin/users`
  - `POST /api/v1/admin/users/{user_id}/disable`
  - `PATCH /api/v1/admin/users/{user_id}`
  - `POST /api/v1/admin/users/{user_id}/reset-password`
  以上路由恢复为 `requireAdmin` + 本地管理员边界，产品级 RBAC 不可触达。
- 补充 Task 5 聚焦 HTTP/OpenAPI/Router 测试，覆盖：
  - `/admin/me` 的 401、普通用户空权限、即时权限返回、fail-closed。
  - 权限目录、角色 CRUD、用户角色绑定 API。
  - 本地管理员兼容路由的 403/成功回归。
  - `user_id` 按 `Id` schema 正则做路径校验。
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
