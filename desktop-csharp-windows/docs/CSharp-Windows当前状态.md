# C# Windows 桌面端当前状态

> 2026-09-07 媒体池筛选与视频模式切换收口：媒体池新增真正生效的“全部/视频/音频”筛选，并与文件名搜索组合；清空搜索或切换筛选会恢复当前源的可见选中项，拖放候选的扩展名校验与文件导入保持一致，避免“看得到但不能操作”或不支持文件进入候选池。视频处理模式在 `Original/CPU4/GPU83` 之间变化时不再只通过 IPC 回读参数冒充 shader 已生效，而是从当前 mpv 位置有界重启单一会话，并恢复声音、暂停状态及 GPU83→CPU4→Original 的既有单向回退；同模式参数仍使用原子 IPC 更新。相关 App/Windows 定向测试、格式检查和本地 x64 Release 构建通过；真实目标 GPU、PortAudio、RTMP 和人工连续页面验收仍待完成。

> 2026-09-07 媒体添加语义修复：顶栏、媒体池按钮和 `Ctrl+O` 选择的文件现在统一按所选顺序追加到当前播放池，不再错误提交“替换全部”而使已有条目从列表消失；系统文件拖放继续使用同一追加语义。按钮统一显示“添加媒体”，并明确提示不会删除已有条目或本地源文件。版本化 JSON 媒体列表仍保留整池替换语义。生产文件删除调用审计确认导入、移除和清空仅修改进程内播放池引用，用户源文件不进入删除路径；仅应用生成的临时缩略图、原子配置临时文件等受管临时产物允许清理。App 入口/UI 定向测试 `2/2`、Core 媒体池测试 `17/17` 通过；本地 x64 Release 构建 `0` 警告、`0` 错误并已重启开发端。未执行全量测试与人工文件选择点击。

> 2026-09-07 插话音量与主音频恢复修复：插话高级区新增 `0%–100%` 音量滑块，对应既有合同的 `-60–0 dB`，拖动实时进入 overlay 混音策略，鼠标释放或键盘调整后复用既有原子配置存储；主媒体音量与 duck 深度保持独立。真实复测证明上轮仅把 duck 释放移出 UI continuation 仍不足，进一步定位到 RTMP 已接入时普通声音定时 N→N+1 切换的互等：本机/RTMP 主轨已晋级，但 RTMP overlay 等待旧总线关闭，旧总线又等待 overlay 晋级，5 秒后 `candidate_activation_timeout` 会终止整个主音频会话。当前两个 overlay 分支都跟随已确认主轨晋级，有真实消费者的 RTMP 主轨仍保留自然确认。主线程集成定向测试 App `13/13`、Core `15/15`、Media `19/19`、Windows 音频/RTMP `23/23` 及格式检查通过；x64 Release 本地构建 `0` 警告、`0` 错误并已重新启动开发端。真实 RTMP + PortAudio + 插话 + 声音周期联合听感、人工页面、重启持久化和长稳仍待验收。

> 2026-09-07 当前画面处理路径显示：主窗口视频参数区、播放状态区和底部视频处理开关现在统一显示当前受管 mpv 会话效果快照中的真实模式：`GPU·GPU83`、`CPU·CPU4`、`未处理·Original`；没有活动视频会话时显示“等待视频”。显示不再根据视频处理开关或资源能力预判，因此 GPU83 启动失败并单向回退 CPU4/Original 后会呈现实际会话模式。未新增硬件探测、线程、缓存或第二状态源。主线程集成复核中 App 状态/UI 定向测试 `29/29`、Windows 控制器测试 `6/6`、格式检查通过，x64 Release 本地构建 `0` 警告、`0` 错误并已重新启动桌面端；真实 GPU83/CPU4 启动后的人工页面观察和目标显卡矩阵仍待验收。

> 2026-09-07 插话收尾与下一轮进度第一层收口：插话 duck 优先级曾由 WPF UI completion continuation 释放，UI 投影延迟时 `MediaDucked` 会继续保持；当前释放已直接绑定底层解码 completion，并由后台 `finally` 覆盖成功、失败和取消，UI 只显示终态，不拥有主音频恢复时机。该接线通过自动化但没有单独解决真实主音频终止，最终根因和修复见本文顶部的 RTMP overlay 切换互等记录。下一轮进度不再把音频宿主短暂不可用误判为配置失效：有效播放、插话配置和候选池仍存在时继续投影真实调度窗口，到 100% 后等待宿主恢复再触发；真正停止、关闭/清空配置或候选失效才清零。集成聚焦筛选 App `24/24`、Core `21/21` 通过，x64 Release 本地构建为 `0` 警告、`0` 错误并已启动新进程；未执行全量测试，真实 PortAudio/RTMP 听感、人工页面连续进度、设备异常和 30 分钟长稳仍待验收。

> 2026-09-07 自动视频周期低感知范围修复：`VideoEffectCyclePlanner` 的职责仍只限于按真实播放位置决定下一次切换时机；持续明显偏色的根因是 `GeneratedVideoEffectSnapshot` 曾把亮度 `-6～+6%`、对比度 `96～104%`、饱和度 `96～106%`、色相 `-4～+4°` 和锐化 `0～6%` 直接用于生产自动周期，偏离《媒体参数范围与默认值》的低感知档。当前生产生成范围已收窄为亮度 `±0.1～0.35%`、对比度/饱和度相对 `100%` 偏移 `±0.1～0.3%`、色相 `±0.05～0.2°`、锐化 `0.1～0.4%`，GPU83 与 CPU4 均消费同一收窄快照；“明显可见”范围只保留给固定测试素材验证映射链真实改变画面，不进入生产随机周期。失败先行测试修复前 `1/1` 失败，修复后相关测试 `3/3`、`ShellStateTests` `18/18` 及格式检查通过；真实视频主观低感知观察、目标 GPU 和长稳仍待验收。

> 2026-09-07 插话候选启动恢复：桌面端加载到 `Enabled=true` 且目录非空的已保存插话配置后，复用现有 `InterludeFilePoolService.ScanDirectory` 在后台执行一次有界、可取消的安全扫描，只恢复候选快照而不自动播放。目录不存在、无权限或超过目录上限时保持持久化配置及已有候选快照不变，并在插话区域及全局状态显示明确错误；关闭或未配置状态不触发扫描。App 定向测试 `3/3` 通过；真实受限目录、人工重启页面观察和真实插话播放仍待验收。

> 2026-09-07 声音周期进度停滞修复：声音变换进度必须在活动音频会话期间持续读取最终 PCM 的真实可听位置，并相对 `AudioEffectCyclePlanner` 当前周期窗口投影；带音轨视频在初播、声音恢复、手动切换和自然续播成功建立最终 PCM 后，均启动同一个声音完成观察器。不能只在参数重新生成时更新，也不能由 UI 动画或独立计时器猜测。暂停、停止、换源、关闭声音处理或会话失效时清零。使用用户当前视频与真实 FFmpeg/PortAudio 的 WPF 定向夹具已确认进度从 0 开始推进；人工页面连续观察、主观听感和长稳仍待验收。

> 2026-09-07 插话频谱显示修复：插话文件实际播放期间，视频/主音频频谱只在诊断 UI 中归零并隐藏，插话频谱继续显示实际 overlay PCM；插话结束、停止或失败收尾后恢复主频谱显示。该显示门控不写入 `FinalPcmBus`、不消费或改写 PCM，也不改变既有基础轨 duck/静音优先级和插话混音策略。当前未完成真实声卡听感、人工页面切换和长稳验收，不把自动化投影检查记作实机通过。

> 2026-09-07 视频/声音/插话变换进度投影：WPF `ShellState` 新增三条 `0–100` 周期进度绑定属性。视频进度只使用现有 mpv/最终 PCM 播放观察时钟与 `VideoEffectCyclePlanner` 真实周期窗口；声音进度只使用最终 PCM 可听时钟与 `AudioEffectCyclePlanner` 候选切换窗口；插话进度直接消费 `InterludeSchedulePlanner.ProgressPercent`。未新增定时器、动画或调度事实源；停止、失效、关闭处理或换代时清零，不保留旧周期假进度。App 聚焦测试 `25/25` 通过；未执行人工页面点击或真实媒体进度观察。

> 2026-09-06 桌面认证接管：C# 已成为后续版本唯一桌面认证事实源。C# 自有设备 ID 不被 Rust 覆盖；仅在 C# ID 缺失时读取固定旧目标 `device-id.autolive.desktop`，按 Windows keyring 的 UTF-16LE 密码编码解码，合法则复制、损坏则停止自动迁移以避免创建第二设备。密码、Rust Refresh Token 和 Rust pending logout 均不迁移。C# 远端 Logout 失败会把最多 8 条待撤销 Token 隔离保存并在启动恢复前重试；激活成功必须包含未来到期时间，心跳周期会本地收回到期授权，并把设备禁用/撤销结果投影到 WPF 门禁且清理当前会话凭据。认证定向 Release 自动化 `47/47`、格式检查和 App Release x64 构建通过；真实 Credential Manager 迁移、控制面 TLS、管理员禁用和跨到期时间实机仍待验收。

更新时间：2026-09-07

> 2026-09-06 WPF 音频诊断与周期输入接入：取消原“实时预览”占位模块，上方区域改为固定布局的最终 PCM 音谱卡片，分别显示主音频与插话的 16 段实时频谱；参数区内部 `ScrollViewer` 滚动不再改变上方卡片的可见性或行高。视频处理周期、普通声音处理周期和插话触发间隔均支持用户输入秒数并做 `1–60s`（插话沿用 `0.5–60s`）范围校验，配置以毫秒写入本地低敏偏好，调度器下一周期按新范围生效。新增周期配置、PCM 双音谱和滚动布局回归测试；C# Release 构建、相关测试和格式检查通过，未执行人工页面点击，Rust 仅作只读参考且未启动。

> 2026-09-06 WPF 音谱卡紧凑化：根据实际页面截图将上方音频诊断区域固定为 `216px`，标题/底部状态行收窄，两个音谱面板和柱状图高度同步压缩，避免诊断卡占用过多工作区；参数区仍独立滚动，音谱数据与周期输入逻辑不变。新增固定高度回归断言，Rust 保持只读且未启动。

> 2026-09-06 WPF 高级控制区样式修正：高级控制、插话文件池、RTMP、抖音回复池四个 `Expander` 已统一绑定 `DarkExpanderStyle`、深色标题模板和 `TextBrush`，修复截图中标题落为黑色导致的低对比度；插话池/M1 状态文案改用 `MutedTextBrush`。实时按钮启停和状态投影逻辑未改；新增 `Advanced_expander_headers_use_the_dark_theme_text_and_header_template` 回归测试并通过。该次只验证 WPF 控件属性与本地构建，未执行人工页面点击，Rust 保持只读且未启动。

> 2026-09-06 客户端隔离边界修正：Rust 与 C# 是两个独立客户端，Rust 仅用于阅读实现方式。C# 不启动、不探测、不调用 Rust，也不使用与 Rust 共享的媒体/输出锁；本端只验证 C# 自身的单实例、媒体输出资源、安装签名和回滚。历史记录中“C#↔Rust 交叉启动/跨客户端争用”不再属于当前 C# 验收条件。

> 2026-09-06 canonical `live.chat/live.state` 未知字段门禁：C# parser 对事件包络和 payload 统一执行固定字段白名单，未知字段直接拒绝，避免 sidecar 夹带未审计正文或状态扩展；Windows parser 定向测试 `17/17` 通过。该修改只收紧 C# 输入边界，不启动或修改 Rust。

> 2026-09-06 canonical M1 认证/开房/单条回复/停止生命周期接线：在显式 `AUTOLIVE_DOUYIN_PROTOCOL=canonical` 下，C# 宿主通过受管 stdin/stdout 依次发送 `auth.qr.start`、等待 `auth.state=confirmed`、发送 `live.open` 并绑定脱敏 `session_id/generation`，成功响应先推进 `RoomResolved`，再消费 `live.state=connected` 进入监听；监听后 fake `live.chat → chat.send → accepted` 已验证队列、动作 ID 和统计 `SnapshotChanged` 投影，停止时已确认会话优先发送 `live.close → auth.logout → shutdown`，扫码未确认会话发送 `auth.cancel → shutdown`，消除响应与事件异步竞态。Windows 抖音相关测试和停止协议定向测试已通过。legacy 模式保持兼容；真实 Conda sidecar、抖音扫码、WebSocket、平台发送、自回显和风险恢复仍待验收，Rust 保持只读。

> 2026-09-06 canonical `live.gap` 接线：C# parser/bridge/Core 已校验并记录 `sidecar_backpressure/reconnect/no_replay` 缺口及正数 `dropped_count`，状态快照保留最近原因和累计丢弃数，WPF 监听状态会显示缺口计数；缺口不伪造补发、不自动重试。Rust 当前旧探针尚未输出该事件，真实 sidecar 的背压/重连/不可重放证据仍待验收。

> 2026-09-06 RTMP 自动重连线程边界修复：`MainWindow.ReconnectRtmpCoreAsync` 的每次停止、启动和发布就绪等待现在统一经窗口 `Dispatcher` 执行；后台退避 continuation 不再直接触碰 WPF 控件或 UI 状态。新增后台入口回归测试，App 定向 `MainWindowRtmpReadinessTests` `3/3` 通过；该修复不等于远端 ZLMediaKit/RTMPS 握手、断线信号或长稳已验收，Rust 保持只读。

> 2026-09-06 canonical `auth.qr` 事件消费：C# parser 现在解码 `v=1/type=event/event=auth.qr` 的 `png_base64`，限制 Base64 字符、PNG 签名、256 KiB 解码上限、过期时间和未知字段；宿主只把未过期合法图片写入本轮随机临时文件，复用 WPF 二维码面板，旧 CLI 文件路径不受影响。该路径不返回或持久化凭据，不把二维码文件存在当作扫码成功；parser 定向测试 `11/11` 通过，真实 sidecar/登录仍待验收，Rust 保持只读。

> 2026-09-06 `live.open` 合同与宿主生命周期接入：C# 新增 Rust 冻结 NDJSON v1 的请求/响应编解码，并由 canonical 宿主实际发送/绑定；按 `request_id` 校验响应，限制 `web_rid/generation/session_id/title/live_status`、64 KiB 行上限、稳定错误码和未知字段，失败正文与 canonical room_id 不进入模型。真实扫码、正式 sidecar、WebSocket 和平台连接仍待验收，Rust 保持只读；Windows 协议与宿主定向测试已通过。

