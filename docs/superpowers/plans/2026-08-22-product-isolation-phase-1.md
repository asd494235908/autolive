# 多产品控制面 Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 autoLive Go 服务端建立 `autolive`/`douyin_desktop` 的产品注册、产品会话、数据约束和跨产品拒绝基础，使设备激活、Profile、租约/用量/审计和管理查询具备同一 product 事实。

**Architecture:** 保留当前模块化单体、兼容快照与 normalized PostgreSQL 双路径。`controlplane.ProductCode` 是唯一产品值对象；PostgreSQL 使用 `products`/`user_products` 及各领域表的 product 字段，MemoryStore 使用相同领域字段做测试回退。登录或激活时确定产品并绑定会话，后续服务端从会话和资源归属校验 product；筛选参数只缩小管理结果，不承担授权。按 2026 年 8 月 21 日（Friday）后的修正规则，0023 只做兼容扩展，不在 Task 3–5 之前删除旧键或把现有写路径收紧到 product-aware 终态。

**Tech Stack:** Go 现有标准库与 `golang-migrate/migrate` 迁移链、PostgreSQL、现有 `MemoryStore`/Repository 边界、Go `httptest`、OpenAPI YAML；不新增依赖，不在服务器构建。

## Global Constraints

- `product` 首批只允许 `autolive` 与 `douyin_desktop`，产品代码不可变。
- 全局用户身份复用，产品准入使用 `user_products`；目标终态里同一 `device_id` 在两个 product 下是两条独立记录，但 0023 兼容阶段仍保留旧全局键，真正切到 `(product, device_id)` 由后续迁移执行。
- 设备、激活码、设备绑定、Profile、模型租约、用量、审计及幂等 scope 必须包含或可确定 product。
- 历史数据先回填 `autolive`，兼容窗口后缺失 product 必须 fail-closed；不得永久默认。
- 最终设备业务唯一键是 `(product, device_id)`；但在旧 SQL 仍按单列键写入的兼容窗口内，迁移必须先保留 `devices.id` 的现有唯一语义，通过默认 `autolive` 和双兼容观察承接旧代码，随后再切换到 product-aware 复合键与外键。
- 服务端授权不能依赖 React 隐藏、客户端传参诚实性或 product 查询筛选。
- 不修改 0001～0022 历史迁移；新增前向迁移必须注册到 catalog，API 启动不隐式执行迁移。
- 新行为先写失败测试并确认失败原因，再写最小实现；每个任务结束运行对应 Go/OpenAPI/迁移检查。
- 密码、Token、激活码明文、模型 API Key、BYOK、短期凭证和模型正文不得进入日志、trace、审计或普通数据库列。
- 服务器只接收本地/CI 构建产物，不上传源码构建；当前任务只改 Go、迁移、OpenAPI 和测试，不改服务器部署。

---

### Task 1: 产品值对象、共享 DTO 与注册表事实

**Files:**
- Create: `backend/internal/controlplane/product.go`
- Create: `backend/internal/controlplane/product_test.go`
- Modify: `backend/internal/controlplane/types.go`
- Modify: `backend/internal/store/memory.go`
- Modify: `backend/internal/store/repository.go`
- Test: `backend/internal/controlplane/product_test.go`, `backend/internal/store/repository_test.go`

**Interfaces:**
- Produces `controlplane.ProductCode`, `controlplane.ProductAutoLive`, `controlplane.ProductDouyinDesktop`, `controlplane.ParseProductCode(string) (ProductCode, error)` and `ProductCode.Valid() bool`.
- Produces `controlplane.ProductSummary` and `controlplane.UserProductMembership` for later migration/session tasks.
- Existing zero-value compatibility records remain readable only in legacy Memory/snapshot tests; new service boundaries reject empty product unless an explicit compatibility caller supplies `autolive`.

- [ ] **Step 1: Write the failing tests**

