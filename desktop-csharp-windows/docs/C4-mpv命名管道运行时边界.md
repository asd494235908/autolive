# C4 mpv Windows 命名管道运行时边界

## 状态

当前状态：代码已接入·待真实 mpv 实机验收。

本记录覆盖 C# Windows 端与一个受管 mpv 会话之间的 JSON IPC 传输层，以及进程宿主与 IPC 的组合生命周期。它不表示真实安装包中的 mpv 已通过实机验收，也不表示 GPU83、CPU4、Original、EOF 或 HWND 视频输出已经在真实媒体中生效。

## 实现范围

- `MpvIpcPipeEndpoint` 只接受 `\\.\pipe\<name>` 形式的 Windows 命名管道；管道名称不允许路径分隔符、冒号、空白、控制字符、Shell 元字符或超过 240 个字符。
- `MpvNamedPipeClient` 是单一连接所有者：一个实例只维护一个 `NamedPipeClientStream`，请求通过有界 `SemaphoreSlim` 串行化，避免多个调用方同时读写同一条 mpv 流。
- 连接使用 `ConnectAsync`、有限连接超时和调用方取消；写入使用 `ArgumentList` 生成的固定 JSON 行，不经过 Shell。
- 读取按 UTF-8 字节逐帧、有界换行；默认单帧上限为 64 KiB，超过上限或非法 UTF-8 会关闭连接并返回脱敏错误。
- 已有 `MpvIpcCommand.TrySerialize`、`MpvIpcFrameParser` 和 `MpvPlaybackSession` 继续分别负责命令白名单、严格帧解析及播放身份校验。传输层不创建播放器业务、不修改活动源、不实现音频或 RTMP。
- `MpvPlaybackIpcGateway` 是组合根：为当前会话分配单调 `request_id`，发送固定命令，并在返回前再次调用 `MpvPlaybackSession.AcceptResponse`；切源期间的迟到响应只返回会话错误，不覆盖新源状态。
- `MpvLaunchPlan` 把已验证的 `mpv.exe`、视频路径、宿主 HWND、命名管道和 `Gpu83/Cpu4/Original` 模式收敛成不可变的 Windows 参数数组；固定关闭 Shell/默认配置，限制命令长度，并拒绝无效句柄、路径、起始位置或未校验运行时。它只生成计划，不自行启动进程。
- `WindowsMpvProcessHost` 消费上述启动计划，以 `ProcessStartInfo.ArgumentList`、隐藏窗口和 Job Object 优先策略启动/回收 Windows 进程；进程启动失败、立即退出、取消和停止超时均映射为稳定状态。
- `WindowsMpvPlaybackRuntime` 是 Windows 组合入口：先启动宿主，再用计划中的同一管道创建客户端并连接，随后才允许通过 `MpvPlaybackSession` 身份分发固定命令；停止时在 2 秒清理预算内尽力发送 `quit`，再释放管道和进程树。
- `.ts`、`.m2ts` 已属于媒体白名单；真实 MPEG-TS 样本可通过 FFprobe 探测，并可由 FFmpeg 与 mpv 正常解码。首帧、切源和效果更新后的就绪判断不能只依赖 mpv 的 `estimated-frame-number`：部分 MPEG-TS 在播放时间持续推进时仍长期返回 `0`，旧逻辑会在超时后误判播放失败。当前判断复用已有播放时间作为容器无关的进展证据，同时继续校验会话身份、媒体路径、暂停状态和 EOF，并保留调用方取消与有界超时。
- 事件帧可在等待请求响应时跳过；响应必须携带当前 `request_id`。响应错配、未知字段、畸形帧会 fail-closed 并使连接进入 `Faulted`，防止把迟到响应应用到错误请求。
- 响应超时、调用方取消和管道断开均关闭当前连接；下次使用必须显式重新连接，不能复用不确定的流状态。
- 关闭操作取消生命周期令牌、释放命名管道，活动请求在异常/取消路径中回收其有界互斥；错误正文不回显管道路径、媒体路径、原始 mpv 错误或原始帧。

## 对外最小入口

```text
MpvIpcPipeEndpoint.TryCreate(path, out endpoint, out error)
new MpvNamedPipeClient(endpoint, options)
new MpvPlaybackIpcGateway(session, client)
ConnectAsync(cancellationToken)
ExecuteAsync(requestId, MpvIpcCommand, cancellationToken)
DisposeAsync()
```

真实播放组合使用 `WindowsMpvPlaybackRuntime.StartAsync(plan, session, options, cancellationToken)`，不允许页面自行创建 `Process` 或自行拼接 IPC 字符串。

调用方负责生成不可预测的临时管道名称，并将同一 `PipePath` 传给受管 mpv 的 `--input-ipc-server`。本边界不接受任意命令字符串，也不允许用用户输入直接拼接进程参数。

MPEG-TS 兼容修复只调整现有 mpv 会话的播放进展判定，不预转码、不生成临时 MP4、不增加播放器或第三方依赖。

## 错误分类

| 情况 | 稳定分类 | 连接处理 |
| --- | --- | --- |
| 管道格式或名称非法 | `InvalidPipeName` | 不连接 |
| 尚未连接、连接已关闭 | `IpcNotConnected` / `IpcDisconnected` | 保持断开 |
| 连接或响应超时 | `IpcTimeout` | 关闭当前连接 |
| 调用方取消 | `IpcCancelled` | 关闭当前连接 |
| EOF、通信异常 | `IpcDisconnected` | 关闭当前连接 |
| 响应 request_id 错配 | `ResponseRequestIdMismatch` | 进入 `Faulted` |
| 未知字段、畸形 JSON/UTF-8、帧过大 | `UnknownField` / `MalformedJson` / `CommandTooLarge` | 进入 `Faulted` |

所有错误都使用稳定中文摘要；不会把 `\\.\pipe\...`、用户媒体路径或 mpv 原始文本返回给 GUI。

## 验证

`tests/GpAutoLive.Media.Tests/MpvNamedPipeClientTests.cs` 使用 Windows 自带 `NamedPipeServerStream` 覆盖：

- 合法/非法管道端点校验；
- 成功连接、固定请求形状、事件跳过和响应 request_id 配对；
- 响应超时、调用方取消和断连；
- 响应 request_id 错配、未知字段和脱敏错误；
- `GpAutoLive.Media` 不启动真实 mpv，不复制媒体 DLL 到主 EXE。`GpAutoLive.Windows.Tests` 另覆盖 Windows 宿主的 Job Object/退出回收、组合运行时的未启动/关闭状态。

## 尚未完成的实机门禁

- 真实资源目录签名、安装包注入和真实 mpv 版本的生命周期监督；代码层的 `MpvLaunchPlan` 启动适配和组合运行时已接入，但尚未用正式 mpv 资源完成矩阵验收；
- 在真实 mpv 版本上验证持久连接、事件字段集合、换源、暂停/恢复、seek、EOF 和退出；
- mpv `gpu-next/libplacebo` 的 GPU83 实际输出、CPU4 单向回退、Original 兜底及最终效果 HWND 绑定；
- PortAudio 音频总线、音画 PTS 校正、RTMP、AkVirtualCamera 与 30 分钟稳定性/目标显卡矩阵测试。
