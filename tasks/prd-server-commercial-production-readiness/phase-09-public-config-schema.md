# Phase 9：公共配置 Schema

Parent PRD：[PRD：服务端商业化与生产就绪](../prd-server-commercial-production-readiness.md)
状态：Not Started
最后更新：2026-08-22

## 目标

为 product/environment/namespace 公共配置增加版本化 JSON Schema Draft 2020-12 注册、校验、dry-run、原子发布和回滚，保留敏感字段拒绝作为另一层安全门禁。

## 主 PRD 上下文

- 目标：G-7、G-8
- 成功标准：SC-7、SC-9
- 需求：FR-22～FR-24、FR-34～FR-37、FR-40，NFR-1、NFR-2、NFR-4、NFR-6、NFR-12、NFR-13
- 场景：公共配置管理员发布与客户端读取

## 阶段发现门禁

- [ ] 找到并重读用户所述已有公共配置实现；当前 `dev-2.0` 未发现对应代码
- [ ] 盘点所有 namespace、生产者/消费者、现有 JSON 样本、敏感字段规则和兼容要求
- [ ] 重读 JSON Schema Draft 2020-12 官方规范和候选成熟 Go validator 当前版本/许可证/安全状态
- [ ] 确认 Schema 是编译内置、管理员注册或两者并存，以及谁可发布 Schema
- [ ] 确认客户端 ETag/revision 缓存与回滚语义
- [ ] 确认 products/user_products 基线、环境枚举、最低版本和 product kill switch 的高风险权限/审计边界

## 范围

### 包含

- product/environment/namespace/schema versions/config revisions 事实、Draft 2020-12 校验和审计。
- 受信本地 `$ref` registry、大小/复杂度上限、dry-run、原子发布/回滚。
- 客户端只读已发布 revision、ETag 与 schema version。
- React namespace/Schema/revision 列表、校验 dry-run、发布和回滚管理页。

### 不包含

- 不允许运行时任意 HTTP(S) `$ref`、代码执行关键字或未受限自定义正则引擎。
- 不将 Secret、密码、Token、内网 URL 或凭证移入“公共”配置。
- 不允许配置授予、延长或覆盖订阅/席位权益。

## 实施清单

- [ ] 根据实际基线新增/迁移 product/environment 范围的 config_namespaces、config_schema_versions 和 public_config_revisions 事实。
- [ ] 选择成熟 Draft 2020-12 Go validator，记录版本、许可证、性能、安全记录和移除路径。
- [ ] Schema 首先对 meta-schema 编译/校验，再注册为不可变版本；同 namespace/version 内容不得替换。
- [ ] 自定义 loader 只从带 allowlist 的内存/数据库 registry 加载 `$ref`，拒绝 network/file/unknown scheme。
- [ ] 限制 Schema/实例字节数、层级、参照数、错误数和校验时间，避免 CPU/内存 DoS。
- [ ] 发布顺序为 namespace 存在 → Schema 编译 → 实例校验 → 敏感字段扫描 → 幂等发布/审计。
- [ ] 新 Schema 发布前对当前已发布和保留 revision dry-run，导出有界 JSON Pointer 错误而不返回敏感值。
- [ ] 客户端 GET 返回 namespace/schema version/revision/ETag，未变更支持 304，回滚只切换指针不改写历史内容。
- [ ] 客户端 GET 从会话产品和受控 environment 确定范围，不能通过参数读取另一产品配置。
- [ ] 最低版本、紧急停用和 product kill switch 需要专用权限、二次确认、幂等、理由和审计；不能修改商业 entitlement。
- [ ] 使用 `public_config.read/manage/publish/rollback` 分离浏览、Schema/revision 管理、发布和回滚。
- [ ] React 编辑与 dry-run 显示有界 JSON Pointer 错误，使用 Ant Design 表单/表格/弹窗，不将敏感值或过大完整实例放入通知。

## 验证策略

运行官方 JSON Schema Test Suite 适用子集和项目 fixture，用 fuzz/property 测试重复 key、深层/循环 ref 与过大输入，API/PostgreSQL 测试覆盖幂等发布/回滚。

## 验证清单

- [ ] 每个已知 namespace 的正确/缺字段/多字段/范围/格式/联合类型 fixture
- [ ] 未知 namespace、错误 draft、无效 Schema、循环/远程 `$ref`、过大输入和灾难正则 fail-closed
- [ ] 敏感字段在 Schema 允许时仍被第二层规则拒绝
- [ ] dry-run 不修改指针，发布和回滚幂等，ETag/304 一致
- [ ] 相同 namespace 在不同 product/environment 独立发布，跨产品读取/回滚/kill switch 被拒绝
- [ ] 校验错误不泄露配置值/内部 Schema 路径
- [ ] React typecheck/test/build 和读取/管理/发布/回滚/403/dry-run 失败的浏览器冒烟通过

## 退出标准

- [ ] 所有公共配置在发布前通过确定性 namespace/schema version 校验与敏感字段门禁
- [ ] Schema 变更、dry-run、发布、回滚和客户端缓存语义可审计/可恢复

## 阶段末多轮复核

- [ ] 1. 意图/覆盖；2. 正确性；3. 简化；4. 边界/命名；5. 重复/清理
- [ ] 6. 安全/隐私；7. 性能/容量；8. 验证充分性；9. 后续阶段；10. 主 PRD 同步

## 发现/决策

- 2026-08-21：规划 JSON Schema Draft 2020-12、版本化 namespace 与无远程 `$ref` 校验，本轮未实施。
- 2026-08-22：纳入 React 公共配置管理页与读/管理/发布/回滚权限。
- 2026-08-22：配置作用域扩展为 product/environment/namespace，并纳入最低版本、紧急停用和不可授予权益边界。
