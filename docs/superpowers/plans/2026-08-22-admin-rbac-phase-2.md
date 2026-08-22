# 管理员 RBAC 与权限化管理端实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. 每个任务先写失败测试，再实现最小代码，主线程审查后提交。

**Goal:** 在不修改桌面端的前提下，为 Go 控制面和 React 管理端建立固定权限目录、产品范围自定义角色、服务端即时鉴权及权限化管理界面。

**Architecture:** 新增 RBAC 领域表承载权限目录、角色、角色权限和产品范围用户角色绑定；保留 users.role 作为迁移窗口兼容字段。Memory 与 normalized PostgreSQL 通过独立 Repository 提供相同语义，HTTP 使用 requirePermission，React 通过 /api/v1/admin/me 消费服务端权限。

**Tech Stack:** Go 1.22、PostgreSQL 前向迁移、现有 Memory Store/SQLMock、net/http、OpenAPI YAML、React 19、TypeScript、React Query、Ant Design 5、openapi-typescript 7.13.0。

## Global Constraints

- 默认使用简体中文沟通、文档和交付总结。
- 只修改 Go 服务端、PostgreSQL、OpenAPI 和 admin-web；不得修改 desktop/、desktop/ui/ 或运行桌面端验证。
- 不修改或删除历史数据库迁移；当前最新为 0023，本阶段新增前向迁移 0024。
- OpenAPI 源文件是唯一外部契约；admin-web/src/api/openapi.generated.ts 只能由生成器产生，禁止手改。
- 所有管理授权由服务端即时判定；前端隐藏导航不能代替 API 鉴权。
- products、user_products、会话产品和现有产品范围解析继续作为唯一产品事实源。
- 写操作使用现有 Idempotency-Key、结构化错误和审计 Outbox；权限、密码、Token 和请求正文不得写入日志/审计。
- 使用当前分支和工作区，不创建 Git worktree；服务器不构建源码。
- 新增行为先写失败测试；交付前执行服务端和管理端验证，但跳过桌面端检查。

## 文件职责地图

| 文件 | 职责 |
| --- | --- |
| backend/internal/controlplane/admin_rbac.go | 权限代码、目录、角色/绑定/授权 DTO 和规范化规则 |
| backend/migrations/0024_管理员RBAC与产品角色.up.sql | RBAC 表、约束、索引、权限种子和兼容回填 |
| backend/internal/store/admin_rbac_repository.go | RBAC Repository 接口和 Memory/SQL 共用记录类型 |
| backend/internal/store/memory_admin_rbac.go | Memory RBAC 读写；memory.go 仅负责状态初始化 |
| backend/internal/store/postgres_admin_rbac_repository.go | normalized PostgreSQL 查询、事务写入和并发保护 |
| backend/internal/service/admin_rbac.go | 权限计算、角色委派、幂等、最后超管保护和审计边界 |
| backend/internal/httpapi/admin_rbac.go | /admin/me、权限目录、角色和用户角色 API |
| backend/internal/httpapi/auth.go、controlplane.go | requirePermission 和现有管理路由权限切换 |
| 接口契约/openapi.yaml | RBAC 外部契约和稳定错误响应 |
| admin-web/src/features/admin-rbac/ | 权限 Hook、403 状态和角色管理页面 |
| admin-web/src/app/router.tsx、AppLayout.tsx | 路由元数据、菜单过滤和权限守卫 |
| tasks/prd-server-commercial-production-readiness/phase-02-admin-rbac.md | 阶段证据和状态更新 |

---

### Task 1: 固定权限目录与 RBAC 领域类型

Files:
- Create: backend/internal/controlplane/admin_rbac.go
- Create: backend/internal/controlplane/admin_rbac_test.go
- Modify: backend/internal/controlplane/errors.go

Produces: PermissionCode、PermissionCatalog()、AdminRole、AdminRoleAssignment、AdminAuthorization、NormalizePermissionCodes([]string)，以及 RBAC 稳定错误码，供后续任务使用。

- [ ] Step 1: 写失败测试。验证权限目录无重复且包含 users.read、roles.manage、activation_codes.reveal、model_pool.rotate_secret、operations.manage；重复权限规范化为唯一稳定顺序；未知权限返回 ADMIN_PERMISSION_UNKNOWN；super_admin 可表达全局范围。

    func TestNormalizePermissionCodes(t *testing.T) {
        got, err := NormalizePermissionCodes([]string{"users.manage", "users.read", "users.manage"})
        if err != nil || !slices.Equal(got, []string{"users.manage", "users.read"}) {
            t.Fatalf("got %#v, err %v", got, err)
        }
        if _, err := NormalizePermissionCodes([]string{"unknown.permission"}); !IsErrorCode(err, "ADMIN_PERMISSION_UNKNOWN") {
            t.Fatalf("unexpected error: %v", err)
        }
    }

