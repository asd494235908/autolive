# Task 3 最终复审 Important 修复报告

## 状态

已修复最终复审剩余的两项 Important。基线为 `7e61782`；未使用 worktree，未修改既有 `.superpowers/sdd/task-3-report.md`。

## 功能边界

- PostgreSQL snapshot/兼容读源继续沿用原有登录行为，不读取规范化产品 membership。
- 只有明确声明 `UsesNormalizedReadSource() == true` 的 repository 才向 authenticator 提供规范化 ProductRepository。
- client profile HTTP 路径按 session actor product 读取并校验设备；不扩展 Task5 管理查询的 product 筛选。
- 本次不改 normalized 激活/心跳、session、audit、OpenAPI 收窄等既有 Task3 修复。

## 根因与修复

### 1. Snapshot 登录回归

路由原先只按 `store.ProductRepository` 做类型断言。snapshot `PostgresRepository` 为兼容 API 实现了该接口，但其 membership 方法按设计返回 `ErrNormalizedProductRepositoryRequired`，导致登录被映射为 503。

路由现在同时要求 `store.NormalizedReadSource` 存在且 `UsesNormalizedReadSource()` 为 true，才注入 ProductRepository。snapshot/兼容读源不再触发 membership 查询，继续使用既有快照认证路径。新增回归测试验证 snapshot fake 登录返回 200 且 membership 方法调用次数为 0。

### 2. Profile product 一致性

原 `GetClientProfile` 没有接收 session product；HTTP 路由直接把 actor product 写入顶层响应，设备读取结果即使属于其他 product 仍会返回混合响应。

新增 `GetClientProfileForProduct` 小边界，normalized 与 Memory 读取路径都在组装返回值前比较合法 product。不一致统一返回 `FORBIDDEN`，不会改变 session device binding 或设备状态；HTTP 路由改用该边界。新增 normalized mismatch、Memory mismatch 和 HTTP mismatch/no-mutation 覆盖。

## RED / GREEN

### RED

- snapshot ProductRepository fake 的登录回归先返回 503，证明路由错误注入了兼容读源。
- HTTP profile mismatch 测试先收到 200，响应同时包含顶层 `autolive` 和设备 `douyin_desktop`。
- normalized profile 测试先因缺少 `GetClientProfileForProduct` 编译失败，证明产品绑定边界尚不存在。

### GREEN

- snapshot fake 登录返回 200，membership 调用次数为 0。
- normalized 和 Memory profile product mismatch 均返回 `controlplane.ErrForbidden`。
- HTTP profile mismatch 返回 403，session 仍绑定原 device ID，设备 product 未变化。

## 改动文件

- `backend/internal/httpapi/router.go`
  - 仅向 authenticator 注入 normalized ProductRepository。
- `backend/internal/httpapi/controlplane.go`
  - client profile 路由改用 product-bound service 边界。
- `backend/internal/service/controlplane.go`
  - 新增 `GetClientProfileForProduct` 及内部 profile 读取边界，在 normalized/Memory 两条路径校验 product。
- `backend/internal/httpapi/auth_test.go`
  - 新增 snapshot 登录回归和 HTTP profile mismatch/no-mutation 测试。
- `backend/internal/service/device_reader_test.go`
  - 新增 normalized、Memory product mismatch 测试，并更新合法 normalized profile 调用。
- `.superpowers/sdd/task-3-auth-product-fix2-report.md`
  - 本次独立复审报告。

## 最终验证

在 `/Users/mac/work/gepin/autoLive` 本地执行：

- `gofmt -w backend/internal/httpapi/router.go backend/internal/httpapi/controlplane.go backend/internal/httpapi/auth_test.go backend/internal/service/controlplane.go backend/internal/service/device_reader_test.go`：通过。
- `cd backend && go test ./internal/httpapi ./internal/service ./internal/store`：通过。
- `cd backend && go test ./...`：通过。
- `cd backend && go vet ./internal/httpapi ./internal/service ./internal/store`：通过。
- `bash tools/check-document-references.sh`：通过。
- `git diff --check`：通过。

`TEST_POSTGRES_URL` 未设置；真实 PostgreSQL 集成测试未执行，不能据此宣称真实数据库路径已验证。未新增依赖，未发现本次改动产生的未使用导入、类型或调试日志。

## 自审与剩余风险

- Profile product 校验位于服务层返回前，HTTP 顶层 product 与设备 product 不再有机会组合成跨产品响应。
- product 检查只接入 client profile HTTP 路径，没有扩展管理端查询筛选。
- snapshot 兼容读源的 membership 方法仍保留用于规范化能力接口兼容；路由不会再把它注入 authenticator。
- 当前环境未提供可调用的子线程/子代理工具，本轮由主线程完成 diff 复审和指定验证。
