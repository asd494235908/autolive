# C20 Windows 抖音 sidecar 受管宿主与事件桥接实现记录

日期：2026-09-03

## 本轮落地

- C# 宿主已接入 canonical M1 认证/开房/单条回复生命周期：在显式 `AUTOLIVE_DOUYIN_PROTOCOL=canonical` 下，通过受管 stdin/stdout 发送 `auth.qr.start`，等待脱敏 `auth.state=confirmed`，再按当前配置房间和会话 generation 发送 `live.open`；成功响应绑定 `(session_id,generation)`，并在进入 `live.state=connected` 前推进 Core 到 `RoomResolved`，避免响应/事件异步竞态。随后 `live.chat → chat.send → accepted` 的本地 fake 管道也已打通，发送结果会发布 `SnapshotChanged` 供 WPF 更新统计；该测试不代表真实抖音登录或平台连接。
- C# canonical `live.gap` 已接入：parser 严格校验当前会话代际、原因白名单 `sidecar_backpressure/reconnect/no_replay`、正数 `dropped_count` 和未知字段；Core 累计缺口事件/丢弃数量并保留最近原因，bridge 将其投影为可见的非终态状态，WPF 监听文案显示累计缺口。该事件不重放缺失弹幕、不触发自动重试，Rust 当前旧探针尚未发出该 canonical 事件。
- canonical 停止路径已补齐：停止 canonical 会话时，若仍在扫码则先有界发送 `auth.cancel`，随后 `shutdown`；若已确认登录则在有直播会话时先发送 `live.close`，再发送 `auth.logout` 清除 sidecar 内存身份，最后 `shutdown`。每条命令最多等待 750ms，超时继续走原有关闭 stdin、Job Object/进程树强制回收。legacy 仍直接走原有受管回收路径。
- C# parser/host 已接入 canonical `auth.qr`：严格校验 NDJSON v1 包络、Base64 字符、PNG 签名、256 KiB 解码上限和过期时间，宿主用 `CreateNew + Flush(true)` 写入当前会话的随机临时 PNG，再由 WPF `OnLoad + Freeze` 展示；解析失败、过期或写入失败均不投影二维码。该路径不承载凭据、不覆盖已有文件，也不把图片生成当作扫码确认。
- C# 新增 `live.open` NDJSON v1 codec 与宿主调用：请求校验 `request_id/web_rid/generation`，响应按相同 `request_id` 绑定并只返回 `session_id/title/live_status` 脱敏字段；未知字段、非法状态、超限行和不稳定错误码 fail-closed。宿主只在 canonical 协议模式调用，legacy 模式保持兼容；本地 fake 管道已证明调用顺序，不等于真实直播间连接。
- WPF M1 面板已接入宿主 `QrPath` 的本地展示：新增 `DouyinQrImageLoader`，在路径规范化、普通 PNG、非 Reparse 和 16 MiB 上限通过后用 `BitmapCacheOption.OnLoad` 解码并冻结图片，停止/新会话清除 Source；错误不回显具体路径。canonical `auth.qr` base64 已先由 parser/host 验证并原子写入临时 PNG；该展示仍不把文件存在视为扫码成功。
- 代码总监复核后收紧发送边界：任务 action ID 改为 Core 入队时生成；出队与写 stdin 前复核任务/会话代际和监听态；stdin 使用带取消令牌的写入/刷新 API；合法迟到 response 被丢弃，不计入无效事件；response 根对象和 result/error 嵌套对象都拒绝未知字段。收到 `auth_expired/rate_limited/risk_controlled` 后 Core 标记 `ReplySendingBlocked`，宿主记录至少 60 秒冷却并保持本轮不自动恢复。
- C# 新增 `WindowsDouyinSidecarProtocol`：按 Rust 冻结的 NDJSON v1 合同序列化 `chat.send` 请求并解析脱敏 response，固定 64 KiB 单行上限、会话代际、正文 100 Unicode/400 UTF-8 字节、稳定错误码和 `accepted/not_sent/rejected/unknown` 终态；不保留 sidecar 错误正文。
- `WindowsDouyinProbeHost` 已把本地回复队列接到受管 stdin/stdout：单请求在途、按 `request_id` 和可选 `client_action_id` 关联、至少 3 秒一条且每分钟最多 5 条；写入/响应超时在派发后归为 `unknown`，不自动重试，停止/释放先关闭 stdin 并取消在途响应。该路径只有 canonical `(session_id,generation)` 建立且 Core 处于监听态时才消费队列，不把本地序列化、出队或进程存活当成平台发送成功。
- `WindowsDouyinProbeHost` 现在优先以 canonical `live.open` 成功响应锁定 `(session_id,generation)`；兼容没有开房响应的 legacy/旧事件路径时，仍可由首条 canonical `live.chat/live.state` 建立身份。后续解析传入同一预期身份；不匹配的迟到/跨代事件不会送入 Core，停止、释放和启动新会话时清空身份。
- C# parser 新增 Rust 冻结的 `live.state` NDJSON v1 消费：校验 `session_id`、正数 `generation`、payload 状态白名单，并保留状态身份供后续代际门禁；bridge 只把 `connected` 和房间结束/鉴权失效/风控/失败投影到现有 Core，`connecting/reconnecting/closed` 不变造“已连接”或“已发送”结果，缺会话代际 fail-closed。
- C# 解析器现在兼容 Rust 冻结的 `live.chat` NDJSON v1 包络：校验版本、事件类型、ASCII `session_id`、正数 `generation` 和 payload 中的 `msg_id/received_at_unix_ms/author_id/nickname/content`；`content` 只计算长度，不写入 C# 事件 DTO，顶层正文键和非法时间直接拒绝。
- `WindowsDouyinProbeHost` 把本轮启动配置的房间号传给 parser，canonical `live.chat` 必须落在当前房间；旧式 `chat_received` 继续要求显式脱敏房间/消息元数据，便于兼容已有本地探针夹具。
- `chat_received` 不再只投影一个布尔标志：解析器现在要求 `WebcastChatMessage`、规范化房间号、消息 ID、发送者 ID、文本长度和自回显/重放标志；正文键 `text/content/body` 直接拒绝，未知附加字段不进入 C# 事件模型。队列 TTL 使用 C# 收到事件的本地时间，不信任 sidecar 时钟。
- `WindowsDouyinProbeEventBridge` 将脱敏元数据送入 `DouyinLiveManager.ObserveChatMetadata`，实际触发单房间校验、消息去重、自回显/重放过滤、随机选句、有界队列和 60 秒过期任务；桥接缺元数据或本地校验失败时 fail-closed，宿主不继续假报运行。
- 新增 `GpAutoLive.Windows/WindowsDouyinProbeEventParser.cs`：只接受参考探针的固定 JSON 事件白名单；canonical NDJSON 单行限制与 Rust 冻结合同一致为 64 KiB，未知字段不会被返回，事件正文、Cookie、Token 和异常文本不进入 C# 状态。
- 新增 `GpAutoLive.Windows/WindowsDouyinProbeHost.cs`：消费 C19 生成的 `ExternalProcessPlan`，使用 `conda run --no-capture-output`、隐藏窗口、重定向双流、有限 stdout/stderr、取消/超时和 `WindowsJobObject` 优先的进程树清理。
- `ReadStdoutAsync` 使用固定字符缓冲逐行组装，在收到换行前同样执行 64 KiB 单行上限，避免 `StreamReader.ReadLineAsync` 对恶意无换行输出产生无界暂存。
- sidecar 事件按固定顺序投影到 `DouyinLiveManager`：扫码、登录、房间解析、公屏连接、脱敏外部弹幕消费、回复尝试、自回显过滤、通过/证据不足/失败；弹幕正文仍不穿过 C# 事件模型。
- WPF 现通过“启动/停止 M1”入口读取显式环境变量：`AUTOLIVE_DOUYIN_ROOT`、`AUTOLIVE_DOUYIN_PROBE`、`CONDA_EXE`（可选 `AUTOLIVE_CONDA_ENV`/`AUTOLIVE_DOUYIN_TIMEOUT_SEC`）；三项路径未全部提供时保持原有纯本地合同模式，不访问网络。
- `DouyinLiveState` 增加 `Inconclusive`，明确区分“未获得完整证据”和成功；WPF 状态文案与配置编辑门禁同步覆盖该终态。

