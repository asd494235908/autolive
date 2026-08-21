# douyin-desktop 接入 autoLive 多产品控制面：发现上下文

Parent PRD：[PRD：douyin-desktop 接入 autoLive 多产品控制面](../prd-douyin-desktop-control-plane-integration.md)
最后更新：2026-08-22

## 已检查输入

- autoLive：`AGENTS.md`、产品/系统/管理/桌面/数据库架构文档、`接口契约/openapi.yaml`、Go Router/领域类型、React 管理路由与商业化 PRD。
- douyin-desktop：仓库约束、架构文档和现存 `contracts/openapi.yaml`；该仓库明确 autoLive 后端/OpenAPI 是唯一服务端来源。
- 当前接口：登录/Refresh/Logout、激活、心跳、Profile、模型租约/摘要，以及用户、设备、激活码、模型池、用量和审计管理路由。

## 当前事实

- autoLive 当前 `DeviceRegistration`、`DeviceSummary`、`ActivationCode`、`ClientProfileResponse`、`ModelLease`、用量和 `AuditLog` 未形成统一产品维度。
- 管理授权当前主要依赖 `admin/user` 与 `requireAdmin`；商业化 PRD 已规划固定权限点和自定义角色，但尚未实施产品范围角色绑定。
- `ModelLease` 当前仅返回 `direct_base_url`；Secret Store 中存在服务端长期 Key 不等于客户端获得了安全认证能力。
- `/client/profile` 当前没有服务端时间、授权复核点、离线硬截止和权益修订号。
- 当前 Router 未实现订阅、订单、套餐、支付、公共配置、制品、错误报告或反馈；这些由商业化 PRD 规划。
- douyin-desktop 旧契约虽出现 product/订阅/反馈等字段，但它是迁移参考，不得反向成为 autoLive 的服务端事实源。

## 可复用基础

- OpenAPI 路由/DTO/错误码/生成类型门禁。
- normalized PostgreSQL、会话绑定、幂等、短事务、审计 Outbox 和固定低基数 Prometheus 标签。
- Secret Store 的加密原语和 key id 规划；短期凭证仍需独立的一次性交付边界。
- React/Vite/TypeScript/Ant Design 管理壳和商业化 PRD 的固定权限目录方案。
- CI 预构建制品、SBOM/Provenance 与 checksum 生成基础。

## 验证面

- 契约：OpenAPI 解析、本地 `$ref`、Router/错误码映射、兼容差异和生成类型无漂移。
- Go/PostgreSQL：迁移、产品范围唯一约束/外键/幂等、并发、跨产品拒绝、提交结果未知和 Race。
- React：产品范围导航/路由/表格筛选、直接 URL/手工 API 403、loading/empty/error/权限变化。
- 时间：可控服务端时间验证 6 小时复核、24 小时硬截止、回拨、恢复网络和密钥轮换。
- 模型：BYOK 不上传；委托模式验证范围、过期、撤销、释放、禁用和敏感信息扫描。
- 跨仓库：预构建服务/镜像、隔离 PostgreSQL、固定 OpenAPI digest 和完整正负 E2E 矩阵。

## 已确认决策

- 使用单数据库硬逻辑隔离，不采用“只加筛选参数”。
- 用户身份全局复用，产品准入使用 `user_products`。
- 普通管理员按产品授权，`super_admin` 才能跨产品。
- Profile 默认建议 6 小时复核，签名离线授权硬上限 24 小时。
- douyin-desktop 首版本地 BYOK；委托短期凭证必须由供应商能力证明后启用。
- 使用分层双 PRD：本 PRD 管 P0，商业化 PRD 管 P1/P2。

## 尚未验证的外部条件

- 尚未选定能签发可撤销、受限、绑定租约短期凭证的模型供应商。
- 尚未确定 CI 不可变契约制品的最终托管位置；P0 可用受保护 CI/Release artifact，Phase 8 再迁移对象存储。
- 本轮未运行代码、迁移、CI、跨仓库 E2E 或服务器部署验证。
