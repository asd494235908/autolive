# PRD：douyin-desktop 接入 autoLive 多产品控制面

## 文档状态

- 状态：Draft
- 文件模式：Split
- 当前阶段：Not Started
- 活动阶段文件：无（本轮只更新规划）
- 上下文：[context.md](./prd-douyin-desktop-control-plane-integration/context.md)
- 设计：[多产品控制面与 douyin-desktop 接入设计](../docs/superpowers/specs/2026-08-22-douyin-desktop-接入-autolive-多产品控制面设计.md)
- 关联规划：[服务端商业化与生产就绪](./prd-server-commercial-production-readiness.md)
- 最后更新：2026-08-22
- 目的：作为 autoLive Go/React 控制面承载 `autolive` 与 `douyin_desktop` 的 P0 接入事实源。

## 分层归属

- 本 PRD 负责 P0：产品硬隔离、产品成员关系、产品范围 RBAC、Profile 授权新鲜度、模型认证闭环、固定 OpenAPI 制品和跨仓库 E2E。
- 商业化 PRD 负责 P1/P2：套餐版本、订单、支付、订阅、设备席位、密码重置、公共配置、桌面制品、错误反馈、备份恢复和供应链门禁。
- 权限事实不重复：商业化 Phase 2 拥有固定权限目录、自定义角色生命周期和通用 React 权限壳；本 PRD 拥有 product 范围语义、产品角色绑定约束和跨产品拒绝。两者在 P0 Phase 1 协同交付同一组角色表/API。
- 两份 PRD 共用服务端 `product` 基线；商业化 PRD 不另建产品模型，本 PRD 不重复定义商业状态机。
- 实现写入范围在 autoLive 仓库的 Go 服务端、PostgreSQL、OpenAPI、CI 和 React 管理面；douyin-desktop 只作为契约消费端和跨仓库 E2E 验证面，不在本专项复制服务端。

## 问题

autoLive 已具备登录、Refresh、Logout、激活、设备、心跳、Profile、模型池、模型租约、调用摘要和审计，可作为多个桌面产品的唯一云端控制面。但当前设备、激活码、Profile、租约、用量、审计和管理权限均没有稳定产品边界；仅在管理查询中增加一个筛选参数不能阻止跨产品读取或写入。

当前 `/client/profile` 没有可验证的授权新鲜度预算，客户端断网后只能自行猜测；`ModelLease` 只返回 `direct_base_url`，没有客户端可安全使用的短期凭证。当前 OpenAPI 也没有固定 commit/digest 制品，douyin-desktop 生成类型时不能依赖开发机绝对路径。

## 目标

- G-1：以单库硬逻辑隔离承载 `autolive` 与 `douyin_desktop`，任何资源访问都不能只依赖前端筛选。
- G-2：保留全局用户身份，通过产品成员关系和产品范围角色决定每个产品的准入与管理权限。
- G-3：由服务端签发可验证的授权新鲜度，让客户端在离线时不自行延长权益。
- G-4：模型调用只使用本地 BYOK 或真实可撤销短期凭证，不向桌面端返回长期共享 API Key。
- G-5：按 autoLive commit/digest 发布不可变 OpenAPI 制品，并建立可复现的跨仓库契约与 E2E 门禁。

## 非目标

- NG-1：不复制 douyin-desktop 已删除的 Go 服务端、数据库、管理端或旧 OpenAPI。
- NG-2：不引入通用 ABAC/策略引擎、数据库级多租户框架、微服务、Kubernetes 或外部消息队列。
- NG-3：不上传 Token、Cookie、完整 Prompt、模型输入输出正文、视频、音频或本地 SQLite 数据。
- NG-4：不恢复实时话术幻化、媒体任务、服务端模型正文代理或请求预占。
- NG-5：本轮不修改 OpenAPI、代码、迁移、CI 或服务器，只确认并写入规划。

## 成功标准

