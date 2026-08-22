# PRD：服务端商业化与生产就绪

## 文档状态

- 状态：Draft
- 文件模式：Split
- 当前阶段：Phase 2 已实现；Phase 3～11 尚未开始
- 活动阶段文件：[Phase 2：管理员 RBAC 与权限化管理壳](./prd-server-commercial-production-readiness/phase-02-admin-rbac.md)
- 上下文：[context.md](./prd-server-commercial-production-readiness/context.md)
- 设计：[服务端商业化与生产能力补齐设计](../docs/superpowers/specs/2026-08-21-服务端商业化与生产能力补齐设计.md)
- P0 依赖：[douyin-desktop 接入 autoLive 多产品控制面](./prd-douyin-desktop-control-plane-integration.md)
- 最后更新：2026-08-22
- PRD 文件：`tasks/prd-server-commercial-production-readiness.md`
- 目的：作为 Go 服务端商业化、React 管理面、安全发布和生产运维的活文档与执行事实源。

## P0 产品基线依赖状态

多产品控制面 Phase 1 已完成服务端基线，商业化 Phase 2 已在其上交付固定权限目录、自定义角色生命周期、产品角色绑定、即时服务端鉴权和 React 权限壳；订阅/席位、支付、制品、公共配置和错误反馈仍未实施。详见 [P0 接入 PRD](./prd-douyin-desktop-control-plane-integration.md) 及其 [Phase 1 阶段文件](./prd-douyin-desktop-control-plane-integration/phase-01-product-isolation.md)。所有新增商业资源必须复用该基线，不得另建产品或成员事实源。

## 问题

当前 Go 控制面已具备用户、设备、激活码、模型号池、租约、用量、审计、PostgreSQL 与部分保留清理/指标能力，但缺少完整商业资源查询、生产支付、密码重置投递、对象存储发布、制品签名验证、备份恢复、追踪和 namespace 级配置 Schema。此前仅有 `admin/user` 粗粒度角色和会话级管理端；这些 RBAC 基线问题已由 Phase 2 修正，剩余商业能力仍待后续阶段。现有激活码仅保存哈希和首个核销设备，无法再次显示新功能上线前的明文，也无法准确管理全部历史设备绑定。

本专项的实施边界包含 Go 服务端和 React 管理端：Go API/领域服务、PostgreSQL/Secret Store、服务端 Worker、支付/邮件/对象存储适配器、服务端发布 CI、生产运维，以及基于 Ant Design 的角色权限和业务管理页。所有商业与运营资源必须消费 P0 接入 PRD 的 `product`、`user_products`、产品会话和产品范围 RBAC 基线。Rust/Tauri 桌面客户端仍不在本专项实施范围。

## 目标

- G-1：管理员能通过有界、可组合、可审计的 API 查询订阅、订单、设备和套餐版本。
- G-2：建立不可变套餐版本、订单、微信支付、对账和订阅权益交付闭环。
- G-3：建立不枚举账号、Token 仅存哈希、可重试可死信的密码重置投递 Worker。
- G-4：新激活码使用独立加密表支持管理员二次认证后重显，并支持查看/修改设备容量与切换已知绑定。
- G-5：在服务端使用私有对象存储、短时预签名 URL、签名清单、SBOM 和 Provenance 建立外部制品分发链。
- G-6：将数据保留、备份/恢复、指标/追踪、容量告警和可验证 CI 发布变成上线门禁。
- G-7：每个 product/environment/namespace 公共配置通过版本化 JSON Schema Draft 2020-12 校验后才能发布。
- G-8：使用自定义角色与服务端固定权限点实现管理员最小权限，并在 React 管理端中提供角色配置、用户角色分配和按权限展示的业务页。
- G-9：套餐版本、订单、支付、订阅、设备席位、公共配置、制品、错误反馈和运维视图按产品隔离，两个客户端不共享商业事实或管理员可见范围。
- G-10：建立经过同意、双重脱敏、有界、可保留删除的客户端错误摘要和最小反馈管理闭环。

## 非目标