```go
func TestParseProductCodeAcceptsOnlyRegisteredProducts(t *testing.T) {
	for _, raw := range []string{"autolive", "douyin_desktop"} {
		product, err := ParseProductCode(raw)
		if err != nil || !product.Valid() {
			t.Fatalf("ParseProductCode(%q) = %q, %v", raw, product, err)
		}
	}
	for _, raw := range []string{"", "AutoLive", "douyin-desktop", "unknown"} {
		if _, err := ParseProductCode(raw); err == nil {
			t.Fatalf("ParseProductCode(%q) unexpectedly accepted", raw)
		}
	}
}

func TestProductFieldsRoundTripThroughControlPlaneDTOs(t *testing.T) {
	device := DeviceRegistration{Product: ProductDouyinDesktop, DeviceID: "dev_00000001", DeviceName: "desktop", Platform: "windows", AppVersion: "2.0.0"}
	encoded, err := json.Marshal(device)
	if err != nil { t.Fatal(err) }
	var decoded DeviceRegistration
	if err := json.Unmarshal(encoded, &decoded); err != nil { t.Fatal(err) }
	if decoded.Product != ProductDouyinDesktop { t.Fatalf("product = %q", decoded.Product) }
}
```

- [ ] **Step 2: Run the focused tests and verify the expected RED failure**

Run: `cd backend && go test ./internal/controlplane -run 'TestParseProductCodeAcceptsOnlyRegisteredProducts|TestProductFieldsRoundTripThroughControlPlaneDTOs' -count=1`

Expected: FAIL because `ProductCode`, registered constants and DTO `Product` fields do not exist yet.

- [ ] **Step 3: Implement the minimal product domain**

Add the two immutable codes and a trimmed exact parser. Add `Product` to `Actor`, `AuditLog`, `AuditLogInput`, `DeviceSummary`, `DeviceRegistration`, `ActivationCode`, `ClientProfile`, `ModelLease`, `ModelLeaseAdminSummary`, `ModelLeaseAdminDetail` and `ModelUsageRecord`; use the existing JSON naming conventions. Add `ProductSummary` and `UserProductMembership` without a generic policy engine. Add `Products` and `UserProducts` maps to `store.State`, initialize them in `NewState`, and make `ensureStateMaps` restore them for old snapshot fixtures.

- [ ] **Step 4: Run focused tests and the existing control-plane tests**

Run: `cd backend && go test ./internal/controlplane ./internal/store -run 'Test(ParseProductCode|ProductFields|NewState|Repository)' -count=1`

Expected: PASS; existing tests may compile with zero-value Product because compatibility behavior is not yet enforced at request boundaries.

- [ ] **Step 5: Self-review and commit**

Run: `cd backend && gofmt -w internal/controlplane/product.go internal/controlplane/product_test.go internal/controlplane/types.go internal/store/memory.go internal/store/repository.go && go test ./internal/controlplane ./internal/store`

Commit: `feat(controlplane): add product domain primitives`

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
- Migration creates immutable `products`, product-scoped `user_products`, adds compatibility `product` columns with `autolive` backfill/defaults to current normalized business tables, keeps 0018 orphan-session recovery semantics, seeds default `autolive` membership for historical/new users, and adds only non-breaking product indexes/references without editing 0001～0022 or cutting over to product-aware composite keys yet.

- [ ] **Step 1: Write failing catalog and SQL contract tests**

Add tests asserting `LatestVersion == 23`, the embedded FS contains `0023_补齐多产品控制面.up.sql`, and the migration text contains the seeds `autolive`/`douyin_desktop`, `user_products`, `product` columns, default `autolive`, default user-product seeding, all product/user-product foreign keys including `model_request_reservations_user_product_fkey`, and non-breaking product indexes. Also assert normalized 0023 SQL does not reintroduce any `auth_sessions` foreign key referencing `devices` while allowing its product and user-product foreign keys, does not raise on 0018-style orphan bindings, and does not drop legacy device keys or force current business-table `product` columns to `NOT NULL`.