- [ ] Step 2: 运行失败测试。命令：cd backend && go test ./internal/controlplane -run 'TestPermissionCatalog|TestNormalizePermissionCodes' -count=1。预期 FAIL，因为 RBAC 类型和目录尚不存在。
- [ ] Step 3: 实现目录和类型。固定定义基础、设备/激活、模型、审计/运维以及预留商业权限；目录返回防御性副本，规范化结果固定排序。定义 AdminAuthorization{UserID, Product, GlobalSuperAdmin, RoleCodes, Permissions}。
- [ ] Step 4: 增加错误码并重跑聚焦测试。覆盖权限不足、角色不存在、内建角色不可修改、角色仍被绑定、委派越权、最后超级管理员保护和产品范围不匹配；预期 PASS。
- [ ] Step 5: 提交。命令：git add backend/internal/controlplane/admin_rbac.go backend/internal/controlplane/admin_rbac_test.go backend/internal/controlplane/errors.go && git commit -m "feat: define admin RBAC domain catalog"。

### Task 2: 0024 迁移、Repository 契约与 Memory 实现

Files:
- Create: backend/migrations/0024_管理员RBAC与产品角色.up.sql
- Create: backend/internal/store/admin_rbac_repository.go
- Create: backend/internal/store/admin_rbac_repository_test.go
- Create: backend/internal/store/memory_admin_rbac.go
- Modify: backend/internal/store/memory.go
- Modify: backend/migrations/catalog.go、catalog_test.go

Consumes: Task 1 领域目录。

Produces: AdminRBACRepository：GetAdminAuthorization、ListAdminPermissions、ListAdminRoles、GetAdminRole、CreateAdminRole、UpdateAdminRole、DeleteAdminRole、ListUserAdminRoles、ReplaceUserAdminRoles。

- [ ] Step 1: 写迁移目录失败测试。要求 catalog 版本 24、文件名准确、版本连续且只允许追加；命令：cd backend && go test ./migrations -run 'Test.*Catalog' -count=1；预期因缺 0024 失败。
- [ ] Step 2: 实现 SQL 迁移。新增 admin_permissions、admin_roles、admin_role_permissions、user_admin_roles；加用户/产品/角色/权限外键、状态约束、产品查询索引、角色绑定唯一索引。product IS NULL 仅表示全局 super_admin，普通角色必须有产品；用约束/触发器和服务端双重保护。种子化完整固定目录和不可删除的 super_admin，授予全部权限；活动 users.role=admin 回填到 super_admin，role=user 不获得管理角色；usr_local_admin 绑定全局超管。不得修改 0001–0023。
- [ ] Step 3: 写 Memory 失败测试。覆盖角色权限并集、按产品过滤、全局超级管理员、重复绑定、角色绑定时禁止删除、替换角色的确定性排序和幂等冲突。
- [ ] Step 4: 实现 Memory 状态和 Repository。在 State 增加权限、角色、角色权限、用户角色绑定 map；所有返回值复制切片，所有写入走 MemoryStore.Run，验证固定权限、状态、产品范围并复用 idempotency map。
- [ ] Step 5: 运行并提交。命令：cd backend && go test ./internal/store ./migrations -run 'RBAC|Catalog' -count=1；随后提交新增迁移、catalog、Repository 和 Memory 文件，提交信息为 feat: add product-scoped RBAC persistence。

### Task 3: normalized PostgreSQL RBAC Repository

Files:
- Create: backend/internal/store/postgres_admin_rbac_repository.go
- Create: backend/internal/store/postgres_admin_rbac_repository_test.go
- Modify: backend/internal/store/repository.go 仅用于接口 wiring

Consumes: Task 1 领域记录和 Task 2 AdminRBACRepository。

- [ ] Step 1: 写 SQLMock 读测试。断言授权查询带 user_id、active 状态和当前 session product 谓词；全局 super_admin 单独放行；权限按代码稳定排序；无绑定返回空权限；数据库错误不转为授权成功。
- [ ] Step 2: 写 SQLMock 写测试。覆盖短事务、参数化 SQL、唯一冲突、角色仍绑定、产品外键失败、同幂等键重放、不同指纹冲突和最后超管保护。
- [ ] Step 3: 实现 Repository。使用现有 operationContext、PostgreSQL 错误翻译、FOR UPDATE 的短事务和固定排序；列表有界；提交未知使用现有 COMMIT_OUTCOME_UNKNOWN。
- [ ] Step 4: 运行并提交。命令：cd backend && go test ./internal/store -run 'AdminRBAC|RBAC' -count=1；提交信息为 feat: implement normalized RBAC repository。