- NG-1：本 PRD 不恢复实时话术幻化、旧媒体任务、服务端模型正文代理或请求预占。
- NG-2：对象存储不存储用户本地视频/音频，不使 Go API 代理大文件数据面。
- NG-3：首期不做退款、优惠券、发票、多币种、短信重置和多支付渠道。
- NG-4：不预先拆微服务、引入 Kubernetes 或外部消息队列。
- NG-5：本商业化专项除已交付的 Phase 2 RBAC 基线外，本轮不实施订阅/支付/制品/配置/生产运维等后续商业能力；P0 Phase 1 的基础代码和迁移由接入专项维护。
- NG-6：本专项不实现 Rust/Tauri 下载器、客户端安装器或客户端签名验证逻辑；桌面端消费能力另立集成任务。

## 成功标准

- SC-1：套餐版本、订单、订阅、设备席位和设备的列表/详情均具备 SQL 分页、固定白名单筛选/排序、稳定错误码和产品范围权限测试。
- SC-2：重复微信回调、结果未知和 Worker 重启不会重复收款或重复交付订阅；未验签、金额不一致或主体不一致的通知不会交付权益。
- SC-3：密码重置不枚举账号，Token 不以明文落库/日志，投递 Worker 的重试、死信、取消和优雅关闭可验证。
- SC-4：功能上线后的新激活码可在 HTTPS 下经二次认证重显；历史明文和未知历史设备不伪造。
- SC-5：未通过签名身份、subject digest 和 Provenance 策略校验的制品无法由服务端发布或签发下载地址；服务端响应提供签名清单、digest 和客户端后续验证所需元数据。
- SC-6：达到已验证的 RPO≤24h、RTO≤4h，备份恢复、回滚、指标告警和 trace 链路有可复现证据。
- SC-7：未注册 namespace、Schema 无效、实例校验失败、包含敏感字段或依赖远程 `$ref` 的公共配置不能发布。
- SC-8：服务器不构建源码，只加载经 CI 验证和签名的发布制品。
- SC-9：管理员可持有多个自定义角色，有效权限为固定权限点并集；服务端对每个受保护操作即时鉴权，React 导航、路由、按钮和 403 状态与当前权限一致，且无法通过直接调用 API 绕过。
- SC-10：有效订阅按产品开放全部功能，套餐只区分价格、周期和设备席位；席位原子占用不超卖，跨产品订阅/席位不能用于激活、Profile、模型或下载。
- SC-11：错误报告不含 dump、截图、本地路径、Token/Cookie、Prompt/业务正文或 BYOK；反馈状态流和管理查询按产品授权、可审计且有界。

## 关键场景

### 场景 1：超级管理员分配最小权限

- 操作者：超级管理员
- 触发：创建自定义角色，从固定权限点中选择能力，再按产品绑定给一个或多个管理员
- 结果：权限立即生效，不得授予操作者自身不具备的权限，角色变更可审计且不能删除最后一个超级管理员

### 场景 2：管理员调查商业事实

- 操作者：管理员
- 触发：按用户、状态、套餐版本和时间窗口筛选订单/订阅
- 结果：获得有界分页结果与详情，不暴露支付凭证或原始通知

### 场景 3：微信支付交付订阅

- 操作者：普通用户、微信支付、支付 Worker
- 触发：用户为固定套餐版本创建订单并扫码支付
- 结果：验签、解密、幂等落库和对账通过后，订单变为已支付并交付一份订阅权益

### 场景 4：用户自助重置密码

- 操作者：用户、投递 Worker
- 触发：用户提交账号标识，Worker 向已验证邮箱发送一次性重置链接
- 结果：外部响应不泄露账号是否存在，Token 单次消费后撤销用户已有会话

### 场景 5：管理员管理激活码

- 操作者：管理员
- 触发：二次认证后重显新激活码，或修改容量/切换已知设备
- 结果：敏感读取受限流和审计保护；切换不增加已核销名额且旧会话失效

### 场景 6：客户端下载可信制品

- 操作者：CI、服务端、已授权客户端
- 触发：CI 上传已签名制品，客户端请求短时下载
- 结果：只有通过签名/Provenance 策略的制品可获得短时 URL，响应携带消费端后续验证所需的签名清单和 digest

### 场景 7：订阅占用产品设备席位

- 操作者：用户、客户端、管理员
- 触发：客户端使用对应产品激活码绑定设备，或管理员释放/转移一个已知席位
- 结果：有效订阅原子占用一个产品席位；并发不超卖，释放/转移撤销旧会话、租约和离线授权，且不影响另一产品

### 场景 8：用户提交脱敏错误与反馈

