# Phase 2：激活码安全管理

Parent PRD：[PRD：服务端商业化与生产就绪](../prd-server-commercial-production-readiness.md)
状态：Not Started
最后更新：2026-08-21

## 目标

为新激活码增加独立加密明文存储与安全重显，建立逐设备绑定事实，并支持容量修改和已知绑定设备切换。

## 主 PRD 上下文

- 目标：G-4
- 成功标准：SC-4
- 需求：FR-14～FR-17，NFR-1、NFR-2、NFR-6、NFR-8
- 场景：场景 4

## 阶段发现门禁

- [ ] 重读激活码多设备设计/计划、迁移 0011/0022 和所有核销/解绑/到期 reader
- [ ] 重读 `sql_secret_store.go`、密钥配置、审计失败关闭和限流规则
- [ ] 核对生产 HTTPS 、管理员密码二次验证方式和密钥轮换运维责任
- [ ] 查询生产库中 `bound_devices > 1` 的历史数量，不读取/导出明文秘密
- [ ] 如历史语义与假设不同，先修订 PRD 与回填方案

## 范围

### 包含

- `activation_code_secrets` 独立表、AAD/key id/version 和多密钥读取。
- `activation_code_device_bindings` 的 slot 与历史未知状态。
- 详情、reveal、容量 PATCH 和 binding switch API。
- 核销、解绑、到期、会话和租约路径的绑定事实同步。

### 不包含

- 不恢复旧激活码明文，不伪造无法还原的历史设备。
- 不允许通过切换设备转移到不同用户或继承旧会话。

## 实施清单

- [ ] 先写迁移和 PostgreSQL 集成失败测试：密文事务、历史状态、slot 唯一约束和回填。
- [ ] 提取 AAD-capable AES-GCM codec，为激活码创建专用 SecretStore，不写入 `model_account_secrets`。
- [ ] 实现 active key id + key ring 配置、启动校验、读旧写新和密钥移除门禁。
- [ ] 创建激活码时同事务写哈希、元数据、密文、幂等和审计 Outbox。
- [ ] 实现 reveal POST：管理员二次验证、专用限流、`no-store/private`、审计失败不返回明文。
- [ ] 实现 binding detail/backfill：首台设备可证明回填，其余 slot 记为 `legacy_unknown`。
- [ ] 实现容量状态机：1～100、不低于已核销、过期/作废拒绝、上调后重新可核销。
- [ ] 实现切换事务：稳定锁顺序、目标待激活/无主且无活动绑定、旧会话/租约撤销、复用 slot 和审计 Outbox。
- [ ] 修正解绑和到期 reader，使绑定明细成为设备到期/归属事实，不再只依赖首台设备字段。
- [ ] 更新 OpenAPI、错误码和服务端架构/密钥文档；不在本专项实现管理页面。

## 验证策略

密码学单元测试 + 服务/路由安全测试 + PostgreSQL 事务/并发/故障集成测试 + HTTPS 管理流程冒烟。

## 验证清单

- [ ] 加密随机 nonce、AAD 篡改、错误 key id、旧 key 读取、密文行替换和敏感日志扫描
- [ ] reveal 的 401/403/404/409/429/503、审计故障、缓存头和明文不进入列表
- [ ] 容量修改与并发核销，切换与解绑/禁用竞态，幂等重试与提交结果未知
- [ ] 历史回填数量守恒，`legacy_unavailable`/`legacy_unknown` 清晰返回
- [ ] Go 全量、Race、PostgreSQL integration、OpenAPI 和 API 管理流程验证通过

## 退出标准

- [ ] 新码可安全重显，旧码无伪恢复，所有明文路径受二次认证/限流/审计/HTTPS 保护
- [ ] 容量、slot、绑定、设备、会话和租约在并发与故障后一致

## 阶段末多轮复核

- [ ] 1. 意图/覆盖；2. 正确性；3. 简化；4. 边界/命名；5. 重复/清理
- [ ] 6. 安全/隐私；7. 性能/容量；8. 验证充分性；9. 后续阶段；10. 主 PRD 同步

## 发现/决策

- 2026-08-21：选定独立激活码加密表，本轮未开始实施。
