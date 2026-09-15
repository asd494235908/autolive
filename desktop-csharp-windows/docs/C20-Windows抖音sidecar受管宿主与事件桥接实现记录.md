# C20 Windows 抖音 sidecar 受管宿主与事件桥接实现记录

日期：2026-09-03

## 2026-09-10 登录安全诊断日志

新增用户要求：保留登录期间的本机日志用于定位错误。只接收严格白名单 `auth.diagnostic`，允许固定阶段、代码、异常类别，以及有界 HTTP/平台数字状态；未知或重复字段一律拒绝，禁止 URL、响应正文、异常正文、Token、Cookie、二维码、用户和房间标识。诊断不得推动聊天状态机，不保存 stderr 原文，不改变登录、重试或验证策略。

首次启动抖音时懒创建 `%LocalAppData%/GpAutoLive/logs/douyin/login.ndjson`，单文件上限 1 MiB，仅保留一个轮转备份。宿主只增加时间、随机运行标识、固定生命周期事件和登录清除原因。`DiagnosticLogPath` 只在实际成功写入后提供，`DiagnosticLogState` 分别说明尚未创建、已写入、写入失败。此功能使用现有 .NET 文件与 JSON API，不增加日志框架。

实现文件：`WindowsDouyinAuthDiagnostic.cs` 统一诊断枚举与边界，事件解析器只传递已校验诊断；Host 在业务桥接前消费诊断，拒绝的诊断只写固定 `diagnostic_rejected`，不累计聊天协议错误。`WindowsDouyinDiagnosticLog.cs` 负责固定文件、轮转和真实写入状态；每个运行最多保存 64 条 sidecar 诊断，额外一条达到上限标记。宿主状态记录按状态变化去重，不为弹幕或成功轮询刷日志；始终不保存 stderr 原文。目录及日志重解析点被拒绝。写入失败后不会后台重试，仅在下一次阶段、诊断或宿主状态变化时尝试；若此前已有日志，失败状态可以保留此前真实路径，不能将路径存在说成当前保存成功。

已执行验证：先观察诊断解析与日志创建目标失败，再实现并通过。`dotnet test tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj --artifacts-path E:/aotlve/desktop-csharp-windows/artifacts/douyin-host-log-tests --filter FullyQualifiedName~Douyin --verbosity quiet`，本轮 Windows 抖音目标 **81/81** 通过（无失败、无跳过）。覆盖未知/重复字段、错误类型和范围、1 MiB 规则的缩小容量轮转、诊断数量上限、无法创建文件、已存在日志被锁后失败并恢复、20 条恶意诊断不影响聊天连接，以及原始 stderr/诊断正文均不落盘。所有进程模拟测试注入临时日志目录，不向真实用户日志写假登录记录。源码、测试的定向格式检查和 `git diff --check` 通过。

正在进行消融实验。没有添加通用日志接口、后台队列、下载、网络依赖或重试机制；复用标准库序列化和文件追加，诊断字段只在一个合同定义。保留严格字段白名单、容量上限、路径保护、真实失败投影和业务隔离。此子任务未运行全仓测试、完整 App 构建、容器健康、页面点击、外部模型调用或真实抖音发送；Python 采集和 UI 由主线程分别整合验证，不能把离线管道通过写成真实登录已验收。

## 2026-09-10 登录保留与本地异常原因补齐

已确认的新边界：同一已监听直播间重复连接应幂等成功，保留现有配置、代际、登录和限频，不再次扫码或发送；换房仍先显式断开。普通房间错误继续保留有效登录。本地 IPC 超时先检查当前进程与已验证会话证据，不能把“进程还在”当成通信成功，也不能一概当作平台认证失效。没有足够证据安全保留会话时可以回收进程，但必须说明是本地通信异常。

最小合同为 `Authenticated` 加 `WindowsDouyinLoginClearReason`：本次尚未登录、无清除原因、明确认证过期、扫码失败/超时、显式停止、辅助进程退出、本地通信异常。枚举只解释清除原因，不创建第二套认证状态；脱敏过程信息继续使用已有进程 ID、退出码、状态与事件名。二维码 `auth.state=expired` 只代表扫码等待过期；只有平台 `auth_expired` 才标记已有认证失效。本轮不新增探活协议、凭据持久化或日志框架。

