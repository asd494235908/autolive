# Task 2 Fix 4 Review Report

日期：2026-08-22

## 改动

- `backend/migrations/0023_补齐多产品控制面.up.sql`
  - 为已有 `user_id` 的 `model_request_reservations` 增加 `(user_id, product) -> user_products(user_id, product)` 复合外键。
  - 使用 `conrelid = 'model_request_reservations'::regclass AND conname = 'model_request_reservations_user_product_fkey'` 的子表和约束名联合幂等守卫。
  - 保留兼容迁移语义：未设置 `NOT NULL`，未修改 `devices` 旧键和历史迁移。

- `backend/migrations/catalog_test.go`
  - 将 `user_products` 复合外键契约从 8 项同步为 9 项，加入 `model_request_reservations_user_product_fkey`。
  - 保留并扩展表限定 guard、复合 FK 定义和 `pg_constraint` 契约清单；该清单由 PostgreSQL 集成断言直接复用。
  - 增加 `products`/`user_products` 内联基础 FK 和 `FOREIGN KEY (product) REFERENCES products(code)` 静态检查。
  - 将旧 `auth_sessions -> devices` 禁止规则改为规范化 SQL 结构检查：同一 `auth_sessions` 建表/改表语句出现 `REFERENCES devices` 即拒绝，不依赖可能变化的约束名，同时允许合法的产品 FK 和用户产品 FK。

- `.superpowers/sdd/task-2-brief.md`
- `docs/superpowers/plans/2026-08-22-product-isolation-phase-1.md`
  - 明确 `model_request_reservations_user_product_fkey` 属于 Task 2 的复合 FK 契约，并同步旧 `auth_sessions -> devices` 的结构语义禁止范围。

## 实际验证

- `cd backend && go test ./migrations ./internal/store -run 'Product|Migration|Catalog' -count=1`
  - 通过：migrations、internal/store 均 `ok`。
- `cd backend && go test -tags=postgres_integration ./migrations -run TestPostgresMigration23CompatibilityAndOrphanRecovery -count=1`
  - 命令通过；当前 `TEST_POSTGRES_URL` 未设置，测试按约定自然 skip，未伪造 PostgreSQL 结果。
- `cd backend && go vet ./migrations`
  - 通过。
- `cd backend && gofmt -w migrations/catalog_test.go migrations/postgres_integration_test.go`
  - 已执行。
- `git diff --check`
  - 通过。
- RED 回归：旧迁移在新增契约下按预期失败，报告缺少 `model_request_reservations_user_product_fkey` 的表限定 guard；补齐迁移后 focused 契约测试通过。

## 未验证项与剩余风险

- 当前环境没有 `TEST_POSTGRES_URL`，因此未实际连接 PostgreSQL；真实 `pg_constraint` 查询需在具备测试数据库的环境重新运行。
- 未推进 Task 3–5，未设置当前业务表 `product` 的 `NOT NULL`，未切换 `devices` 复合旧键，未恢复 `auth_sessions -> devices` 外键。
