# 管理员 RBAC 与权限化管理端设计

## 文档状态

- 状态：已确认设计，待实施
- 所属阶段：服务端商业化与生产就绪 Phase 2
- 关联阶段：多产品控制面 Phase 1
- 变更范围：Go 服务端、PostgreSQL、OpenAPI、React 管理端
- 明确排除：Rust/Tauri 桌面端、桌面端生成文件和桌面端验证

## 1. 目标与边界

当前控制面使用 `users.role` 的 `admin/user` 两级角色，所有管理接口共用 `requireAdmin`，管理端只判断是否存在登录会话。该模型无法表达产品范围、多个自定义角色、单个权限点授权和权限实时撤销。

本阶段将建立固定权限目录、自定义角色、角色权限关系和按产品绑定的用户角色关系。服务端是唯一授权事实源，每个受保护请求按当前用户、当前会话产品读取有效权限；Access Token 和持久化 Session 不保存完整权限快照。React 管理端只消费 `/api/v1/admin/me` 展示界面，前端隐藏不能替代服务端鉴权。

本阶段不实现订阅、订单、支付、套餐、席位、Profile 离线签名、模型委托凭证、制品分发、公共配置、错误报告或反馈业务；这些能力只消费本阶段的权限和产品范围事实。也不修改桌面端。

## 2. 方案与取舍

### 2.1 选定方案：兼容迁移 + 服务端即时鉴权

新增专用 RBAC 领域表和 Repository/Service 边界，保留 `users.role` 作为兼容字段和迁移窗口内的身份标记。迁移将现有 `role=admin` 用户回填为内建 `super_admin` 绑定，`role=user` 用户不自动获得管理角色；新的管理访问全部经过固定权限点。

`super_admin` 是全局角色，可跨产品读取和管理；普通角色绑定到一个明确产品。一个用户在同一产品可以绑定多个角色，有效权限为并集，不提供用户级 allow/deny 覆盖。角色目录全局复用，产品范围只存在于用户角色绑定，不建立第二套产品或成员事实源。

### 2.2 未选方案

- 继续扩展 `admin/user`：改动小，但无法满足自定义角色、产品范围、权限委派和立即撤销。
- 只在 React 中隐藏菜单：不能防止直接调用 API，也不能满足服务端授权和审计要求。
- 只支持 PostgreSQL：会破坏现有 Memory 测试和本地开发边界；本阶段同时提供 Memory 与 normalized PostgreSQL 实现。

## 3. 权限与数据模型

### 3.1 固定权限目录

权限代码由 Go 常量/目录和迁移种子共同维护，启动时检查代码目录与数据库种子完全一致。管理员只能从目录选择权限，不能提交任意权限字符串。

目录分组如下：

- 基础：`dashboard.read`、`users.read`、`users.manage`、`roles.read`、`roles.manage`、`roles.assign`、`admin_security.manage`
- 设备/激活：`devices.read`、`devices.manage`、`activation_codes.read`、`activation_codes.manage`、`activation_codes.reveal`、`activation_codes.switch_device`
- 模型：`model_pool.read`、`model_pool.manage`、`model_pool.test`、`model_pool.rotate_secret`、`model_leases.read`、`model_leases.reclaim`、`model_usage.read`
- 审计与运维：`audit_logs.read`、`operations.read`、`operations.manage`
- 预留商业能力：`plans.read`、`plans.manage`、`plans.publish`、`orders.read`、`orders.reconcile`、`subscriptions.read`、`subscriptions.adjust`、`payments.read`、`payments.reconcile`、`password_resets.read`、`password_resets.retry`、`artifacts.read`、`artifacts.manage`、`artifacts.publish`、`artifacts.revoke`、`public_config.read`、`public_config.manage`、`public_config.publish`、`public_config.rollback`、`error_reports.read`、`error_reports.manage`、`feedback.read`、`feedback.manage`

预留权限在本阶段只进入目录，不创建未实现的业务路由或页面。

### 3.2 PostgreSQL 迁移

新增前向迁移 `0024`，不修改历史迁移，包含：