### Task 4: RBAC Service、委派和审计

Files:
- Create: backend/internal/service/admin_rbac.go
- Create: backend/internal/service/admin_rbac_test.go
- Modify: backend/internal/service/controlplane.go、test_helpers_test.go（仅用于注入 Repository/fixture）

Consumes: Tasks 2–3 的 Repository 和现有审计 Outbox/幂等边界。

Produces: GetAdminAuthorization、权限/角色读取、角色 CRUD、用户角色读取和替换服务方法。

- [ ] Step 1: 写失败服务测试。覆盖权限并集、产品不匹配、超管范围、停用立即失效、普通管理者不能授予自身没有的权限、目标角色权限必须是操作者权限子集、普通角色不能全局、内建角色不可改、绑定角色不可删、最后超管/本地管理员保护、幂等重放/冲突和脱敏审计。
- [ ] Step 2: 实现授权读取。规范化角色输入和权限顺序；Repository 缺失/读取失败 fail-closed；只有持久化/兼容的 usr_local_admin 超管绑定具有全局权限，不能把任意 RoleAdmin 自动当作全局超管。
- [ ] Step 3: 实现角色和绑定写入。在服务边界校验全部字段后再计算幂等指纹；角色替换前读取操作者当前产品权限，目标权限必须为子集；绑定普通角色必须指定产品；只有现有全局超管可以分配/撤销超管；变更通过现有审计 Outbox 记录目标、结果和错误码，不记录秘密或正文。
- [ ] Step 4: 运行并提交。命令：cd backend && go test ./internal/service -run 'AdminRBAC|RBAC' -count=1；提交信息为 feat: enforce admin RBAC delegation rules。

### Task 5: OpenAPI、/admin/me、角色 API 和权限中间件

Files:
- Create: backend/internal/httpapi/admin_rbac.go
- Create: backend/internal/httpapi/admin_rbac_test.go
- Modify: backend/internal/httpapi/auth.go、controlplane.go、router.go
- Modify: 接口契约/openapi.yaml

Consumes: Task 4 service methods。

Produces: GET /api/v1/admin/me、权限目录、角色 CRUD、用户角色读取/替换及 generated client contract。

- [ ] Step 1: 先写 OpenAPI。定义权限、角色、绑定、本人授权、CRUD 请求/响应、Idempotency-Key、字段长度和 401/403/404/409 响应；预留商业权限只进目录，不创建业务路由。
- [ ] Step 2: 写失败 HTTP/契约测试。真实 Router 验证 /admin/me 登录成功/401、普通用户空权限、各权限点对应 403/成功、产品参数不能扩大范围、未知权限 400、幂等冲突 409。
- [ ] Step 3: 实现 requirePermission。签名：func (a *authenticator) requirePermission(code controlplane.PermissionCode, next func(http.ResponseWriter, *http.Request, controlplane.Actor)) http.Handler。它从 Bearer Session 获取 actor，按会话 product 查询即时权限；读取失败返回服务不可用，禁止默认放行。GET /admin/me 只需有效 Bearer，普通用户返回空权限。
- [ ] Step 4: 按矩阵替换所有生产 requireAdmin。用户/设备/激活码/模型/租约/审计/安全路由分别使用设计文档中的固定权限点；rg -n 'requireAdmin' backend/internal/httpapi 不得再有生产路由调用。
- [ ] Step 5: 生成和验证。命令：cd admin-web && pnpm api:generate；以及 cd backend && go test ./internal/httpapi -run 'AdminRBAC|Permission|Contract' -count=1；提交信息为 feat: gate admin APIs by fixed permissions。

### Task 6: React 权限壳和 RBAC 管理页

Files:
- Create: admin-web/src/features/admin-rbac/useAdminAuthorization.ts
- Create: admin-web/src/features/admin-rbac/AdminForbiddenPage.tsx
- Create: admin-web/src/features/admin-rbac/AdminRbacPage.tsx
- Create: admin-web/src/features/admin-rbac/adminRbac.test.mjs
- Modify: admin-web/src/types/api.ts、src/app/router.tsx、src/components/AppLayout.tsx
- Modify: current management pages only for permission-based read/write controls