> 2026-09-06 M1 二维码展示接线：WPF 读取宿主已验证的临时 PNG 路径，`DouyinQrImageLoader` 在 16 MiB、PNG 扩展名、普通文件和非 Reparse 门禁后以 `OnLoad + Freeze` 解码，扫码面板只在加载成功时显示，停止/新会话清空图片；canonical `auth.qr` base64 已由 parser/host 转为本地临时 PNG，二维码文件存在仍不等于扫码确认，真实 sidecar、平台登录和 WebSocket 仍待验收，Rust 保持只读。二维码定向测试通过 `6/6`，符号链接用例因当前环境权限跳过。

> 2026-09-06 Rust NDJSON `live.chat` 对齐：C# parser 已支持 `v=1/type=event/session_id/generation/payload` 包络，并将 `msg_id/author_id/nickname/content` 收敛为脱敏元数据；宿主注入当前配置房间，非法包络、顶层正文和超界字段 fail-closed。平台真实 sidecar、`chat.send`、扫码和 WebSocket 仍待验收，Rust 保持只读。

> 2026-09-06 M1 发送门禁复核收口：回复任务在 Core 入队时生成 `client_action_id`；宿主在发送前复核任务/会话代际，stdin 写入/刷新可取消，合法迟到 response 不累计坏事件，response 根/result/error 均拒绝未知字段；`auth_expired/rate_limited/risk_controlled` 会阻断本轮新发送并保留至少 60 秒冷却，不自动恢复。canonical `live.open` 宿主绑定已通过本地管道测试，但正式 sidecar、平台发送和自回显仍待验收，Rust 保持只读。

> 2026-09-06 canonical `chat.send` 宿主边界：C# 新增 Rust 冻结 NDJSON v1 请求/响应编解码，校验 64 KiB 单行、代际、正文 100 Unicode/400 UTF-8 字节和稳定错误码；`WindowsDouyinProbeHost` 通过受管 stdin/stdout 做单请求在途关联，固定至少 3 秒一条、每分钟最多 5 条，派发后超时/断连记录 `unknown` 且不自动重试，停止/释放先关闭 stdin。该代码不把序列化成功、队列出队、本地 fake response 或本机进程存活计为平台接受；正式 sidecar、扫码、WebSocket、自回显和远端权限仍待验收，Rust 保持只读。

> 2026-09-06 Rust NDJSON `live.state` 对齐：C# parser 已校验会话 ID、正数 generation、payload 状态白名单，并由 bridge 处理 `connected`、房间结束、鉴权失效、风控和失败；`connecting/reconnecting/closed` 不会被误报为已发送成功，缺会话代际直接 fail-closed。`chat.send`、真实扫码、WebSocket 和跨代迟到事件丢弃仍待实现/验收，Rust 保持只读。

> 2026-09-06 canonical 事件代际门禁：`WindowsDouyinProbeHost` 现在以首条 `live.chat/live.state` 的 `(session_id,generation)` 建立本轮身份，后续不匹配事件被拒绝，停止/释放/新会话会清空身份。该门禁已接入 C# 消费边界，但 `live.open` 响应绑定、真实 `chat.send`、扫码和 WebSocket 仍待实现/验收，Rust 保持只读。

> 2026-09-06 抖音 M1 脱敏事件消费接线：`chat_received` 事件现在只接受 `WebcastChatMessage`、房间/消息/发送者标识、正文长度和自回显/重放标志，拒绝 `text/content/body`；桥接进入 `DouyinLiveManager.ObserveChatMetadata`，真实执行单房间校验、去重、随机回复、有界队列和本地 60 秒 TTL。平台真实 sidecar、扫码、WebSocket、发送和自回显仍未验收，Rust 保持只读。

> 2026-09-06 AkVirtualCamera 安装清单对齐：C# 安装探测现在要求 Rust/NSIS 正式包的 `release-ready.json`、x86/x64 DirectShow、x64 Assistant/Manager、`bin/akvirtualcamera-sidecar-x64.exe` 和 `bin/vcam_capi.dll` 七项固定文件全部存在，避免不完整包配合旧 PnP 设备误报 `Installed`；新增清单回归。探测保持只读，不执行安装/注册/签名/设备修改，真实签名发布和设备验收仍待完成，Rust 保持只读。

> 2026-09-06 AkVirtualCamera 状态投影修复：sidecar 下游客户端数量从 0/正数变化时，`WindowsVirtualCameraOutputCoordinator` 现在在 Core 成功切换 `Ready`/`Streaming` 后发布脱敏 `SnapshotChanged`，WPF 不再保留旧的下游连接状态；新增 `Ready→Streaming→Ready` 回归。该修复不启动真实 sidecar、不改变安装/签名/DirectShow 门禁，Rust 保持只读。

> 2026-09-06 AkVirtualCamera 资源路径修复：C# sidecar locator 现在优先查找正式 staging 目录 `akvirtualcamera/bin/akvirtualcamera-sidecar-x64.exe`，兼容旧版 `virtual-camera/bin` 目录；两条路径继续经过固定文件名、普通文件、非 Reparse、x64 PE 和签名探测。locator 定向测试 `7/7` 通过；真实 sidecar、DirectShow、下游兼容、签名安装和长稳仍待验收，Rust 保持只读。

> 2026-09-06 AkVirtualCamera host 安全门禁收口：`WindowsVirtualCameraSidecarHost` 启动前再次执行 Authenticode 校验，未签名/无效 sidecar 在进程创建前返回 `InvalidPlan`，不依赖 WPF 入口是否已经做过探测。新增拒绝未签名计划测试，host 定向测试 `4/4` 通过；真实签名安装、sidecar、DirectShow、下游兼容和长稳仍待验收，Rust 保持只读。

> 2026-09-06 AkVirtualCamera writer 异常收口：受管 30fps writer 对未分类运行时异常也转换为脱敏 `Failed/WriteFailed`，避免后台任务 fault 后快照继续停在 `Running`，使 coordinator 健康监视可以统一收敛 Core 状态；未新增重试、线程或依赖，writer 相关 Windows 测试 `8/8` 通过，Rust 保持只读。

> 2026-09-06 AkVirtualCamera 运行时故障收口：`WindowsVirtualCameraOutputCoordinator` 增加 250ms 有界健康监视，统一把 WGC、sidecar host、Named Pipe client 和 writer 的失败/退出终态投影到 Core `Failed`；主窗口在故障态仍保留停止入口，停止前先取消并等待健康任务，再按 writer→GPU→client→sidecar 回收资源。新增运行时故障上报与停止清理回归，Windows 受影响测试 `8/8` 通过；不自动重启、不绕过签名/安装/下游门禁，真实 sidecar、DirectShow、WGC 可见帧和长稳仍待验收，Rust 保持只读。

> 2026-09-06 主窗口导入黄金路径修复：媒体探测在后台 continuation 完成后，提交前的 WPF 输出停止/状态投影现通过所属 `Dispatcher` 执行，避免跨线程访问导致新媒体池在提交前被拒绝。使用 `D:\xz\8d020eb133350a74bbc4daec1f33bbc1.mp4` 与已校验 C# 媒体运行包的真实导入→首项播放夹具通过；不改变媒体原子提交、输出停止或取消边界，Rust 保持只读。

> 2026-09-06 本轮真实黄金路径复验：使用 `D:\xz\8d020eb133350a74bbc4daec1f33bbc1.mp4`（FFprobe 72.3 秒、720×1280、AAC 单声道），`VideoPlaybackAudioFallbackTests` 完整 `8/8` 通过，含主窗口导入、视频/声音启动、PortAudio 失败降级、声音处理切换、暂停/恢复和真实 EOF→第二项换源；`MainWindowRtmpReadinessTests` `3/3` 通过，包含后台重连回 Dispatcher。该结果仍不替代 RTMP 远端、人工页面、目标设备和长稳验收。

> 2026-09-06 用户测试媒体 EOF 门禁适配：`D:\xz\8d020eb133350a74bbc4daec1f33bbc1.mp4` 实际时长约 72.3 秒，连续换源夹具不再固定等待 20 秒，而是依据导入后首项时长采用 20～120 秒的有界 `时长+10 秒`预算，避免把长视频错误判定为 EOF 失败；播放逻辑、身份校验、取消和资源释放未放宽，Rust 保持只读。

> 2026-09-06 RTMP 重连跨线程边界修复：有限重连第 2 次及以后尝试会在 `WindowsRtmpReconnectCoordinator` 的后台 continuation 中执行，C# `MainWindow` 现在将每次停止、启动和发布就绪等待统一切回窗口 `Dispatcher`，避免后台线程直接访问 WPF 控件；新增 `RunOnDispatcherAsync` 回归测试。该修复不改变媒体身份保护、有限重试预算、取消和远端握手门禁，Rust 保持只读。

> 2026-09-06 C# 插话预设真实消费边界：修复 `MainWindow.AudioInterlude` 只显示 `presetSummary`、未把选择结果送入 FFmpeg 计划的缺口。固定模式和随机单轨通过 `InterludeAudioSelection.TryCreateBoundedAudioEffectParams` 将唯一 p01～p22 ID 作为现有本地确定性 `VoiceLibraryId`，进入 `FfmpegPcmDecodePlanBuilder` 的受管单输入 `-af` 链；空选择、非法 ID 和多轨选择 fail-closed。该实现是可验证的 ID-only 最小边界，不伪造 Rust `audio-value-presets.ts` 的 35 字段一一映射，也不把多轨/完整 22 套 DSP 标为已生效。Core 10/10、Media 17/17、Contracts 3/3 通过；真实听感、声卡、RTMP 和长稳仍待验收，Rust 保持只读。

> 2026-09-06 C# 自动插话调度接入：WPF 通过现有 250ms `DispatcherTimer` 观察 `InterludeSchedulePlanner`，对齐 Rust 的首次立即、插话结束后再开始有界随机间隔、源代次重置、暂停/固定话术/麦克风门控和相邻文件不重复；手动试播结束后也会重置下一次等待。调度实际启动仍经过既有播放串行门、媒体身份校验、单轨预设投影和最终 PCM overlay 清理。Core 插话规划器定向 `14/14`，App Release x64 构建 0 警告/0 错误；真实 WPF 页面点击、声卡/RTMP-only、自动插话听感和长稳仍待验收。

> 2026-09-06 C# Phase 0 fixture 门禁：`SyncFixtureContractTests` 读取共享目录中的 7 份 JSON，校验 GPU83→CPU4→Original、单窗口 EOF、N/N+1 PCM 双消费者、RTMP 有限退避、虚拟摄像头固定规格、抖音 M1 本地有界串行队列和 8 类稳定错误；定向测试 `1/1` 通过。该证据只代表 C# 合同门禁，不替代 Rust 侧行为测试或真实设备/远端/发布验收。

> 2026-09-06 插话与 PortAudio 生命周期收口：插话文件在本机 PortAudio 与 RTMP 音频会话的 EOF、取消、停止和 decoder 释放前，都会清理 `FinalPcmBus` 本地/RTMP overlay 尾部，不关闭仍由基础轨使用的总线；PortAudio 恢复路径将取消、普通失败和原生/对象生命周期异常映射为稳定结果，原生异常归类为可重试 `RestartFailed`，保留最多 3 次有界重试。相关 Media `FinalPcmBus` 测试 `6/6`、Windows 插话/音频控制器测试 `11/11` 与 `14/14`、PortAudio 恢复专项 `13/13`，本轮 Windows/Media/App 定向回归分别 `28/28`、`29/29`、`7/7` 通过；Rust 的 22 套预设 DSP、多轨候选、真实声卡/voice、远端 RTMP 和长稳仍待验收。

> 2026-09-06 麦克风基础可听桥接：WindowsMicrophonePcmBridge 将受管 PortAudio 输入环缓中的说话/挂起帧按活动最终 PCM 总线映射到 overlay，静音期间丢弃积压；主窗口只在存在活动最终 PCM 总线时开放启用入口，媒体切换、媒体池编辑和停止播放前先停止麦克风。固定话术/麦克风共享 overlay 时只静音/duck 基础轨，不再误静音 overlay。新增桥接、无总线 fail-closed、媒体停止边界和混音策略测试；定向测试 8/8 通过。AEC、降噪、AGC、完整 VAD、真实麦克风/声卡、页面点击和长稳仍待验收，Rust 保持只读。
> 2026-09-06 并行生命周期复核：虚拟摄像头 coordinator 现在逐项收集 writer/GPU/Named Pipe client/sidecar host 清理结果，清理失败不伪造 `Stopped`，取消后执行一次有界补偿清理，并在未完成清理前拒绝再次启动；虚拟摄像头受影响测试 38/38 通过。媒体输出所有权改为先获取 C# 专属 Mutex、再获取 C# 单实例锁，退出逆序释放；新增 C# 内核争用、abandoned 接管和生命周期顺序测试。真实 sidecar/DirectShow/WGC 可见帧、C# 输出资源仍待验收，Rust 保持只读。
> 2026-09-06 RTMP 重连发布门禁：`MainWindow` 现在在有限重连中锁定当前媒体身份，停止前后拒绝跨源重连；启动后最多等待 10 秒，只有所选轨道达到 `Publishing`/声音运行且无错误才返回成功，超时或进程失败继续按既有有限预算重试。新增 `MainWindowRtmpReadinessTests` 2/2；远端 `192.168.10.22:1935` 当前不可达，真实握手、断线恢复和长稳仍待服务器恢复后复验，Rust 保持只读。
> 2026-09-06 本批复验：Windows 测试项目全量 236/236、App 关键回归 6/6、`dotnet format --verify-no-changes --no-restore` 通过，C# App Release x64 构建 0 警告/0 错误；同步矩阵 JSON 解析通过，`git diff --check` 无错误。远端 `192.168.10.22:1935` 本次复探不可达，不能把历史 RTMP 烟测或本机进程启动当作当前远端/人工验收，Rust 保持只读。

> 2026-09-06 固定话术/SAPI → 最终 PCM P0 接线：`MainWindow` 现在把 `WindowsAudioPlaybackController.ActiveFinalPcmBus` 提供给 `WindowsSystemSpeechAdapter`；`WindowsSapiSpeechBridge` 将 SAPI 输出捕获为 48kHz/16-bit PCM，经既有 overlay 分支送入 PortAudio 及已挂接的 RTMP 消费者。无活动总线时拒绝启动，不回退默认音频设备；取消、抢占、关闭和终态会清理未消费 overlay，保留“麦克风 > 固定话术 > 插话 > 原媒体”优先级。新增总线缺失/PCM 接入/取消清理测试；使用仓库锁定 SDK 定向测试 `11/11` 通过，真实 SAPI voice、声卡、RTMP-only、人工页面点击和长稳仍待验收，Rust 保持只读。

