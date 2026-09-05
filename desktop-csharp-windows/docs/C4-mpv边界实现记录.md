# C4 mpv 边界实现记录

## 状态

当前状态：纯逻辑、Windows 命名管道传输与受管宿主组合代码已接入·待真实 mpv 实机验收。

本记录只覆盖 C# Windows 侧的纯逻辑契约，不代表 mpv 已启动，也不代表 GPU83、CPU4 或 Original 已经在真实媒体画面中生效。

## 已实现

- `MpvIpcCommand` 只生成固定命令：`loadfile replace`、暂停/恢复、播放速度、绝对 seek、GPU83 参数、CPU4 固定滤镜、允许列表属性读取和退出。
- 所有 IPC 请求都使用 JSON 对象和递增调用方提供的非零 `request_id`；不拼接 shell 命令，不接受任意命令名、脚本或滤镜字符串。
- 发送命令行和 GPU83 `glsl-shader-opts` 快照均有字节上限；CPU4 仅接受亮度、对比度、饱和度和色相四个固定参数及产品范围。
- 响应帧严格限制根对象和字段，区分成功、属性暂不可用、命令拒绝、request_id 不匹配、未知字段和 malformed JSON；错误正文不回显 mpv 原始错误文本。
- `MpvActiveSource` 只能从视频 `SourceMediaDto` 和 Core 的 `MediaPlaybackIdentity` 创建；纯音频不会绑定 mpv 视频会话。
- `MpvPlaybackSession` 保留一个活动源，替换源会使旧 generation/revision/index/loop 请求失效；迟到响应 fail-closed。
- 参数快照替换是整批提交，返回固定 IPC 命令序列；`record with` 绕过创建入口时，在生成命令前仍会再次校验。
- `MpvIpcPipeEndpoint` 和 `MpvNamedPipeClient` 已提供 Windows 命名管道传输边界：单连接所有者、串行请求、有限帧读取、超时/取消、错配 fail-closed 和脱敏关闭。
- `MpvPlaybackIpcGateway` 已把会话身份与命名管道组合为单一发送入口；它只分配 request_id、发送固定命令并复验响应归属，不启动进程。
- `WindowsMpvProcessHost` 只消费不可变 `MpvLaunchPlan`，以隐藏窗口启动 mpv，优先使用 Job Object 回收进程树，并对立即退出、取消和停止超时返回脱敏状态。
- `WindowsMpvPlaybackRuntime` 按“启动宿主→连接同一管道→固定命令分发→quit→释放管道/进程”的顺序组合三个边界；它不重复发送启动计划中已经带入的首个媒体源。
- `WindowsMpvPlaybackController.WatchPlaybackStateAsync` 已把 Media 状态观察转发到 WPF；视频启动后由窗口消费当前身份的 `eof-reached`，EOF 后在同一 mpv 进程内换源或切换到声音会话。
- 外置运行资源白名单已包含 `d3dcompiler_43.dll`；IPC 事件解析已覆盖 mpv 文档列出的公共/事件专属字段，仍对响应帧保持未知字段拒绝。

## 边界与未接入

- 常规自动化测试仍使用 Windows 自带 `cmd.exe` 或本地管道模拟；新增显式 `WindowsMpvRealFixtureTests` 后，在提供外置资源、短视频和 Windows HWND 的机器上已真实验证 `Original/Cpu4/Gpu83` 三种模式的命名管道播放时间与 EOF。命名管道客户端仍只连接由宿主创建的管道，不自行拼接播放器命令。
- `loadfile` 的路径已通过现有绝对路径和扩展名策略校验；WPF 已接入 EOF 观察和 HWND 传入，真实文件首帧、连续 EOF/换源和健康指标仍待实机门禁。
- GPU83 的 shader 选项只完成安全快照与命令形状，尚未接入 libplacebo、shader 资源校验、GPU 能力探测和 GPU83 → CPU4 → Original 单向健康降级。
- CPU4 命令只完成固定滤镜契约，尚未在真实 mpv 中验证滤镜安装、切换和参数生效。
- PTS/FPS 读取、音画纠偏、PortAudio、真实 seek/换源和 30 分钟长稳属于后续 C4 实机门禁。

## 验证

`GpAutoLive.Media.Tests` 覆盖：

- shader 键值白名单、重复项、非有限数值和总大小限制；
- mpv JSON 命令形状、时间戳/速度上限；
- 成功响应、属性不可用、request_id 不匹配、未知字段和事件帧；
- 单活动源替换后的迟到响应拒绝；
- 会话—传输网关的当前身份成功响应和切源后迟到响应拒绝；
- GPU83/CPU4 整批参数命令数量和 CPU4 数值映射。

纯逻辑测试不启动真实 mpv；Windows 宿主测试使用受控本机进程验证生命周期，命名管道测试使用本地 `NamedPipeServerStream` 模拟端，不复制媒体资源。详细传输边界见 [`C4-mpv命名管道运行时边界.md`](./C4-mpv命名管道运行时边界.md)。

最新 v44 回归后，全量自动化测试为 267 项通过；该数字仍不代表真实 mpv 首帧、HWND 渲染或目标 GPU 矩阵已验收。