- SC-1：同一个 `device_id` 可分别存在于两个产品，但激活码、会话、资源引用和管理员权限不能跨产品混用。
- SC-2：普通管理员即使省略 `product` 查询参数，也只能看到其授权产品；只有内建 `super_admin` 可跨产品管理。
- SC-3：Profile 返回服务端时间、授权状态、建议复核时间和 24 小时离线硬截止；回拨本机时钟不能延长授权。
- SC-4：douyin-desktop 首版使用本地 BYOK；只有供应商支持可撤销、受限、短时凭证时才开放托管直连。
- SC-5：OpenAPI 制品包含 commit、版本、生成器和 SHA-256 元数据，消费端按 digest 生成类型且不依赖 `/Users/mac/...`。
- SC-6：跨仓库 E2E 覆盖主路径、跨产品拒绝、离线时间边界、401/409/429/超时/取消，并扫描敏感信息泄露。

## 关键决策

### 产品与身份

- `products` 注册表使用不可变代码；首批种子为 `autolive`、`douyin_desktop`。
- `users` 保持全局身份，`user_products` 是每产品准入事实；禁用某产品成员关系不影响用户在其他产品的身份。
- 客户端登录或激活时绑定单一产品会话，后续请求不能通过 body/query/header 临时切换产品。
- 设备、激活码、绑定、Profile、模型租约、用量、审计、订阅、订单、席位、制品和公共配置均必须可确定产品。
- 所有相关唯一约束、幂等 scope 和跨表引用包含产品；例如设备唯一性为 `(product, device_id)`。
- 历史数据回填为 `autolive`；旧客户端省略产品只允许在明确兼容窗口内映射为 `autolive`，窗口结束后 fail-closed。

### 管理权限

- 固定权限点与可复用自定义角色仍是权限目录事实源；用户角色分配增加产品范围。
- 普通管理员的资源范围取其产品角色并集；查询参数只能缩小范围，不能扩大范围。
- 内建 `super_admin` 可跨产品，但高风险跨产品操作必须确认、幂等、记录理由并审计。
- 不预留任意资源表达式、deny 规则或通用 ABAC；新增产品范围权限时使用明确表和固定动作。

### Profile 与离线授权

- Profile 至少返回 `product`、`server_time`、`authorization_status`、`authorization_checked_at`、`revalidate_at`、`offline_valid_until`、`reason_code`、`entitlement_revision`。
- 服务端签名离线授权绑定 user/device/product/subscription/seat/revision/key id；建议每 6 小时在线复核，离线硬上限 24 小时。
- 客户端不能延长服务端签名的截止时间；运行中使用单调时钟，持久化上次服务端时间，本机时钟明显回拨时进入 `offline_stale` 并要求联网。
- 恢复网络、应用回到前台、申请模型凭证和下载制品前重新校验。硬截止后禁止开始新的受保护任务，但允许保存本地数据和安全清理。
- 在线发现用户、产品成员、设备、订阅或席位失效时立即拒绝；允许的离线撤销传播上限为 24 小时。

### 模型认证

- 认证模式固定为 `local_byok`、`delegated_short_term`、`unavailable`。
- douyin-desktop 首版采用 `local_byok`；密钥只进入操作系统安全存储和调用进程内存，不进入 autoLive、Renderer、日志、错误报告或调用正文摘要。
- BYOK 不创建虚假服务端租约；调用摘要标记 `source=local_byok` 且不伪装成供应商权威账单。
- `delegated_short_term` 仅在供应商能签发绑定 lease/user/device/product、可撤销且有效期不超过租约的凭证时启用。
- 短期凭证只在创建/续租成功响应中交付一次；列表、详情、数据库普通列、管理端、日志和 trace 均不得回显。
- 供应商不满足条件时返回 `unavailable` 和稳定错误码，不以 `direct_base_url` 制造可调用假象。

### 固定契约与跨仓库验证

- autoLive CI 为指定 commit 生成单一 OpenAPI、`openapi.sha256` 和 `contract-metadata.json`；元数据包含 commit、契约版本、生成时间、生成器版本、兼容基线和 digest。
- 制品按版本/commit 不可变发布；消费端只接受固定 digest，不使用 `latest`、分支浮标或开发机绝对路径。
- P0 可使用受保护 CI/Release artifact；商业化 Phase 8 迁移到对象存储时保持同一 digest，不产生第二份内容事实源。
- 兼容门禁检查路径/字段删除、必填变化、枚举收窄、状态码和稳定错误码变化；破坏性变更必须走显式版本流程。
- douyin-desktop 的依赖更新通过审查后的 commit/digest 变更完成，不静默自动升级，也不手改生成类型。
- E2E 使用预构建服务/镜像、隔离 PostgreSQL、固定契约和可控时钟；服务器不构建源码。

