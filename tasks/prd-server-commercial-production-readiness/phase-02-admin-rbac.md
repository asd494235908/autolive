# Phase 2：管理员 RBAC 与权限化管理端

Parent PRD：[PRD：服务端商业化与生产就绪](../prd-server-commercial-production-readiness.md)
状态：Not Started
最后更新：2026-08-22

## 目标

用“自定义角色 + 固定权限点”替换当前 `admin/user` 粗粒度判断，并让 React 管理端根据服务端返回的有效权限展示菜单、路由和操作。

## 主 PRD 上下文

- 目标：G-8
- 成功标准：SC-9
- 需求：FR-29～FR-35，NFR-1、NFR-2、NFR-10～NFR-12
- 场景：场景 1

## 阶段发现门禁

- [ ] 重读 `controlplane.Actor`、`requireAdmin`、Session Store、管理员初始化和最后管理员保护
- [ ] 重读 `admin-web/src/app/router.tsx`、`AppLayout.tsx`、Session 状态和 OpenAPI 生成类型
- [ ] 盘点全部当前及后续 `/api/v1/admin/*` 路由，形成“路由/动作 → 权限点”唯一矩阵
- [ ] 核对现有 `role=admin/user` 数据量、非本地管理员账号和兼容迁移策略
- [ ] 确认角色委派规则、内置 `super_admin` 保护和高风险操作二次认证仍符合主 PRD

## 范围

### 包含

- 固定权限目录、内置 `super_admin`、自定义角色、角色权限和用户多角色关系。
- 当前管理员有效角色/权限摘要、角色 CRUD、用户角色分配和服务端权限中间件。
- React“角色与权限”页面、用户角色分配、权限化导航/路由/按钮和 403 页面。
- 权限变更审计、立即生效、兼容回填和防提权约束。

### 不包含

- 不允许管理员创建任意权限代码，不支持用户级 allow/deny 覆盖。
- 不把前端隐藏菜单当作授权，不把完整权限长期写入 Access Token。
- 不允许自定义角色获得或分配内置 `super_admin` 身份。

## 固定权限目录

- 基础：`dashboard.read`、`users.read`、`users.manage`、`roles.read`、`roles.manage`、`roles.assign`、`admin_security.manage`。
- 设备/激活：`devices.read`、`devices.manage`、`activation_codes.read`、`activation_codes.manage`、`activation_codes.reveal`、`activation_codes.switch_device`。
- 商业：`plans.read`、`plans.manage`、`plans.publish`、`orders.read`、`orders.reconcile`、`subscriptions.read`、`subscriptions.adjust`、`payments.read`、`payments.reconcile`。
- 投递/制品/配置：`password_resets.read`、`password_resets.retry`、`artifacts.read`、`artifacts.manage`、`artifacts.publish`、`artifacts.revoke`、`public_config.read`、`public_config.manage`、`public_config.publish`、`public_config.rollback`。
- 现有控制面：`model_pool.read`、`model_pool.manage`、`model_pool.test`、`model_pool.rotate_secret`、`model_leases.read`、`model_leases.reclaim`、`model_usage.read`、`audit_logs.read`。
- 运维：`operations.read`、`operations.manage`。

权限目录由代码/OpenAPI 和迁移种子共同版本化；管理员只能把已注册权限分配给角色。

## 实施清单

- [ ] 新增 `admin_permissions`、`admin_roles`、`admin_role_permissions`、`user_admin_roles` 前向迁移、唯一约束、外键和索引。
- [ ] 创建不可删除/不可改名的内置 `super_admin`，固定拥有全部权限；`usr_local_admin` 永久绑定该角色。
- [ ] 将现有 `role=admin` 用户兼容回填为 `super_admin`，`role=user` 不获得管理角色；保留旧字段直至所有授权路径切换并完成回滚窗口。
- [ ] 建立 PermissionCatalog/RoleRepository/PermissionReader，启动时校验代码目录和数据库种子无缺失/未知项。
- [ ] 用 `requirePermission(permissionCode)` 替换后续管理路由的粗粒度 `requireAdmin`；每次受保护请求按用户 ID 读取当前有效权限，撤销后立即生效。
- [ ] 禁止普通角色管理者授予自身不拥有的权限；分配角色时，目标角色的权限也必须是操作者当前权限的子集。只有 `super_admin` 可分配/撤销 `super_admin`，且不得移除最后一个有效 `super_admin` 或本地管理员绑定。
- [ ] 提供仅需有效认证会话且只返回本人角色/权限的 `GET /api/v1/admin/me`、权限目录读取、角色列表/详情/创建/编辑/删除和用户角色读取/替换 API；用户角色替换要求 `roles.assign`，所有写操作要求幂等键。
- [ ] 角色删除前要求无用户绑定；角色停用后相关权限立即失效，活动 Session 不需要等待重新登录。
- [ ] 权限目录、角色和用户角色变更写入语义化审计 Outbox，不记录密码、Token 或二次认证正文。
- [ ] React Session 启动后读取 `/admin/me`；根据权限过滤导航、守卫路由和操作按钮，直接 URL 无权限显示 403，服务端 403 仍是最终结果。
- [ ] 使用 Ant Design `Table`、`Form`、`Checkbox.Group`/`Tree`、`Modal` 和 `Result` 实现角色列表、角色编辑、权限分组和无权限状态，不覆盖 `.ant-*` 内部样式。
- [ ] 用户管理页增加多角色分配；不能在 UI 中移除本地管理员的 `super_admin`，服务端同时拒绝绕过请求。
- [ ] 同步 OpenAPI、错误码、Go DTO、React 生成类型、管理系统架构和权限矩阵文档。

## 验证策略

用权限纯逻辑单元测试、PostgreSQL 迁移/并发集成测试、HTTP 路由权限矩阵和 React 组件/浏览器流程共同证明后端授权与前端展示一致。

## 验证清单

- [ ] 未登录 401、已登录无权限 403、权限存在成功；菜单隐藏不能绕过 API
- [ ] 多角色权限并集、角色停用/撤销立即生效、Session 不含陈旧授权
- [ ] 现有管理员迁移后权限不丢失，普通用户不意外获得管理权限
- [ ] 最后 `super_admin`、本地管理员、越权授予、未知权限代码和已绑定角色删除均被拒绝
- [ ] 角色/权限/用户角色变更幂等、并发安全、提交未知可核对且审计完整
- [ ] React 覆盖 loading、empty、error、403、角色冲突和提交重试；键盘焦点与表单标签可访问
- [ ] Go 全量、Race、PostgreSQL integration、OpenAPI、React typecheck/test/build 和浏览器管理流程通过

## 退出标准

- [ ] 所有管理 API 都有明确固定权限点，后端不再以单一 `role=admin` 作为最终授权
- [ ] React 管理端只展示当前有效权限允许的页面/操作，并能正确处理权限实时变化
- [ ] 权限变更可审计、不可越权，升级兼容和回滚边界有证据

## 阶段末多轮复核

- [ ] 1. 意图/覆盖；2. 正确性；3. 简化；4. 边界/命名；5. 重复/清理
- [ ] 6. 安全/隐私；7. 性能/容量；8. 验证充分性；9. 后续阶段；10. 主 PRD 同步

## 发现/决策

- 2026-08-22：用户确认采用“自定义角色 + 固定权限点”，角色可多选，权限取并集，不支持用户级覆盖。
