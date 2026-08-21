# Task 2 Product Fix 2 Report

- 日期：2026 年 8 月 21 日（Friday）
- 状态：DONE

## 本轮范围

- 仅修复审查要求的 Important：
  - `backend/migrations/0023_补齐多产品控制面.up.sql` 中剩余的产品范围 FK 幂等检查，禁止只按全局 `conname` 判断，改为按当前表 `conrelid = '<table>'::regclass` + `conname` 联合判断。
  - `backend/migrations/catalog_test.go` 增加静态契约，要求关键表的产品范围 FK 检查必须带 `conrelid` 条件，并禁止退化回全局 `conname` guard。
  - `.superpowers/sdd/task-2-brief.md` 顶部增加显式说明：原始终态描述已被 0023 的两阶段兼容扩展方案取代，后续以阶段计划和 fix report 为准。
- 未重新引入 0023 的 `NOT NULL` 收紧、复合主键终态或 Task 3–5 代码。

## 修改文件

- `backend/migrations/0023_补齐多产品控制面.up.sql`
- `backend/migrations/catalog_test.go`
- `.superpowers/sdd/task-2-brief.md`

## 实际命令与结果

1. RED：新增静态契约后验证失败
   - `cd backend && go test ./migrations -run 'Test(Migration23ProductIsolationContract|Migration23ProductScopedConstraintChecksAreTableQualified)' -count=1`
   - 结果：FAIL
   - 关键输出：`missing "WHERE conrelid = 'devices'::regclass AND conname = 'devices_user_product_fkey'"`

2. 格式化
   - `cd backend && gofmt -w migrations/catalog_test.go`
   - 结果：PASS

3. GREEN：迁移静态契约回归
   - `cd backend && go test ./migrations -run 'Test(Migration23ProductIsolationContract|Migration23ProductScopedConstraintChecksAreTableQualified)' -count=1`
   - 结果：PASS
   - 输出：`ok  	autoLive/backend/migrations	1.208s`

4. 指定聚焦单元测试
   - `cd backend && go test ./migrations ./internal/store -run 'Product|Migration|Catalog' -count=1`
   - 结果：PASS
   - 输出：
     - `ok  	autoLive/backend/migrations	0.405s`
     - `ok  	autoLive/backend/internal/store	1.085s`

5. tagged focused test：migration 兼容回放
   - `cd backend && go test -tags=postgres_integration ./migrations -run 'TestPostgresMigration23CompatibilityAndOrphanRecovery' -count=1`
   - 结果：包级 PASS
   - 输出：`ok  	autoLive/backend/migrations	2.361s`

6. tagged focused test：legacy write 默认 product
   - `cd backend && go test -tags=postgres_integration ./internal/store -run 'TestPostgresProductCompatibilityDefaultsLegacyWritesToAutolive' -count=1`
   - 结果：包级 PASS
   - 输出：`ok  	autoLive/backend/internal/store	1.721s`

7. 环境检查
   - `cd backend && if [ -n "$TEST_POSTGRES_URL" ]; then echo set; else echo unset; fi`
   - 结果：`unset`
   - 说明：当前环境未配置 `TEST_POSTGRES_URL`，上面两个 tagged focused test 按约定未实际连接 PostgreSQL，属于 skip 通过路径。

8. Diff 检查
   - `git diff --check`
   - 结果：PASS

## 本轮修复内容

- 0023 中 `devices`、`activation_codes`、`auth_sessions`、`model_leases`、`model_usage_records`、`audit_logs`、`user_authorization_policies`、`variant_tasks` 的产品范围 FK 幂等检查，全部改为表限定的 `pg_constraint` 判断，避免同名约束在其他表上误判为已存在。
- 新增静态门禁，明确要求关键产品 FK 检查带 `conrelid = '<table>'::regclass`，并禁止使用全局 `conname`-only guard。
- Task 2 brief 顶部同步为两阶段兼容扩展口径，避免后续子线程按已过时的 0023 终态描述继续推进。

## 未验证项

- 当前环境 `TEST_POSTGRES_URL` 未配置，因此 2026 年 8 月 21 日（Friday）这轮修复没有实连 PostgreSQL 验证 tagged focused tests，只确认了代码路径和 skip 语义。
- 本轮没有实现或验证 Task 3–5；product-aware 复合键、外键终态与 `NOT NULL` 收紧仍以后续阶段迁移为准。
