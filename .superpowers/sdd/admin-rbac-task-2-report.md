# Admin RBAC Phase 2 / Task 2 报告

日期：2026-08-22

提交 SHA：`13c93772b6b1e0393c0f73d15e1ac9ef9bd4ee7e`（short: `13c9377`）

## 需求边界

- 只读取并执行 `/Users/mac/work/gepin/autoLive/.superpowers/sdd/admin-rbac-task-2-brief.md`
- 未读取/使用旧的 `.superpowers/sdd/task-2-brief.md`
- 未创建 worktree
- 未触碰 `desktop/` 或 `desktop/ui/`
- 直接在当前工作区完成 0024 migration、`AdminRBACRepository` 契约、Memory 实现

## 真实改动文件

- `backend/migrations/catalog.go`
- `backend/migrations/catalog_test.go`
- `backend/migrations/0024_管理员RBAC与产品角色.up.sql`
- `backend/internal/store/memory.go`
- `backend/internal/store/admin_rbac_repository.go`
- `backend/internal/store/admin_rbac_repository_test.go`
- `backend/internal/store/memory_admin_rbac.go`
- `/Users/mac/work/gepin/autoLive/.superpowers/sdd/admin-rbac-task-2-report.md`

## 实现摘要

1. migration 0024
   - 新增 `admin_permissions`、`admin_roles`、`admin_role_permissions`、`user_admin_roles`
   - 为产品、用户-产品归属、角色、权限建立外键与索引
   - 约束 `super_admin` 只能是全局角色/绑定
   - 通过 trigger 校验角色绑定的 product 与角色 product 一致
   - 从 `PermissionCatalog()` 落库权限种子
   - 初始化内建 `super_admin`
   - 回填 `users.role = 'admin'` 的 active 用户到全局 `super_admin`

2. store 契约
   - 新增 `AdminRBACRepository`
   - 新增 `AdminRoleRecord`、`AdminRoleWriteRecord`、`AdminRoleDeleteRecord`、`UserAdminRoleReplaceRecord`

3. Memory 实现
   - 在 `State` 中新增管理员 RBAC 相关 map
   - 初始化时自动 seed 权限目录和内建 `super_admin`
   - 实现：
     - `GetAdminAuthorization`
     - `ListAdminPermissions`
     - `ListAdminRoles`
     - `GetAdminRole`
     - `CreateAdminRole`
     - `UpdateAdminRole`
     - `DeleteAdminRole`
     - `ListUserAdminRoles`
     - `ReplaceUserAdminRoles`

4. TDD / 自审修正
   - 先补 migration contract test，再补 store 行为测试
   - 自审时发现 memory 幂等实现先写记录、后做业务校验；在无事务的 memory store 中会污染失败请求
   - 已通过红绿测试修正为：先检查已有幂等记录，业务成功后再写入幂等记录

## 执行命令与输出

### 1) migration 红灯

```bash
cd backend && go test ./migrations -run 'Test.*Catalog|TestLatestVersionIsAdminRBACMigration24|TestMigration24AdminRBACContract' -count=1
```

输出摘录：

```text
LatestVersion = 23, want 24
read migration 0024: open 0024_管理员RBAC与产品角色.up.sql: file does not exist
```

### 2) migration 绿灯

```bash
cd backend && go test ./migrations -run 'Test.*Catalog|TestLatestVersionIsAdminRBACMigration24|TestMigration24AdminRBACContract' -count=1
```

输出：

```text
ok  	autoLive/backend/migrations	0.937s
```

### 3) store 首轮红灯（缺契约/实现）

```bash
cd backend && go test ./internal/store -run 'AdminRBAC|RBAC' -count=1
```

输出摘录：

```text
... ListAdminPermissions undefined
... CreateAdminRole undefined
... ReplaceUserAdminRoles undefined
```

### 4) store 实现后首轮失败（NewState 初始化问题）

```bash
cd backend && go test ./internal/store -run 'AdminRBAC|RBAC' -count=1
```

输出摘录：

```text
internal/store/memory.go:114:21: undefined: state
internal/store/memory.go:115:9: undefined: state
```

### 5) store 第二轮失败（测试前提与既有 membership 语义冲突）

```bash
cd backend && go test ./internal/store -run 'AdminRBAC|RBAC' -count=1
```

输出摘录：

```text
--- FAIL: TestMemoryStoreAdminRBACAuthorizationUsesPermissionUnionAndProductFilter
ReplaceUserAdminRoles(scoped) error = ADMIN_PRODUCT_SCOPE_MISMATCH
```

原因：

- `memoryUserHasProduct` 的既有语义是：用户没有任何 membership 时，默认拥有 `autolive`
- 但一旦用户已经存在显式 membership，就必须显式拥有对应 product
- 测试只给了 `douyin_desktop` membership，缺少 `autolive` membership，已补齐测试前提

### 6) store 绿灯

```bash
cd backend && go test ./internal/store -run 'AdminRBAC|RBAC' -count=1
```

输出：

```text
ok  	autoLive/backend/internal/store	1.280s
```

### 7) 自审新增回归测试红灯（失败请求污染幂等键）

```bash
cd backend && go test ./internal/store -run 'TestMemoryStoreReplaceUserAdminRolesFailedAttemptDoesNotConsumeIdempotency' -count=1
```

