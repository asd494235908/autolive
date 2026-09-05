# douyin-desktop Cloud Auth and Sync Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task with review checkpoints.

**Goal:** 在不迁移现有桌面业务后端的前提下，让 douyin-desktop 通过本地 sidecar 使用 autoLive 登录和设备授权，并跨设备同步白名单用户资产。

**Architecture:** React 只访问本机 FastAPI；sidecar 持有内存 Access Token 和 Keyring Refresh Token，通过 HTTPS 调用 autoLive。autoLive 以 `user + product` 保存 revision 化 JSON 事实，设备认证、CAS 和幂等在服务端执行；本地 SQLite 保存 cursor、云 revision、内容哈希与冲突，不保存远端 Token。

**Tech Stack:** Go 1.22 `net/http`、PostgreSQL、OpenAPI；Python 3.11/FastAPI/httpx/keyring/SQLite/Alembic；React/TypeScript/Ant Design/TanStack Query/Vitest。

---

## 冻结的并行所有权

- DEV-665-A（左侧栏 autoLive 服务端）：只修改 `/Users/mac/work/gepin/autoLive/backend/**`、`接口契约/openapi.yaml`、本 PRD/服务端文档；不得修改任何 autoLive 客户端或管理端。
- DEV-665-B（当前会话 sidecar 云会话）：新增 `desktop_backend/integrations/autolive/**`、`desktop_backend/application/cloud_session_service.py`、`tests/unit/test_autolive_*.py`；不修改共享装配、迁移或前端。
- DEV-665-C（当前会话本地资产同步）：新增 `desktop_backend/application/cloud_sync_service.py`、`desktop_backend/infrastructure/cloud_sync_repository.py`、`desktop_backend/infrastructure/cloud_sync_codec.py`、`tests/unit/test_cloud_sync_*.py`；不修改共享装配、迁移或前端。
- DEV-665-D（左侧栏桌面体验）：只修改 `apps/desktop/src/features/cloud/**` 及其同目录测试；不修改 ShellPort、路由、生成契约或 Python。
- 主线程：共享 Alembic 迁移、SQLModel/AI 仓储兼容、FastAPI schema/routes/app 装配、ShellPort/AppRoutes/导航、OpenAPI 生成、跨层测试、文档、代码总监审查与修复。

## Task 1：autoLive 服务端存储与领域契约（DEV-665-A）

**Files:**

- Create: `backend/migrations/0028_douyin_desktop_user_asset_sync.up.sql`
- Modify: `backend/migrations/catalog.go`
- Create: `backend/internal/controlplane/client_sync.go`
- Create: `backend/internal/store/client_sync_repository.go`
- Create: `backend/internal/store/postgres_client_sync_repository.go`
- Create/modify focused tests beside these files

**TDD:**

- [ ] 先写迁移目录、产品/用户复合外键、revision、墓碑、payload 大小和幂等唯一约束的失败测试并确认 RED。
- [ ] 先写 kind/schema/秘密键/数量/大小校验和 CAS/幂等领域失败测试并确认 RED。
- [ ] 最小实现迁移与领域类型；PostgreSQL 短事务锁 workspace，分配 revision 并保存回执。
- [ ] 运行目标 Go 测试并确认 GREEN；不得引入通用 JSON 数据库或兼容内存回退。

## Task 2：autoLive 认证 API 与 OpenAPI（DEV-665-A）

**Files:**

- Create: `backend/internal/httpapi/client_sync.go`
- Modify: `backend/internal/httpapi/router.go`
- Modify only required service/store construction files under `backend/`
- Modify: `接口契约/openapi.yaml`
- Create/modify focused HTTP, isolation and PostgreSQL integration tests

**TDD:**

- [ ] 先写 GET 分页、POST CAS/幂等、跨用户/产品/设备、失效授权和秘密拒绝的失败测试。
- [ ] 复用现有桌面认证上下文和设备绑定检查；请求体不能提交 scope。
- [ ] 实现两个窄端点、稳定错误映射和 OpenAPI schema，保持批次/正文严格上限。
- [ ] 运行受影响 Go 单元、HTTP、迁移与 PostgreSQL 集成测试；安全/迁移边界完成后运行 `go test ./...`。

## Task 3：sidecar 云会话（DEV-665-B）

**Files:**

- Create: `desktop_backend/integrations/autolive/client.py`
- Create: `desktop_backend/integrations/autolive/contracts.py`
- Create: `desktop_backend/integrations/autolive/credentials.py`
- Create: `desktop_backend/integrations/autolive/device_identity.py`
- Create: `desktop_backend/application/cloud_session_service.py`
- Create: `tests/unit/test_autolive_client.py`
- Create: `tests/unit/test_autolive_credentials.py`
- Create: `tests/unit/test_cloud_session_service.py`

**TDD:**