- [ ] **Step 2: Run migration tests and verify RED**

Run: `cd backend && go test ./migrations -run 'Test(Latest|Migration)' -count=1`

Expected: FAIL because catalog version remains 22 and migration file is absent.

- [ ] **Step 3: Add the forward migration**

Create `products(code, status, created_at)` with a check for the two seed codes and unique code. Create `user_products(user_id, product, status, entitlement_revision, created_at, updated_at)` with composite primary key and user/product indexes. Insert both products idempotently, and add a default-membership trigger so post-0023 legacy user creation also gets `autolive` membership. Add product columns to `devices`, `activation_codes`, `auth_sessions`, `model_accounts`, `model_leases`, `model_usage_records`, `model_request_reservations`, `audit_logs`, `audit_outbox`, `model_pool_test_results`, `user_authorization_policies` and any existing device-binding table; backfill existing rows to `autolive`, keep defaults for legacy inserts, preserve 0018 orphan-session recovery semantics, and add only non-breaking product foreign keys/indexes, including the `(user_id, product) -> user_products` reference on `model_request_reservations`, that do not require Task 3–5 code propagation. Historical `variant_*` tables may receive only the product column required for future cleanup; they remain outside current reads/writes and must not be reintroduced into the product API.

- [ ] **Step 4: Add the repository seam and tests**

Define the smallest repository interface in `repository.go`; implement normalized PostgreSQL reads/writes with parameterized SQL and short transactions. Add tests for both products, missing membership, idempotent membership creation, disabled membership, and cross-product lookup returning not found/forbidden semantics. Add migration/SQL contract coverage verifying every declared product/user-product foreign key by child table, parent table, and columns, proving 0018-style orphan sessions still migrate forward, legacy writes without explicit `product` default to `autolive`, and session product is persisted. Keep product registry data separate from the legacy snapshot.

- [ ] **Step 5: Run migration and repository checks**

Run: `cd backend && gofmt -w internal/store/postgres_product_repository.go internal/store/postgres_product_repository_test.go internal/store/repository.go && go test ./migrations ./internal/store -run 'Product|Migration|Catalog' -count=1`

Expected: PASS in unit mode; PostgreSQL-tagged tests must be run when a local PostgreSQL URL is available.

- [ ] **Step 6: Commit**

Commit: `feat(store): add product registry and membership migration`

### Task 3: 产品绑定认证会话与客户端请求边界

**Files:**
- Modify: `backend/internal/httpapi/auth.go`
- Modify: `backend/internal/httpapi/auth_test.go`
- Modify: `backend/internal/httpapi/controlplane.go`
- Modify: `backend/internal/httpapi/router.go`
- Modify: `backend/internal/store/session_store.go`
- Modify: `backend/internal/store/session_store_test.go`
- Modify: `backend/internal/controlplane/types.go`
- Modify: `接口契约/openapi.yaml`

**Interfaces:**
- Login accepts a required `product` for new clients; the compatibility window maps omitted product to `autolive` only on explicitly marked legacy requests.
- `sessionRecord`, persisted sessions and `controlplane.Actor` carry Product. Refresh preserves the stored Product and never accepts a replacement product.
- Activation and heartbeat must compare request/device/session Product before any state mutation.

- [ ] **Step 1: Write failing HTTP/session tests**

Add tests for: login with `douyin_desktop` returns a session actor with that product; refresh preserves it even when a product-like query/header is supplied; missing product is rejected for strict requests; a `douyin_desktop` session cannot activate an `autolive` code/device; heartbeat with a different product is rejected before mutation.

- [ ] **Step 2: Run focused auth tests and verify RED**

Run: `cd backend && go test ./internal/httpapi ./internal/store -run 'Product|Login|Refresh|Session|Heartbeat|Activation' -count=1`

Expected: FAIL because login/session records do not carry product and request handlers do not validate it.

- [ ] **Step 3: Implement session product binding**