- `admin_permissions`：权限代码、分组、描述、目录版本和启用状态。
- `admin_roles`：角色 ID、稳定代码、名称、描述、内建标记、状态和时间字段。
- `admin_role_permissions`：角色到固定权限的多对多关系，主键为 `(role_id, permission_code)`。
- `user_admin_roles`：用户、角色、产品范围和时间字段；`product IS NULL` 只表示全局 `super_admin`，普通角色必须绑定已注册产品。

迁移为目录种子创建内建 `super_admin` 并授予全部权限；将已有活动 `role=admin` 用户回填到该角色，将 `role=user` 保持无管理角色。`usr_local_admin` 的全局 `super_admin` 绑定由数据库约束/触发器和服务端双重保护，不能删除、停用或撤销最后一个有效超级管理员。

`user_admin_roles` 使用产品外键、用户/角色外键、有效状态约束、按产品查询索引和全局绑定唯一索引。角色删除前必须没有用户绑定；内建角色不可改名、删除或停用。所有写操作仍使用现有幂等记录边界，不在事务内执行网络调用。

### 3.3 Memory 兼容实现

Memory `State` 增加权限目录、角色、角色权限和用户角色绑定集合。初始化时种子与 PostgreSQL 相同；测试可以显式构造普通产品角色。旧测试中只设置 `usr_local_admin` 的 `RoleAdmin` 仍映射到内建超级管理员，其他测试管理员必须通过显式角色绑定表达权限，避免用 `RoleAdmin` 继续绕过新授权模型。

## 4. 服务端边界

### 4.1 授权读取

新增 `AdminAuthorizationReader`/`AdminRBACRepository` 边界，提供：

- 当前用户在会话产品下的有效角色和权限并集。
- 是否拥有全局 `super_admin` 范围。
- 权限目录、角色列表/详情和用户角色绑定的有界读取。

`ControlPlane` 提供角色和用户角色的校验、委派、幂等写入与审计输入构造。HTTP 层只负责认证、请求解析、产品范围解析和错误映射，不直接读 Memory 状态或拼接 SQL。

### 4.2 `requirePermission`

`requireAdmin` 迁移为 `requirePermission(permissionCode)`。中间件顺序为：读取 Bearer Session → 校验用户和会话产品 → 调用当前权限读取 → 无权限返回稳定 `403` → 处理业务。权限撤销后不需要等待重新登录即可影响下一次请求。

当前管理路由按以下矩阵迁移：

| 路由能力 | 权限点 |
| --- | --- |
| 管理员密码轮换 | `admin_security.manage` |
| 用户列表、用户设备和授权摘要读取 | `users.read` |
| 用户创建、编辑、禁用、重置密码和授权策略写入 | `users.manage` |
| 设备列表/详情 | `devices.read` |
| 设备禁用/解绑 | `devices.manage` |
| 激活码列表 | `activation_codes.read` |
| 激活码创建/作废 | `activation_codes.manage` |
| 模型账号列表 | `model_pool.read` |
| 模型账号创建/编辑/禁用 | `model_pool.manage` |
| 模型测试/密钥轮换 | `model_pool.test` / `model_pool.rotate_secret` |
| 用量、租约、审计查询 | `model_usage.read` / `model_leases.read` / `audit_logs.read` |
| 租约回收 | `model_leases.reclaim` |

已有产品范围解析继续以会话产品为授权上限；查询参数只能收窄范围，不能扩大范围。全局超级管理员的跨产品高风险写操作沿用现有幂等和审计边界，并要求理由/确认字段的扩展点；本阶段先保证普通角色不能越过产品范围。

### 4.3 管理 API

新增 OpenAPI 路由：

- `GET /api/v1/admin/me`：返回本人用户、会话产品、角色、有效权限和是否全局超级管理员；不返回他人数据。
- `GET /api/v1/admin/permissions`：返回固定权限目录，需 `roles.read`。
- `GET /api/v1/admin/roles`、`GET /api/v1/admin/roles/{role_id}`：有界读取，需 `roles.read`。
- `POST /api/v1/admin/roles`、`PATCH /api/v1/admin/roles/{role_id}`、`DELETE /api/v1/admin/roles/{role_id}`：需 `roles.manage`，写操作必须带 `Idempotency-Key`。
- `GET /api/v1/admin/users/{user_id}/roles`、`PUT /api/v1/admin/users/{user_id}/roles`：需 `roles.assign`；请求只能绑定操作者当前权限子集内的角色，且必须指定产品范围。

