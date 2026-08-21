# Phase 4：固定契约与跨仓库 E2E

Parent PRD：[PRD：douyin-desktop 接入 autoLive 多产品控制面](../prd-douyin-desktop-control-plane-integration.md)
状态：Not Started
最后更新：2026-08-22

## 目标

将指定 autoLive commit 的 OpenAPI 发布为不可变、可校验制品，并用固定契约和隔离数据库完成跨仓库主路径与负向矩阵验证。

## 主 PRD 上下文

- 目标：G-5
- 成功标准：SC-5、SC-6
- 需求：FR-21～FR-25，NFR-2、NFR-5～NFR-7

## 发现门禁

- [ ] 盘点 autoLive OpenAPI 生成/校验与 douyin-desktop 类型生成入口，删除绝对路径依赖方案。
- [ ] 选择现有 CI 平台可用的不可变 artifact/Release 托管方式并确认权限、保留和 digest 语义。
- [ ] 评估成熟 OpenAPI compatibility diff 工具，记录版本、许可证、规则覆盖和误报处理。
- [ ] 明确跨仓库触发权限、依赖更新评审和失败归属，不授予消费仓库生产秘密。

## 范围

### 包含

- OpenAPI、SHA-256 和契约元数据三件套。
- 破坏性差异门禁、消费端固定 digest 类型生成和审查式升级。
- 预构建服务/镜像、隔离 PostgreSQL、可控时钟和敏感信息扫描的跨仓库 E2E。

### 不包含

- P0 不要求对象存储、签名真实性或自动更新器；这些由商业化 Phase 8/11 补齐。
- 不在服务器或 E2E 环境临时从源码构建生产制品。

## 实施清单

- [ ] CI 从唯一 OpenAPI 源生成 `openapi.yaml`、`openapi.sha256`、`contract-metadata.json`。
- [ ] metadata 记录 autoLive commit、契约版本、生成时间、生成器版本、兼容基线和 digest。
- [ ] 制品按版本/commit 不可变发布，禁止 `latest`/分支浮标覆盖同名内容。
- [ ] 兼容门禁覆盖路径/字段删除、必填变化、枚举收窄、状态码和稳定错误码变化。
- [ ] douyin-desktop 记录 commit/digest 后生成 TypeScript 类型；生成漂移或手工修改导致 CI 失败。
- [ ] 建立审查式 pin 更新流程，禁止静默自动升级服务端契约。
- [ ] E2E 启动预构建服务/镜像与隔离 PostgreSQL，固定产品种子、用户、激活码、时钟和限流配置。
- [ ] 主路径覆盖登录、激活、心跳、Profile、认证模式、摘要、释放、退出。
- [ ] 负向覆盖禁用/过期/跨产品、401 Refresh、409 幂等、429、超时、取消、重启和提交结果未知。
- [ ] 使用相同 device ID 和跨产品激活码验证设备、租约、用量、审计及后续订阅/席位矩阵。
- [ ] 扫描 CI 日志、测试报告和 artifact，确保无 Token、BYOK、短期凭证或预签名 URL。

## 验证清单

- [ ] 任意字节变更导致 digest 校验失败；错误 commit/digest 或元数据不一致不能生成类型。
- [ ] 破坏性变更未经显式版本流程不能发布，兼容新增可通过。
- [ ] 在无 `/Users/mac/...` 路径和无本地仓库邻接前提下可完整复现。
- [ ] 产品、时间和错误矩阵稳定通过，不依赖任意 sleep。
- [ ] 商业化 Phase 8 迁移对象存储后，同一 digest 继续成立且没有第二内容事实源。

## 退出标准

- [ ] douyin-desktop 可从固定 autoLive 契约可靠生成类型并完成跨仓库 E2E。
- [ ] P0 全部成功标准有证据，后续商业阶段可在同一产品事实之上实施。

## 发现/决策

- 2026-08-22：用户确认 OpenAPI 固定制品和跨仓库 E2E 纳入 P0；升级必须显式固定 commit/digest。
