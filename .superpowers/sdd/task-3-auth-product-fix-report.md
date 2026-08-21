# Task 3 Auth/Product 修复复审报告

## 边界

- 修复规范化 PostgreSQL 激活、心跳、session、登录 product membership 和审计 product 事实传播。
- 保留 MemoryStore 无 normalized ProductRepository 的兼容测试路径；该路径不会自动授予 `douyin_desktop` membership。
- 撤回尚未完成 Task 4/5 的租约、用量和激活码 product required 契约声明；未推进完整租约/用量/管理筛选 product 传播。
- 未修改旧的 `.superpowers/sdd/task-3-report.md`。

## 主要文件

- `backend/internal/store/postgres_device_activation_repository.go`
- `backend/internal/store/postgres_device_heartbeat_repository.go`
- `backend/internal/store/postgres_repository.go`
- `backend/internal/store/session_store.go`
- `backend/internal/store/postgres_audit_repository.go`
- `backend/internal/store/postgres_audit_outbox_repository.go`
- `backend/internal/httpapi/auth.go`、`router.go`
- `backend/internal/controlplane/types.go`
- `接口契约/openapi.yaml`、`接口契约/错误码.md`
- 相关 SQL mock、session、登录、membership、审计和 product mismatch 测试

## 验证

- 首轮 focused tests：按要求先运行，确认失败后实现。
- `cd backend && go test ./internal/httpapi ./internal/store -run 'Product|Login|Refresh|Session|Heartbeat|Activation|Audit' -count=1`：通过。
- `cd backend && go test ./...`：通过。
- `go test -race ./internal/httpapi ./internal/store`：store 通过；HTTP 既有限流指标测试在 race 负载下因 bcrypt 延迟增大返回 401 而非预期 429，命令失败，未伪造为通过。
- `go vet ./internal/httpapi ./internal/store`：通过。
- `bash tools/check-document-references.sh`：通过。
- `git diff --check`：通过。

## 未验证项与剩余风险

- `TEST_POSTGRES_URL` 未设置，未进行真实 PostgreSQL 验证；仅执行 sqlmock/内存测试。
- race 失败是既有限流测试的时序/负载敏感性，需后续单独稳定化，不在本次 Task 3 product 修复范围内。
