# douyin-desktop 云登录、授权与跨设备同步设计

日期：2026-09-05  
状态：已批准进入实施  
批准边界：autoLive 只修改 Go 服务端、PostgreSQL 迁移、服务端 OpenAPI 与相关服务端文档；douyin_spider 可修改 React/Tauri、本地 Python sidecar、测试和文档。

## 1. 目标

1. douyin-desktop 使用 autoLive 现有软件账号体系登录，固定产品为 `douyin_desktop`。
2. 登录会话复用现有 15 分钟 Access Token、30 天 Refresh Token、令牌轮换、重放吊销、产品成员关系和设备授权。
3. 同一软件账号在管理员授权的设备数量内可登录多台电脑，并同步桌面全局人设、回复策略、非秘密模型配置、知识库、知识规则和长期记忆。
4. 抖音账号登录、Cookie、浏览器人工验证和平台操作继续由每台电脑的本地 sidecar 与 `douyin_cli` 承担。
5. 断网、冲突或云端失败不得伪装为同步成功，不得静默覆盖本地未同步修改。

## 2. 非目标

- 不把 douyin_spider 的 88 个本地业务 API、任务执行器、WebSocket 或抖音能力迁入 Go。
- 不上传 Cookie、ticket、验证码、`credential_ref` 实值、Access/Refresh Token、AI API Key、Keychain 引用、完整本地 SQLite、LanceDB/FTS 索引、任务、评论、发送队列、模型正文或审计明细。
- 不同步抖音登录态；新电脑必须对需要使用的抖音账号重新人工登录。
- 不实现自助注册、短信/邮箱验证码、第三方 SSO、离线授权签名、实时推送、通用文件盘或通用 JSON 数据库。
- 不修改 autoLive 自有 Rust/C# 桌面端、React 管理端或部署服务器。

## 3. 总体架构

```text
React/Tauri
    │ 仅访问 127.0.0.1 + 本机进程令牌
    ▼
douyin_spider Python sidecar
    ├─ 本地抖音能力、任务、SQLite、FTS/LanceDB、Keychain
    └─ HTTPS → autoLive /api/v1/client/*
                    ├─ 软件账号登录/刷新/退出
                    ├─ 产品成员与设备授权
                    └─ user + product 作用域同步事实
```

Renderer 永远不直接持有 autoLive Token，也不放宽 Tauri CSP。远程请求只从 Python sidecar 发出；现有本地 API 和进程握手保持不变。

## 4. 登录与设备授权

### 4.1 软件账号

- 本地 UI 新增云账号门禁页，只收集用户名和密码。
- sidecar 调用 `POST /api/v1/client/auth/login`，请求固定 `product=douyin_desktop`。
- 密码只在本次请求内存中存在，不持久化、不记录日志。
- Access Token 仅保存在 sidecar 进程内存；Refresh Token 写入现有 OS Keyring，Renderer 和本地 SQLite 均不可读取。
- sidecar 启动时尝试用 Keyring 中的 Refresh Token 恢复会话；刷新失败后删除失效凭据并回到登录页。

### 4.2 设备身份和授权

- 每个安装生成一次随机 `device_id`，格式为 `dydesk_<32位小写十六进制>`，保存在 app-data 的非秘密配置中；不采集 MAC、硬盘序列号或其他硬件指纹。
- 登录成功后 sidecar 使用现有 `/api/v1/client/activate` 自动绑定设备。autoLive 当前授权是管理员预先绑定到软件账号的设备额度，不要求用户输入明文激活码。
- 未分配授权、激活码撤销或过期、设备数超限、用户/产品成员/设备被禁用时 fail-closed，并显示服务端稳定错误；Profile 与同步端点执行相同的激活码状态边界。
- 同一账号可在授权 `max_devices` 范围内绑定多台电脑；授权边界由 autoLive PostgreSQL 和服务端检查执行。
- 登录完成后读取 `/api/v1/client/profile`；只有 `product=douyin_desktop`、用户与设备均有效时进入产品。
- 本期要求在线授权。云端不可达时允许停留在登录/错误页，不自行延长权益，也不删除本地数据。

## 5. 同步数据边界

### 5.1 同步内容

同步项使用固定 allowlist：

- `persona_version`：桌面全局人设版本。
- `policy_version`：桌面全局回复策略版本；自动发送仍受本地和账号级安全门禁。
- `model_config`：检索模式、Base URL、模型名和 Embedding 维度；不含任何 Key 或 Keyring 引用。
- `knowledge_source`：来源类型、标题、状态和内容摘要。
- `knowledge_document`：文档版本、解析/分块版本、状态和元数据。
- `knowledge_chunk`：规范化文本、顺序和 locator；单项有界。
- `knowledge_rule`：规则条件、指导、状态和版本事实。
- `memory`：可复用场景、回复模式、标签、排除项、置信度、生命周期和 revision；只带脱敏来源引用，不复制本地反馈/评论完整链。