> 2026-09-06 最终效果窗口按视频尺寸等比调整：C# `FinalEffectSnapshot` 现在携带当前视频的宽高；首次显示按当前显示器工作区和窗口非客户区等比缩放，视频较大时先缩放到工作区内，视频较小时保留最小尺寸。最终效果窗通过 Windows `WM_SIZING` 在普通窗口拖拽边缘/角点时锁定当前视频宽高比，全屏期间不拦截系统尺寸，退出全屏后按当前视频重新校准。新增等比尺寸计算、主窗口宽高投影和最终效果窗口回归测试；人工拖拽、多屏切换、DPI 和真实视频窗口像素验收仍待可用桌面环境，Rust 保持只读。

> 2026-09-06 Rust/C# 展示一致性修复：确认两端最终效果窗口都保持黑色纯画布，视频使用独立视频表面，声音使用黑底，不在弹窗内重复放置播放控制或处理开关；本次截图中的控件实际属于 C# 主窗口底部播放栏。C# 底部视频/声音处理开关继续保留可访问的 `CheckBox` 语义和“已启用/已关闭”状态文案，但自定义开关已对齐 Rust `Switch size="small"` 的 28×16 轨道、12×12 滑块，并修正模板列宽与内容伸展布局，避免轨道超出列宽造成显示偏移。新增 WPF 几何回归测试；人工页面点击与真实窗口像素验收仍待可用桌面控制环境，Rust 保持只读。

> 2026-09-06 RTMP 状态门禁与重连预算：受管 FFmpeg 通过 stderr 上的 `-progress pipe:2` 输出建立正向进度证据，只有 `out_time_ms/out_time_us/total_size` 为正值后才投影“推流中”；主窗口将启动、重连、停止和失败与已发布状态分开显示，重连也只有所选画面/声音轨道都达到已发布状态才报告成功，避免把进程存活当作远端握手成功。默认重连为初次启动后最多 5 次重试，退避 `1/2/4/8/15s`；目标 `192.168.10.22:1935` 当前复探测不可达，因此远端握手、三轨道、恢复和长稳仍待服务器恢复后复验，Rust 保持只读。

> 2026-09-05 sidecar 首帧启动门禁：`WindowsVirtualCameraSidecarOutputWriter` 现在必须在 2 秒有界预算内完成第一帧固定 `1280×720 YUY2@30fps` 写入，才返回启动成功并进入 `Running`；断管道、取消和超时会回收 worker 并返回脱敏失败，不再把 worker/IPC 连接存在冒充实际帧交付。新增断开管道首帧失败回归测试；使用项目锁定 SDK 的 Windows 目标测试 `227/227` 通过，格式检查和 Release 构建通过，`git diff --check` 通过。真实 sidecar、DirectShow、下游兼容、目标 GPU、签名/许可证和长稳仍待验收，Rust 保持只读。

> 2026-09-05 声音与最终效果窗口修复：底部输出音量滑块已从静态占位接入现有最终 PCM 混音策略，0～100% 映射为有限 dB 增益，实时作用于本机 PortAudio 与 RTMP 分支；声音处理开关新增单调版本号，异步重建期间再次切换会拒绝过期请求，避免旧声音会话覆盖最新状态。最终效果窗口已对齐 Rust 的视频纯画布样式，删除 Footer、状态/等待文字及弹窗内播放控制，默认/全屏往返仍保持 mpv 视频子 HWND 与 WGC 顶层 HWND。声音专项此前 `135/135`、FinalEffect/声音状态目标此前 `10/10`、Release x64 构建此前 `0` 警告/`0` 错误；本轮样式回归与构建结果以本次交付记录为准，真实声卡和人工页面点击仍待验收。

> 2026-09-05 最终效果窗口样式对齐 Rust：删除 C# FinalEffectWindow 底部 Footer、左上角表面标签、等待播放文字和弹窗内播放/停止/进度/关闭控件；客户区改为黑色视频画布，标题、默认尺寸 `1280×720`、最小尺寸 `320×180` 与 Rust `final-effect` 运行时窗口一致。播放控制回到主窗口，原生标题栏仍可关闭；mpv 视频子 HWND、WGC 顶层 HWND、单实例和输出生命周期不变。新增/调整 WPF 回归契约覆盖无控件纯画布与全屏往返；人工页面点击、真实声卡、WGC 下游和长稳仍待验收。

> 2026-09-05 C# 媒体池搜索与导入按钮样式修复：搜索框从只读占位文本改为真实输入框，按文件名或媒体类型过滤 `VisibleMediaItems`，空白搜索恢复完整池，空结果显示明确空态；完整 `MediaItems` 和 `MediaPoolService` 播放池不被搜索改写，过滤后的选中项操作按 ViewModel 身份映射回源池索引。底部主导入按钮继续复用既有 `ImportButton_Click → RunImportAsync` 链，仅补齐 32px 高度、居中对齐和与顶栏一致的加号/快捷键布局。新增搜索投影、源池不变、过滤选中索引和按钮样式回归；目标测试 `2/2`、既有媒体池忙碌态测试 `1/1`、格式检查通过，Release x64 构建 `0` 警告/`0` 错误。CUA 未返回 C# 原生窗口，本轮不宣称人工页面点击或视觉像素验收；Rust 端保持只读。

## 当前结论

工程仍处于“实施中”，不是正式发布完成。C0/C1/C2 已完成；C3～C7 的核心代码边界、WPF 工作台和发布骨架已接入，有限音频项的 N/N+1 预载、单一 PortAudio 输出切换、声音周期候选和基于 PortAudio callback `timeInfo` 的可听时钟投影已有代码/夹具证据，WPF 也已在完整 shader 哈希匹配时选择 GPU83、旧资源包保持 CPU4；FFprobe 对封面图视频流和无效平均帧率的导入边界已与 Rust 对齐，媒体池的真实编辑控件与底部主导入入口已恢复可见，顶栏与底部导入入口已接入同一忙碌态投影。但真实设备、网络、签名、安装和长稳门禁尚未全部通过。2026-09-03 已确认两张 v2 设计稿为授权态主工作台的正式 UI 实施基准；首屏与下滑态的静态网格、视频/声音只读快照和连续滚动结构已接入，下滑续页的数据所有者和真实运行时生效回显仍未完成。

> 2026-09-05 黄金路径最终复验：App 测试程序集已加入 `[assembly: DoNotParallelize]`，真实 WPF/窗口/mpv 资源不再与普通测试并行竞争；App 全量串行测试 `79/79` 通过，其中 `VideoPlaybackAudioFallbackTests` `6/6`、媒体拖放边界 `7/7`。真实 WPF 导入→FFprobe→原子入池→首项选中→mpv 播放、EOF 自动换源和 PortAudio 失败时视频保活均通过；人工页面点击仍待可用桌面控制环境。

> 2026-09-05 播放契约对齐：C# `MediaPoolOwner.ResumePlayback` 与 Rust `PlaybackCore::resume` 统一为只允许 `Ready/Paused → Playing`；`Stopped` 后的再次播放统一走 `StartPlayback`。主窗口仅在 Core 逻辑状态为 `Playing/Paused` 且 mpv 身份、运行态一致时复用暂停切换，`Ready/Stopped` 走带首帧门禁的新启动路径；失败结果继续投影原 Core 快照，不以进程存活宣称播放成功。Core 媒体池测试 `16/16` 通过；真实人工导入、暂停/恢复/停止和 EOF 换源仍待桌面环境复验。

> 2026-09-05 纯音频处理中途位置保持：切换声音处理或重新生成声音快照时，C# 从现有 PortAudio 可听时钟读取位置，再从该位置重建 FFmpeg PCM 会话；已知时长限制在末尾前 1ms，暂停态保持暂停。App 稳定测试 `55/55`、格式检查通过，Release 构建 `0` 警告/`0` 错误；视频声音路径继续使用 mpv `time-pos` 恢复，声卡长稳与断设备恢复仍待验收。

> 2026-09-05 声音周期候选修复：声音处理开启时不再同时预载下一媒体项，唯一候选槽专用于同源周期效果，避免把下一项误提交为效果候选而提前切源；声音处理关闭时仍保留原有 N+1 预载路径。App 稳定测试 `55/55`、Release 构建 `0` 警告/`0` 错误；声卡长稳、断设备恢复和 RTMP 联合门禁仍待验收。

> 2026-09-05 启动链修复：`tools/start-csharp-development.cmd` 现在每次使用仓库锁定的本地 SDK 以 `Platform=x64` 编译当前 C# App，再启动对应 `bin\x64\Release` 产物，避免普通 `dotnet build -c Release` 输出到另一目录后继续运行旧 EXE；构建失败会 fail-closed。

> 2026-09-05 启动脚本默认运行包修复：修正 CMD 括号块变量提前展开导致默认 v2 运行包为空的问题；不传参数时现在会正确选择 `csharp-gpu83-real-v2`，缺失时回退 v90。默认脚本本地构建并拉起目标 EXE 的短冒烟已通过；人工导入/播放仍需可用桌面控制环境复验。

> 2026-09-05 视频启动降级接入：新鲜视频会话按 `GPU83 → CPU4 → Original` 单向尝试；每次失败先由同一 mpv 控制器完成清理，不创建第二播放器或回升会话。实际启动模式会回显到状态栏，GPU83 故障触发后的实机降级和目标显卡矩阵仍待验收。

> 2026-09-05 PortAudio 持续 xrun 有界重建：输出快照新增 callback 状态标志/xrun 计数；健康观察器忽略短暂启动欠载，持续达到 1024 callback 且至少 75% xrun 时复用单一输出流重建，成功后更新基线，连续 3 次仍异常则 fail-closed。Windows 音频相关测试 28/28、真实声音效果链 1/1、真实同源声音周期切换 1/1 通过；真实拔插、驱动重置和 30 分钟长稳仍待验收。

## 当前 UI 设计基准

- 首屏：[`gpautolive-csharp-windows-gui-v2-main-20260903.png`](./assets/gpautolive-csharp-windows-gui-v2-main-20260903.png)
- 下滑续页：[`gpautolive-csharp-windows-gui-v2-scroll-20260903.png`](./assets/gpautolive-csharp-windows-gui-v2-scroll-20260903.png)
- 两张图共同定义 `1586×992` 授权态主工作台；第二张是首屏中央参数区的内部滚动续页，不是独立页面或新业务状态。
- 视频和普通声音效果值必须按周期由系统自动/随机生成并只读展示；用户只编辑周期范围、预设池和输入/输出等规则配置。禁止把本周期结果做成 Slider、可编辑数值框或仅禁用的假控件。
- 设计稿没有覆盖登录页。登录页继续按独立登录门禁、错误映射、Refresh、Logout 和远程测试配置验收，不能用工作台截图宣称登录页 1:1。

## 可复核证据

| 项目 | 当前证据 |
| --- | --- |
| 最新候选 | `artifacts/csharp-gpu83-real-v2`（缺失时由启动脚本回退 `csharp-windows-controller-20260903-v90`） |
| 主 EXE | 8 个根运行文件；当前本地 Release 产物 `GpAutoLive.exe` `172,544` bytes；`GpAutoLive.dll` `311,808` bytes |
| 历史全量测试基线 | 502/502（Contracts 28、Core 77、Media 115、Windows 211、App 56、Installer 15；该行仅保留历史批次，不代表当前最新计数） |
| 最近真实启动门禁 | `E:\下载\csharp-golden-av.mp4`：CPU4 与完整 GPU83 真实 mpv 夹具在控制器进入 `Playing` 前确认 `estimated-frame-number > 0`，各 `1/1`；未观察到首帧会返回 `FirstFrameNotObserved` 并清理运行时 |
| 最近 WGC 最终表面复验 | WGC frame pool 已注册 `FrameArrived`，由专用线程排空 `TryGetNextFrame` 并在停止时取消订阅；`GraphicsCaptureItem` 使用 activation factory + `IGraphicsCaptureItemInterop.CreateForWindow` 严格 ABI；生产绑定为最终效果窗口顶层 HWND，mpv 仍使用视频子 HWND；实际 WPF `FinalEffectWindow` GPU83 与 CPU4 最终表面像素夹具各 `1/1` 通过。目标 GPU 矩阵、AkVirtualCamera 下游和发布门禁仍待验收 |
| 历史完整验证基线 | 项目锁定 SDK 下 `dotnet format --verify-no-changes` 通过；串行全量 `dotnet test` 498/498；该行保留较早批次的分层证据，当前结果以“2026-09-05 最新增量复验”为准 |
| 2026-09-05 最新增量复验 | Windows 全量 `223/223`、Media 全量 `131/131`、App 稳定分组 `60/60`、App 全量串行 `79/79`，其中 `VideoPlaybackAudioFallbackTests` `6/6`、媒体拖放边界 `7/7`、音频控制器边界 `14/14`；WPF GPU83 最终表面像素 `1/1`、CPU4 对照 `1/1`；App Release x64 构建成功，串行门禁解决了共享 WPF Dispatcher、环境变量、HWND 和 mpv 资源竞态。GPU83 原生 Win32 YUY2 像素夹具 `1/1`、真实主窗口导入→原子媒体池→mpv 播放、EOF 自动换源及声音故障保活/暂停恢复均通过；人工页面点击、真实声卡长期稳定、RTMP/RTMPS、AkVirtualCamera/DirectShow、目标 GPU 矩阵和长稳仍待验收；Rust 仍仅作只读参考。 |
| C3～C5 离线矩阵 | 8 行、167 项通过；4 个真实门禁保持 deferred |
| 本机媒体夹具 | `artifacts/csharp-gpu83-real-v2` 外置 mpv 的 Original/CPU4/GPU83 夹具均通过播放时间、seek、EOF；GPU83 已加载经 manifest/SHA-256 校验的完整 shader；FFmpeg PCM 1/1 通过；本轮用 `E:\下载\csharp-golden-av.mp4` 完成真实 FFprobe 导入、主窗口媒体池播放、GPU83/CPU4 WPF 最终像素和 PortAudio 输出门禁 |
| 发布包 | v90 Manifest/SHA-256 核验通过；`-RequireSigned`/`-PlanOnly` 行为符合预期；本轮 GUI 源码改动未重新生成 v90；当前源码 WPF 启动/关闭冒烟通过；`tools/start-csharp-development.cmd` 默认优先使用已校验的 `artifacts/csharp-gpu83-real-v2`，缺失时才回退 v90；显式运行包参数行为不变 |
| 作用域 | 当前 C# 长任务只允许 `desktop-csharp-windows/`；`tools/verify-scope.ps1` 会对仓库中已有的 Rust/Tauri 变化 fail-closed，本轮不把 Rust 变化视为 C# 实施结果 |
| 开发窗口 | 当前已从更新后的本地 Release 产物启动；启动时使用 `GpAutoLive - 本地开发控制面`，指向固定测试控制面 HTTP 地址 |