已实现同规范化房间的幂等连接。`live.close` 响应超时仅在已收到当前会话 `closed`、进程仍存活且运行令牌未取消时保留登录；普通房间错误回到已登录待连接。`live.open` 无响应必须核对当前绑定代数与已确认监听证据，无法确认时按本地通信异常清理。断开中的真实 `auth_expired` 优先于已观察到的 `closed`，后续取消和进程回收不得覆盖明确的认证失效原因。软件首次进入、尚未启动抖音时的空停止保留“本次尚未登录”。

本轮实际验证：`dotnet test tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj --artifacts-path E:/aotlve/desktop-csharp-windows/artifacts/douyin-host-tests --filter FullyQualifiedName~Douyin --verbosity quiet`，Windows 抖音目标 **64/64** 通过；新增离线场景包括断开事件存在/缺失的命令超时、二维码过期/失败/取消、断开时认证失效竞态、普通房间错误与重连 IPC 无响应。同房幂等先观察失败再修复。目标源码与测试的 `dotnet format whitespace --verify-no-changes`、`git diff --check` 通过。模拟房间错误最初使用了不在既有协议白名单内的测试码，修正为已有 `room_not_live` 后完成验证；未放宽生产白名单。

正在进行消融实验。本轮复用既有 `TimeProvider` 验证命令期限，未增加超时配置或等待重试；仅保留必要的清除原因枚举及单个断开事件标记，没有新队列、缓存、探活协议、日志体系或依赖。旧会话校验、进程存活、取消令牌、明确错误与未知结果边界全部保留。未删除无关旧代码。本子任务没有运行全仓测试或完整 App 构建，也没有执行容器健康、页面点击、外部模型调用或真实平台发送；这些结果不得由离线管道用例代替。

## 2026-09-10 单条手动发送入口

用户明确暂不开启自动发送。本轮在当前已认证且正在监听的直播间提供 `SendChatAsync(text, cancellationToken)`，不依赖回复池、不入自动回复队列、不自动重试。正文复用本地回复合同的 1～80 个 Unicode 字符/320 UTF-8 字节和可打印字符限制。手动与自动共享发送锁和限频预算；手动遇到正在发送或限频直接返回“未发送”，不后台排队。暂停状态需先恢复监听。

返回 `WindowsDouyinChatSendResult(Outcome, Message)`；平台 `accepted` 只表示接受提交，用户在既有弹幕列表核对本人行，不生成虚假回显，也不增加另一套回显等待系统。明确拒绝、未发送和结果未知分别报告；断开/停止或会话变化后不向旧房间补发。此处实施和本地模拟验证不授权发送真实平台消息。

已实现：`WindowsDouyinProbeHost.ManualSend.cs` 复用原发送请求、动作 ID、结果关联和终态处理；`DouyinLiveRules.TryNormalizeReply` 同时供回复池与手动正文使用，避免两套边界。自动发送在锁外等预算、取得发送锁后无等待地原子复查；预算被并发发送占用时释放锁重等，所以房间断开不会等待 60 秒限频窗口。手动发送从不等待预算或发送锁。

实际验证：仓库 `.tools/dotnet/dotnet.exe test` 使用 `--artifacts-path E:/aotlve/desktop-csharp-windows/artifacts/douyin-host-tests --filter FullyQualifiedName~Douyin --verbosity quiet`，Contracts **6/6**、Core **17/17**、Windows **56/56** 通过。新增 5 项离线管道用例覆盖未连接/非法正文拒绝、关闭自动回应且空回复池仍可手动发送、80 个四字节 emoji、并发与限频仅派发一条、仅真实管道入站才出现本人行、未知结果不重试、已派发后取消以及自动等预算时及时断开。该“入站”是本地模拟 sidecar，不是抖音平台实测。目标 `dotnet format whitespace --verify-no-changes` 和 `git diff --check` 通过。

正在进行消融实验。没有新增手动队列、重试、回显状态机、发送组件依赖或持久化；只抽出已有正文校验并复用发送入口。保留输入边界、同锁预算核对、写入前代际校验和未知结果语义。本子任务未运行全仓测试/完整构建、容器健康、页面点击、外部模型或真实业务发送；App 构建与真实验证由主线程分别记录。

## 2026-09-09 同次运行内保留抖音登录