- 操作者：用户、客户端、管理员
- 触发：用户同意上传有界错误摘要或提交文本反馈
- 结果：客户端和服务端双重脱敏，服务端按产品幂等接收、采样/限流和保留删除；管理员仅在授权产品内处理最小状态流

## 发现摘要

- 证据、当前系统和验证面见 [context.md](./prd-server-commercial-production-readiness/context.md)。
- 当前 Router 只有用户、设备、激活码、模型账号/租约/用量和审计管理路由，未见订阅、订单、套餐版本或支付领域。
- 初始发现曾只有 `admin/user` 角色和统一 `requireAdmin`，React 管理端仅有会话守卫；该发现已由 Phase 2 的权限目录、角色绑定、固定权限中间件和 React 权限壳修正。
- 当前设备有列表/详情，但缺少本 PRD 要求的完整筛选与排序契约。
- 当前设备、激活码、租约、用量和审计已具备 P0 Phase 1 的服务端 `product` 硬隔离与管理查询范围；迁移 0023 仍处于兼容阶段，固定权限/商业资源和终态复合唯一键尚未完成，本 PRD 只消费 P0 产品事实。
- 现有保留 Worker 和 Prometheus 是可复用基础；保留范围、容量告警、备份恢复和 tracing 未形成生产闭环。
- 现有制品链能生成 SHA-256、SBOM 和 BuildKit Provenance，运行资源通过 SSH/rsync 发布并验证清单；尚无对象存储、下载授权、签名身份和下游 Provenance 验证。
- 代码中未找到用户所述“已有公共配置只校验 JSON/敏感字段”的对应服务端路由或领域实现；实施前按用户提供的外部现状重新核对，但本 PRD 已将 namespace 级 Schema 作为完整目标纳入。
- 当前未见错误报告或反馈路由/领域/迁移；douyin-desktop 旧契约只作迁移参考，不能成为服务端事实源。

## 需求

### 功能需求

