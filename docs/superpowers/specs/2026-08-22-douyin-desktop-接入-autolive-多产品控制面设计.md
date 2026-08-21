# douyin-desktop 接入 autoLive 多产品控制面设计

日期：2026-08-22
状态：已确认，待实施
关联 PRD：[P0 接入 PRD](../../../tasks/prd-douyin-desktop-control-plane-integration.md)、[P1/P2 商业生产 PRD](../../../tasks/prd-server-commercial-production-readiness.md)

## 1. 结论

autoLive 继续作为 `autolive` 与 `douyin_desktop` 的唯一 Go/React 控制面。首版使用一个 PostgreSQL 集群和单库硬逻辑隔离：产品必须进入会话、领域主键/唯一约束/引用、幂等和管理员授权，而不是只作为列表筛选参数。

身份使用“全局用户 + 每产品成员关系”；管理员使用“固定权限点 + 自定义角色 + 产品范围绑定”。Profile 由服务端签发建议 6 小时复核、24 小时硬截止的离线授权。douyin-desktop 首版模型认证使用本地 BYOK；只有供应商能提供可撤销、受限、绑定租约的短期凭证时，才启用托管直连。

OpenAPI 由 autoLive CI 按 commit/digest 发布不可变制品，douyin-desktop 固定消费。跨仓库 E2E 使用预构建服务/镜像与隔离 PostgreSQL，不依赖本地绝对路径，也不在服务器构建源码。

## 2. 分层双 PRD

| 层级 | 事实源 | 负责范围 |
| --- | --- | --- |
| P0 接入层 | `prd-douyin-desktop-control-plane-integration.md` | product、user_products、产品会话/RBAC、Profile 新鲜度、模型认证、OpenAPI 制品、跨仓库 E2E |
| P1/P2 商业生产层 | `prd-server-commercial-production-readiness.md` | 套餐/订单/支付/订阅/席位、密码重置、配置、桌面制品、错误反馈、备份恢复、可观测与供应链 |

两层共享 `product` 基线。商业层的订阅、席位、配置、制品等只能引用 P0 的产品注册表和成员关系；接入层只消费商业层的权益结果，不复制商业状态机。

## 3. 产品硬隔离

### 3.1 数据模型

- `products(code, status, ...)`：`code` 不可变，首批种子 `autolive`、`douyin_desktop`。
- `users`：全局身份和凭证，不按产品复制账号。
- `user_products(user_id, product, status, entitlement_revision, ...)`：产品准入与修订事实。
- 产品业务资源必须包含或可由受约束引用确定 `product`：设备、激活码、设备绑定、Profile、模型租约、用量、审计，以及商业层的订阅、订单、席位、配置、制品和错误反馈。

产品字段不是装饰字段：相关唯一键、外键/领域引用和幂等 scope 都必须包含产品。例如设备唯一性为 `(product, device_id)`，跨产品使用相同 `device_id` 是两条独立记录。

### 3.2 会话边界

登录或激活时确定产品并写入会话。后续请求不能通过 query/body/header 临时切换产品；显式提示与会话不一致时 fail-closed。激活码声明自己的产品，不能跨产品核销。

历史数据确定性回填为 `autolive`。旧客户端省略产品只在明确兼容窗口内映射为 `autolive`，并计数、告警和设置退出条件；收紧后缺失产品直接拒绝，避免永久兼容变成绕过路径。

### 3.3 管理员授权

权限代码仍由服务端固定目录管理，自定义角色可跨产品复用；具体管理员的角色分配带产品范围。普通管理员查询可见范围为“自身授权产品 ∩ 请求筛选”，因此省略或伪造筛选都不能扩大权限。

内建 `super_admin` 可跨产品。跨产品高风险写操作需要显式确认、理由、幂等和审计。首版不引入任意资源表达式、deny 规则或通用 ABAC。

## 4. Profile 授权新鲜度

### 4.1 返回契约

Profile 至少返回：

- `product`
- `server_time`
- `authorization_status`
- `authorization_checked_at`
- `revalidate_at`
- `offline_valid_until`
- `reason_code`
- `entitlement_revision`
- 服务端签名离线授权及其 `key_id`

签名载荷绑定 user/device/product/subscription/seat/revision。尚未实施商业层时，subscription/seat 使用明确的未启用语义，不伪造权益。

### 4.2 时间策略

- 默认建议在线复核：签发后 6 小时。
- 默认离线硬截止：签发后 24 小时。
- 服务端可根据风险缩短两个时间，客户端不能延长。
- 网络恢复、应用回到前台、申请模型凭证和下载制品前重新校验。

客户端进程内使用单调时钟，安全存储最后一次服务端时间。本机时钟明显回拨、签名无效、产品/设备不匹配或超过硬截止时进入 `offline_stale` 并要求联网。

硬截止后禁止开始新的受保护任务、模型调用和制品下载；允许保存本地数据、导出必要结果和安全释放资源。在线发现用户、产品成员、设备、订阅或席位失效时立即拒绝。离线模型无法实现实时撤销，本方案明确最大传播窗口为 24 小时。

## 5. 模型认证闭环

### 5.1 模式