## 已接入的功能边界

- WPF 登录、自动激活、Refresh、Logout、媒体池、唯一最终效果窗口、视频/纯音频播放入口。
- 登录失败/Refresh 失败会清理状态机要求删除的旧 Refresh Token；串行门前取消返回受控错误；WPF 登录调用有异常收口。未配置控制面时明确提示配置要求且不发网络请求；测试和本地 `development` 环境允许固定远程地址 `http://101.96.208.132:9090` 或 loopback HTTP，其他远程 HTTP 仍拒绝；已提供远程测试、本地开发和本地离线启动配置，正式环境仍要求 HTTPS。GUI 与两张 v2 设计稿的静态差异已记录在 [`GUI视觉对照审计-20260903`](./GUI视觉对照审计-20260903.md)，当前不判定为 1:1。
- 直接双击 EXE 不会读取 `launchSettings.json`；已新增 `tools/start-csharp-development.cmd`，显式注入 `development` 环境和固定 HTTP 控制面地址，避免本地开发端反复出现“控制面未配置”。正式环境仍保持显式 HTTPS 门禁。
- 2026-09-05 真实黄金路径复验：使用 `E:\下载\csharp-golden-av.mp4` 和 `artifacts/csharp-gpu83-real-v2`，WPF `VideoEffectPixelFixtureTests` 的 GPU83 与 CPU4 最终表面像素夹具各 `1/1` 通过；Windows 实时声音效果链→FFmpeg→PortAudio→最终 PCM `1/1` 通过；主窗口真实导入→FFprobe→原子媒体池→首项选中→mpv 播放，以及无效 PortAudio 设备时视频保活合计 `2/2` 通过。该证据仍不替代真实声卡长期稳定性、RTMP/RTMPS、WGC 下游、sidecar/DirectShow、目标显卡矩阵和人工页面验收。
- 2026-09-05 GPU83 最终表面复验：独立真实 mpv GPU83 完整 shader 夹具 `1/1` 通过；实际 WPF `FinalEffectWindow` GPU83 WGC 像素夹具 `1/1` 通过，确认处理参数更新后最终视频表面像素发生变化。目标显卡矩阵、真实下游和长稳仍待验收。
- 2026-09-05 GPU83 原生窗口像素夹具收敛：显式 Win32 窗口重绘夹具已复用受管 mpv GPU83、WGC/D3D11→YUY2 和前后帧哈希链，最终 YUY2 像素 `1/1` 通过；Windows 受影响测试 `217/217`、格式检查通过。该证据不替代目标 GPU 矩阵、AkVirtualCamera 下游和发布门禁。
- C# 桌面端产品图标已从 Rust/Tauri `desktop/src-tauri/icons/` 复制为本地 `src/GpAutoLive.App/Assets/app-icon.png` 与 `app-icon.ico`；主标题栏、EXE、设置窗口和最终效果窗口共用该产品图标资源。
- mpv JSON IPC、HWND 播放、受身份保护 seek/EOF、FFmpeg PCM、固定容量 PCM 总线、PortAudio 输入/输出和本地麦克风能量门控；固定话术已接入同一最终 PCM overlay，SAPI/声卡仍待真实验收。
- 2026-09-04 媒体闭环增量：C# mpv 启动后的 Original/GPU83/CPU4 初始模式统一经 `MpvPlaybackSession.UpdateEffects` 固定 IPC 命令应用；CPU4 不再同时出现在静态启动参数和动态滤镜链中；`WindowsMpvPlaybackController.UpdateEffectsAsync` 与换源后的效果重应用入口已接入。GPU83 使用经 manifest/SHA-256 校验的完整 shader 运行包；启动和更新都会对 GPU83 固定白名单、CPU4 固定滤镜值做运行时回读，并由 `estimated-frame-number` 观察下一帧。WPF GPU83/CPU4 最终表面像素已有显式夹具证据，但不标记为所有参数均完成真实环境验收。
- 2026-09-04 视频处理开关增量：WPF“视频处理”开关在已有视频会话处于播放/暂停时，按当前系统生成快照和资源能力门禁调用 `WindowsMpvPlaybackController.UpdateEffectsAsync`；无活动 mpv 会话时只更新待播放配置，不宣称运行时已生效。播放态下一帧有效回显已由后续 `estimated-frame-number` 观察接入，WPF GPU83/CPU4 最终表面像素也已有显式夹具证据；目标显卡矩阵和发布门禁仍未完成。
- 2026-09-04 开关接线修复：视频处理改由 `ShellState.VideoProcessing` 属性变化统一提交，声音处理保留单一 WPF `Click` 入口，避免绑定状态变化后只停留在“壳状态”。视频播放中切换声音开关现在读取当前 mpv `time-pos`，只停止并重建 FFmpeg/PortAudio 声音会话，按该位置恢复且不重启视频；位置不可读时 fail-closed，保留视频并提示重新播放。已用 `E:\下载\csharp-golden-av.mp4` 完成真实声音偏移解码和最终 PCM 输出验证；这仍不等同于 GPU83 完整渲染或真实声音频谱验收。
- 2026-09-04 实时声音链补充：C# `FfmpegAudioFilterBuilder` 已把 Rust realtime 链中可流式执行的倍速、内置滤镜音高微移、淡入、已知源时长的淡出、混响、降噪、相位扰动和颤音接入 FFmpeg `-af`；低/中/高 EQ 中心频率统一为 Rust 当前 realtime 链的 `200/1000/8000 Hz`。未知时长不伪造淡出位置，系统快照的混响/降噪标签也会进入对应数值映射。以 `E:\下载\csharp-golden-av.mp4` 执行的显式 Windows 夹具已验证该滤镜链能真实解码并产出最终 PCM/PortAudio 帧；需要反向缓冲的复杂变换、专用 DSP 和第二音频输入仍保持未接入。
- 2026-09-04 生成声音快照接线：`GeneratedAudioEffectSnapshot` 现在生成范围为 `-0.5～0.5` 半音的 `PitchShiftSemitones`，`MainWindow` 播放参数映射会把它传入现有 FFmpeg 内置音高滤镜；App 范围测试已覆盖该字段。动态范围、压缩和音色仍只读展示，不进入 C# 播放参数或 FFmpeg 滤镜，避免把没有正式对应字段的近似处理冒充为已生效。该值只代表已接入的实时子集，不代表复杂 DSP 或完整 Rust 音频链已经完成。
- 2026-09-05 声音周期频谱字段映射补齐：`GeneratedAudioEffectSnapshot` 现在按正式 `AudioEffectParams` 契约生成并映射频谱扰动、频谱盲区和高频扰动的 6 个字段；现有 `FfmpegAudioFilterBuilder` 因此能在周期候选中消费对应 `afftfilt`/`bandreject` 链。动态范围、压缩和音色仍只读展示，不扩展为未建模的 C# 运行时参数。App 映射与滤镜触发测试 `2/2` 通过。
- 2026-09-04 状态文案修复：中央视频结果卡改为按资源选择并说明 GPU83/CPU4/Original 的真实条件，不再把单一历史路径显示成所有资源包的当前状态；普通声音结果卡补齐中频 EQ dB，避免显示层漏掉已存在的正式字段。
- 2026-09-04 混合媒体池修复：纯音频自然 EOF 后遇到视频项时，保持播放池 `Playing`，重新启动统一视频入口建立 mpv 与视频声音会话；新增 Core 回归测试锁定音频→视频切换不得把混合池误停。
- 2026-09-04 视频效果回读补充：`WindowsMpvPlaybackController` 通过固定 `MpvIpcProperty.VideoFilterChain` 白名单读取 mpv `vf` 属性；真实 CPU4 夹具在初始效果提交后回读到 `@autolive_cpu4` 活动链，并在 `UpdateEffectsAsync` 后逐项回读新的亮度、对比度、饱和度和色相值，锁定“提交 → 运行时链存在 → 运行时参数变化”的运行时证据。该证据不代表 GPU83 shader、下一帧画面像素或完整参数已经验收。
- 2026-09-04 GPU83 基线资源补充：C# 媒体 manifest 允许列表新增 `gpu83.hook`，并校验其相对路径、大小和 SHA-256；`MpvLaunchPlan` 在 GPU83 模式下只绑定已验证的 `--glsl-shaders` 路径，缺少资源时 fail-closed。`MpvVideoEffectSnapshot.TryCreateGpu83BaselineColor` 只通过固定键把亮度、对比度、饱和度和色相四项传入 shader；`WindowsMpvPlaybackController.UpdateEffectsAsync` 现在还必须回读并逐项匹配 `glsl-shader-opts`，否则更新失败关闭。使用 `E:\下载\csharp-golden-av.mp4` 的真实 mpv 夹具已验证初始和更新后的四项值、播放时间、seek 和 EOF。旧 v90 资源包的 WPF 播放保持 CPU4；完整 C# v2 资源包按完整 shader 哈希选择 GPU83；完整 83 项参数、下一帧有效回显、目标 GPU 矩阵和像素级验收仍未完成。
- 2026-09-04 GPU83 完整契约接线：C# 新增 `MpvGpu83ShaderSnapshot`，按 Rust 83 项映射表将静态像素/合成参数、PTS 调度输入和明确不可用的历史纹理/未验证字段分开；`MpvVideoEffectSnapshot.TryCreateGpu83` 只提交可实际表达的白名单选项，并保留不可用字段清单。C# 自有 `Resources/gpu83.hook` 与 Rust 当前 shader 内容做了只读对照，`stage-media-runtime.ps1` 现在会把它纳入外置包 manifest/SHA-256。使用 `E:\下载\csharp-golden-av.mp4` 和独立 C# 运行时包完成完整 GPU83 启动、更新、播放时间、seek、EOF 及静态/调度选项回读 1/1；这证明资源和 IPC 闭环，不证明 GPU83 下一帧像素、目标显卡矩阵或 WPF 已切换到完整 GPU83。
- 2026-09-04 WPF 完整 GPU83 选择接线：系统生成的视频快照先映射为正式 `VideoEffectParams`，`锐度`进入完整 shader 参数；只有受 manifest/SHA-256 校验且哈希等于当前 C# `gpu83.hook` 的资源才选择完整 GPU83，旧 v90 基线或缺少完整资源时继续使用已验收的 CPU4。Gamma、曝光和降噪文案没有对应的已接入 C# 视频字段，仍明确不标记为已生效。完整 GPU83 参数/调度回读夹具通过，但下一帧像素、目标显卡矩阵和 WPF 原生首帧仍待验收。
- 2026-09-04 FFprobe 兼容性修复：按 Rust 只读对照忽略音频容器中的 `attached_pic` 封面图视频流，并在 `avg_frame_rate` 为 `0/0` 或其他无效值时回退 `r_frame_rate`；新增两个 C# 边界测试，未放宽媒体扩展名、路径校验、整批原子提交或登录授权门禁。
- 2026-09-04 媒体池入口修复：C# WPF 媒体池真实的上移、下移、移除和清空控件从零高度隐藏行恢复为可见布局；顶栏“添加媒体”增加自动化标识，并与既有导入入口共享忙碌态。该修复没有放宽登录/设备授权或改变导入失败保留旧池的语义。
- 固定话术、插话文件池、PCM 混音/优先级、RTMP 画面/最终 PCM 声音会话、有限重连协调器。
- AkVirtualCamera 契约、GPU/WGC/sidecar 探测和输出门禁；抖音 M1 本地合同、配置 JSON、队列和受管 sidecar 启动边界。
- 外置 GPU/WinRT/媒体运行库、Manifest、安装/回滚/卸载骨架、Runtime 探测、签名流水线和 WPF 安装维护壳。
- C7 安装、回滚、卸载脚本共用跨维护壳事务锁 `Local\\GpAutoLive.CSharp.Windows.InstallTransaction.v1`；占用时立即 fail-closed，避免并发覆盖 `current.json` 回滚链。详见 [`C7 安装事务锁审查记录`](./C7-安装事务锁审查记录.md)。
- C7 媒体/输出租约使用 C# 专属的 `Local\\GpAutoLive.CSharp.MediaOutput.Owner.v1`；Rust/Tauri 不参与 C# 运行时，C# 不探测 Rust。详见 [`C7 媒体输出所有权与单实例审查记录`](./C7-媒体输出所有权与单实例审查记录.md)。当前只验收 C# 自身进程内外的资源生命周期。
- C7 安装事务读取已有 `current.json` 时校验 schema、活动版本、相对路径和上一版本；指针激活失败/取消时清理本次新版本目录并保留旧指针。`test-c7-install-boundaries.ps1` 已覆盖损坏指针、WhatIf 不落盘和激活失败清理。
- WPF 主窗口已按职责拆分为 RTMP、性能、虚拟摄像头、抖音 M1、音频/插话/麦克风、播放、媒体池 partial；共享生命周期和媒体池编辑状态仍由 `MainWindow.xaml.cs` 唯一持有。C6 已为麦克风与抖音后台快照接入有界 latest-wins UI 更新队列；媒体列表保持 Recycling 虚拟化，缩略图通过有界异步 FFmpeg 首帧链加载并在失败时回退类型图标，不保留无界日志历史列表。
- GUI 结构对齐已进入 v2 双页实施轮：根窗口以 `1586×992` 为基准，工作区约按 `23.2:50.3:26.5` 三栏与 `10px` 间距布局；左栏保留媒体池/播放设置，中栏预览下方使用四个 Tab、分类导航和中央独立滚动区；首屏视频结果改为两行八项系统随机只读快照，下滑态隐藏分类/Tab 并按声音 8 项、场景 5 项、设备 6 项单行卡片显示，右栏仍按 RTMP→虚拟摄像头→麦克风→固定话术→抖音弹幕排列。右栏主卡只保留设计稿配置/状态，已有高级操作进入折叠区并继续复用原事件处理。夹具已验证首屏和下滑静态网格，但没有真实媒体首帧、真实周期刷新、原生运行窗口或目标 DPI/像素级截图，仍不判定为 1:1。详见 [`GUI视觉对照审计-20260903`](./GUI视觉对照审计-20260903.md)。
- 顶栏已按 v2 主图收口为设备授权、账户、授权到期、性能、`64px` 设置入口和窗口控制；授权态不再占用顶栏显示独立退出按钮，退出登录保留在汉堡菜单中并继续复用原登录状态机。
- C4 Media 层新增固定双槽 `AudioPcmTrackSwitchOutputSource` 和 `FinalPcmBusTrackSwitch`：显式提交、在 N 自然 EOF 或目标输出帧到达时切换到 N+1；纯逻辑夹具覆盖临时欠载、连续交付、按帧切换和 N+2 拒绝，Windows 控制器已把真实 FFmpeg 候选预载、有限 EOF/周期观察、单 PortAudio 输出和 RTMP 分支接入该双出口。候选总线容量固定且有背压，未提交候选不会影响当前输出；真实 `E:\下载\csharp-golden-av.mp4` 已完成一次 N/N+1 切换，8 秒真实音频周期夹具也已完成同源候选切换。可听时钟已接入 PortAudio 输出帧/延迟快照和 WPF 视频进度投影；真实设备时钟长期稳定性、设备故障恢复、过载重建和远端 RTMP 仍待验收。
- C5 RTMP 宿主自然/快速退出统一经过有界 PCM 串行回收，并等待 stderr 读取任务结束后再释放进程资源；远端握手、断开确认和音画成组重连仍由上层 deferred。
- 2026-09-04 音频输出增量：`WindowsAudioPlaybackController.ActiveFinalPcmBus` 暴露当前会话的最终 PCM 总线只读入口；RTMP 声音会话可消费该总线的稳定 RTMP 分支而不重复解码或关闭总线，播放停止会先停止 RTMP 以保护总线所有权。有限音频项新增一个候选预载解码器、候选容量等待、EOF 提交和单一输出源切换；当前项与下一项之间不重启 PortAudio，RTMP 分支在已连接时参与同一切换边界。没有本地音频总线时仍保留原独立 RTMP 解码路径；可听时钟已接入输出帧与 DAC 延迟快照，设备恢复和真实网络输出仍待验收。

