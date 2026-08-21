# 服务端商业化与生产就绪：发现上下文

Parent PRD：[PRD：服务端商业化与生产就绪](../prd-server-commercial-production-readiness.md)
最后更新：2026-08-21

## 已检查输入

- 需求/架构：`AGENTS.md`、`产品需求文档.md`、`系统架构总览.md`、`管理系统架构.md`、`长任务开发总计划.md`、`服务端未完成功能长任务开发计划.md`、`数据库迁移与密钥存储约束.md`。
- 契约/路由：`接口契约/openapi.yaml`、`backend/internal/httpapi/controlplane.go`、`backend/internal/httpapi/auth.go`、`backend/internal/httpapi/audit.go`、`backend/internal/httpapi/rate_limit.go`。
- 存储/迁移：`backend/migrations/0001～0022`、`backend/internal/store/repository.go`、`postgres_*_repository.go`、`sql_secret_store.go`、`retention_postgres.go`。
- 运维/发布：`.github/workflows/backend-quality.yml`、`.github/workflows/desktop-package.yml`、`tools/check-backend-release.mjs`、`deploy/runtime-resources/*`。
- 激活码专项：`docs/superpowers/specs/2026-08-21-激活码多设备绑定设计.md`、迁移 0022 及相关 service/store/httpapi 测试。
- 官方资料：微信支付 API v3 回调/证书/官方 Go SDK、JSON Schema Draft 2020-12、OpenTelemetry Go、SLSA Provenance、Sigstore Cosign 和 GitHub Artifact Attestations。

## 当前系统摘要

- Go Router 已提供登录/Refresh/Logout、管理员换密码、用户、设备、激活码、模型账号、租约、用量和审计路由。
- 未发现 subscriptions、orders、plans/plan versions、payment attempts 或 WeChat Pay 路由/领域/迁移。
- 用户管理员可直接设置新密码并撤销会话，但未发现公开自助密码重置 Token、邮件投递 Outbox 或 Worker。
- 设备已有列表/详情、心跳、禁用和解绑；需扩展固定白名单筛选、排序和索引。
- 激活码只保存 SHA-256 哈希/前缀，列表脱敏；迁移 0022 只增加 `max_devices`/`bound_devices`，仍只保存首次核销主体和设备。
- PostgreSQL SecretStore 已有 AES-256-GCM 密文和事务写入模式，但固定用于 `model_account_secrets`，当前格式缺少 AAD/key id/version。
- Retention Scheduler 已处理会话、幂等、模型测试、审计、孤立设备绑定和 staged model secret；尚缺完整业务数据保留表、容量告警和归档。
- `/metrics` 已使用 Prometheus Go client 提供 HTTP、连接池、限流、审计、模型号池/租约和清理指标；未接入 OpenTelemetry traces。
- Backend CI 已生成 Docker/OCI 制品、SBOM、BuildKit Provenance 和 SHA-256 元数据；未生成/验证可信签名和下游发布策略。
- Desktop package CI 从外部 HTTPS 下载 FFmpeg 并校验固定 SHA-256，生成运行资源清单；通过 SSH/rsync 上传静态服务器，非对象存储或权限化外部下载。
- 未在 `dev-2.0` 分支找到公共配置的服务端实现；将用户提供的“已校验 JSON 与敏感字段”视为实施前必须复核的外部基线。

## 可复用模式

- OpenAPI 路由/错误码/DTO/生成类型门禁。
- normalized Repository + PostgreSQL 短事务 + 行锁/唯一约束 + `COMMIT_OUTCOME_UNKNOWN`。
- `Idempotency-Key` + 指纹冲突语义。
- `audit_outbox` 和失败关闭审计。
- `FOR UPDATE SKIP LOCKED` 有界清理、Context 取消和优雅关闭 Scheduler。
- SecretStore 的 AES-GCM 原语与事务写入方式，但激活码使用独立表/存储边界。
- Prometheus 独立 registry 和固定低基数标签。

## 验证面

- Go：`gofmt -l .`、`go vet ./...`、`go test ./...`、`go test -race ./...`、`go build ./...`、`govulncheck`。
- PostgreSQL：从空库执行全部迁移，`postgres_integration` 覆盖事务、并发、锁等待、断连、提交结果未知和查询计划。
- 契约：OpenAPI YAML/本地 `$ref`、Router 路径/方法、Go 错误码/DTO；React/Rust 只执行现有生成契约兼容检查，不在本专项修改消费端业务代码。
- 微信支付：官方 SDK fixture/回调样本、重入和金额不一致测试；上线门禁需要小额真实支付/查询验收。
- 制品：服务端签名验证、错误 signer/issuer/ref/digest 拒绝、对象存储预签名 URL 时效/权限，以及向消费端返回签名清单和哈希元数据。
- 运维：隔离恢复演练、故障注入、告警测试、trace 串联、SIGTERM 和 Worker 租约恢复。

## 官方外部依据

- [微信支付 API v3 官方 Go SDK](https://github.com/wechatpay-apiv3/wechatpay-go)
- [微信支付回调验签/应答/重入规则](https://pay.wechatpay.cn/doc/v3/merchant/4012647435)
- [微信支付平台证书与公钥边界](https://pay.wechatpay.cn/doc/v3/merchant/4012069411)
- [JSON Schema Draft 2020-12](https://json-schema.org/draft/2020-12)
- [OpenTelemetry Go](https://opentelemetry.io/docs/languages/go/)
- [GitHub Artifact Attestations](https://docs.github.com/en/actions/concepts/security/artifact-attestations)
- [Sigstore Cosign Verify](https://docs.sigstore.dev/cosign/verifying/verify/)
- [SLSA Build Provenance](https://github.com/slsa-framework/slsa/blob/main/spec/build-provenance.md)

## 尚未验证的外部条件

- 未获取微信商户号/密钥，未执行真实支付。
- 未选定邮件和对象存储供应商，未验证实际限额/区域/合规条件。
- 未设置生产 OTLP backend、对象存储权限或 CI 签名信任策略。
- 未执行备份恢复演练，RPO/RTO 只是规划验收目标，不是已完成事实。