| 模式 | 使用条件 | 密钥位置 | 服务端租约 |
| --- | --- | --- | --- |
| `local_byok` | douyin-desktop 首版默认 | OS 安全存储 + 调用进程内存 | 不创建假租约 |
| `delegated_short_term` | 供应商支持受限、短时、可撤销凭证 | 创建/续租响应一次性交付 | 真实绑定 lease/user/device/product |
| `unavailable` | 无 BYOK 或供应商能力不合格 | 无 | 返回稳定错误和恢复建议 |

BYOK 不上传 autoLive，不进入 Renderer、日志、trace、审计、错误报告或调用正文摘要。摘要标记 `source=local_byok` 和非权威属性，不能伪装成供应商账单。

委托凭证有效期不能超过租约；续租签发新凭证，不能延长旧秘密。释放、设备/产品成员禁用、超时和回收触发撤销。凭证只在创建/续租成功响应中返回一次，列表、详情、数据库普通列和 React 管理端不得再次读取。

供应商不能同时满足短时、受限、绑定和可撤销时，托管直连保持关闭。长期共享 API Key 即使包装在短期接口中也不符合设计。

## 6. OpenAPI 固定制品

autoLive CI 从唯一契约源为指定 commit 生成：

1. `openapi.yaml`
2. `openapi.sha256`
3. `contract-metadata.json`

metadata 包含 autoLive commit、契约版本、生成时间、生成器版本、兼容基线和 digest。制品按版本/commit 不可变；禁止消费 `latest`、分支浮标或开发机绝对路径。

P0 可托管在权限受控的 CI/Release artifact。商业化 Phase 8 迁移到对象存储时必须保持同一 digest，不能重新生成第二份内容。SHA-256 证明完整性；签名真实性和 Provenance 由后续供应链门禁补齐。

兼容门禁至少检查路径/字段删除、必填变化、枚举收窄、HTTP 状态码和稳定错误码变化。douyin-desktop 记录 commit/digest 后生成 TypeScript 类型；生成产物不能手改，升级通过审查式 pin 变更完成。

## 7. 跨仓库 E2E

测试环境使用预构建 autoLive 服务/镜像、隔离 PostgreSQL、固定 OpenAPI digest、固定产品种子和可控时钟。主路径为：

`登录 → 激活 → 心跳 → Profile → 认证模式 → 调用摘要 → 释放 → 退出`

负向矩阵覆盖：

- 用户、产品成员、设备禁用和授权过期；
- 相同 device ID 的跨产品隔离与跨产品激活码拒绝；
- 管理员产品范围、租约/用量/审计以及后续订阅/席位范围；
- 401 Refresh、409 幂等冲突、429、超时、取消、进程重启和提交结果未知；
- 6 小时复核、24 小时硬截止、本机时钟回拨和网络恢复；
- CI 日志、测试报告和 artifact 的 Token/BYOK/短期凭证敏感扫描。

契约升级不静默自动跟随。每次 douyin-desktop 更新 pin 都需要兼容差异和 E2E 通过。

## 8. 商业与运营补充

商业层按 product 建立套餐版本、订单、支付、订阅和设备席位。订阅有效即开放该产品全部功能；套餐只区分价格、周期和席位，不按 Token 或功能等级划分。激活码是设备绑定凭证，订阅和席位才是商业事实，二者不能混为一个状态。

席位激活必须原子占用，不能并发超卖。解绑、禁用、转移席位要撤销会话、模型凭证和离线授权；转移不转移旧会话。访问判定顺序为用户启用 → 产品成员启用 → 设备启用 → 订阅有效 → 席位有效。

公共配置按 product/environment/namespace 管理 Schema、revision、ETag、发布、回滚、紧急停用和最低客户端版本，但配置不能授予商业权益。桌面制品按 product/channel/platform/arch 管理，并支持最低版本、灰度比例、强制升级、撤回和防降级信任策略。

错误报告必须用户同意、客户端和服务端双重脱敏、有界批量、采样、指纹去重、幂等、限流和保留删除；首版拒绝 dump、截图、本地路径、Token/Cookie、Prompt/正文和 BYOK。服务端专项负责接收契约、二次脱敏和管理面，客户端预脱敏接线由消费端任务实施并通过 fixture/E2E 验收。反馈只提供 `open → in_progress → resolved/closed` 最小状态流，不含附件、外部工单同步和通知。

`product` 允许作为低基数指标标签，用户/设备/订单等 ID 不允许。备份恢复需校验产品关系、席位占用和跨产品引用；Tracing、HTTPS、签名/Provenance 和恢复演练是生产放量门禁。

## 9. 发布顺序

1. 扩展契约和可空字段，创建产品注册表与成员关系。
2. 回填历史数据为 `autolive`，建立产品范围索引/引用和会话绑定。
3. 双兼容观察旧客户端，完成跨产品负向测试后收紧必填。
4. 上线 Profile 签名新鲜度，再上线模型 BYOK/可选委托模式。
5. 发布固定 OpenAPI 制品并完成跨仓库 E2E。
6. 商业化 PRD 在同一 product 基线上实现 P1/P2，不复制事实源。

## 10. 本轮范围

本轮只将已确认设计写入设计文档与分层 PRD；未修改服务端代码、OpenAPI、数据库迁移、React、douyin-desktop、CI 或服务器部署。
