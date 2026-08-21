# Phase 7：制品存储与下载

Parent PRD：[PRD：服务端商业化与生产就绪](../prd-server-commercial-production-readiness.md)
状态：Not Started
最后更新：2026-08-21

## 目标

将当前短期 CI artifact 和 SSH 静态资源发布扩展为服务端私有对象存储、签名/Provenance 真实性校验、权限化短时下载和可供消费端验证的签名元数据链。

## 主 PRD 上下文

- 目标：G-5
- 成功标准：SC-5、SC-8
- 需求：FR-18～FR-21、FR-28，NFR-1、NFR-2、NFR-4、NFR-8
- 场景：场景 5

## 阶段发现门禁

- [ ] 选定对象存储厂商/区域/CDN 和 S3-compatible 能力，核对请求签名、分片、版本、对象锁和预签名 URL 限制
- [ ] 重读 `.github/workflows/desktop-package.yml`、`backend-quality.yml`、`deploy/runtime-resources/*` 和当前制品清单格式，仅识别服务端兼容边界
- [ ] 确认托管 CI 平台、OIDC issuer、repository/workflow/ref 和签名信任根
- [ ] 重读 Sigstore/GitHub Attestation/SLSA 官方验证方式和私有仓库限制
- [ ] 确认哪些制品需要订阅权限，哪些是公共运行资源

## 范围

### 包含

- artifact releases/files、签名清单、SBOM/Provenance 引用、发布状态。
- CI 直传对象存储或受控上传会话，服务端只处理元数据/授权，不中继大文件。
- 签名/Provenance 策略验证、发布、撤销、短时下载，以及响应中的签名清单/digest/信任策略版本。

### 不包含

- 不存储用户本地音视频，不将 Go API 作为文件代理/CDN。
- 不将 HTTPS 或 SHA-256 单独视为发布者真实性证明。
- 不修改 Rust/Tauri 下载器、安装器或本地验签逻辑。

## 实施清单

- [ ] 新增 artifact release/file 迁移，约束 version/channel/platform/arch/object key/digest 唯一性和发布状态。
- [ ] 建立 ObjectStore 最小适配器：元数据 HEAD、受控写入会话、短时 GET URL、删除/保留不由业务层拼接 URL。
- [ ] CI 生成签名 manifest、SBOM 和 SLSA Provenance，签名秘密优先使用 OIDC/keyless 或 KMS，不使用仓库长期私钥。
- [ ] 发布服务验证 signer identity、issuer、repository、workflow/ref、subject digest、SBOM/Provenance 关联，失败即拒绝。
- [ ] 对象 HEAD 的大小/摘要与签名 manifest 一致后才从 draft 变为 published。
- [ ] 下载入口校验订阅、用户、设备、渠道、平台和发布状态，签发短时、单对象、只读 URL。
- [ ] 预签名 URL 不进日志/trace/审计，只记录脱敏制品和结果聚合。
- [ ] 下载 API 返回已验证 manifest、版本/平台/架构、整文件 SHA-256 和信任策略版本，供后续客户端集成使用。
- [ ] 保留现有静态资源链作为过渡回退，通过受控发布开关切换，不双向写两个事实源。

## 验证策略

使用本地 S3-compatible fixture/专用测试桶、Cosign/GitHub attestation 验证样本和 API 权限/篡改样本证明服务端链路。

## 验证清单

- [ ] 错误 signer/issuer/repository/workflow/ref/digest/provenance 任一不匹配无法发布
- [ ] 未发布/撤销/无订阅/设备不符/过期 URL 无法下载
- [ ] URL 只允许 GET/固定 key/短 TTL，不包含存储长期凭证
- [ ] 文件篡改、截断、平台不符和 manifest 替换时服务端拒绝发布或签发下载地址
- [ ] 对象存储故障、签名验证超时、并发发布和幂等重试有稳定结果

## 退出标准

- [ ] 服务端只为具有授权且可证明来自受信 CI 的制品签发下载地址
- [ ] 服务器不构建源码、不中继大文件、不持有客户端长期存储凭证

## 阶段末多轮复核

- [ ] 1. 意图/覆盖；2. 正确性；3. 简化；4. 边界/命名；5. 重复/清理
- [ ] 6. 安全/隐私；7. 性能/容量；8. 验证充分性；9. 后续阶段；10. 主 PRD 同步

## 发现/决策

- 2026-08-21：规划私有对象存储 + 短时预签名 URL + 签名/Provenance 验证，本轮未实施。
