> 当前范围（2026-08-22）：Task 2 已采用 `0023_补齐多产品控制面.up.sql` 的“两阶段兼容扩展方案”。本任务只建立产品注册、用户产品成员关系、兼容 product 字段回填/默认值、产品外键、用户产品复合外键和回归验证；不得把后续阶段的约束收紧提前到 0023。

> 后续阶段边界：Task 3–5 完成产品字段传播、写路径切换和兼容数据清理后，才可以另行迁移到 `product` `NOT NULL`、`devices` 的 `(product, device_id)` 复合业务唯一键以及相应的严格旧约束。Task 2 不执行这些操作，也不恢复 0018 已删除的 `auth_sessions -> devices` 外键。

### Task 2: PostgreSQL 产品注册、成员关系与历史回填迁移

**Files:**
- Create: `backend/migrations/0023_补齐多产品控制面.up.sql`
- Modify: `backend/migrations/catalog.go`
- Modify: `backend/migrations/catalog_test.go`
- Create: `backend/internal/store/postgres_product_repository.go`
- Create: `backend/internal/store/postgres_product_repository_test.go`
- Test: `backend/internal/store/postgres_integration_test.go`

**Interfaces:**
- Produces `store.ProductRepository` with `ListProducts`, `GetProduct`, `GetUserProductMembership` and `EnsureUserProductMembership`.
- Migration creates immutable `products` and product-scoped `user_products`; adds compatibility product columns with `autolive` backfill/defaults to current normalized business tables; binds product columns to `products`; adds the product/user-product foreign keys that are safe during the compatibility window; and creates product-aware indexes without editing 0001～0022. Existing legacy keys and nullable write paths remain in place.

- [ ] **Step 1: Write failing catalog and SQL contract tests**

Add tests asserting `LatestVersion == 23`, the embedded FS contains `0023_补齐多产品控制面.up.sql`, and the migration text contains the seeds `autolive`/`douyin_desktop`, `user_products`, compatibility `product` columns with defaults/backfill, all product foreign keys, all `user_products` composite foreign keys (including `variant_tasks_user_product_fkey`), and product-aware indexes. The static contract must reject reintroduction of the 0018-dropped `auth_sessions -> devices` foreign key and must not require Task 3–5-only `NOT NULL`/device-key cutover text.

- [ ] **Step 2: Run migration tests and verify RED**

Run: `cd backend && go test ./migrations -run 'Test(Latest|Migration)' -count=1`

Historical plan note: this RED step applied before 0023 was created. For the current Task 2 review fix, run the focused checks after adding the regression contract and require them to pass against the existing migration.

- [ ] **Step 3: Add the forward migration**

Create `products(code, status, created_at)` with a check for the two seed codes and unique code. Create `user_products(user_id, product, status, entitlement_revision, created_at, updated_at)` with a composite primary key and user/product indexes. Insert both products idempotently and seed `autolive` membership for historical/new users. Add compatibility product columns to `devices`, `activation_codes`, `auth_sessions`, `model_accounts`, `model_leases`, `model_usage_records`, `model_request_reservations`, `audit_logs`, `audit_outbox`, `model_pool_test_results`, `user_authorization_policies`, `idempotency_records` and `variant_tasks`; backfill `autolive`, preserve defaults for legacy writes, add the safe product foreign keys and all user/product composite foreign keys, and keep the 0018 orphan-session recovery semantics. Do not set these columns `NOT NULL`, do not replace the existing `devices` key, do not remove legacy keys, and do not restore `auth_sessions -> devices`.

- [ ] **Step 4: Add the repository seam and tests**

Define the smallest repository interface in `repository.go`; implement normalized PostgreSQL reads/writes with parameterized SQL and short transactions. Add tests for both products, missing membership, idempotent membership creation, disabled membership, and cross-product lookup returning not found/forbidden semantics. Add static and PostgreSQL-tagged migration coverage: after migrating to `LatestVersion`, query `pg_constraint` and verify each product foreign key and each `user_products` composite foreign key belongs to the intended child table and parent table, including `variant_tasks_user_product_fkey`. Keep the existing orphan auth-session replay and legacy default-value tests. Do not assert the 0018-deleted `auth_sessions -> devices` foreign key.

- [ ] **Step 5: Run migration and repository checks**

Run: `cd backend && gofmt -w internal/store/postgres_product_repository.go internal/store/postgres_product_repository_test.go internal/store/repository.go && go test ./migrations ./internal/store -run 'Product|Migration|Catalog' -count=1`

Expected: PASS in unit mode. The PostgreSQL-tagged test must run against `TEST_POSTGRES_URL` when it is available; without that variable it must naturally skip and the report must say so. No Task 3–5 propagation or constraint cutover is part of this task.

- [ ] **Step 6: Commit**

Commit: `feat(store): add product registry and membership migration`