> 2026-09-04 N/N+1 生命周期补充：C# `WindowsAudioPlaybackController` 现在只允许一个 N+1 候选，候选 FFmpeg 解码移除 `-re` 并通过固定容量总线背压；WPF 音频完成观察者在当前项 EOF 后提交候选，`FinalPcmBusTrackSwitch` 同步本机、RTMP、插话本机和插话 RTMP 四个分支，成功切换不重启 PortAudio。候选未就绪、取消或提交失败时回退到原有停止后重启路径。纯逻辑切换测试和 `E:\下载\csharp-golden-av.mp4` 真实 PortAudio N/N+1 测试均通过 1/1；真实时钟稳定性、设备拔插/重建、过载重建、带插话跨边界或远端 RTMP 仍未验收。

> 2026-09-04 可听时钟补充：PortAudio callback 读取官方 `PaStreamCallbackTimeInfo` 的 `currentTime/outputBufferDacTime`，累计已写入设备的 PCM 帧并计算输出延迟；`WindowsAudioPlaybackController` 在会话启动和 N+1 晋级边界重设锚点，WPF 仅在同一媒体身份且时间信息有效时用该快照投影视频进度，缺失时间信息时回退 mpv 位置。该增量通过 3 个可听时钟单元测试，并在真实夹具中观察到 PortAudio 已送出帧；声卡拔插/睡眠唤醒、真实延迟稳定性和长稳仍未验收。

> 2026-09-04 过载边界补充（历史记录）：C# 主音频轨曾在最终 PCM 总线或已连接 RTMP 分支达到容量水位时进入可取消背压，避免生产线程在正常容量范围内覆盖活动消费者的旧帧；v1.61 已收口为仅对本机 PortAudio 分支节流，RTMP 保持独立固定容量丢旧，避免 RTMP 泵暂时未读反向卡住本机声音。真实声卡驱动异常、过载重建和远端 RTMP 长稳仍未验收。
- v2 UI 已完成首屏网格、视频八项随机只读结果卡、下滑态声音八项/场景五项/设备六项排布和状态续页骨架的第一轮迁移；视频结果卡当前按最新截图收窄为仅显示参数标签和值，移除大图标、γ 符号和迷你趋势线。视觉夹具还会按 v2 设计稿裁剪左侧六项缩略图并单独注入中央预览裁剪图，但这不代表真实 FFmpeg 首帧。“重新生成”会更新本地有界随机快照，并在有活动的纯音频会话或视频 mpv 会话中提交当前可用的声音/视频效果；视频声音会话按 mpv 当前位置重建并继续保持画面，不再静默延迟。声音周期调度已接入 C# 播放链并有真实音频夹具证据；规则编辑、完整真实频谱、真实生成源和下一帧有效回显仍未完整验收，不能把现有 UI 视为 v2 已完成。
- 媒体池条目文案已按 v2 主图调整为“时长 + 分辨率/画幅比”或“时长 + 采样率/声道类型”；原始帧率、声道数等字段仍保留在 `SourceMediaDto` 和视图模型中，不改变播放与探测数据。
- 顶栏设备状态已按 v2 主图改为盾牌勾形矢量图标，填充色继续绑定真实授权状态；标题栏仍保持设置、窗口控制和汉堡菜单退出登录路径。
- 右栏五张输出/互动卡已补齐标题语义图标，采用 WPF 矢量路径且不新增依赖；图标不代表对应功能已经接入，真实状态仍由运行时投影。
- 中央参数区共享滚动条已修正垂直 Track 方向，首屏拇指在顶部、下滑续页按正常方向反馈；首屏和下滑截图夹具均已重新验证。
- 2026-09-04 WGC 可选像素夹具等待收敛：测试辅助改为有界 `DispatcherFrame` 和有界后台线程帧等待，移除嵌套同步 `Dispatcher.Invoke` 及临时诊断日志；生产播放/WGC 路径未改变。普通 WPF/实际 `FinalEffectWindow` 顶层 HWND 启停隔离测试仍为 `2/2`，mpv 挂载后的最终 WGC 像素仍待在可捕获窗口环境验收。
- 2026-09-04 WGC 无帧门禁复验：C# 按 Rust 对齐 activation factory + `CreateForWindow`，并在同一捕获线程保留 100ms 有界轮询；原生 Win32、普通 WPF 和实际 `FinalEffectWindow` 均可建立 `Running` frame pool，但当前 Windows 11 26200/虚拟显示适配器环境在 5 秒内未产生 `FrameCount`，没有把它伪造为像素通过。临时强断言和诊断代码已撤回，启动/停止隔离测试保持 `2/2`。
- 滚动拇指颜色已从强调绿收敛为设计稿中性灰，避免滚动提示与有效状态色混淆。
- 下滑态中央提示已增加左右方向引导符，与设计稿的“向下滚动 · 参数与状态”提示形态一致。
- 中央预览已按目标内部比例校准：基准尺寸下预览画面为 `759×276`，媒体状态行高度与设计稿一致，参数区起始位置保持在 `y=477`。
- 底部播放栏已按 v2 主图校准：五个播放按钮使用 `82/82/82/88/88px` 宽度，中央拆分为当前媒体行、`320px` 进度轨道和时间文本，视频/声音处理开关改为标题加状态副标签的纵向结构，音量轨道为 `120px` 并显示 `80%`；均继续复用原播放、seek 和处理开关绑定，未新增未接入的音量业务逻辑。
- 顶部操作带已按设计稿校准为 `53px` 行高，首页/工作区导航和中央操作按钮统一为 `36px` 高度；中央五个工具按钮宽度为 `110/102/100/98/100px`。视频/声音快照的正向值补齐 `+` 号，右栏摄像头、麦克风、固定话术和抖音卡片的静态高度也已按目标落点收口。夹具只同步设计稿示例值，不伪造真实推流、摄像头或抖音连接状态。
- 中央参数区 Tab 已校准为 `30px` 高度和设计稿横向节奏；视频分类入口与单行 `1×8` 结果卡的标签/数值基线已按目标坐标调整，结果卡仍是独立的系统生成只读投影。
- 视频 8 个结果参数已从两行四列收口为单行八列 `1×8` 紧凑网格，卡片高度 `62px`，仅显示参数标签和值；宽度由中央参数区均分，未引入横向滚动，绑定和只读边界不变。
- 中央参数滚动容器已按实际 WPF 布局树校准：滚动条收窄至 `12px` 并右移到目标区域，内容保留等量右侧空间；滚动方向和真实数据边界不变。
- 右栏抖音弹幕卡的服务器地址字段已移除无效空按钮列并铺满整行，房间 ID 的断开连接按钮列保持独立。
- 视频结果卡单行八列的列间留白已按目标图像素收口；最新静态扫描中八列边界与目标区间一致。
- 右栏五张功能卡的状态徽标已统一保留约 `60px` 固定几何并居中短状态文字；该调整不改变 RTMP、虚拟摄像头、麦克风、固定话术和抖音的真实状态投影。
- 顶部操作带的 7 个动作按钮已改用 WPF 原生 `Path` 线性图标与文本组合，去除依赖字体字形的占位图标；按钮尺寸、事件和自动化名称保持不变。
- 下滑声音快照的规则控制行已按目标图收口为 `70/164/91/164` 列节奏，下拉框与重新生成按钮高度分别为 `34/36px`；控件仍保持禁用态，未改变规则编辑接入状态。
- 首屏视频 8 个指标当前不再显示装饰图标，统一使用共享的紧凑标签/值样式；结果仍是系统生成的只读投影，不代表真实采样或效果已接入。
- 预览头部和底部播放栏的控制图标已统一为 WPF 原生线性 `Path`，原有尺寸、事件、自动化名称和播放绑定保持不变。
- 中央视频快照标题行的状态徽标固定约 `78px × 28px` 并垂直居中，声音/高级视觉续页固定约 `70px` 并居中，视频首屏按目标图增加右侧内缩；徽标内容仍由真实状态投影，未将待接入能力伪装为已生效。
- 视频、声音、场景和设备状态续页的快照徽标现统一复用 `28px` 高度与居中文本样式；仅统一显示基线，不改变各模块的状态来源或待接入语义。
- 中央参数区分类入口已统一显式使用水平/垂直居中、零内边距和像素对齐；视频处理激活态仍使用主色按钮，分类点击事件与隐藏续页逻辑不变。
- 登录门禁的账号和密码输入框已统一为 `36px` 高度/最小高度，修复此前仅按文本行高测量导致的细条显示；账号绑定、密码事件与安全清空逻辑不变。
- 登录区四个操作按钮已统一显式高度和内容垂直居中：主登录按钮 `34px`，刷新会话、离线续播、设备激活提示按钮 `30px`。
- 自绘标题栏的设置、最小化、最大化/还原和关闭按钮已补齐 `WindowChrome.IsHitTestVisibleInChrome=True`，右上角按钮可正常接收点击；窗口生命周期和资源释放逻辑不变。
- 标题栏右侧四个操作按钮已统一增加 `4px` 相邻间距，设置与窗口控制按钮不再紧贴，最右侧边界保持不变。
- 为恢复本地 x64 构建，媒体导入结果到播放池 UI 投影已补齐最小兼容重载；只复用公共快照投影，不改变导入、失败保留旧池或播放池提交语义。
- 播放完成方法中两个同作用域的 `startedAudio` 局部变量已改为不冲突命名，恢复 App x64 编译入口；播放分支和错误处理语义不变。
- 实机窗口复核后已修正参数区启动滚动状态：移除初始化阶段自动 `BringIntoView`，窗口加载时回到视频首屏；用户主动下滑和分类入口定位不变。
- 底部状态带已补齐与设计稿一致的三段分栏：应用状态、丢帧/性能/上行信息和网络状态；左侧生产态复用 `StatusMessage`，性能文字复用已有采样投影，丢帧、上行、网络仍按真实可用性显示占位，不伪造运行数字。
- 下滑声音续页已按像素差异收口为顶部内距 `7px`、底部内距 `8px`、段间距 `5px`、第一排结果卡上间距 `15px`；声音标题、结果卡、高级视觉和播放状态的续页基线已重新验证。
- 右侧“麦克风插话”卡的设置入口已改为右对齐 `34×30px` WPF 线性齿轮图标，保留原禁用态、自动化名称和待接入提示。
- 顶部设置入口与麦克风设置入口已共用 `SettingsGearIconGeometry` 矢量资源，避免字体齿轮受字体和 DPI 影响。
- 右侧 RTMP 卡底部已改为左侧操作、右侧 `188px` 动态状态文案的分栏布局；状态仍由真实校验/推流逻辑投影，未补造网络质量状态。
- 2026-09-04 GUI 控件整改第一轮已落地：`App.xaml` 合并颜色、字体和控件资源字典，清理旧的重复 Button/TextBox/PasswordBox/ComboBox 模板；ComboBox 已接管暗色箭头、Popup、条目高亮、键盘焦点和可读禁用值，按钮、输入框和普通复选框统一基线和禁用态。针对最新截图，顶部“工作区”页签已补回正常边界，左侧播放设置两个下拉框固定为 `34px`，Slider 统一为独立 `6px` 圆角轨道和 `12px` 带边界滑块；右栏输入框保持原卡片高度，避免布局溢出。媒体池移除目标稿不存在的重复“就绪”列，标题栏 Rust 同源 PNG 显示尺寸调整为 `26×26`，并增加像素对齐和高质量缩放。授权态首屏/下滑夹具已重生成并构建通过；真实原生窗口、DPI、键盘和任务栏图标验收未完成，仍不判定为 1:1。详见 [`CSharp-Windows-GUI控件与像素还原整改方案-20260904`](./CSharp-Windows-GUI控件与像素还原整改方案-20260904.md)。

## 未验收门禁

- ZLMediaKit RTMP/RTMPS 的握手、鉴权、断线信号和音画成组恢复。
- PortAudio 真实声卡、拔插、驱动重置、睡眠唤醒、硬超时和可听混音；SAPI 真实语音包/设备输出。
- AkVirtualCamera 的 WGC 可见帧、DirectShow x86/x64、sidecar ACL、签名、许可证、下游兼容和多 GPU。
- 抖音真实 Conda/Python sidecar、扫码、协议兼容、发送、自回显和退出恢复。
- 代码签名证书/时间戳、干净机安装升级卸载、Windows 10/11 矩阵，以及 NoPlayback30m/JointPlayback30m 长稳证据。
- C6 真实性能门禁：100 项媒体池 60Hz 滚动、30 分钟内存/GC 趋势及与 Rust/Tauri 同机原始对照报告。
- C7 C# 自身媒体/输出所有权：C# 使用专属 Mutex 和单实例文件锁，Rust/Tauri 不参与 C# 运行时；当前只剩 C# 自身重复启动、崩溃恢复、权限级别和输出资源回收的实机证据。
- v2 UI 设计验收：中央只读快照、声音/高级视觉下滑续页、规则与结果边界、`已生效` 真实回显、滚动/键盘/DPI/像素级截图仍待实施和验收。

## 时间估算