Extend login JSON and OpenAPI with `product`; parse with `ParseProductCode`; persist Product in memory and SQL session records; preserve it on refresh/rotation; never infer it from User-Agent or app version. Add a single request helper that compares the authenticated actor product with the resource/request product and returns the existing stable forbidden/conflict application error.

- [ ] **Step 4: Wire activation and heartbeat checks**

Require product on `DeviceRegistration`, copy it into `DeviceSummary`, activation records and audit input, and check it against the authenticated actor/session before calling the repository. Heartbeat must use the session/device product rather than trusting a caller-supplied replacement.

- [ ] **Step 5: Update OpenAPI and run contract tests**

Add `ProductCode` enum, required `product` fields for login/device registration/profile-relevant responses, and stable 403/409 response descriptions. Run: `bash tools/check-document-references.sh && cd backend && go test ./internal/httpapi ./internal/store && go test ./internal/httpapi -run Contract -count=1`.

- [ ] **Step 6: Commit**

Commit: `feat(auth): bind sessions to product`

### Task 4: 产品字段贯穿设备、激活码、Profile、租约、用量与审计

**Files:**
- Modify: `backend/internal/service/controlplane.go`
- Modify: `backend/internal/service/activation_multi_device_test.go`
- Modify: `backend/internal/service/device_test.go`
- Modify: `backend/internal/service/model_lease_options.go`
- Modify: `backend/internal/service/model_usage_options.go`
- Modify: `backend/internal/service/audit_log_options.go`
- Modify: normalized PostgreSQL repositories under `backend/internal/store/postgres_*.go` that read/write these domains
- Modify: relevant service/store tests discovered by `rg -n 'CreateModelLease|RecordDirectLLMCall|RecordAudit|ActivateDevice|Heartbeat' backend/internal`

**Interfaces:**
- Every write record receives Product from the authenticated session/service boundary; no repository derives product from IDs or User-Agent.
- Every normalized query either accepts product as a fixed filter or derives it through a product-constrained foreign key.
- Existing compatibility page readers preserve old no-filter methods but new product-aware methods are the only path used by strict normalized/admin flows.

- [ ] **Step 1: Add failing cross-product service/store tests**

