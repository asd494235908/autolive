> 说明（2026-08-21）：本文件中“Task 2 原始终态描述”已被 `0023_补齐多产品控制面.up.sql` 的“两阶段兼容扩展方案”取代；后续实现与审查以 [`/Users/mac/work/gepin/autoLive/docs/superpowers/plans/2026-08-22-product-isolation-phase-1.md`](/Users/mac/work/gepin/autoLive/docs/superpowers/plans/2026-08-22-product-isolation-phase-1.md) 和 `task-2-product-fix-report.md` / `task-2-product-fix2-report.md` 为准，不再按本文件里已过时的终态收紧描述推进 Task 3–5。

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
- Migration creates immutable `products`, product-scoped `user_products`, adds non-null product columns with `autolive` backfill to current normalized business tables, binds `auth_sessions` to product, converts the device logical key and all live device foreign keys to product-aware constraints, and creates product-aware indexes/constraints without editing 0001～0022.

- [ ] **Step 1: Write failing catalog and SQL contract tests**

Add tests asserting `LatestVersion == 23`, the embedded FS contains `0023_补齐多产品控制面.up.sql`, and the migration text contains the seeds `autolive`/`douyin_desktop`, `user_products`, `product` columns, `NOT NULL` constraints after backfill, and product-aware indexes.

- [ ] **Step 2: Run migration tests and verify RED**

Run: `cd backend && go test ./migrations -run 'Test(Latest|Migration)' -count=1`

Expected: FAIL because catalog version remains 22 and migration file is absent.

- [ ] **Step 3: Add the forward migration**

Create `products(code, status, created_at)` with a check for the two seed codes and unique code. Create `user_products(user_id, product, status, entitlement_revision, created_at, updated_at)` with composite primary key and user/product indexes. Insert both products idempotently. Add product columns to `devices`, `activation_codes`, `auth_sessions`, `model_accounts`, `model_leases`, `model_usage_records`, `model_request_reservations`, `audit_logs`, `audit_outbox`, `model_pool_test_results`, `user_authorization_policies` and any existing device-binding table; backfill `autolive`, then convert the `devices` key and every live device reference to product-aware composite constraints, set `NOT NULL`, add product foreign keys/compound indexes, and preserve old data counts. Historical `variant_*` tables may receive only the product column required to preserve an existing device foreign key; they remain outside current reads/writes and must not be reintroduced into the product API.

- [ ] **Step 4: Add the repository seam and tests**

Define the smallest repository interface in `repository.go`; implement normalized PostgreSQL reads/writes with parameterized SQL and short transactions. Add tests for both products, missing membership, idempotent membership creation, disabled membership, and cross-product lookup returning not found/forbidden semantics. Add migration/SQL contract coverage proving the device composite key prevents the same `device_id` from colliding across products and that session product is persisted. Keep product registry data separate from the legacy snapshot.

- [ ] **Step 5: Run migration and repository checks**

Run: `cd backend && gofmt -w internal/store/postgres_product_repository.go internal/store/postgres_product_repository_test.go internal/store/repository.go && go test ./migrations ./internal/store -run 'Product|Migration|Catalog' -count=1`

Expected: PASS in unit mode; PostgreSQL-tagged tests must be run when a local PostgreSQL URL is available.

- [ ] **Step 6: Commit**

Commit: `feat(store): add product registry and membership migration`
