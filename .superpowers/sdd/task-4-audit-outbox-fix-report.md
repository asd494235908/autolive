# Task4 定向修复报告：审计 Outbox 产品边界

## 范围

本次只处理最终代码审查指出的两项：

1. 让 `RecordAuditWithOutbox` 与事务内 `enqueueAuditOutboxTx` 共用严格的审计目标产品校验。
2. 将 `model_account -> model_accounts` 纳入 PostgreSQL 和 Memory 审计目标白名单。

未实现 Task5 管理端全局列表产品筛选，未触碰实时话术幻化、旧 `VariantTask` 或其他历史任务报告。

## 修复内容

- 将严格目标校验放入 `enqueueAuditOutboxTx` 的共同事务边界，直接调用该 helper 的设备、租约、用量和模型池事务都会校验目标。
- 对白名单目标使用 `id + product` 查询；匹配不到时继续检查 NULL/冲突产品并 fail-closed。只有 `Outcome == "failure"` 时才允许不存在的已知目标写入失败审计；成功或未知结果拒绝不存在目标。
- 保留创建事务合法路径：空 `target_id` 不查询；资源已在同一事务中插入后再校验；设备目标与 `device_id` 相同的审计避免重复查询。
- PostgreSQL 与 Memory 均支持 `model_account`，并对跨产品、NULL、非法产品和不存在目标增加测试。

## 验证

- TDD RED：新增严格 Outbox/模型账号测试在实现前按预期失败。
- `cd backend && go test ./internal/service ./internal/store -count=1`：通过。
- `cd backend && go test ./...`：通过。
- `cd backend && go test -race ./internal/service ./internal/store`：通过。
- `cd backend && go vet ./...`：通过。
- `gofmt`、`git diff --check`：通过。
- 真实 PostgreSQL：未执行，当前 `TEST_POSTGRES_URL` 未设置；SQLMock 覆盖了产品谓词、Outbox 事务和失败/成功目标语义。