Cover same `device_id` in two products, activation code mismatch, lease ownership mismatch, usage summary product mismatch, audit target product, and product-filtered admin pages. Assert the rejected operation does not change counters, status, lease slots, idempotency records, or audit success facts.

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cd backend && go test ./internal/service ./internal/store -run 'Product|Activation|Device|Lease|Usage|Audit' -count=1`

Expected: at least one new cross-product test fails because current repositories have no product predicate/field propagation.

- [ ] **Step 3: Implement the smallest propagation path**

Pass Product through activation, heartbeat, device lifecycle, lease create/renew/release/reclaim, direct-call summary and audit inputs. Update MemoryStore and normalized SQL together; use parameterized product predicates and compound indexes from migration 0023. Keep secrets and model正文 out of all new fields.

- [ ] **Step 4: Implement Profile product response**

Return the authenticated actor/session product in `ClientProfile`; if product membership or device product is disabled/mismatched, return the existing authorization error without guessing a product from the client.

- [ ] **Step 5: Run focused and normalized tests**

Run: `cd backend && gofmt -w internal/service internal/store && go test ./internal/service ./internal/store && go test -race ./internal/service ./internal/store`.

- [ ] **Step 6: Commit**

Commit: `feat(controlplane): enforce product ownership across resources`

### Task 5: 管理端 product 筛选与服务端授权收窄

**Files:**
- Modify: `backend/internal/httpapi/controlplane.go`
- Modify: `backend/internal/httpapi/audit.go`
- Modify: `backend/internal/httpapi/activation_page_test.go`
- Modify: `backend/internal/httpapi/device_contract_test.go`
- Modify: store/service page option files under `backend/internal/store` and `backend/internal/service`
- Modify: `接口契约/openapi.yaml`

**Interfaces:**
- `product` query is optional only for `super_admin` compatibility; ordinary admin queries intersect it with authorized products and cannot widen scope by omission or repetition.
- Current `admin/user` role remains compatibility-only until commercial Phase 2 delivers fixed permissions/custom roles; this task must not create a second role table.

- [ ] **Step 1: Write failing admin filter tests**

Seed both products with same-looking user/device IDs. Test omitted product, each product, unknown product, repeated product, and ordinary admin vs local `super_admin`; verify returned totals and 403/400 behavior.

- [ ] **Step 2: Run RED**

Run: `cd backend && go test ./internal/httpapi -run 'Admin.*Product|Product.*Admin|Device.*Product|Activation.*Product' -count=1`

Expected: FAIL because existing admin readers have no product option or authorization intersection.

- [ ] **Step 3: Implement bounded product options**

Add a shared validated `product` option to users/devices/activation/model lease/usage/audit page readers. Use allowlisted values, fixed page bounds and parameterized SQL; do not build a generic query engine. Ensure response DTOs include Product for every listed resource.

- [ ] **Step 4: Add OpenAPI contract coverage**

Document product query enum, product response fields and forbidden cross-product behavior. Do not hand-edit generated client artifacts.

- [ ] **Step 5: Run full backend verification for the completed slice**

Run: `cd backend && gofmt -w . && go vet ./... && go test ./... && go test -race ./... && go build ./...`; then run `bash tools/check-document-references.sh` and the repository OpenAPI contract command documented by the existing CI workflow.

- [ ] **Step 6: Commit**

Commit: `feat(admin): scope control-plane queries by product`

### Task 6: Phase 1 集成验证与文档状态回写

**Files:**
- Modify: `tasks/prd-douyin-desktop-control-plane-integration.md`
- Modify: `tasks/prd-douyin-desktop-control-plane-integration/phase-01-product-isolation.md`
- Modify: `tasks/prd-server-commercial-production-readiness.md`
- Modify: `系统架构总览.md`
- Modify: `管理系统架构.md`
- Modify: `数据库迁移与密钥存储约束.md`
- Create/Modify: contract fixtures under the existing backend test directories only when required by the verified API

**Interfaces:**
- Documentation records actual implemented scope, migration version, compatibility window and remaining P0 Phase 2～4 gaps.
- No document claims Profile offline signing, model delegated credentials, commercial seats or artifact signing are implemented by Phase 1.

- [x] **Step 1: Write an integration test matrix**

Record deterministic cases for login→activation→heartbeat→Profile in both products, same device ID isolation, cross-product activation rejection, admin filter scope, migration replay and restart.

- [x] **Step 2: Run the matrix**

Run the exact commands from Tasks 2–5 plus `go test -tags=postgres_integration ./internal/store` when PostgreSQL is available; record unsupported environment checks without marking them passed.

- [x] **Step 3: Update docs from evidence**

Mark only completed Phase 1 checklist items, add the actual migration/version and compatibility behavior, and leave Profile 6h/24h, model auth, OpenAPI artifact and cross-repo E2E as Not Started until their phases are implemented.

- [x] **Step 4: Final review and commit**

Run: `git diff --check && bash tools/check-document-references.sh && git status --short --branch`.

Commit: `docs: record product isolation phase 1 implementation`

## Plan Self-Review

- Coverage: Tasks 1–5 map to P0 Phase 1 product registry, data/session ownership and admin filtering; Task 6 prevents overclaiming later phases.
- Dependency: migration/catalog precedes normalized writes; session product precedes resource propagation; admin query work follows product fields.
- Scope: no subscription/order/payment/config/artifact/error-report implementation is included; those remain commercial Phase 3–11.
- Security: product mismatch is checked before mutation; secrets and model正文 remain excluded; product filters are allowlisted and bounded.
- Simplification: no ABAC, no second role system, no generic query builder, no new dependency, no server-side build.