Consumes: Task 5 generated OpenAPI types and endpoints.

- [ ] Step 1: 写失败前端测试。权限集合缺失/加载中/请求错误均不得放行；菜单和直接 URL 使用同一权限元数据；403 fixture 显示无权限；权限变更后重新取数。
- [ ] Step 2: 实现 useAdminAuthorization()。以 React Query key ['admin-me'] 获取权限，提供 isLoading、error、permissions、roles、isSuperAdmin、can(permission)、refresh()；不得把权限写入登录 Session 或 Token。
- [ ] Step 3: 实现路由和菜单守卫。NavItem 与 route 增加 requiredPermission；菜单过滤与直接 URL 守卫复用同一元数据；加载态使用现有 Ant Design 模式，拒绝使用 AdminForbiddenPage。
- [ ] Step 4: 实现 RBAC 页面。用 Ant Design Table、Form、Checkbox.Group/Tree、Modal、Descriptions、Tag、Result 实现角色列表、创建/编辑/停用/删除、权限分组和产品范围用户角色替换；内建超管控件禁用但服务端仍拒绝非法请求，不覆盖 .ant-* 样式。
- [ ] Step 5: 为既有页面接入权限。dashboard/users/devices/activation/model pool/model lease/audit/security 使用固定读写权限；手工 API 403 显示错误状态；测试 loading、empty、error、403、键盘标签和提交重试。
- [ ] Step 6: 运行并提交。依次运行 cd admin-web && pnpm api:check、pnpm typecheck、pnpm test、pnpm build；提交信息为 feat: add permission-aware admin management UI。

### Task 7: 集成验证、文档闭环和主线程审查

Files:
- Modify: tasks/prd-server-commercial-production-readiness/phase-02-admin-rbac.md
- Modify: tasks/prd-douyin-desktop-control-plane-integration/phase-01-product-isolation.md
- Modify: tasks/prd-server-commercial-production-readiness.md
- Modify: 系统架构总览.md、管理系统架构.md、长任务开发总计划.md
- Create: backend/internal/httpapi/admin_rbac_integration_test.go only if existing contract tests cannot cover the matrix

- [ ] Step 1: 在文档前补集成测试。Memory 和 normalized SQLMock 覆盖完整 401/403/成功权限矩阵；增加 postgres_integration 标签的 0024 重放、种子一致性、产品角色、停用立即失效和最后超管保护测试。没有 TEST_POSTGRES_URL 时只能明确 skip，不得伪称真实数据库通过。
- [ ] Step 2: 执行服务端/管理端门禁。依次运行：
    cd backend && go test ./...
    cd backend && go test -race ./...
    cd backend && go vet ./...
    cd backend && go build ./...
    cd backend && go test -tags=postgres_integration ./migrations -run 'RBAC|Migration' -count=1
    cd admin-web && pnpm api:check
    cd admin-web && pnpm typecheck
    cd admin-web && pnpm test
    cd admin-web && pnpm build
    bash tools/check-document-references.sh
    git diff --check
    test -z "$(gofmt -l backend/internal 2>/dev/null)"
- [ ] Step 3: 更新阶段文档。只勾选有命令/测试证据的事项；记录真实 PostgreSQL 是否执行、桌面端验证按要求跳过，以及剩余发布环境门禁。
- [ ] Step 4: 主线程代码总监审查和清理。检查 git diff、未使用导入/导出、残留生产 requireAdmin、生成文件漂移和本次暴露的死代码；运行 git status --short --branch、git diff --check 后提交文档/清理。

## 依赖与审查门

- Task 1 → Tasks 2–5；Task 2/3 → Task 4；Task 4 → Task 5；Task 5 → Task 6；Task 7 在 Tasks 1–6 后执行。
- 每个任务结束后主线程审查 diff、检查未使用代码并执行聚焦测试，再接受下一任务。
- API 契约稳定后，后端补强与管理端页面可由独立子线程并行；不得创建 worktree，也不得主动停止运行中的子线程。

## 计划自审

- 覆盖设计中的目录、迁移/种子、Memory/PostgreSQL、即时鉴权、现有路由矩阵、RBAC API、React 权限壳、审计/幂等、测试、文档和桌面端排除。
- 已检查计划步骤，每项都有实际路径、接口或测试命令；没有 TBD、TODO 或稍后实现等占位描述。
- 类型链路一致：Task 1 提供权限类型，Task 2 提供 Repository，Task 4 提供服务，Task 5 提供 HTTP/OpenAPI，Task 6 消费生成类型，Task 7 汇总证据。