## 功能需求

### 产品硬隔离

- FR-1：建立不可变 `products` 注册表和全局用户到产品的 `user_products` 关系。
- FR-2：客户端会话在认证/激活时绑定产品；后续请求中的产品提示与会话不一致时返回稳定 403/409 错误。
- FR-3：设备、激活码、绑定、Profile、租约、用量和审计的读写、唯一约束、幂等键与引用完整性包含产品。
- FR-4：激活码只能绑定声明产品；同一物理设备在不同产品是两条独立产品设备记录。
- FR-5：历史数据确定性回填为 `autolive`，兼容窗口、监控和最终必填切换可审查且可前滚。
- FR-6：管理端产品范围由服务端授权交集决定，所有管理列表支持有界产品筛选但不得以筛选代替鉴权。
- FR-7：固定权限目录保持全局，自定义角色可复用，用户角色绑定按产品生效；`super_admin` 是唯一默认跨产品角色。
- FR-8：审计记录操作者产品范围、目标产品和拒绝原因，但不记录秘密或高基数指标标签。

### Profile 授权新鲜度

- FR-9：Profile 返回经服务端计算的授权状态、服务端时间、建议复核时间、离线硬截止、原因码和权益修订号。
- FR-10：离线授权签名绑定用户、设备、产品、订阅、席位、修订和签名 key id，且可通过密钥轮换验证。
- FR-11：默认 `revalidate_at` 为签发后 6 小时，`offline_valid_until` 不晚于签发后 24 小时；服务端可缩短，不允许客户端延长。
- FR-12：用户/产品成员/设备/订阅/席位失效会增加权益修订并使后续在线校验立即拒绝。
- FR-13：客户端时钟回拨、授权签名无效、产品不匹配或超过截止时间时状态为 `offline_stale`/等价稳定拒绝状态。
- FR-14：硬截止后的安全降级允许保存和清理，但禁止开始新的模型、下载和其他受保护操作。

### 模型认证闭环

- FR-15：OpenAPI 明确模型认证模式，不再把只有 URL 的租约视为可执行凭证。
- FR-16：BYOK 密钥永不上传服务端；摘要只含来源、模型标识、用量/延迟和脱敏结果，不含正文或密钥。
- FR-17：短期凭证必须受限、短时、可撤销并绑定 lease/user/device/product；任一条件不成立即不可开放托管直连。
- FR-18：短期凭证只在创建/续租响应交付一次，释放、禁用、超时和回收触发撤销；续租签发新凭证而非延长旧秘密。
- FR-19：客户端、管理端和普通查询不能再次读取凭证；日志、trace、审计和错误正文执行敏感字段拒绝。
- FR-20：不支持托管直连时返回 `unavailable` 和稳定恢复建议，不自动降级为共享长期 Key。

### OpenAPI 与跨仓库 E2E

- FR-21：CI 发布绑定指定 autoLive commit 的 OpenAPI、SHA-256 和契约元数据三件套。
- FR-22：douyin-desktop 类型生成记录并校验 commit/digest，兼容差异与生成漂移导致 CI 失败。
- FR-23：跨仓库 E2E 覆盖登录→激活→心跳→Profile→可用认证模式→调用摘要→释放→退出。
- FR-24：负向矩阵覆盖用户/设备/产品成员禁用、授权过期、跨产品激活/查询、401、409、429、超时、取消、重启和提交结果未知。
- FR-25：可控时钟验证 6 小时复核、24 小时硬截止、时钟回拨和恢复联网；日志/制品扫描证明无 BYOK、Token 或短期凭证泄露。

## 非功能需求

- NFR-1：产品隔离在服务端、数据库约束和测试中共同执行，不能依赖 React 隐藏或客户端传参诚实性。
- NFR-2：`product` 可作为固定低基数指标标签；user/device/lease/order 等 ID 不作为指标标签。
- NFR-3：产品状态、席位和授权修订更新使用短事务、唯一约束/行锁与幂等语义，不在事务内调用供应商。
- NFR-4：签名和凭证采用版本化 key id；密钥不可用、签名失败和状态不确定均 fail-closed。
- NFR-5：API、迁移和兼容发布遵循先扩展、回填、双兼容、收紧、清理顺序，不修改历史迁移。
- NFR-6：所有新增列表有界分页/筛选；产品隔离索引与跨产品拒绝使用真实 PostgreSQL 集成测试验证。
- NFR-7：服务端只部署本地/CI 预构建制品，跨仓库 E2E 不授权在生产服务器构建源码。