本次用户要求：软件运行期间扫码一次，普通断开或重新打开直播间复用有效内存登录；退出软件、退出桌面账号、显式完整停止必须销毁凭据，不持久化抖音登录。实现中的 API 为 `DisconnectAsync`（仅关闭房间）、`StartAsync`（已认证宿主直接重开房间）和 `StopAsync`/`DisposeAsync`（完整退出）。断开后宿主为 `Ready`，Core 为 `LoggedIn`，快照 `Authenticated=true`；真实凭据仍只在受管 Python 进程内。

房间代际、显示记录和旧回复任务在重新打开时重置，同账号限频记录和风控阻断在本次登录内保留，断开重连不能绕过限频。扫码、命令、首次连接和重连仍有界；只有明确认证失效或受管进程/通信故障才丢弃登录并重新扫码。本地模拟测试不等于真实平台登录/发送成功。

已实现：`WindowsDouyinProbeHost.LoginSession.cs` 集中拥有本次运行内登录的房间开关边界；宿主快照只携带认证布尔值，不含凭据。自然 `closed/room_ended` 同样保留登录；`auth_expired` 清除认证并回收旧 sidecar，下次连接重新扫码。`DisconnectAsync` 先取消旧任务发送资格，等待已在途发送收口后执行 `live.close`；完整停止从进入停止状态开始就不再派发新任务。退役房间的合法迟到事件被丢弃，不进入新窗口或回复队列。

本轮实际验证命令：使用仓库 `.tools/dotnet/dotnet.exe test` 对 `tests/GpAutoLive.Core.Tests/GpAutoLive.Core.Tests.csproj` 和 `tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj` 分别运行 `--artifacts-path E:/aotlve/desktop-csharp-windows/artifacts/douyin-host-tests --filter FullyQualifiedName~Douyin --verbosity quiet`，Core **17/17**、Windows **51/51** 通过。新增模拟管道验证同进程扫码后断开/换房、40 条旧房间迟到事件、重复断开幂等、自然关闭保留登录、Dispose 回收进程、重连超时与旧 timer 回调隔离、明确认证失效后新进程重新扫码。`dotnet format whitespace` 仅处理本次宿主与测试文件，目标 `git diff --check` 通过。

正在进行消融实验。本轮只复用已有 `LoggedIn` 状态、canonical 命令、发送锁、进程监控和单个启动计时器；未增加账号存储、持久化凭据、新协议、自动重连框架或额外重试。删除重复的开房状态补写，避免后到 continuation 复活已经断开的房间。保留代际、限频、风控和有界退出，因为它们防止串房、误发和残留凭据。此子任务未运行全仓测试/构建、容器健康、页面点击或真实平台发送；本地模拟结果不得代替主线程的真实扫码与业务验收。

> 2026-09-09 后续实施覆盖：新增 C# 专属持续 sidecar 与本机实时弹幕窗口，正文、昵称和时间允许进入 C# 内存显示 DTO，旧条目中“只传元数据/正文不进入 C#”仅描述当时实现，不再限制本次明确需求。状态与日志继续只含脱敏元数据；观看和自动回复分离。当前实施与验证状态见 [实时弹幕窗口实施方案](./2026-09-09-抖音实时弹幕窗口实施方案.md)。

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

## 直播间输入说明（2026-09-09）

抖音卡片移除写死的只读“弹幕服务器” `wss` 地址，将实际输入框标为“直播间链接 / 房间号”，显示标准链接示例和分享短链接不支持提示。仍只接受 `https://live.douyin.com/` 加 1～20 位数字，或直接输入 1～20 位数字；复用 `DouyinRoomTextBox` 和既有规范化校验，不新增短链接解析、网络请求或连接配置，真实平台验收状态不变。

本次验证：PowerShell `[xml]` 解析、输入框唯一性/可编辑性/长度限制、链接提示和停止按钮事件静态断言通过；`git diff --check` 通过。于 C# 目录执行 `.\.tools\dotnet\dotnet.exe build .\src\GpAutoLive.App\GpAutoLive.App.csproj -c Release -p:Platform=x64 --no-restore --verbosity minimal`，0 警告、0 错误；通过开发启动脚本重新打开后窗口响应正常。本次为局部界面说明与布局调整，未运行产品自动化测试、全仓构建、页面点击、容器检查或抖音真实登录/发送验证；不涉及外部模型调用。最小化检查仅删除多余服务器展示，保留既有校验、事件和会话安全边界，无新增抽象或依赖。

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