- FR-1：OpenAPI 作为订阅、订单、套餐版本、设备席位、支付、激活码、制品、公共配置、错误报告和反馈 API 的唯一外部契约。
- FR-2：管理列表支持页码/页大小、资源白名单筛选、RFC3339 时间窗口、白名单排序和数据库中的 `COUNT + LIMIT/OFFSET`。
- FR-3：订阅列表/详情返回用户、套餐版本、状态、起止时间、来源订单和脱敏权益摘要。
- FR-4：订单列表/详情支持用户、渠道、状态、套餐版本、商户订单号和时间筛选，不返回微信密钥或原始通知。
- FR-5：套餐版本一经发布即不可原地修改，订单保存当时价格/权益快照。
- FR-6：设备管理 API 支持用户、状态、在线派生状态、OS、客户端版本、心跳时间和排序筛选。
- FR-7：订单、支付尝试和订阅使用显式状态机、唯一约束和幂等键。
- FR-8：首期生产支付适配器使用微信支付 API v3 官方 Go SDK 创建 Native 支付并查询/关闭订单。
- FR-9：微信回调必须对原始 body 验签/解密，校验金额、币种、APPID、商户号和订单号，通知重入只交付一次。
- FR-10：支付结果未知不盲目重建订单，由对账 Worker 查询供应商状态并恢复。
- FR-11：订阅权益只由已验证支付事实或受审计的管理员命令交付。
- FR-12：密码重置请求返回中性结果，Token 只存哈希且单次消费，成功后撤销用户会话。
- FR-13：密码重置投递 Worker 使用持久化 Outbox，具有幂等、批次、超时、重试、退避、死信和优雅关闭。
- FR-14：功能上线后创建的激活码在独立表使用 AES-256-GCM、AAD、`format_version` 和 `key_id` 加密保存明文。
- FR-15：激活码重显必须是管理员、经密码二次认证、专用限流、审计、HTTPS 和 `no-store` 保护的 POST 操作。
- FR-16：激活码详情返回逐设备绑定；历史无法还原的明文/设备明确标记不可用/未知。
- FR-17：激活码容量为 1～100，不能低于已核销名额；切换设备使用原 slot，撤销旧设备会话/租约，并不转移会话给新设备。
- FR-18：制品元数据包含版本、渠道、平台/架构、对象 key、字节数、SHA-256、签名 bundle、SBOM 和 Provenance 引用。
- FR-19：制品只有在签名者身份、issuer、repository/workflow/ref、subject digest 和 Provenance 策略通过后才能发布。
- FR-20：对象存储保持私有，下载前校验订阅/设备权限并签发短时、只读、单对象预签名 URL。
- FR-21：服务端下载响应必须返回或引用已验证的签名清单、制品 digest、平台/架构和信任策略版本；消费端执行安装前校验属于后续客户端集成任务。
- FR-22：每个公共配置 namespace 必须绑定不可变 JSON Schema Draft 2020-12 版本，未注册 namespace 拒绝发布。
- FR-23：Schema 校验禁止运行时任意远程 `$ref`，敏感字段拒绝规则在 Schema 校验后仍必须执行。
- FR-24：Schema 发布前 dry-run 已有配置，客户端读取响应携带 namespace、schema version、revision 和 ETag。
- FR-25：保留 Worker 覆盖新数据集，每类数据有 TTL、批次、超时、索引、容量告警和审计例外。
- FR-26：PostgreSQL 和对象存储有加密备份、失效域隔离、保留和隔离恢复演练。
- FR-27：Prometheus 增加商业/Worker/存储/备份/发布低基数指标，OpenTelemetry 通过 OTLP 导出 HTTP、DB、Worker 和供应商追踪。
- FR-28：CI 生成和签名 SBOM/Provenance，部署和对象存储发布前都要验证，服务器不构建源码。
- FR-29：权限目录是服务端固定、版本化的权限点集合，自定义角色只能组合已登记权限点，管理员不能创建任意权限代码。
- FR-30：一个管理员可绑定多个角色，有效权限取并集；首期不提供用户级 allow/deny 覆盖，避免第二个权限事实源。
- FR-31：系统保留内建 `super_admin` 角色；本地管理员永久具有超级权限，现有 `role=admin` 账号通过可审查迁移保持兼容，且系统不得移除最后一个可用超级管理员。
- FR-32：受保护管理 API 使用按权限点鉴权的 `requirePermission` 等价边界，每次请求计算当前有效权限，不把长期可过期权限快照作为 Token 中的唯一授权依据。
- FR-33：提供当前管理员/权限目录、角色 CRUD 和用户角色分配 API；写操作具有幂等、防越权委派、最后超管保护和审计。
- FR-34：React 管理端通过 `/api/v1/admin/me` 或等价契约获取当前角色与权限，按权限控制导航、路由、按钮和操作列，并提供统一无权限/403 状态；前端隐藏不替代服务端鉴权。
- FR-35：React 管理端提供角色管理、产品范围用户角色分配，以及套餐版本、订单、订阅、设备席位、支付/对账、密码重置投递、激活码安全操作、制品发布、公共配置、错误报告、反馈和运维状态页，统一使用 Ant Design 和 OpenAPI 生成契约。
- FR-36：商业层依赖 P0 接入 PRD 的 `products`、`user_products`、产品会话和产品范围角色绑定，不另建第二套产品/成员事实源。
- FR-37：套餐版本、订单、支付、订阅、设备席位、公共配置、制品、错误报告和反馈均绑定 `product`；管理授权在服务端取操作者产品范围交集，筛选参数不能扩大权限。
- FR-38：设备席位包含 slot、subscription、product、device、绑定/释放/转移时间和状态；激活原子占用且不得超卖，释放/禁用/转移撤销旧会话、租约和离线授权，转移不继承旧会话。
- FR-39：订阅有效即开放该产品全部功能，套餐只区分价格、周期和设备席位，不按 Token 余额或功能等级分层；访问判定顺序为用户 → 产品成员 → 设备 → 订阅 → 席位。
- FR-40：公共配置作用域为 product/environment/namespace，支持 Schema、revision、ETag、发布、回滚、紧急停用和最低客户端版本；配置不得授予或延长商业权益。
- FR-41：桌面制品作用域为 product/channel/platform/arch，支持最低版本、灰度比例、强制升级、撤回和防降级信任策略；下载仍需在线复核产品订阅与席位。
- FR-42：错误报告必须经用户同意、客户端预脱敏和服务端再次脱敏，使用有界批量、采样、指纹去重、幂等、限流、TTL 和删除；本专项实现服务端契约/二次脱敏/管理面并以消费端 fixture/E2E 验收，不修改 Rust/Tauri；首版拒绝 dump、截图、本地路径、Token/Cookie、Prompt/正文和 BYOK。
- FR-43：反馈只提供 product/user 归属、受限文本和 `open → in_progress → resolved/closed` 最小状态流；首版不做附件、外部工单同步或通知。
- FR-44：指标允许使用固定低基数 `product` 标签并建立每产品容量/错误预算；用户、设备、订单、报告等 ID 不进入指标标签，单一产品的噪声不得拖垮其他产品。