知识同步以规范化文本和结构化分块为事实，不上传原文件二进制。目标设备按同步事实重建本地可读副本和 FTS/LanceDB 派生索引。

语义标识固定为：模型配置 `global-model-config`，人设 `global-persona-v{version}`，策略 `global-policy-v{version}`，且版本号必须与 payload 一致。知识项必须保持来源→文档→分块/规则的引用完整；规则只能引用同一文档且 `enabled=true` 的分块，分块迁移文档或停用前必须先消除既有规则引用，删除按规则→分块→文档→来源执行。

### 5.2 不同步内容

- `accounts` 的可登录状态、昵称、远端 UID 哈希和 `douyin-cli-account:*` 引用。
- `account_subjects`、任务、视频、评论、点赞、发送、反馈、模型运行和审计事实。
- Keychain 秘密、模型 Key、Cookie、Refresh Token。
- FTS5、LanceDB、索引任务运行态和任何可重建向量。

## 6. 服务端同步协议

### 6.1 数据模型

新增三个服务端事实：

1. `client_sync_workspaces(product, user_id, current_revision, created_at, updated_at)`：每个用户产品空间的单调全局 revision。
2. `client_sync_items(product, user_id, kind, item_id, revision, payload_jsonb, deleted, updated_by_device_id, updated_at)`：当前同步项及墓碑；主键为 `(product,user_id,kind,item_id)`。
3. `client_sync_mutations(product, user_id, device_id, mutation_id, response_jsonb, created_at)`：写入重试幂等回执；主键包含产品、用户、设备和 mutation。

每次接受写入时在短事务内锁定 workspace、分配新 revision、更新 item、保存幂等回执。重复 mutation 返回同一回执；相同 mutation 不同载荷返回幂等冲突。

### 6.2 HTTP API

- `GET /api/v1/client/sync/items?after_revision=<n>&limit=<1..200>`
  - 返回 revision 大于 cursor 的当前项/墓碑，按 revision 升序。
  - 响应包含 `items`、`next_cursor`、`has_more` 和 `server_revision`。
- `POST /api/v1/client/sync/items`
  - 单批最多 100 项。
  - 每项包含 `mutation_id`、`kind`、`item_id`、`base_revision`、`deleted` 和 payload。
  - 新建要求 `base_revision=0`；更新/删除要求等于服务端当前 revision。
  - 冲突返回 `409 SYNC_CONFLICT`，不部分伪装成功。

所有同步请求必须是 `douyin_desktop` 桌面会话，且产品、产品成员、用户、设备和未过期激活绑定均有效，并从认证上下文取得 product/user/device；请求体不能替换作用域。

### 6.3 边界

- kind 必须来自固定 allowlist；`item_id`、数量、层级深度、字符串长度和 payload 字节数均有上限。
- 服务端拒绝秘密字段名、非 JSON 数值、未知字段和不符合 kind schema 的 payload。
- 服务端拒绝不符合固定语义标识、缺失父项、引用禁用分块、分块更新破坏既有规则、跨文档规则引用或删除后仍留下活动子项的变更。
- 同步读取只返回当前用户产品空间；管理员列表和其他产品不能旁路读取正文。
- 不新增 WebSocket；启动、手动同步和 sidecar 有界周期同步足够满足本期目标。

### 6.4 固定 payload 契约

服务端按 kind 校验以下精确字段；除 `metadata`、`locator` 和人设/策略的 `content` 外不接受额外字段。所有 ID 均是 1～128 字符的安全标识，时间使用 UTC RFC3339，哈希为 64 位小写十六进制：

- `persona_version`：`version`、`content`、`created_at`。
- `policy_version`：`version`、`score_threshold`、`minimum_confidence`、`daily_send_quota`、`automation_level`、`auto_send_enabled`、`content`、`created_at`。
- `model_config`：`retrieval_mode`、`chat_base_url`、`chat_model`、`embedding_base_url`、`embedding_model`、`embedding_dimensions`、`config_version`、`updated_at`；明确禁止两个 `*_api_key_ref` 字段。
- `knowledge_source`：`source_type`、`title`、`original_name`、`source_key`、`content_hash`、`status`、`created_at`、`updated_at`；不含本机 `stored_path`。
- `knowledge_document`：`source_id`、`version`、`content_hash`、`parser_version`、`chunker_version`、`status`、`metadata`、`created_at`；不含本机 `stored_path`、embedding 和索引运行态。
- `knowledge_chunk`：`document_id`、`ordinal`、`text`、`locator`、`chunker_version`、`enabled`、`created_at`。
- `knowledge_rule`：`document_id`、`condition_kind`、`condition_text`、`reply_guidance`、`literal_terms`、`record_ids`、`rule_fingerprint`、`status`、`created_at`、`updated_at`。
- `memory`：`reusable_situation`、`response_pattern`、`tags`、`exclusions`、`confidence`、`revision`、`expires_at`、`lifecycle_state`、`created_at`、`updated_at`、`cloud_source_ref`；不含本地账号 ID、反馈/回复/评论/任务 ID、命中统计和向量状态。