第一次输出摘录（先修测试前提）：

```text
ReplaceUserAdminRoles(first attempt) error = <nil>, want product scope mismatch
```

第二次输出摘录（命中真实问题）：

```text
retry assignments = []controlplane.AdminRoleAssignment(nil), want []controlplane.AdminRoleAssignment{...}
```

### 8) 自审回归测试绿灯

```bash
cd backend && go test ./internal/store -run 'TestMemoryStoreReplaceUserAdminRolesFailedAttemptDoesNotConsumeIdempotency' -count=1
```

输出：

```text
ok  	autoLive/backend/internal/store	1.296s
```

### 9) gofmt

```bash
gofmt -w /Users/mac/work/gepin/autoLive/backend/migrations/catalog.go /Users/mac/work/gepin/autoLive/backend/migrations/catalog_test.go /Users/mac/work/gepin/autoLive/backend/internal/store/admin_rbac_repository.go /Users/mac/work/gepin/autoLive/backend/internal/store/admin_rbac_repository_test.go /Users/mac/work/gepin/autoLive/backend/internal/store/memory.go /Users/mac/work/gepin/autoLive/backend/internal/store/memory_admin_rbac.go
```

输出：无

### 10) 提交前聚焦验证

```bash
cd /Users/mac/work/gepin/autoLive/backend && go test ./internal/store ./migrations -run 'AdminRBAC|RBAC|Catalog|Migration24' -count=1
```

输出：

```text
ok  	autoLive/backend/internal/store	0.876s
ok  	autoLive/backend/migrations	0.421s
```

### 11) 提交

```bash
git -C /Users/mac/work/gepin/autoLive add backend/migrations/catalog.go backend/migrations/catalog_test.go 'backend/migrations/0024_管理员RBAC与产品角色.up.sql' backend/internal/store/memory.go backend/internal/store/admin_rbac_repository.go backend/internal/store/admin_rbac_repository_test.go backend/internal/store/memory_admin_rbac.go && git -C /Users/mac/work/gepin/autoLive commit -m 'feat: add product-scoped RBAC persistence'
```

输出：

```text
[dev-2.0 13c9377] feat: add product-scoped RBAC persistence
 7 files changed, 1277 insertions(+), 2 deletions(-)
 create mode 100644 backend/internal/store/admin_rbac_repository.go
 create mode 100644 backend/internal/store/admin_rbac_repository_test.go
 create mode 100644 backend/internal/store/memory_admin_rbac.go
 create mode 100644 backend/migrations/0024_管理员RBAC与产品角色.up.sql
```

### 12) 提交后最终验证

```bash
cd /Users/mac/work/gepin/autoLive/backend && go test ./internal/store ./migrations -run 'AdminRBAC|RBAC|Catalog|Migration24' -count=1
```

输出：

```text
ok  	autoLive/backend/internal/store	1.216s
ok  	autoLive/backend/migrations	1.683s
```

## 自审结论

- 需求简报要求的 migration / repository contract / memory implementation 已落地
- 关键行为已覆盖：
  - 权限并集
  - 按产品过滤
  - 全局超级管理员
  - 重复绑定去重
  - 绑定中的角色不可删除
  - 替换角色的确定性排序
  - 幂等冲突
  - 失败请求不消耗幂等键
- 已检查未使用导入；当前新增文件未留下未使用 import

## Concerns

1. 本轮按用户要求只做 focused tests；未执行真实 PostgreSQL 应用/回滚 0024 migration，也未补 Postgres repository 实现（不在本 Task 2 范围内）。
2. `memoryUserHasProduct` 仍保留仓库既有语义：用户若完全没有 membership，则默认视为拥有 `autolive`。本次实现已与该语义保持一致，但它会影响未来 RBAC 测试前提设计。

---

## 2026-08-22 Task 2 修复补记

### 修复范围

- 仅修改 `backend/migrations/catalog_test.go`
- 将过时的 `TestLatestVersionIsProductIsolationMigration23` 断言移除，消除与 `LatestVersion = 24`、`TestLatestVersionIsAdminRBACMigration24` 的冲突
- 新增确定性 catalog 测试：解析嵌入 migration 文件名，断言版本号严格为 `0001..LatestVersion`、无重复、无缺号
- 未修改任何旧 migration SQL 正文
- 未触碰 `desktop/`

### 红灯证据

```bash
cd backend && go test ./migrations -count=1
```

输出摘录：

```text
--- FAIL: TestLatestVersionIsProductIsolationMigration23 (0.00s)
    catalog_test.go:74: LatestVersion = 24, want 23
FAIL
```

### 本次改动后的验证

```bash
cd backend && go test ./migrations -count=1
```

```text
ok  	autoLive/backend/migrations	0.696s
```

```bash
cd backend && go test ./internal/store -run 'AdminRBAC|RBAC' -count=1
```

```text
ok  	autoLive/backend/internal/store	1.029s
```

```bash
gofmt -w /Users/mac/work/gepin/autoLive/backend/migrations/catalog_test.go
```

输出：无

```bash
git -C /Users/mac/work/gepin/autoLive diff --check
```

输出：无

### 结论

- migration catalog 现在只有一套最新版本事实：`LatestVersion = 24`
- 新增的连续性测试补上了“只能追加、不能缺号/重复”的门禁