### 非功能需求

- NFR-1：所有管理读写在服务端验证角色、动作和资源，不依赖前端隐藏按钮。
- NFR-2：密码、Token、BYOK/短期凭证、明文激活码、微信密钥/原始回调、邮箱、错误报告正文和预签名 URL 不进入普通日志、指标、trace 或审计 payload。
- NFR-3：金额使用整型分和显式币种，不使用浮点数。
- NFR-4：外部调用有超时、取消、有界重试和重定向/SSRF 限制，不在数据库事务内执行。
- NFR-5：Worker 有所有者、并发上限、锁租约、取消、优雅关闭和死信运维入口。
- NFR-6：迁移是可审查前向迁移，不修改 0001～0022；每阶段说明发布顺序、回滚/前滚和锁表影响。
- NFR-7：列表无 N+1 和无界内存分页；索引使用代表性数据的 `EXPLAIN (ANALYZE, BUFFERS)` 验证。
- NFR-8：生产明文激活码、微信支付、错误/反馈提交和制品下载仅通过 HTTPS。
- NFR-9：首期备份目标 RPO≤24h、RTO≤4h，只有隔离恢复演练可作为达标证据。
- NFR-10：服务端新代码按领域拆分文件，复用现有 Outbox、幂等、限流和可观测边界，删除本次暴露的未使用 Go 代码/导入/配置。
- NFR-11：服务端是授权唯一权威，角色变更在后续请求中即时生效；权限查询有索引和有界资源使用，不引入第二个不可审计缓存事实源。
- NFR-12：React 管理页覆盖 loading、empty、error、403 和请求竞态/取消，具备键盘、焦点和可读标签等基本可访问性，不覆盖 Ant Design 内部样式。
- NFR-13：产品隔离通过服务端授权、数据库唯一约束/引用和真实 PostgreSQL 集成测试共同证明；任何缺失/不一致 product 都 fail-closed。
- NFR-14：错误/反馈请求体、字段长度、批大小、速率、保留期和管理分页均有上限，敏感字段扫描失败时拒绝而非带病入库。

## 假设

- A-1：首个付费交付场景为桌面客户端/管理 Web 显示微信 Native 支付二维码。
- A-2：首期密码重置投递渠道为邮箱，编程边界不绑定具体 SMTP/邮件供应商。
- A-3：对象存储提供 S3-compatible 基本语义；具体厂商、区域和 CDN 在 Phase 8 发现门禁确认。
- A-4：保留 Prometheus 指标，首期只用 OpenTelemetry 增加 tracing，避免重复指标事实源。
- A-5：功能上线前的激活码明文无法恢复；历史多设备除首台外无法准确归属。
- A-6：首期权限冲突按角色并集解决，不定义 deny 优先级；业务资源范围权限在需求出现前不预留通用策略引擎。
- A-7：默认受控交付采用管理员创建/邀请用户；公开注册只有在运营明确启用后才增加邮箱验证、防枚举、限流和滥用治理。
- A-8：设备席位转移可配置冷却期；紧急管理员覆盖必须填写原因并审计，不转移活动会话。

## 依赖/约束

- 现有 PostgreSQL normalized 读写、Outbox、会话、限流、Prometheus、OpenAPI 契约门禁和 CI 构建链。
- 共享顺序为本 PRD Phase 1 契约基线 → P0 Phase 1 与本 PRD Phase 2 协同交付 → P0 Phase 2～4 → 本 PRD Phase 3～11；商业资源不得绕过 P0 自行增加弱筛选字段。
- 现有 React/Vite/TypeScript 管理端、Ant Design、路由/会话守卫和 OpenAPI 生成类型链。
- 微信支付商户号、APPID、API v3 密钥、商户私钥/证书或微信支付公钥；所有秘密由 Secret Store/外部秘密注入管理。
- 受控邮件投递凭证、可信域名、重置链接 origin 和已验证用户邮箱数据。
- 私有对象存储、托管 CI OIDC 身份、Sigstore/GitHub Attestation 能力和服务端发布验证信任策略。
- 生产 HTTPS、对象存储 TLS、PostgreSQL TLS 和不高于 9999 的公网监听端口约束。