在测试设备、证书、Runtime 安装权限和外部账号均可用的前提下，剩余代码整合约 5～8 个工作日，真实设备/网络/发布门禁约 10～20 个工作日，合计约 3～5 周。该估算不是验收承诺，外部依赖不可用时顺延。

详细历史记录见 [`CSharp-Windows桌面端实施计划`](./CSharp-Windows桌面端实施计划.md)。
> 2026-09-03：中央参数区已将亮度、对比度、饱和度、色相绑定到 WPF 参数草稿，范围与现有 `MpvVideoEffectSnapshot` 校验契约一致；锐度、伽马、曝光、增益、降噪、白平衡、去闪烁、色彩空间、缩放和裁剪仍禁用并标记为待接入。媒体导入后会按已校验 FFmpeg 资源异步提取 96×54 首帧 JPEG 并回填列表，当前项会同步投影到中央确认预览，失败时保留图标回退。右栏补齐 RTMP 复制/显示/重连、摄像头设备名称/规格、麦克风输入设备标签和固定话术管理/自动循环视觉位。根工作区已按目标图收紧为 8px 栏间距，底部播放栏增加 10px 外边距、当前媒体区和 140px 音量轨道。参数草稿尚未接入 mpv 运行时提交入口，不能标记为媒体效果已生效。

> 2026-09-04 C# 黄金路径修复：媒体资源校验支持显式 `AUTOLIVE_MEDIA_RUNTIME_ROOT` 外置安装根目录，仍必须通过既有 manifest/SHA-256 校验；导入成功与列表选中现在统一提交媒体操作投影，首项、缩略图和池操作按钮保持同步，媒体导入先完成整批 FFprobe，全部成功后才在原子提交前停止输出，探测失败不再打断旧播放；播放中拒绝让 UI 选中项脱离活动输出身份。视频播放现在同时建立 mpv 画面和 FFmpeg PCM/PortAudio 声音链，有音轨视频不再静音；视频处理以真实可验收的 mpv CPU4 `vf set` 完整滤镜链应用四项参数，声音处理以 FFmpeg `-af` 应用增益、三段均衡、倍速、淡入、混响、降噪、相位扰动和颤音。GPU83 已接入四参数基线 shader 资源，完整 83 项 shader、下一帧回显、真实 WPF 登录/首帧/声卡和带音轨视频实机验收仍未完成，不能标记为全部效果已生效。

> 2026-09-04 媒体池提交顺序修复：C# 媒体导入现在先串行完成整批 FFprobe，再通过 WPF 提供的提交前停止回调停止旧输出并原子提交新池；任一候选探测失败时不会提前停止当前播放，旧池和旧输出保持不变。顺序回归测试已通过。

> 2026-09-04 C# 开发启动修复：`tools/start-csharp-development.cmd` 现在要求并注入外置媒体运行时根目录（可传入发布包根目录覆盖默认本地候选）；带音轨视频首次播放若尚未选择输出设备，会在同一受管资源边界内懒加载 PortAudio 设备，优先使用默认输出，驱动未返回默认索引时选择首个已验证输出设备。仍不回退 PATH、不绕过登录/授权、不修改 Rust/Tauri 目录；无可用运行时或声卡时保持失败关闭并显示原因。

> 2026-09-04 音频会话状态修复：重复启动或插话前置条件被拒绝时只返回稳定错误，不再把现有主音频会话污染为 `Failed`；无活动会话的 Pause/Resume 仍保持原有 fail-closed 状态契约。

> v2 设计基准覆盖说明：上述参数草稿记录属于旧 UI 轮次；当前按 v2 设计稿实施时，视频/声音效果结果改为系统生成的只读快照，规则/范围/预设池作为唯一用户可编辑区域。旧实现未删除，待 UI-2/UI-3 迁移时按测试和文档边界收口。

> 2026-09-04 当前参数生成行为补充：本周期快照仍由 C# 本地有界生成器产生；点击“重新生成本周期参数”时，纯音频活动会话和视频 mpv 活动会话会提交当前可用的效果快照。视频声音会读取 mpv 当前位置，仅重建 FFmpeg/PortAudio 会话并保持视频画面；低/中/高 EQ 快照字段使用 dB 单位，与 FFmpeg `equalizer` 输入一致。该补充覆盖此前“重新生成仅更新快照”的旧记录。

> 2026-09-04 实时声音子集补充：C# 的 `FfmpegAudioFilterBuilder` 复用 Rust realtime 音频链中可直接由 FFmpeg 流式执行的倍速、内置滤镜音高微移、淡入、已知源时长淡出、自然动态响度包络、确定性本地音色 EQ、混响、降噪、相位扰动和颤音滤镜；未知时长不加入淡出，需要反向缓冲的复杂变换在当前实时 PCM 管道中明确不加入。WPF 系统快照的混响/降噪、预设 ID 和随机预设周期通过受限映射进入同一 `-af` 链；动态范围/压缩没有对应的正式 C# 参数，不显示为已生效。

> 2026-09-04 视频路径文案修正：中央快照改为显示“按资源选择”，并说明完整 shader 哈希匹配时使用 GPU83、旧/不完整资源时单向回退 CPU4、关闭时为 Original；不再把历史 CPU4 文案当成所有资源包的当前运行路径。声音快照补齐中频 EQ dB 展示，避免模型字段存在但 UI 漏显。

> 2026-09-04 作用域门禁补充：`tools/verify-scope.ps1` 现在同时检查 `desktop/` 下已跟踪和未跟踪路径；当前工作区因已有 Rust/Tauri 源码、共享锁 crate 与 `target-douyin-test` 产物而按约束返回失败。本轮 C# 实施没有修改这些 Rust 路径，也没有通过回滚或删除来掩盖门禁结果。

> 2026-09-04 显式夹具复验：使用 `E:\下载\csharp-golden-av.mp4` 和 `artifacts/csharp-gpu83-real-v2`，C# Windows 真实媒体夹具通过 FFprobe/mpv 完整 GPU83 导入、播放时间、seek、EOF 与初始/更新 shader 参数回读 `2/2`；同一运行包下声音控制器真实 PortAudio/FFmpeg、实时效果、N/N+1 预载切换测试 `11/11` 通过。该结果仍不替代 WPF 原生点击、像素效果、声卡稳定性和远端 RTMP 验收。

> 2026-09-04 视频启动模式修复：视频处理关闭时强制选择 Original；开启时才按完整 shader 哈希在 GPU83 与 CPU4 间选择。首次启动不再把系统生成快照替换成默认 GPU83 baseline，启动与运行时更新统一消费当前已校验的视频参数。新增 `VideoPlaybackModeSelector` 四组合单元测试。

> 2026-09-04 视频声音独立故障修复：mpv 画面启动成功后，PortAudio 设备未选择、资源缺失或声音流启动失败不再关闭 mpv、回滚媒体池或阻断视频播放；WPF 保持媒体池 `Playing`，显示声音输出不可用，后续可重新建立声音会话。新增真实 WPF/外置 mpv 夹具 `1/1`；纯音频仍保持声音输出 fail-closed。

> 2026-09-04 外部夹具与 WPF 层复验：后续外部测试资产固定优先从 `E:\下载` 查找，本次只使用 `E:\下载\csharp-golden-av.mp4`，未读取 `E:\下载\load_log`。按 `artifacts/csharp-gpu83-real-v2` 包根目录注入运行时后，真实 mpv CPU4 `2/2`、GPU83 `2/2`、FFmpeg/PortAudio 音频路径 `2/2` 通过；WPF `VideoPlaybackAudioFallbackTests` 的主窗口导入→FFprobe→原子入池→首项选中→mpv 播放，以及无效 PortAudio 设备时视频保活，共 `2/2` 通过。该证据仍不替代原生人工页面点击、GPU83 像素效果、真实声卡稳定性、远端 RTMP、虚拟摄像头和发布门禁；Rust 端本轮保持只读且未修改。

> 2026-09-04 CPU4 像素效果门禁补充：新增仅在 `AUTOLIVE_TEST_PIXEL_EFFECTS=1` 显式开启的 C# WPF 夹具，使用 `E:\下载\csharp-golden-av.mp4`、最终效果视频 HWND 和现有 WGC/D3D11 GPU→YUY2 受管转换链，对比 Original 与极端 CPU4 参数提交后的最终帧 SHA-256；`VideoEffectPixelFixtureTests` `1/1` 通过，证明“mpv 参数提交→最终 WPF 视频表面像素变化”。该测试保持普通套件隔离，不新增运行时逻辑、播放器或依赖；GPU83 像素变化、目标显卡矩阵、真实人工点击和其他发布门禁仍待验收。

> 2026-09-04 下一帧有效回显接线：C# `MpvIpcProperty.EstimatedFrameNumber` 复用 mpv 的有界帧号属性；视频处理开关和本周期重新生成在播放态提交效果后，会在 2 秒内轮询确认帧号前进，未观察到下一帧则返回 `EffectiveFrameNotObserved`，不把参数 IPC 回读冒充画面已生效。暂停态不等待下一帧，恢复播放后由同一会话继续验证；CPU4 最终 WPF 视频表面像素哈希夹具仍已通过，GPU83 像素变化、目标显卡矩阵、真实人工点击和其他发布门禁仍待验收。

> 2026-09-04 下一帧夹具复验：新增 `MpvIpcProperty.EstimatedFrameNumber` 固定 JSON 契约测试 `1/1`；使用 `E:\下载\csharp-golden-av.mp4`，在 `artifacts/csharp-windows-controller-20260903-v90` 的 CPU4 和 `artifacts/csharp-gpu83-real-v2` 的完整 GPU83 真实 mpv 夹具中，播放态效果更新后确认 `estimated-frame-number` 前进，并验证关闭效果回到 Original，CPU4/GPU83 各 `1/1`。干净环境全量测试重新通过 `494/494`；这仍不代表 GPU83 最终像素、目标显卡矩阵或人工页面验收已完成。

> 2026-09-04 启动首帧门禁复验：`WindowsMpvPlaybackController.StartAsync` 增加可选 `waitForFirstFrame`，主窗口和真实 CPU4/GPU83 夹具在标记 `Playing` 前均确认 `estimated-frame-number > 0`，2 秒超时返回 `FirstFrameNotObserved` 并清理运行时；使用 `E:\下载\csharp-golden-av.mp4` 的 CPU4、完整 GPU83 各 `1/1` 通过。该证据仍不代表 WPF 最终视频表面首帧、GPU83 最终像素、目标显卡矩阵或人工页面验收已完成。

> 2026-09-04 WGC 事件取帧补充：`WindowsGraphicsCaptureWindowSession` 为 `CreateFreeThreaded` frame pool 注册 `FrameArrived`，事件只唤醒专用捕获线程，线程集中排空 `TryGetNextFrame`、执行同设备 D3D11 转换并在停止时取消订阅；事件回调对停止/释放竞态做安全保护。该修复补齐 WGC 会话的事件驱动消费边界，但同机预留子 HWND 仍返回 `ItemUnavailable`，GPU83/WGC 最终像素门禁继续待验收。

> 2026-09-04 WGC WinRT 工厂调用补充：C# `GraphicsCaptureItem` 已通过 activation factory 获取 `IGraphicsCaptureItemInterop`，以严格的 `CreateForWindow` 句柄、目标 IID 和 `ref` ABI 创建捕获项，并释放原始 COM 指针；高版本 `TryCreateFromWindowId` 仍优先使用。该修复已通过格式、构建和 C# 全量测试，但同机预留子 HWND 仍返回 `ItemUnavailable`，所以不把它计为 WGC 像素通过。

> 2026-09-04 WGC 捕获目标修正：虚拟摄像头/WGC 现在绑定最终效果窗口的顶层 HWND，mpv 仍绑定 `ReservedVideoSurface` 子 HWND；未创建第二个窗口或播放器。顶层句柄通过 `WindowInteropHelper` 在窗口显示后读取。随之复跑的可选像素夹具未在有界时间内完成，已中止并清理测试宿主，未计入像素通过；普通 C# 全量测试不受影响。

> 2026-09-04 实时声音预设同步：C# 将系统生成的“随机预设”映射为 `NaturalDynamic` 4 秒低幅度响度包络，并将预设 ID 映射为确定性本地 EQ；新增 2 个滤镜链测试，确认真实消费、同 ID 稳定输出和不引入 `loudnorm`。本轮串行全量测试为 `496/496`，Release 构建为 0 警告/0 错误；动态范围、压缩等没有正式字段对应关系的展示值仍不伪装为已生效。Rust 端保持只读。

> 2026-09-04 C# 桌面端真实 UI 自动化复验：重新打开 `GpAutoLive` 后，通过本机 WPF UI Automation 完成“添加媒体→选择 `E:\下载\csharp-golden-av.mp4`→导入→媒体池显示 1 项→点击播放”；随后分别关闭再开启视频处理和声音处理，播放状态、媒体池和当前媒体均保持稳定。截图为实际导入并播放状态，Codex 原生 CUA 服务不可用，因此不宣称已覆盖人工焦点/DPI/多屏验收。

> 2026-09-04 WGC 窗口隔离门禁复验：新增两个 C# WPF 真实测试，分别用普通可见 WPF 窗口和实际 `FinalEffectWindow` 顶层 HWND 启动、停止 WGC frame pool，`2/2` 通过；该结果确认 WGC 会话、WinRT 工厂调用和顶层窗口目标边界可工作，但不证明 mpv 挂载到预留视频子 HWND 后的最终像素。预装 WGC 的实验性像素夹具改动已撤回，避免保留无界等待；串行全量测试为 `498/498`，Rust 端保持只读。

> 2026-09-04 C# 运行时链复核：对已打开的 C# `GpAutoLive` 实例通过受管 mpv 命名管道读取 `estimated-frame-number`、`time-pos`、`pause` 和 `path`，确认当前源为 `E:\下载\csharp-golden-av.mp4` 且处于播放态；实际进程命令行包含 `--vo=gpu-next`、完整 C# 资源包的 `gpu83.hook`，并观察到 FFmpeg 子进程消费增益、周期响度包络、确定性 EQ、倍速、降噪等 `-af` 链。该证据确认 C# 控制器到 mpv/FFmpeg 的运行时链路真实接通，不把它当作 GPU83 最终像素、真实声卡或 WGC 下游通过。

> 2026-09-04 WGC D3D11 设备标志补齐：C# `WindowsGraphicsCaptureWindowSession` 创建 D3D11 设备时同时使用 `D3D11_CREATE_DEVICE_BGRA_SUPPORT` 与 `D3D11_CREATE_DEVICE_VIDEO_SUPPORT`，与 Rust 只读参考的 WGC 设备能力声明对齐；不新增依赖、不修改 Rust。当前版本格式检查通过、Release 构建 `0` 警告/`0` 错误；非 App 测试项目 `443/443` 通过，App 测试按稳定分组 `53+1+1=55/55` 通过，合计 `498/498`。连续运行两个真实视频窗口用例仍存在 WPF 测试宿主收尾挂起风险，两个用例分别单独运行均已通过，不将该风险冒充为业务失败。

