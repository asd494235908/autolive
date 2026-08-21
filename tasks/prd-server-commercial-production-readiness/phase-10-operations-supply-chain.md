# Phase 10：生产运维与供应链

Parent PRD：[PRD：服务端商业化与生产就绪](../prd-server-commercial-production-readiness.md)
状态：Not Started
最后更新：2026-08-22

## 目标

用可重复的保留清理、备份恢复、指标/追踪、容量告警、CI 签名/Provenance 和部署前验证证明服务端可生产运行。

## 主 PRD 上下文

- 目标：G-6、G-8
- 成功标准：SC-6、SC-8、SC-9
- 需求：FR-25～FR-28、FR-34、FR-35，NFR-2、NFR-4～NFR-12
- 场景：所有场景的生产保障

## 阶段发现门禁

- [ ] 重读现有 retention scheduler/metrics、`/livez`/`/readyz`、优雅关闭、CI 和发布脚本
- [ ] 盘点所有新表/对象的分类、TTL、归档、法务冻结和容量增长率
- [ ] 确认 PostgreSQL/对象存储备份产品、加密/KMS、失效域、保留和隔离恢复环境
- [ ] 确认 OTLP collector/backend、采样、传输、保留和敏感属性策略
- [ ] 确认 CI OIDC、签名/attestation 平台限制和部署验证信任策略

## 范围

### 包含

- 全数据集保留/清理/归档、容量告警和数据字典。
- PostgreSQL + 对象存储加密备份、隔离恢复、RPO/RTO 证据和 Runbook。
- Prometheus 业务/运维指标和 OpenTelemetry OTLP traces。
- CI 制品签名、SBOM/Provenance、签名验证、发布清单、迁移/回滚与服务器无构建门禁。
- React 运维视图：Worker 积压/死信、备份新鲜度/恢复演练摘要、存储容量、发布验证与告警链接。

### 不包含

- 不为了“可观测”重复发布两套业务指标或记录高基数/敏感标签。
- 不在生产数据库上做未演练的破坏性恢复或以备份文件存在代替恢复证据。

## 实施清单

- [ ] 为每个数据集建立 owner、分类、TTL、清理键、索引、批次、冻结/归档和容量告警矩阵。
- [ ] 扩展 Retention Scheduler 而非建立无主时器，使用有界批次、`SKIP LOCKED`、超时、取消、数据集隔离和优雅关闭。
- [ ] 建立 PostgreSQL 定时加密备份、完整性检查、异地/跨失效域保留和过期删除。
- [ ] 建立对象存储版本/对象锁或等价保留、清单备份和孤儿/丢失对象核对。
- [ ] 至少每季度恢复到隔离环境，执行迁移、数量/引用/密文可读性核对，记录实际 RPO/RTO。
- [ ] 扩展 Prometheus：订单/支付结果、Worker lag/retry/dead letter、对象存储、备份/恢复、发布验证，仅用固定低基数标签。
- [ ] 接入 OpenTelemetry Go tracing 与 OTLP exporter，覆盖 HTTP、DB、Worker、支付/邮件/对象存储出站调用和 trace context 传播。
- [ ] 建立告警/Runbook：支付对账积压、死信、连接池、保留失败、备份过期、恢复超时、制品验签失败。
- [ ] CI 使用托管 OIDC 生成签名/SBOM/SLSA Provenance，所有 Action/基础镜像/工具版本可复现并受门禁。
- [ ] 部署前验证签名身份、issuer、workflow/ref、subject digest、SBOM/Provenance、迁移版本与回滚清单；服务器脚本不含构建命令。
- [ ] 进行一次签名制品发布、迁移、健康验收、回滚/前滚和服务器重启演练。
- [ ] 将只读运维视图绑定 `operations.read`，将重试/解除冻结/发布等操作绑定 `operations.manage` 或对应领域权限，不在前端暴露密钥或预签名 URL。
- [ ] React 运维页只显示低基数摘要和有界运维记录，通过明确开放链接跳转外部指标/trace 系统，不在管理端重造监控平台。

## 验证策略

在自动测试之外，本阶段必须有隔离恢复、告警演练、trace 串联、签名策略负测试和服务器只加载制品的环境证据。

## 验证清单

- [ ] 保留 Worker 大数据批次、锁竞争、取消、部分数据集失败和 SIGTERM
- [ ] 备份缺失/损坏/密钥不可用、隔离恢复、数据/对象核对与实际 RPO/RTO 记录
- [ ] metrics 无高基数/秘密，trace 可关联 HTTP→DB→Worker→provider 且 exporter 故障不拖垮业务
- [ ] 错误签名、issuer、workflow/ref、digest、Provenance 和 SBOM 任一失败均阻止发布
- [ ] `go test ./...`、Race、Vet、Build、govulncheck、PostgreSQL integration、OpenAPI、React typecheck/test/build/浏览器冒烟、Rust 契约兼容检查与镜像冒烟全通过
- [ ] 服务器无源码、无 Go/pnpm/Rust/Docker 构建，只验证/加载/迁移/启动/冒烟

## 退出标准

- [ ] 实际恢复演练证明 RPO≤24h、RTO≤4h，关键告警和 Runbook 可操作
- [ ] 制品从 CI 到对象存储和服务器下载授权的签名/Provenance 链可验证，并输出后续消费端所需验证元数据
- [ ] 全部主 PRD 成功标准有证据或经用户明确延期

## 阶段末多轮复核

- [ ] 1. 意图/覆盖；2. 正确性；3. 简化；4. 边界/命名；5. 重复/清理
- [ ] 6. 安全/隐私；7. 性能/容量；8. 验证充分性；9. 后续阶段；10. 主 PRD 同步

## 发现/决策

- 2026-08-21：规划保留 Prometheus 作为指标事实源、使用 OpenTelemetry OTLP 增加 tracing，并以签名/Provenance 验证作为发布门禁；本轮未实施。
- 2026-08-22：纳入受权限保护的 React 运维摘要页，明确不在管理端重造指标/trace 平台。