## 风险/边界情况

- 支付回调重复、延迟、丢失、签名探测、证书/公钥轮换、金额不一致和未知提交结果。
- 回调履约短事务提交结果未知或回调丢失；必须由本地幂等事实和主动查单恢复，不重复订阅，也不把已确认付款长期伪装成未付款。
- 历史激活码和绑定数据缺口无法通过迁移消除，界面/API 必须显式表达未知。
- 对象存储预签名 URL 泄露、无限制续期、签名者策略过宽或仅校验 SHA-256 会导致伪造制品被信任。
- 备份可创建不等于可恢复；只有隔离恢复与业务核对能证明 RPO/RTO。
- 不受限的 JSON Schema 远程 `$ref`、灾难正则、过大 Schema/实例或 Schema 变更可造成 SSRF/DoS/配置不兼容。
- 自定义角色可能通过越权委派、并发更新或删除最后超管导致提权或锁死，必须在服务端事务中实施授权子集和最后超管保护。
- React 菜单隐藏不是安全边界；会话中的陈旧权限、直达 URL 和手工 API 请求必须由服务端当前权限校验拒绝。
- 产品字段若只进入 DTO/筛选而未进入唯一约束、外键/领域引用、幂等和角色范围，会造成静默串数据。
- 错误报告可能成为秘密和个人数据外泄通道；必须双重脱敏、严格拒绝字段、有界存储和可执行删除。

## 执行规则

- 先执行 Phase 1；Phase 2 与 P0 Phase 1 使用同一权限模型协同交付，随后完成 P0 Phase 2～4，再按本 PRD Phase 3～11 顺序执行。任何并行都必须使用不重叠写入集，由主线统一审查。
- 实施任一阶段前读本 PRD、当前 Phase、context 和对应专项约束。
- 每阶段先写失败测试/等价验证，再写实现；OpenAPI 变更先于手写 DTO/路由，生成文件不手改。
- 外部供应商和平台规则在阶段开始时重新查阅官方文档，不依赖本 PRD 的时点性记忆。
- 服务器只接收本地/CI 构建且签名的制品，不上传源码构建。
- 每阶段结束后更新当前 Phase、主 PRD 和受影响的后续 Phase，并完成 Go/React 代码总监复核；Rust/Tauri 只做契约兼容检查，不在本专项修改业务代码。

## 阶段索引

| 阶段 | 状态 | 目标 | 验证重点 | 文件 |
| --- | --- | --- | --- | --- |
| Phase 1：契约与领域基线 | Not Started | 锁定资源、状态机、事务、错误码和迁移顺序 | 契约/迁移/安全边界 | [phase-01-contract-domain-baseline.md](./prd-server-commercial-production-readiness/phase-01-contract-domain-baseline.md) |
| Phase 2：管理员 RBAC 与权限化管理壳 | 已实现；PG/浏览器验收待补 | 固定权限点、自定义角色、多角色并集与 React 权限壳 | 防提权/立即生效/403 | [phase-02-admin-rbac.md](./prd-server-commercial-production-readiness/phase-02-admin-rbac.md) |
| Phase 3：激活码安全管理 | Not Started | 加密明文、绑定明细、容量与切换管理页 | 密钥/事务/历史兼容 | [phase-03-activation-security.md](./prd-server-commercial-production-readiness/phase-03-activation-security.md) |
| Phase 4：商业核心事实 | Not Started | 套餐版本、订单、订阅和权益状态机 | 价格快照/幂等/并发 | [phase-04-commercial-core.md](./prd-server-commercial-production-readiness/phase-04-commercial-core.md) |
| Phase 5：管理查询 API 与管理页面 | Not Started | 补齐订阅、订单、套餐版本和设备查询/筛选及 React 页面 | SQL 分页/筛选/权限/URL 状态 | [phase-05-admin-query-apis.md](./prd-server-commercial-production-readiness/phase-05-admin-query-apis.md) |
| Phase 6：微信支付 | Not Started | Native 下单、验签回调、对账、权益交付与支付管理页 | 官方 SDK/回调幂等/未知结果 | [phase-06-wechat-pay.md](./prd-server-commercial-production-readiness/phase-06-wechat-pay.md) |
| Phase 7：密码重置 Worker | Not Started | 一次性 Token、邮件 Outbox、可恢复投递与管理页 | 防枚举/限流/重试/死信 | [phase-07-password-reset-worker.md](./prd-server-commercial-production-readiness/phase-07-password-reset-worker.md) |
| Phase 8：制品存储与下载 | Not Started | 对象存储、发布清单、签名校验、短时下载与发布页 | 真实性/授权/完整性 | [phase-08-artifact-distribution.md](./prd-server-commercial-production-readiness/phase-08-artifact-distribution.md) |
| Phase 9：公共配置 Schema | Not Started | product/environment/namespace JSON Schema 注册、校验、发布与管理页 | Draft 2020-12/SSRF/兼容性 | [phase-09-public-config-schema.md](./prd-server-commercial-production-readiness/phase-09-public-config-schema.md) |
| Phase 10：客户端错误报告与反馈 | Not Started | 双重脱敏错误摘要、最小反馈状态流和产品范围管理页 | 隐私/限流/保留/权限 | [phase-10-client-reports-feedback.md](./prd-server-commercial-production-readiness/phase-10-client-reports-feedback.md) |
| Phase 11：生产运维与供应链 | Not Started | 保留、备份恢复、指标/追踪、签名/Provenance、发布和运维视图 | RPO/RTO/告警/可验证发布 | [phase-11-operations-supply-chain.md](./prd-server-commercial-production-readiness/phase-11-operations-supply-chain.md) |