- [ ] 用合成 HTTP transport 和内存凭据端口先覆盖登录→激活→Profile、刷新一次、退出、未授权、超时、429 和脱敏错误 RED。
- [ ] 实现固定产品、HTTPS 默认、随机持久设备 ID、内存 Access Token 和 Keyring Refresh Token；密码/Token 不进 repr/log。
- [ ] 写请求只在同 mutation ID 下重放；非幂等请求不自动重试。
- [ ] 运行目标 conda pytest 与 Ruff 并确认 GREEN。

## Task 4：sidecar 本地同步核心（DEV-665-C）

**Files:**

- Create: `desktop_backend/infrastructure/cloud_sync_codec.py`
- Create: `desktop_backend/infrastructure/cloud_sync_repository.py`
- Create: `desktop_backend/application/cloud_sync_service.py`
- Create: `tests/unit/test_cloud_sync_codec.py`
- Create: `tests/unit/test_cloud_sync_repository.py`
- Create: `tests/unit/test_cloud_sync_service.py`

**TDD:**

- [ ] 先覆盖八种 kind 精确序列化、秘密字段排除、规范化 hash 和墓碑 RED。
- [ ] 先覆盖空云首传、先拉后推、增量 cursor、同项双改冲突、失败不前移和 mutation 重试 RED。
- [ ] 实现最小 codec、SQLite 状态仓储与单次同步算法；不引入队列、多 worker、缓存或通用插件层。
- [ ] 运行目标 conda pytest 与 Ruff 并确认 GREEN。

## Task 5：本地迁移、资产应用和 FastAPI 装配（主线程）

**Files:**

- Create: `migrations/versions/0021_cloud_auth_sync.py`
- Modify: `desktop_backend/infrastructure/models.py`
- Modify: `desktop_backend/infrastructure/database.py`
- Modify: `desktop_backend/infrastructure/ai_repository.py`
- Create: `desktop_backend/api/routes/cloud.py`
- Modify: `desktop_backend/api/schemas.py`
- Modify: `desktop_backend/api/v1.py`
- Modify: `desktop_backend/api/app.py`
- Create/modify focused migration, repository and API tests

**TDD:**

- [ ] 先写 0020→0021、空库→head 和重复升级失败测试；确认云记忆来源二选一约束 RED。
- [ ] 添加云会话状态、登录、退出、同步、冲突读取和两种解决动作 API 失败测试。
- [ ] 以最小共享接线将两个服务注入默认应用；Token 不进入响应或数据库。
- [ ] 应用云知识后复用现有索引重建入口；索引失败单独报告，不能回滚已确认云事实或冒充成功。
- [ ] 运行直接上下游 conda pytest；迁移为核心共享变更，目标绿色后运行 Python 全量回归。

## Task 6：React 云门禁与同步状态（DEV-665-D + 主线程）

**Files:**

- Create: `apps/desktop/src/features/cloud/CloudGate.tsx`
- Create: `apps/desktop/src/features/cloud/CloudStatus.tsx`
- Create: `apps/desktop/src/features/cloud/CloudConflicts.tsx`
- Create: `apps/desktop/src/features/cloud/*.test.tsx`
- Modify by main: `apps/desktop/src/app/shell-port.tsx`
- Modify by main: `apps/desktop/src/app/AppRoutes.tsx`
- Modify by main: `apps/desktop/src/app/navigation.tsx` and/or global status integration point
- Regenerate by main: `apps/desktop/src/lib/generated/openapi.ts`

**TDD:**

- [ ] 组件任务先用假端口覆盖登录、授权错误、同步中/成功/失败、手动同步和冲突选择 RED。
- [ ] 实现 Ant Design 原生组件；密码字段不保留到持久状态，页面不直接请求 autoLive。
- [ ] 主线程接入真实 ShellPort 和路由，生成本地 OpenAPI 类型并验证无漂移。
- [ ] 运行目标 Vitest、TypeScript；共享导航和门禁稳定后运行前端全量 Vitest 与本地 Vite build。

## Task 7：跨仓库集成、代码总监审查与文档

**Files:**

- Create: `tests/integration/test_cloud_auth_sync_e2e.py`
- Update: `/Users/mac/work/gepin/douyin_spider/DEVELOPMENT_TASKS.md`
- Update affected `tasks/` architecture/phase and `docs/` operation files in both repositories
- Create completion report under `douyin_spider/docs/superpowers/reports/`

**Steps:**

- [ ] 使用两个临时本地数据库和合成 autoLive HTTP 服务验证首机上传→第二机拉取→增量→冲突→退出；不连接真实抖音。
- [ ] 等待所有并行任务自然完成，主线程只审查本次变更并修复 Critical/Important。
- [ ] 执行秘密扫描、未使用代码/依赖检查、`git diff --check`（autoLive）和受影响验证。
- [ ] 明确记录未运行的 Windows 安装包、容器、外部模型和真实业务结果。
- [ ] 宣布“正在进行消融实验。”，删除无法对应已批准需求或验收的抽象、fallback、缓存、队列和配置。
