# Task 3：认证会话绑定 Product 报告

## 功能边界

- 登录请求必须提交合法 `product`；仅当请求缺省 product 且明确携带 `X-Client-Compatibility: legacy` 时，才兼容映射为 `autolive`。普通缺省或非法 product 返回 HTTP 400 / `INVALID_REQUEST`。
- 内存 session、持久化 `AuthSession`、`Actor` 均保存 product；`requireBearer` 从持久化 session 恢复 product。
- refresh 只继承已存 session product，query、header 和 body 中的替代值不会覆盖它。
- 激活、心跳在设备绑定、session 绑定或业务状态写入前校验请求 product 与 actor product；不匹配使用现有 `ErrForbidden` 语义，非法请求使用 `ErrInvalidRequest`。
- `ClientProfileResponse` 顶层和 `DeviceSummary` 返回 product；激活记录、心跳记录、审计输入以及内存激活结果携带 product。现有无 product 的激活码创建路径保持 `autolive` 兼容默认。
- OpenAPI 增加兼容 header 说明和 `ProductCode` 字段/required 约束，覆盖登录、设备、Profile、激活码、租约、用量和审计镜像 DTO。
- 未推进 Task 4/5 的完整 product 传播、产品级资源隔离或模型租约业务改造；未修改 Task 2 迁移文件和迁移语义。

## 改动文件

- `backend/internal/httpapi/auth.go`：登录 product 校验、legacy 标记、session/refresh 持久化与恢复。
- `backend/internal/store/session_store.go`：SQL session Create/Get/Rotate 保存、恢复和校验 product。
- `backend/internal/httpapi/controlplane.go`：激活/心跳 product 预校验和 Profile product 返回。
- `backend/internal/controlplane/types.go`、`backend/internal/service/controlplane.go`、`backend/internal/httpapi/audit.go`：请求、业务记录和审计 product 传递。
- `接口契约/openapi.yaml`：兼容 header、ProductCode 及相关 schema 字段。
- 相关 HTTP、SQL mock、PostgreSQL fixture 和契约测试：显式补充 `autolive`，新增 `douyin_desktop`、非法/缺省/legacy、refresh 覆盖尝试、跨产品激活/心跳、持久化恢复和 Profile 返回测试。
- 未修改 `backend/internal/httpapi/router.go`：现有路由组合已覆盖本次行为，无需新增路由。

## 命令结果

以下命令均在本次最终工作区执行并通过：

- `cd backend && go test ./internal/httpapi ./internal/store -run 'Product|Login|Refresh|Session|Heartbeat|Activation' -count=1`：通过。
- `cd backend && go test ./internal/httpapi ./internal/store`：通过。
- `cd backend && go test ./internal/httpapi -run Contract -count=1`：通过。
- `bash tools/check-document-references.sh`：通过。
- `cd backend && go vet ./internal/httpapi ./internal/store`：通过。
- `gofmt -w` 相关 Go 文件：通过执行，无格式化残留。
- `git diff --check`：通过。

开发过程中先执行了 focused RED：新增 product/session 测试在实现前按预期因 `AuthSession.Product` 和相关行为未实现而失败；实现后 focused GREEN 通过。

## 未验证项与剩余风险

- 未在本次命令中单独连接真实 PostgreSQL 数据库执行集成测试；SQL mock 和现有 package 测试已通过。真实数据库验证仍依赖运行环境提供数据库连接。
- 未运行全仓库所有语言/客户端测试；本任务只执行了要求的 Go httpapi/store、契约、文档引用和 vet 检查。
- Task 4/5 的完整产品级资源传播与隔离仍未实现，后续不得将本报告中的 Task 3 行为误认为完整 product 隔离完成。
- 未执行服务器构建；符合项目“本地开发机或 CI 构建、禁止服务器构建”的约束。