## 全部阶段结束后的多轮复核

- [ ] 1. 需求覆盖：每个 FR、NFR 和成功标准已满足或显式延期。
- [ ] 2. 跨阶段集成：套餐→订单→支付→订阅→下载权限无重复事实源或断链。
- [ ] 3. 正确性：成功、重复、失败、结果未知、过期、取消、并发和恢复路径全部覆盖。
- [ ] 4. 简化复核：未引入无实际负载证据的微服务、队列、缓存或抽象。
- [ ] 5. 重复/清理：死代码、无用导入/依赖、临时开关、敏感调试日志和重复组件已删除。
- [ ] 6. 安全/隐私：产品授权、身份、密钥、支付、邮箱、激活码、BYOK、错误报告、预签名 URL 和制品信任边界通过。
- [ ] 7. 性能/负载：产品公平性、管理查询、报告接收、支付回调、Worker、保留清理和对象存储无无界资源使用。
- [ ] 8. 验证：单元、PostgreSQL 集成、API E2E、供应商沙箱/小额真实验收、恢复演练和发布签名校验适合风险。
- [ ] 9. 文档/运维：OpenAPI、错误码、架构、Runbook、迁移、回滚、告警和支持文档同步。
- [ ] 10. PRD 收尾：状态、变更记录、延期项和实际证据更新完成。
- [ ] 11. 管理端验收：权限矩阵、直达 URL、操作按钮、403、加载/空/错误态和键盘/焦点通过类型检查、组件测试与浏览器冒烟。

## 开放问题

- 在 Phase 6 开始前确认微信 Native 支付是否为首个上线交付形态；若改为 JSAPI/App/H5，必须先修订 Phase 6。
- 在 Phase 7 开始前确认邮件供应商、发件域和用户邮箱验证流程。
- 在 Phase 8 开始前确认对象存储厂商/区域/CDN、公司 CI 平台与签名信任根托管方式。

## 变更记录

- 2026-08-21：创建拆分式主 PRD，纳入管理查询、微信支付、密码重置 Worker、激活码独立加密表、制品链、生产运维和 namespace 级 JSON Schema；根据用户要求全部作为服务端专项且本轮不实施。
- 2026-08-22：根据确认方案纳入 React 管理端、自定义角色与固定权限点，将计划扩展为 10 个 Phase；Rust/Tauri 仍为非目标。
- 2026-08-22：采用分层双 PRD，商业生产层消费 P0 product 基线；加入产品席位、产品范围配置/制品、双重脱敏错误报告与最小反馈，将计划扩展为 11 个 Phase。
- 2026-08-22：完成 Phase 2 RBAC 服务端与 React 权限化管理端实现；Go 全量/Race/vet/build、管理端契约/类型/测试/构建和文档引用检查通过，真实 PostgreSQL 因 `TEST_POSTGRES_URL` 未设置、浏览器与桌面端按范围未验证。