## 生命周期边界

- 计划校验失败不会启动进程，也不会改变核心会话。
- 进程自然退出但没有 `probe_passed` 时标记 `Inconclusive`；`probe_failed`、`reply_failed`、超时、输出越界和读取失败均 fail-closed。
- 主动停止、窗口关闭和宿主释放都会取消读取、终止进程树、有限等待并清除内存队列/去重状态；停止超时不会伪装成成功。
- QR 路径只有在事件路径由已验证启动计划提供、文件存在且不是 ReparsePoint 时才投影到快照；停止时清除投影，不删除用户未明确交给宿主的文件。

## 明确未完成

本轮没有在真实 Conda 环境、真实 `Douyin_Spider`、抖音网络、二维码登录、WebSocket、平台发送、自回显或账号/许可证门禁上宣称通过。sidecar 仍需目标 Windows 设备的真实兼容验收；`desktop/` Rust/Tauri 文件未修改。

当前仓库 Rust `douyin_live.rs` 与 `douyin_live_compat_probe.py` 仍是命令行驱动的一次性兼容探针：Rust 启动时 stdin 为 null，Python 在收到外部弹幕后自行调用发送 API；它们不是本轮 C# canonical NDJSON sidecar。C# 的 stdin/stdout 发送代码只有接入支持 `live.open/live.state/live.chat/chat.send` 的正式 sidecar 后才能进入真实平台闭环，不能将旧探针输出当作该闭环证据。