## 依赖与约束

- 商业实体和席位依赖 [服务端商业化与生产就绪 PRD](./prd-server-commercial-production-readiness.md)，其产品范围必须消费本 PRD 的 `products`/`user_products` 基线。
- 当前 OpenAPI、Router、normalized PostgreSQL、Session、审计 Outbox、幂等、Secret Store、Prometheus 和 React Ant Design 管理面可复用。
- 供应商短期凭证能力是外部条件；未选定并验证前，douyin-desktop 保持 BYOK-only。
- 本 PRD 不改变当前“不开发实时话术幻化”的版本范围。

## 风险与边界

- 只给表加 `product` 而未修改唯一约束、外键、幂等 scope 或管理授权，会形成静默串数据。
- 兼容默认 `autolive` 若无截止窗口，会让新客户端伪装旧客户端绕过产品声明。
- 离线授权无法做到实时撤销；本方案明确最大传播窗口为 24 小时，敏感操作仍需在线校验。
- 将长期共享 Key 包装成“短期租约”不构成安全闭环，必须验证供应商真实撤销和范围能力。
- OpenAPI digest 只证明完整性；真实性签名和 Provenance 由商业化 Phase 8/11 的供应链门禁补齐。

## 执行规则

- 共享顺序为商业化 Phase 1 契约基线 → 本 PRD Phase 1 与商业化 Phase 2 协同交付 → 本 PRD Phase 2～4 → 商业化 Phase 3～11；商业资源不得抢跑产品基线。
- 每阶段先更新 OpenAPI/错误码和失败测试，再实现数据库、Go、React 与 CI；生成文件不手改。
- 阶段开始时重读主 PRD、当前 Phase、context、仓库约束和受影响调用方。
- 任何外部供应商能力在实施时查官方资料并完成撤销/范围/过期探测，不能只依据宣传文档。
- 阶段结束后更新本 PRD、商业化 PRD 的依赖和根文档；本轮不实施任何阶段。

## 阶段索引

| 阶段 | 状态 | 目标 | 验证重点 | 文件 |
| --- | --- | --- | --- | --- |
| Phase 1：产品硬隔离与管理范围 | Not Started | 建立产品、成员、会话、数据约束和产品范围 RBAC | 跨产品拒绝/兼容迁移/权限 | [phase-01-product-isolation.md](./prd-douyin-desktop-control-plane-integration/phase-01-product-isolation.md) |
| Phase 2：Profile 与离线授权 | Not Started | 服务端时间、签名新鲜度、6h/24h 和时钟回拨 | 签名/撤销窗口/安全降级 | [phase-02-profile-offline-authorization.md](./prd-douyin-desktop-control-plane-integration/phase-02-profile-offline-authorization.md) |
| Phase 3：模型认证闭环 | Not Started | BYOK 默认和可选短期委托凭证 | 不泄密/可撤销/无假租约 | [phase-03-model-authentication.md](./prd-douyin-desktop-control-plane-integration/phase-03-model-authentication.md) |
| Phase 4：固定契约与跨仓库 E2E | Not Started | 不可变 OpenAPI 制品、兼容门禁和产品矩阵 E2E | digest/复现/负向矩阵 | [phase-04-contract-artifact-cross-repo-e2e.md](./prd-douyin-desktop-control-plane-integration/phase-04-contract-artifact-cross-repo-e2e.md) |

## 最终评审

- 状态：Pending
- 评审时点：Phase 4 完成后
- 必须确认：产品隔离、离线授权、模型认证、契约制品、跨仓库 E2E 均有可复现证据，且商业化 PRD 未建立重复事实源。

## 变更记录

- 2026-08-22：基于 autoLive 与 douyin-desktop 当前契约建立接入缺口草案。
- 2026-08-22：用户确认单库硬逻辑隔离、全局身份加产品成员、6 小时建议复核/24 小时离线硬上限、BYOK 首发和分层双 PRD；拆分为四个 P0 阶段，本轮不实施。