> 2026-09-04 媒体池替换边界修复：`MediaPoolOwner.ReplaceAll` 不再错误叠加旧播放池长度，已有 100 项时重新导入 1 项可按替换后长度原子提交；`Append` 仍按旧池与新候选总数限制 100 项。新增回归测试覆盖该导入黄金路径，失败测试先复现后修复。

> 2026-09-04 主任务复验：媒体池回归测试 Core `77/77`、D3D11/WGC Windows 测试 `210/210`，Contracts `28/28`、Installer `15/15`、Media `115/115`；App 常规 `53/53` 与两个真实窗口用例单独 `1/1+1/1` 通过，稳定分组总计 `500/500`。C# Windows 项目 Release 构建 `0` 警告/`0` 错误，Rust 端保持只读。

> 2026-09-05 WGC 输出上下文接线修复：发现生产路径此前只提交 `VirtualCameraOutputManager` 的 GPU/WGC 帧，却没有由主窗口同步播放状态、视频源和暂停/停止事实，sidecar 输出泵因此始终按默认上下文发送黑帧；同时首次成功 GPU→YUY2 回读没有把 `HasValidFrame` 设为真。现在媒体快照统一同步输出上下文，WGC 会话在首个成功回读后标记有效帧，并在会话重启/停止时清除旧帧事实。新增 C# 接线门禁测试；本轮未修改 Rust，真实 sidecar/DirectShow、WGC 最终像素和设备发布门禁仍待验收。

> 2026-09-05 C# WGC/GPU 输出边界修复：GPU YUY2 打包全屏三角形的右下顶点修正为 `float2(3.0, -1.0)`，采样改为与 Rust 参考一致的源像素半偏移和 U/V 平均，并新增硬件夹具覆盖右下角及源纹理左上角；WGC GPU 输出会话在启动成功前等待首个“转换并提交”的真实帧，超时/取消/捕获失败均回滚到 `Installed`，避免 writer 在没有有效帧时永久发送黑帧；三槽回读按序列选择最新完成帧，独立释放 WinRT/D3D11 资源。当前硬件测试和首帧失败回滚测试已通过；真实 WGC 可见帧、sidecar/DirectShow、下游兼容、目标 GPU 矩阵和长稳仍保持待验收，状态仍为“代码已接入·待验收”。

> 2026-09-05 开发启动入口修正：`tools/start-csharp-development.cmd` 默认优先使用已校验的 `csharp-gpu83-real-v2` 完整 shader 运行包，缺失时回退 `csharp-windows-controller-20260903-v90`；显式传入运行包根目录的行为不变。运行时仍必须通过既有 manifest/SHA-256 校验，未改变登录、授权和 fail-closed 门禁。

> 2026-09-05 最终效果窗口呈现收口：`FinalEffectWindow` 继续作为唯一独立播放窗口，保留 `Owner`、mpv 子 HWND 和 WGC 顶层 HWND 绑定；仅关闭其独立任务栏按钮（`ShowInTaskbar=False`），避免同一 C# 进程被看成两个桌面端。新增 WPF 回归测试覆盖该呈现边界，不改变播放、媒体池、登录授权或输出资源生命周期。

> 2026-09-05 未接入入口明确化：顶部 `整理媒体`、`导入列表`、`保存配置`、`加载配置` 目前没有正式业务契约，已明确禁用并增加说明；可用媒体入口仍为媒体池 `导入媒体`、顶栏 `添加媒体` 和系统拖放。没有把 PDF/DOCX 或未定义的列表文档格式伪装成可导入能力。

> 2026-09-05 WGC 生命周期收口：每个 WGC worker 绑定独立停止信号，worker 退出并完成 WinRT/D3D11 清理后才释放该信号，避免停止超时后的 `DisposeAsync` 竞态；虚拟摄像头停止失败现在返回 `CaptureFailed`，不再伪装为 `Stopped`。受影响 Windows 测试 `5/5` 通过；真实 WGC 帧、sidecar/DirectShow 和发布门禁仍未验收。

> 2026-09-05 媒体换源与声音分流收口：mpv 同进程换源现在在返回成功前有界确认目标 `path`、非 EOF、播放意图和播放态首帧；暂停态换源会重新保持暂停，确认失败清理当前会话。`FinalPcmBus` 默认跳过未接入 RTMP 的缓冲分支，RTMP 会话创建/停止显式绑定/解绑并清除旧尾部，候选总线继承绑定状态。声音开关由 `ShellState.PropertyChanged` 统一触发重配置，不再只依赖 WPF Click；当前实时 `-af` 子集保持不变，动态范围/压缩/音色展示仍待正式字段与算法接入。媒体总线定向测试 `19/19`、真实 mpv 换源 `1/1`、真实 FFmpeg/PortAudio 消费 `1/1` 通过；直接双击未携带 `runtime/media/<version>` 的开发 EXE 仍按设计 fail-closed，应使用发布包或 `tools/start-csharp-development.cmd`。

> 2026-09-05 视频周期触发接入：新增基于真实 mpv 播放位置的 `5–8s` 有界视频效果周期规划器，播放中且视频处理开启时复用现有串行播放命令自动生成并提交下一轮视频快照；缺失位置/时长、暂停、回退、切源和关闭处理均不伪造触发。没有新增播放器、线程、缓存或捕获链；声音不因周期触发而重启 PortAudio，继续使用连续 `NaturalDynamic` 链。规划器 `2/2`、App 稳定测试 `53/53`、Release 构建和格式检查通过；自动声音 N/N+1 周期、GPU83 最终像素和真实 WPF 宿主复验仍待验收。

> 2026-09-05 声音周期候选接入：新增 `AudioEffectCyclePlanner`，以 PortAudio `timeInfo` 投影的可听位置为事实源，在目标位置前约 `1s` 预载同源新参数，并通过现有最终 PCM 总线按输出帧目标提交唯一 N+1 候选；PortAudio/RTMP 分支不新增输出流，旧 FFmpeg 解码器在切换后取消、Join 并释放。目标帧计算包含设备延迟，暂停会丢弃未提交候选；视频源声音和纯音频源共用同一观察器。声音滤镜会继续进入候选 FFmpeg `-af` 链，但真实声卡可听周期、RTMP/ZLMediaKit 和长稳仍待验收，Rust 端保持只读。媒体切换测试 `7/7`、App 声音/视频规划与状态 `16/16`、Windows 音频/时钟稳定筛选 `16/16`、App 稳定测试 `55/55`、Release 构建 `0` 警告/`0` 错误、格式检查通过。

> 2026-09-05 声音周期真实夹具：在显式环境门控下使用 C# 已校验运行包的真实 FFmpeg/PortAudio 和 8 秒正弦音频，新增周期候选夹具 `1/1` 通过；同一 PortAudio 输出流在约 4 秒目标切换到同源新参数候选，初始/周期最终 PCM 总线均发布帧且会话正常完成。该结果不替代真实声卡长期稳定、RTMP/ZLMediaKit、用户主观听感和长稳门禁，Rust 端保持只读。

> 2026-09-05 RTMP 状态与视频效果桥接：C# RTMP 声音会话现在拥有并观察 FFmpeg PCM 分流泵/解码生产任务；异常会固定投影 `PumpFailed`/`DecodeFailed`、取消会话并有界停止宿主，正常停止取消不产生假失败。受管 RTMP 进程终态通过脱敏事件刷新 WPF，失败状态启用有限重连入口；当前默认预算已与 Rust 对齐为初次启动后最多 5 次重试、退避 `1/2/4/8/15s`。RTMP 画面发布在视频处理开启时接收已校验 CPU4 四项快照并生成固定 `eq + hue` `-vf`，关闭处理不生成 `-vf`；不把 mpv 标签、任意滤镜或 GPU83 参数当作 FFmpeg 直推实现。RTMP 命令计划测试 `7/7`、Windows 受影响测试和 Release x64 构建以本轮记录为准；真实 ZLMediaKit/RTMPS、远端握手、重连结果和长稳仍待验收，Rust 端本轮保持只读。

> 2026-09-05 WGC 句柄就绪与虚拟摄像头前置门禁：最终效果窗口的 mpv 视频子 HWND 与 WGC 顶层 HWND 现在统一校验窗口有效、可见且客户区非零，窗口显示后先完成布局再刷新输出绑定；虚拟摄像头只有在当前源为视频、播放状态为播放/暂停、mpv 活动身份一致且运行时已进入 `Running` 时才允许启动。新增关闭最终效果窗口后拒绝捕获句柄的 WPF 回归测试，WPF/WGC 隔离测试 `3/3` 通过。该门禁不把 WGC `Running` 冒充有效像素，真实可见帧、sidecar/DirectShow、GPU83 最终像素、目标 GPU 矩阵和发布门禁仍待验收；Rust 端保持只读。

> 2026-09-05 多媒体池 EOF 黄金路径：新增真实 WPF 夹具，使用 `E:\下载\csharp-golden-av.mp4` 与独立临时副本，经主窗口导入入口原子加入两项媒体池，启动第一项后等待 mpv EOF，确认媒体池自动进入第二项且保持 `Playing`；该测试 `1/1` 通过。未新增播放器、观察线程或绕过登录/运行包校验；人工点击、GPU83 最终像素、真实声卡/RTMP、WGC 下游和长稳仍待验收。

> 2026-09-05 WGC surface 解包修复：诊断确认 WGC 已收到原始帧，但旧 C# `Marshal.GetIUnknownForObject(frame.Surface)` 二次 RCW 路径逐帧返回 `SurfaceUnavailable`；现改为从 WinRT `IObjectReference` 直接取得 `IDirect3DDxgiInterfaceAccess`，并将 WGC D3D11 设备改为枚举非 Software/Remote DXGI adapter 后以 `D3D_DRIVER_TYPE_UNKNOWN` 创建。显式 GPU83 原生 Win32 窗口最终 YUY2 像素哈希夹具 `1/1` 通过；Windows 全量 `217/217`、App 全量 `62/62`、格式检查和本机 x64 Release 构建（0 警告/0 错误）通过。实际 WPF `FinalEffectWindow` 子 HWND 挂载、AkVirtualCamera sidecar/DirectShow、目标 GPU 矩阵、真实声卡/RTMP、人工页面和长稳仍待验收，Rust 端保持只读。

> 2026-09-05 WPF GPU83 最终表面闭环：新增显式环境门控的 `VideoEffectPixelFixtureTests.Explicit_wpf_fixture_confirms_gpu83_changes_final_video_frame`，使用真实 C# 导入→媒体池→播放入口，在完整 `csharp-gpu83-real-v2` 运行包下将 GPU83 绑定到实际 `FinalEffectWindow` 顶层 HWND，通过 WGC/D3D11→YUY2 回读比较参数更新前后帧哈希；GPU83 `1/1`、CPU4 对照 `1/1`、App 全量 `63/63`、App 格式检查通过。该证据确认 WPF 最终表面像素已变化，但不替代目标 GPU 矩阵、AkVirtualCamera sidecar/DirectShow、真实声卡/RTMP、人工页面和长稳门禁。

> 2026-09-05 导入列表文档接入：顶部“导入列表”现在读取严格版本化 JSON（`schema_version: 1`、`data.items[].path`），只接受本地绝对媒体路径和现有扩展名白名单，拒绝未知字段、重复/空/超限条目；成功后复用现有 `RunImportAsync`、FFprobe 和 `ReplaceAll` 原子提交，列表文件不携带运行时元数据、效果参数、凭据或 RTMP 地址。Core 全量 `81/81`（媒体列表读取 `4/4`）、App 全量 `79/79`（入口回归 `3/3`）通过；人工页面点击、真实设备和长稳仍待验收，Rust 端保持只读。

> 2026-09-05 Windows 集成状态文案收口：`WindowsCapabilityBoundary.Status` 已从“正式需求·待实施/未接入”改为“代码已接入·待验收”，避免主窗口把已经接入的媒体播放与 WGC 能力显示成未接入；该文案不代表真实设备、下游兼容、签名或长稳已通过。App 全量测试 `63/63`、Windows 格式检查通过。

> 同一修复后的 WPF `FinalEffectWindow` CPU4 最终视频表面像素夹具重新通过 `1/1`；这与原生 Win32 GPU83 `1/1` 一起确认 C# WGC→D3D11→YUY2 转换链已能产出并区分真实像素。GPU83 在 WPF `FinalEffectWindow` 子 HWND 挂载下的独立像素门禁、AkVirtualCamera sidecar/DirectShow、目标 GPU 矩阵、真实声卡/RTMP、人工页面和长稳仍待验收。

> 2026-09-05 C# 声音实际消费链复审：`AudioEffectParams` 中已接入的增益（输入/输出/响度合并）、200/1000/8000Hz EQ、倍速、内置音高、淡入/已知时长淡出、混响、降噪、相位、颤音、`NaturalDynamic` 和本地预设均由 `FfmpegAudioFilterBuilder` 生成，并由 `FfmpegPcmDecodePlanBuilder` 通过单个受管 `-af` 参数传给 FFmpeg。修正非空空白 `voice_library_id` 被 `IsNullOrWhiteSpace` 静默忽略的问题，失败先行回归测试已保留。`DynamicRangeDb`、`Compression`、`Tone` 仍不是正式 C# 参数字段，不映射为近似滤镜。声音模块测试 `121/121` 通过，Rust、App 主窗口和其他声音消费代码未修改。

> 2026-09-05 媒体导入/输出生命周期收口：`RunImportAsync` 在请求工厂完成后取得共享 `_playbackCommandSerial`，覆盖运行包、FFprobe、停止旧输出和媒体池原子提交，避免探测期间播放/停止/换源交错；导入取消、运行包失败或任一探测失败继续保留旧池。用户关闭 `FinalEffectWindow` 时复用 `StopMediaForMutationAsync`，成功路径统一停止 RTMP、虚拟摄像头、观察者、插话、PortAudio、mpv 和媒体池，并同步实际池快照；主窗口关闭仍走既有最终 Dispose 路径。`WindowsRtmpAudioSession` 的共享 PCM 链改用 `FinalPcmBus.Channels` 构造解码计划、混音器和分流泵，单声道共享总线不再被固定双声道拒绝。Windows 全量 `224/224`；新增关闭窗口、导入闸门和单声道 RTMP 回归测试均通过。App 全量串行尝试 `80/81`，唯一失败是顺序运行下真实 WPF/mpv 夹具的 90 秒宿主超时；两个真实 mpv 夹具隔离均 `1/1`，因此 App 全量不标记为通过。未修改 Rust、未新增第三方依赖。