`chat.send` 的 C# 编解码、stdin/stdout 受管关联和保守限频已接入 canonical 宿主生命周期；但还没有正式 sidecar response、平台业务状态码、自回显证据或真实人工恢复证据，因此当前只能标记为“代码已接入·待验收”。

## 验证

- `dotnet build GpAutoLive.Windows.slnx -c Release --no-restore`：0 警告、0 错误。
- `dotnet test GpAutoLive.Windows.slnx -c Release --no-build --no-restore --logger "console;verbosity=minimal"`：Contracts 25、Core 75、Media 91、Windows 126、App 33，合计 **350 项通过**。
- 新增测试覆盖事件白名单/长度边界、事件桥接顺序与终态、核心 sidecar 终态、无效计划 fail-closed 和立即退出进程的非成功投影。
- 新增 canonical 宿主管道测试：本地 PowerShell fake sidecar 通过 `auth.qr.start → auth.qr → auth.state=confirmed → live.open → live.state=connected` 顺序，验证响应 request ID、session/generation 绑定和 `RoomResolved → Listening` 状态推进；该测试不宣称真实平台业务成功。
- 新增 canonical 单条回复管道测试：fake sidecar 在监听后发送 `live.gap` 和一条脱敏 `live.chat`，C# 完成缺口统计、队列入队、出队、`chat.send` request ID/动作 ID 关联和 `accepted` 统计投影；仍不宣称真实平台接受或自回显。
- 本轮实际复验：Windows 抖音相关测试 `41/41` 通过；`auth.logout` canonical 序列化定向测试 `1/1`、Core 抖音状态测试 `13/13`、App 协议工厂定向测试 `4/4` 通过；Release 构建 `0` 警告、`0` 错误；格式检查、作用域检查和 7 份共享 fixture 校验通过。
- `dotnet format GpAutoLive.Windows.slnx --verify-no-changes --no-restore`：通过；`tools/verify-scope.ps1`：通过，`desktop/` 跟踪文件未改变。

## v63 发布复验

- 正式候选：`artifacts/csharp-windows-controller-20260903-v63`，根目录 8 个运行文件、1,270,503 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、432,779 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，其中 5 个二进制资源通过硬链接复用。
- 资源清单 5/5 大小与 SHA-256 匹配；使用 `.tools/dotnet` 启动/关闭成功，标题为 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，未发现 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。
- 5 秒空闲基线 6 个样本：私有工作集峰值 85,643,264 bytes、工作集峰值 148,094,976 bytes、CPU 峰值 1.54%。该数据仅是同机空闲对照，不等价于真实 sidecar、网络或 30 分钟长稳门禁。

## v64 安全边界修正复验

- 发布候选：`artifacts/csharp-windows-controller-20260903-v64`，根目录 8 个运行文件、1,270,503 bytes；独立符号包 7 个文件、432,951 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，其中 5 个二进制资源通过硬链接复用。
- 资源清单 5/5 大小与 SHA-256 匹配；启动/关闭冒烟标题为 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。
- 5 秒空闲基线 6 个样本：私有工作集峰值 85,688,320 bytes、工作集峰值 148,262,912 bytes、CPU 峰值 2.32%。该数据仅是同机空闲对照，不等价于真实 sidecar、网络或 30 分钟长稳门禁。
