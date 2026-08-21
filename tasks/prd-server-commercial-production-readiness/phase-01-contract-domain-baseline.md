# Phase 1：契约与领域基线

Parent PRD：[PRD：服务端商业化与生产就绪](../prd-server-commercial-production-readiness.md)
状态：Not Started
最后更新：2026-08-22

## 目标

在实现任何商业能力前，锁定资源名称、状态机、数据分类、幂等、审计、错误码、迁移和发布顺序，避免后续阶段建立多个事实源。

## 主 PRD 上下文

- 目标：G-1～G-10
- 成功标准：SC-1～SC-11
- 需求：FR-1～FR-44，NFR-1～NFR-14
- 场景：全部场景的共享基础

## 阶段发现门禁

编码前重新检查：

- [ ] `产品需求文档.md`、`系统架构总览.md`、`管理系统架构.md`、`数据库迁移与密钥存储约束.md`
- [ ] `接口契约/openapi.yaml`、`接口契约/错误码.md`、`backend/internal/httpapi/contract_test.go`
- [ ] `admin-web/src/app/router.tsx`、`AppLayout.tsx`、Session/路由守卫、OpenAPI 生成类型与 Ant Design 页面模式
- [ ] `backend/migrations/catalog.go`、`backend/migrations/catalog_test.go`、已部署最新迁移版本
- [ ] 数据分类、保留、加密、审计和是否允许 API 回显的字段矩阵
- [ ] 微信支付、JSON Schema、OpenTelemetry、Sigstore/SLSA 官方文档仍适用
- [ ] 主 PRD 假设仍成立；如改变，先更新本 PRD 和后续 Phase
- [ ] P0 接入 PRD 的 products/user_products、产品会话、产品范围 RBAC 与 Profile 修订语义已稳定

## 范围

### 包含

- 建立领域词汇、状态转换表、金额/时间/ID 约定和数据分类。
- 锁定计划路由、DTO、错误码、权限、幂等 scope 和审计 action/target 命名。
- 锁定固定权限目录、角色委派规则和“API/路由/导航/操作 → 权限点”矩阵。
- 锁定商业资源的 product 外键/唯一约束/幂等 scope、席位状态机、访问判定顺序和跨产品拒绝错误码。
- 编写迁移序列、兼容发布、前滚/回滚和数据回填策略。
- 定义共享 Worker 租约/重试/死信协议和 provider adapter 最小接口。

### 不包含

- 不实现具体业务路由、供应商调用或 Worker。
- 不变更 0001～0022 历史迁移。

## 实施清单

- [ ] 为 plan/plan version/order/payment attempt/subscription/seat/reset token/delivery/artifact/config schema/error report/feedback 定义唯一词汇。
- [ ] 为 permission/role/user-role/effective-permission 定义唯一词汇，锁定多角色并集和无用户级覆盖语义。
- [ ] 将每个状态的允许转换、终态、恢复路径和管理员例外写入设计。
- [ ] 为所有新字段标注 public/internal/secret/personal/payment/audit-only 分类。
- [ ] 定义列表筛选/排序白名单、页大小和时间窗口上限。
- [ ] 定义商业写入幂等指纹和 `COMMIT_OUTCOME_UNKNOWN` 核对语义。
- [ ] 定义 Worker 任务表的领取、锁超时、重试、死信、取消和优雅关闭共享约定。
- [ ] 定义所有商业/配置/制品/报告资源对 P0 product 的引用、产品范围权限和不得跨产品的数据库约束。
- [ ] 定义“用户 → 产品成员 → 设备 → 订阅 → 席位”访问判定、席位转移冷却/管理员覆盖和授权 revision 失效语义。
- [ ] 将预计迁移按领域分组，并说明大表加列/索引的锁表影响。
- [ ] 建立 OpenAPI/错误码/Go DTO/React 生成类型的变更顺序和 CI 失败门禁；Rust 生成类型只做兼容检查。
- [ ] 同步产品、系统、管理系统、数据库和运维文档。

## 验证策略

本阶段以契约和静态验证为主：文档链接、OpenAPI 解析/本地引用、错误码映射、迁移 catalog 和数据分类审查。

## 验证清单

- [ ] `bash tools/check-document-references.sh`
- [ ] OpenAPI YAML、本地 `$ref`、Router 方法和共享错误响应检查通过
- [ ] 迁移 catalog 和从空库迁移测试通过
- [ ] 金额、时间、幂等、审计和秘密分类无两义
- [ ] 后续 10 个 Phase 与最终状态机/迁移序列、权限矩阵一致

## 退出标准

- [ ] 阶段目标完成，共享语义已定稿
- [ ] 无未解释的双事实源、密钥落点或事务边界
- [ ] 后续 Phase 已根据最终发现修订

## 阶段末多轮复核

- [ ] 1. 意图/覆盖；2. 正确性；3. 简化；4. 边界/命名；5. 重复/清理
- [ ] 6. 安全/隐私；7. 性能/容量；8. 验证充分性；9. 后续阶段；10. 主 PRD 同步

## 发现/决策

- 2026-08-21：阶段文件创建，本轮未开始实施。
- 2026-08-22：纳入管理员 RBAC、React 管理页与权限矩阵的契约基线。
- 2026-08-22：纳入 P0 产品基线、设备席位、错误报告/反馈和 11 阶段依赖。