单项序列化后最多 256 KiB；`knowledge_chunk.text` 最多 64 KiB，其余字符串沿用本地领域上限且不得超过 4 KiB。`metadata`、`locator`、`content` 深度最多 8 层、总键数最多 200。服务端拒绝大小写及连接符归一化后包含 `cookie`、`password`、`private`、`api_key`、`token`、`ticket`、`credential_ref` 的任意嵌套键。

## 7. 本地同步算法

本地新增同步状态表，只记录 `(kind,item_id)` 的云 revision、上次同步内容哈希和结果；不复制 Token。

一次同步按固定顺序执行：

1. 校验云会话和设备授权。
2. 读取当前本地同步项并计算规范化 SHA-256。
3. 分页拉取 `after_revision` 之后的云端变化。
4. 若本地当前哈希仍等于上次同步哈希，应用云变化；若两端都修改同一项，记录冲突并保留本地内容，不静默覆盖。
   远端父项或被引用项墓碑遇到本地依赖时也形成显式冲突并阻止依赖上传；用户选择云端版本后按依赖逆序删除。
5. 扫描本地尚未同步的变化，以保存的云 revision 为 `base_revision` 分批推送。
6. 远端应用先写入 SQLite finalize journal；写入成功回执和 cursor 后按需重建知识/记忆派生索引，崩溃重启按依赖顺序重放。

首次同步规则：云空间为空时上传本地资产；云空间非空时先拉取并合并不同 ID。单例 `model_config` 以云端为准，但本机 Keychain Key 保留；相同 ID 的双向修改进入冲突。

同步由登录成功、启动恢复、用户手动点击和 sidecar 单一有界周期任务触发。并发触发合并为一次，不建立通用队列或多 worker。

## 8. 记忆来源兼容

当前本地记忆强制引用完整反馈链，而其他设备没有该链。为不上传评论/反馈正文，本地迁移增加明确的 `origin_kind=local_feedback|cloud_sync` 与可空本地来源外键，并保存 `cloud_source_ref`。约束要求两种来源二选一：本地记忆继续保留原外键完整性；云同步记忆只保存脱敏来源引用。检索和生命周期行为保持一致。

## 9. 错误、恢复和冲突

- 登录失败、授权失败、令牌失效、限流、超时、同步冲突和 schema 不兼容均映射为稳定本地错误，不回显远端正文。
- 401 时 sidecar 最多刷新一次并重放幂等读请求；写请求只在持有相同 mutation id 时重放。
- 429 尊重 `Retry-After`，周期任务有界退避；不无限重试。
- 同步失败不修改已确认的 cursor，不删除本地数据，不显示“已同步”。
- 冲突记录包含 kind、item id、本地/云 revision 和安全摘要；本期提供“保留本地并重试”与“采用云端”两种显式解决动作。

## 10. 验证与发布门禁

- Go：领域校验、PostgreSQL 仓储、幂等、CAS 冲突、产品/用户/设备隔离、HTTP 契约和真实迁移回放。
- Python：云客户端、Keyring 不泄露、刷新恢复、设备 ID、序列化、首次同步、增量同步、双写冲突、记忆云来源和索引重建。
- React：登录门禁、授权错误、同步状态、手动同步和显式冲突解决。
- 跨仓库：登录→设备授权→首机上传→第二台设备登录→拉取知识/记忆/配置→修改→回传的合成 E2E。
- 不执行真实抖音写操作，不把自动化测试当作平台授权或真实业务结果。
- 只运行受影响测试；数据库迁移和认证边界完成后扩大到相关 Go/Python/前端集成测试。桌面安装包只在本地目标 Windows 环境构建。

## 11. 消融边界

- 保留本地 sidecar，不在 Go 复制业务后端。
- 使用轮询和单调 revision，不新增消息队列、WebSocket、Redis、对象存储或通用工作流。
- 只同步明确 allowlist 的用户资产，不上传完整数据库或派生索引。
- 使用现有登录、Refresh、产品成员、设备授权、Keyring、SQLite 和索引重建能力。