> 2026-09-05 C# 单实例进程门禁修复：冷启动实测发现旧命名 Mutex 仍允许第二个 `GpAutoLive.exe` 进入可见窗口；新增 `WindowsSingleInstanceLease`，使用 `%LocalAppData%\GpAutoLive\locks\csharp-instance.lock` 的 `FileShare.None` 独占句柄作为 C# 进程边界。`App.OnStartup` 先取 C# 单实例租约，再取 C# 专属媒体/输出租约；第二次启动直接退出。锁定 SDK 下单实例回归 `1/1`、Release x64 构建 `0` 警告/`0` 错误；冷启动 A/B 为 A 保持运行、B 已退出、进程数 `1`。主控窗口与 `FinalEffectWindow` 仍是同一进程的两个窗口，未新增播放器，Rust 端未修改。真实声卡、远端 ZLMediaKit/RTMPS、AkVirtualCamera 下游、人工页面和长稳仍待验收。

> 2026-09-05 声音启动与总线背压修复：音频控制器不再把“已创建 PortAudio/FFmpeg 会话”直接当作启动成功，而是在 3 秒有界预算内确认首批 PCM 已写入目标；取消、无 PCM、解码提前结束均返回稳定错误并执行有界收尾。最终 PCM 总线的 RTMP 环缓继续按自身固定容量丢弃最旧帧，不再因 RTMP 泵暂时未读而反向阻塞本机声音；显式真实夹具验证循环/暂停/恢复/有限总线及带音轨视频效果消费 `2/2`，Windows 全量 `217/217`。

> 2026-09-05 C# 主窗口导入/播放/媒体池路径复审：`MainWindow.MediaPool.cs` 的登录门禁在请求工厂和运行包初始化前返回，未授权状态明确显示“请先完成登录与设备授权”；运行包清单或 FFprobe 缺失时保留脱敏失败原因，并追加“请安装或修复 C# 媒体运行包后重试”的操作提示。新增 `VideoPlaybackAudioFallbackTests` 的未授权导入、缺失 manifest 保留旧池/操作提示边界；强化 EOF 夹具从真实 `RunImportAsync` 经已校验 manifest、FFprobe、原子入池和 `Ready` 启动后，等待主窗口“已自动切换到第 2 项”，再确认媒体池 `Playing`、mpv 活动身份一致且控制器/运行时为 `Playing/Running`。该测试类 `5/5`、App 全量 `65/65`、App 构建 `0` 警告/`0` 错误、两个 App 项目格式检查和 `git diff --check` 通过；没有把旧 IPC 命令接收成功或单独的池索引变化冒充换源完成。Rust、Media/Windows 生产层和其他文件未因本次审查修改；人工页面点击、真实声卡长期稳定及发布包/目标设备门禁仍待验收。

> 2026-09-05 mpv IPC 故障状态收口：`WindowsMpvPlaybackController` 现在把“IPC 已关闭但 mpv 进程仍存活”投影为 `Faulted`，`ShutdownAsync` 不再静默吞掉底层停止失败；新增真实故障注入与暂停态换源/恢复/停止夹具，分别为 `1/1`、`1/1`，控制器基线 `6/6`，Windows 格式检查通过。暂停换源测试在显式 Dispose 后再清理临时源文件；真实连续 EOF、多 GPU、设备和长稳仍待验收。

> 2026-09-05 RTMP 启动/停止资源收口：`WindowsRtmpOutputManager` 在停止超时且未确认进程退出时保留 PID、Job、stdin 和 `Stopping` 状态，允许后续重试；`WindowsRtmpAudioSession` 分别观察 Producer/Pump，启动期 `PumpFailed/DecodeFailed` 不再返回成功，宿主停止失败时不清空仍可重试的会话资源，Dispose 只有成功回收后才释放生命周期信号。RTMP 管理器/声音会话 `14/14`、Windows 全量 `220/220`、格式检查和本地 x64 Release 构建通过；真实 ZLMediaKit/RTMPS、远端恢复、真实声卡、三轨道和长稳仍待验收。

> 2026-09-05 媒体池忙碌态按钮投影修复：复现 WPF `ItemsSource` 尚未完成绑定时，导入忙碌态结束后 `ListBox.SelectedIndex=-1` 使“下移”按钮继续禁用；现由 `SetMediaMutationButtonsEnabled` 在非空池的短暂绑定窗口回退到媒体池当前索引，随后仍由真实列表选择驱动。失败断言补充了 `selected/items/stateItems` 证据，修复后目标 App 测试通过；未修改 Rust、声音滤镜、WGC 或单实例文件。

> 2026-09-05 导入入口与 GPU83 回读兼容收口：修复底部主导入按钮仍为 `Collapsed` 的 UI 缺口；授权后底部与顶栏入口均可见、可用并进入同一 `RunImportAsync`，未授权门禁保持不变。`MpvShaderOptionsSnapshot` 兼容 mpv 的 `glsl-shader-opts` 字符串回读，同时保留固定键集合和额外键 fail-closed。授权入口测试、字符串/数字/额外键回读测试均通过；App `66/66`、Media `122/122`、Release x64 构建 `0` 警告/`0` 错误、App/Media 格式检查通过。真实页面点击、目标 GPU、声卡/RTMP、虚拟摄像头下游和长稳仍待验收。

> v1.66（2026-09-05）RTMP 草稿撤回与 mpv 播放池门禁：主线程复审后撤回未形成闭环的 RTMP stdout 进度字段、读取任务、命令行参数和对应测试；RTMP 当前继续以受管进程、stderr、最终 PCM 分流、停止重试和脱敏状态为准。修正无音频 stdin 停止路径只在实际取得 `_audioSerial` 后释放锁，避免无音频会话污染后续音频生命周期。新增真实 mpv 连续 EOF→多项换源→新源实际播放和单项循环身份测试；Windows `222/222`、Media `123/123`、App `66/66`，Media RTMP 命令计划 `6/6`、Windows RTMP 管理器 `6/6`，格式检查和启动脚本本地 Release x64 构建 `0` 警告/`0` 错误通过。真实页面点击、真实声卡、远端 ZLMediaKit/RTMPS、AkVirtualCamera 下游、目标 GPU 矩阵、签名和长稳仍待验收。

> v1.67（2026-09-05）C# 音视频参数真实消费与声音故障恢复：`GeneratedAudioEffectSnapshot.ToAudioEffectParams()` 现在把频域扰动、频谱盲区和高频扰动正式字段送入 `AudioEffectParams`，由 FFmpeg `-af` 生成受限频域链；动态范围、压缩和音色展示值没有正式字段对应关系，继续不映射。CPU4 启动滤镜与运行时 `vf-command` 共享固定 `eq@autolive_cpu4_eq`、`hue@autolive_cpu4_hue` 标签，修复 IPC 成功但参数未命中已安装滤镜的问题。视频声音首次启动失败后，暂停/恢复按 mpv `time-pos` 有界重试 PCM→PortAudio，恢复失败仍保持画面并显示声音不可用。Windows `222/222`、Media `126/126`、App 稳定分组 `57/57`、`VideoPlaybackAudioFallbackTests` `6/6`（声音故障暂停/恢复夹具单独 `1/1`），三个项目格式检查、同步夹具 `7` 个 JSON 和启动脚本本地 Release x64 构建均通过（0 警告/0 错误）。App 全量串行复跑在 WPF 多真实窗口收尾阶段超过两分钟无输出，未计入通过；有效声卡下的恢复成功、人工页面点击、容器健康、外部模型、真实 ZLMediaKit/RTMPS、AkVirtualCamera/DirectShow、目标 GPU 矩阵、签名和长稳仍未验证，Rust 端继续只读。

> v1.68（2026-09-05）C# WPF 真实播放测试串行隔离与媒体池门禁收口：媒体池导入、拖放、排序、移除和清空统一增加登录、关闭和导入忙碌门禁；导入/运行包失败时投影实际 `_mediaPool.Snapshot`，保留旧池，不用失败结果伪造新池。真实视频夹具关闭前显式停止既有播放链；App 测试程序集加入 `[assembly: DoNotParallelize]`，解决共享 WPF Dispatcher、环境变量、HWND 和 mpv 资源的默认并行竞态。`FfmpegPcmDecodePlan` 对没有实时消费者的音频字段、未知时长淡出和采样率不一致 fail-closed；GPU83 对 source fps、shader key 和已知未消费字段收紧白名单。Windows `222/222`、Media `131/131`、App 稳定分组 `60/60`、App 全量串行 `72/72`，真实 `VideoPlaybackAudioFallbackTests` `6/6`。未执行容器健康、人工页面点击、外部模型、有效声卡恢复成功、真实 ZLMediaKit/RTMPS、AkVirtualCamera/DirectShow、签名、目标 GPU 矩阵和长稳验收，Rust 端继续只读。

> v1.71（2026-09-05）C# Rust 对齐的自动视频周期参数接线：`GeneratedVideoEffectSnapshot` 保留旧 UI 投影和未接入字段语义，同时生成正式 `VideoEffectParams` 与 `AdvancedEffectParams`；生成规则对齐 Rust `sample_automatic_video_parameters` 的已验证范围。`MainWindow.Effects` 的完整 GPU83 路径传入当前周期高级参数，不再固定传 `AdvancedEffectParams.Default`；CPU4 仍只消费亮度、对比度、饱和度和色相四项。新增映射测试；App 全量串行测试 `74/74`、本机 Release x64 构建和格式检查通过。目标显卡矩阵、真实声卡、远端输出、下游虚拟摄像头和人工页面仍未验收；Rust 只读。

> v1.72（2026-09-05）C# 媒体拖放与音频启动计划边界收口：`MediaDropPayload.HasCandidateFiles` 复用现有媒体扩展名白名单，预览阶段拒绝空路径、目录形态、不支持扩展名和超限候选，不触碰文件 I/O；真实 FFprobe 探测、原子入池和旧池保护仍由 `MediaImportCoordinator` 负责。`WindowsAudioPlaybackController.StartAsync` 在创建输出前校验计划声道与输出采样率，不一致返回 `invalid_plan`，避免声音开关状态与实际输出链路分叉。Windows 全量 `223/223`、Media 全量 `131/131`、App 全量串行 `79/79`，拖放边界 `7/7`、音频控制器边界 `14/14`，格式检查、本机 x64 Release 构建和显式 WPF GPU83 最终表面像素夹具均通过；真实声卡、远端输出、下游虚拟摄像头、人工页面和长稳仍待验收，Rust 只读。

> 2026-09-05 声音开关异步竞态修复：`ShellState.AudioProcessing` 增加仅用于异步重配置判定的单调版本；`MainWindow.Effects` 在读取视频位置、停止旧会话、启动新 FFmpeg/PortAudio 会话、暂停同步和候选准备的边界拒绝过期请求。连续切换时旧请求不会用最新值误启动错误滤镜，已启动的过期会话会沿现有停止/Join 路径收尾，队列中的最新开关继续完成重配置；纯音频与视频音频仍共用一个 `WindowsAudioPlaybackController` 和一个 PortAudio 输出链。失败先行的版本测试、纯音频切换和视频切换测试 `3/3` 通过；Windows 音频专项筛选 `58/58` 通过。真实声卡听感、设备拔插/睡眠恢复、RTMP 远端和 30 分钟长稳仍未验收，未修改 Rust 或弹窗 Features/Playback 文件。
> 2026-09-05 RTMP 最终 PCM 首帧门禁：`WindowsRtmpAudioSession.StartAsync` 现在必须在有界 3 秒预算内观察到分流泵已向 FFmpeg 转发至少一帧最终 PCM，才返回成功；泵失败或无首帧会停止会话并返回 `PumpFailed`，避免 UI 将“泵已启动”误报为“声音已消费”。共享总线无生产者的测试夹具已改为显式提供单声道 PCM；真实 ZLMediaKit、声卡和网络稳定性仍待验收。

> 2026-09-05 C# 音频字段消费边界复核：`FfmpegAudioFilterBuilder` 已将音高微移调整到播放速度之前，与 Rust 音频链保持一致；高频扰动的开关、间隔、强度和目标电平继续经 `AudioEffectParams` 进入同一受管 FFmpeg `-af`。`DynamicRangeDb`、`Compression`、`Tone` 没有正式 C# 参数与算法对应，继续只读展示/未接入，不使用近似压缩或 EQ 冒充生效。失败先行顺序测试修复后通过；本轮不修改 App 快照、Rust、播放器、线程或队列。

> 2026-09-06 真实局域网 RTMP 传输烟测（状态进度门禁接入前的历史记录）：使用用户提供的 `D:\xz\8d020eb133350a74bbc4daec1f33bbc1.mp4`，沿用已校验 C# 外置媒体运行包 v90 的 FFmpeg，以 `192.168.10.22:1935`、临时 `live` 应用/流发布 60 秒；远端回读确认 H.264 `1280×720@30fps` 和 AAC `48000Hz/2ch`，发布自然退出码 `0`，停止后流不可回读，且无本次测试进程残留。随后直接调用当时版本的 C# `WindowsRtmpOutputManager.StartAsync` 进行画面-only 真实发布，返回成功并选择 `h264_amf`，远端回读确认 H.264，`StopAsync` 成功收敛到 `Idle`。该证据覆盖真实服务器、标准端口、C# 画面发布管理器和 H.264/AAC/FLV 传输，但不覆盖 WPF 页面点击、C# 最终 PCM 分流、远端握手状态投影、断线恢复或长稳；当时管理器仍显示 `Starting`，不伪装成已确认 `Publishing`。当前实现已新增正向进度门禁；桌面控制 RPC 未配置，真实 C# WPF 全轨推流门禁继续保持待验收，Rust 端保持只读。

> 2026-09-06 AkVirtualCamera sidecar 活动写入取消收口：修正 `WindowsVirtualCameraSidecarClient.WriteFrameAsync` 在固定帧活动写入期间收到取消时仍保留已连接 Named Pipe 的缺口；现在会关闭当前管道、归还池化帧缓冲，并返回 `Cancelled`、`Retryable=true`、状态 `Failed`。失败先行回归 `Cancellation_during_write_closes_pipe_and_enters_retryable_failure_state` `1/1` 通过；该修复只收紧管道和资源生命周期，不新增重连、进程存活或像素交付假设。真实 sidecar/DirectShow、当前用户 ACL、GPU→sidecar 连续帧、签名/许可证、目标 GPU、下游兼容、人工页面和 30 分钟长稳仍待验收，Rust 端保持只读。