稳定错误至少覆盖：权限不足、未知权限、角色不存在、内建角色不可修改、角色仍被绑定、角色委派越权、最后超级管理员保护、产品范围不匹配、幂等冲突和提交结果未知。密码、Token、权限密钥和完整审计正文不进入响应、日志或审计 payload。

### 4.4 审计与幂等

角色创建/修改/停用/删除、权限变更、用户角色替换和高风险拒绝写入既有审计 Outbox 边界，记录操作者、产品、目标角色/用户、结果、状态码、错误码和 request ID，不记录密码、Token 或大段请求正文。幂等指纹包含动作、目标、产品、角色列表和权限列表的规范化结果。

## 5. 管理端设计

React 启动后通过 React Query 请求 `/api/v1/admin/me`，权限数据作为服务端状态，不复制进长期业务 Store。会话刷新后重新获取；收到管理 API `403` 时刷新 `/admin/me` 并显示无权限状态。

- 导航和路由元数据声明所需权限；无权限直接访问显示 Ant Design `Result` 的 403 页面。
- 新增角色列表/详情/编辑页，使用 `Table`、`Form`、`Checkbox.Group` 或 `Tree`、`Modal`。
- 用户管理页增加按产品的多角色分配操作；本地管理员的超级管理员绑定显示为不可操作。
- 现有用户、设备、激活码、模型池、租约、审计页面按权限隐藏读/写操作按钮；隐藏只改善体验，API 403 仍须正确处理。
- loading、empty、error、403、提交竞态和失败重试均由现有页面模式和 Ant Design 组件处理，不覆盖 `.ant-*` 内部样式。

## 6. 失败、并发与兼容策略

- 角色权限写入和用户角色替换在短事务内完成，先锁定目标用户/角色和幂等记录，再校验委派边界；不跨事务执行外部调用。
- 并发删除/停用/分配使用唯一约束、行锁和最后超级管理员检查；结果未知保留幂等记录并允许同键核对。
- 权限读取失败 fail-closed，不能把数据库故障当作超级管理员或空产品范围成功。
- 旧 `users.role` 只作为兼容迁移/登录响应字段；所有管理授权路径切换完成后再评估删除，不在本阶段删除历史字段。
- 产品会话仍由现有 P0 基线确定，角色绑定不接受请求体临时切换会话产品。

## 7. 验证策略

### Go/数据库

- 权限并集、产品范围、超级管理员和本地管理员保护的纯逻辑单元测试。
- Memory 与 SQLMock Repository 的 CRUD、幂等、角色停用立即失效、未知权限和提交未知测试。
- PostgreSQL 迁移重放、种子一致性、外键/唯一约束、并发分配和最后超级管理员集成测试（有 `TEST_POSTGRES_URL` 时执行）。
- 当前所有管理路由的 401/403/成功权限矩阵，直接调用 API 不能绕过前端隐藏。
- `go test ./...`、`go test -race ./...`、`go vet ./...`、`go build ./...`。

### OpenAPI/管理端

- OpenAPI 本地引用、路由门禁和管理端 `pnpm api:check`。
- 管理端 typecheck/test/build；验证导航、直接 URL、按钮权限、403、权限变化和竞态请求。
- 文档引用和差异检查。

本阶段不运行、不修改、不验证 `desktop/` 或 `desktop/ui`。

## 8. 发布顺序与退出标准

1. 先发布 `0024`、权限目录、Repository 和兼容种子，确认旧登录/管理路由仍可读。
2. 发布 `/admin/me`、角色 API 和权限中间件测试，逐路由切换 `requirePermission`。
3. 发布管理端权限壳和角色/用户角色页面。
4. 完成迁移、API、管理端和审计验证后，更新 Phase 2、商业化 PRD、P0 PRD、系统架构和管理系统架构文档。

退出条件：所有现有管理 API 都有固定权限点；普通管理员的产品范围和角色委派在服务端即时生效；超级管理员和本地管理员保护、幂等、审计和兼容迁移有证据；管理端展示与服务端权限一致；桌面端保持未修改。
