# C5 RTMP 有限重连协调器实现记录

日期：2026-09-03  
范围：仅修改 `desktop-csharp-windows`；`desktop/` Rust/Tauri/React 仍为只读参考。

## 2026-09-06 WPF 重连尝试的 Dispatcher 边界修复

- `WindowsRtmpReconnectCoordinator` 继续保持无 UI 上下文；其退避和尝试回调使用
  `ConfigureAwait(false)`，因此 WPF 上层不能依赖回调入口天然位于 UI 线程。
- `MainWindow.ReconnectRtmpCoreAsync` 现在通过 `RunRtmpReconnectAttemptAsync` 将每次
  尝试统一送入窗口 `Dispatcher`；`StopRtmpCoreAsync`、`StartRtmpCoreAsync`、轨道就绪
  等待及身份失效后的清理均在同一 UI 边界内执行。这样第 2 次及后续重试不会从线程池
  直接访问 WPF 控件或 UI 状态。
- `MainWindowRtmpReadinessTests.Rtmp_reconnect_attempt_from_background_returns_to_window_dispatcher`
  覆盖后台入口回到窗口 Dispatcher；该测试只验证本地线程边界，不宣称真实网络重连。

## 2026-09-06 Rust 重试预算对齐

- 默认策略现在是初次启动后最多 5 次重试（共 6 次尝试），退避固定为 `1/2/4/8/15s`；自定义 `maxAttempts` 仍限制在 1～6 次，不会产生无限重试。
- 该调整只改变现有纯逻辑协调器的默认预算，保留取消、并发拒绝、异常脱敏和由上层成组重建音画会话的边界；真实断开信号仍未自动接入宿主。
- 本轮新增 Rust 预算回归测试；`WindowsRtmpReconnectCoordinatorTests` 与 RTMP 输出状态组合测试通过，真实 ZLMediaKit/RTMPS 断线恢复仍待验收。

## 本轮取消竞态收口

- 重连尝试回调返回后再次检查调用方取消令牌；即使回调恰好返回成功，只要调用方已取消，也回到 `Idle/Cancelled`，不会错误发布为本机 `Publishing`。
- 新增对应纯逻辑回归测试。协调器仍不访问网络、不接收地址或 FFmpeg 原文，也未接入 `WindowsRtmpOutputManager` 的 `Process.Exited`。

本轮使用项目内 `.tools\dotnet\dotnet.exe` 完成聚焦 RTMP 输出/重连测试 15/15、Windows 测试项目全量 194/194、解决方案格式检查和 `tools/verify-scope.ps1`；真实网络断开信号与音画成组恢复仍未验收。

## 2026-09-06 宿主重连发布门禁接线

- `MainWindow.ReconnectRtmpCoreAsync` 已成为用户触发的显式重连入口：调用既有协调器前固定当前 `MediaPlaybackIdentity`，停止前后和启动前检查身份，避免媒体切换期间重播旧源。
- `StartRtmpCoreAsync` 返回后不再直接视为重连成功；上层在最多 10 秒的有界等待内观察 `WindowsRtmpOutputManager` 的 `Publishing`（来自正向 `-progress` 证据）及所选声音会话的运行/无错误状态。身份变化为不可重试失败，启动超时或进程失败仍按既有协调器预算重试。
- 新增 `MainWindowRtmpReadinessTests`，覆盖启动中不得报告已发布、音画所选轨道均就绪才成功两项组合，定向测试 2/2 通过。该测试不宣称远端握手或断线恢复。

## 2026-09-06 用户停止与媒体切换取消边界

- `MainWindow.ReconnectRtmpCoreAsync` 使用独立于播放命令串行锁的链接取消令牌；用户点击停止时先取消该令牌，再等待既有停止命令进入串行队列，因此重连退避期间停止按钮不会被重连任务长期占用。
- WPF 状态投影同时观察 `WindowsRtmpReconnectCoordinator` 的 `Reconnecting` 快照：重连退避/尝试期间显示“重连中”，停止按钮保持可用，重连按钮和开始按钮保持禁用。媒体源变更前的统一停止入口也会先取消重连。
- 重连尝试内部为了清理旧会话调用停止逻辑时显式保留重连令牌，避免“重连自己的清理动作”误取消当前序列；窗口关闭仍会先取消重连，再取消窗口级资源。
- 新增 `MainWindowRtmpReadinessTests.Stop_is_available_while_reconnect_coordinator_is_in_backoff`，把“协调器处于退避时仍可停止”的 UI 投影规则固定为纯逻辑合同；不把本地取消结果解释为远端断线恢复。

本轮使用项目内锁定 SDK 完成新增停止投影与配置选择的定向编译验证；真实 ZLMediaKit/RTMPS 远端握手、断线确认、三轨道恢复和长稳仍待验收。

## 2026-09-06 重连配置快照

- `MainWindow` 仅在画面/声音宿主启动成功后捕获 `_lastRtmpConfig`；用户显式开始新推流时会用当前编辑器值覆盖该快照，预检失败或启动失败不会污染历史会话配置。
- 用户重连时优先复用产生当前失败会话的配置，不重新读取可能已被用户改动的地址、轨道或编码参数；没有历史启动快照时才使用当前编辑器配置。
- 用户停止成功、媒体源变更成功停止以及纯音频自然结束成功停止时清除快照；重连尝试内部拆旧会话不会清除，保证有限重试仍使用同一配置。
- 当启动失败但本地宿主仍保留 PID 或最终 PCM stdin 时，WPF 将该资源视为可回收活动，停止按钮保持可用并进入既有 `StopAsync` 清理路径。
- 该快照只存在当前进程内，不写日志、不持久化完整地址；真实远端鉴权和重连成功仍必须由服务器门禁证明。

## 本次实现

- 新增 `GpAutoLive.Windows/WindowsRtmpReconnectCoordinator.cs`，把 RTMP 断开后的有限重连顺序、退避、取消和终态收敛为独立纯逻辑组件。
- 默认最多执行 6 次底层重连尝试（初次启动后重试 5 次），尝试之间使用 1s、2s、4s、8s、15s 的有界退避；不使用随机抖动，不创建后台任务、网络连接或无界队列。
- 底层尝试通过 `Func<int, CancellationToken, Task<WindowsRtmpReconnectAttempt>>` 注入，1 基序号只表示“第几次重连调用”。协调器不接收地址、路径、命令行、stream key 或 FFmpeg 原始输出。
- `Reconnecting` 只表示当前重连序列正在等待或执行；成功进入 `Publishing`；不可重试失败进入 `Failed`；重试用尽使用固定 `ReconnectExhausted` 错误；调用方取消回到 `Idle`，并返回可重试的 `Cancelled`。
- 未知异常只映射为固定 `StartFailed`，异常正文不进入快照、错误消息或日志。重连计数为已经开始的重连调用数，最大值受策略上限约束。
- 底层尝试若违反非空合同返回 `null`，同样按固定、可重试的 `StartFailed` 处理，最终仍受最大尝试次数约束，不会抛出 `NullReferenceException`。
- `WindowsRtmpFailureCode.ReconnectExhausted` 与现有 Windows RTMP 宿主错误词汇对齐，但当前宿主仍由上层在确认断开后显式调用协调器；本增量不把 `Process.Exited` 直接绑定为自动网络重连，也不声称握手、首包或 ZLMediaKit/RTMPS 已通过。

## 不在底层输出管理器自动触发重连的边界

协调器仍不在 `WindowsRtmpOutputManager` 内自动调用。宿主的 `Process.Exited` 只能证明本地 FFmpeg 进程结束，不能证明远端 RTMP/RTMPS 握手、鉴权或网络断开原因；而且重连所需的配置、媒体源身份、首选编码器和 `WindowsRtmpAudioSession` 所有权由 WPF 上层持有。当前由 `MainWindow` 在失败态和用户显式重连动作下成组停止、按媒体身份重建并等待所选轨道达到发布门禁，不会只重启一个宿主。

上层在具备“当前源身份仍匹配、声音会话已停止、重新启动结果可分类”的真实接线后，可以使用以下最小调用形状；示例不执行网络探测，也不把 `StartAsync` 返回的本地 `Publishing` 误写成远端握手成功：

```csharp
var reconnect = new WindowsRtmpReconnectCoordinator();
var result = await reconnect.ReconnectAsync(async (_, cancellationToken) =>
{
    var stopped = await rtmpManager.StopAsync(CancellationToken.None);
    if (!stopped.IsSuccess)
    {
        return WindowsRtmpReconnectAttempt.Failed(
            stopped.Error?.Code ?? WindowsRtmpFailureCode.StopTimedOut,
            retryable: false);
    }

    var started = await StartCurrentSourceAsync(cancellationToken);
    return started.IsSuccess
        ? WindowsRtmpReconnectAttempt.Succeeded()
        : WindowsRtmpReconnectAttempt.Failed(
            started.Error?.Code ?? WindowsRtmpFailureCode.StartFailed,
            started.Error?.Retryable == true);
}, cancellationToken);
```

该调用形状用于说明上层所有权；生产入口已补齐媒体身份校验、成组回收和有限的轨道就绪等待。远端断开信号仍需在真实服务器环境中复验，不把本地进程退出单独解释为远端握手失败或成功。

## 验证

新增 `WindowsRtmpReconnectCoordinatorTests`，覆盖：

- 成功重连的 `Reconnecting → Publishing` 状态和计数；
- 不可重试失败立即进入 `Failed`；
- 可重试失败最多 6 次、退避顺序和 `ReconnectExhausted` 终态；
- 退避期间取消、尝试期间取消均不再发起后续尝试并回到 `Idle`；
- 尝试抛出含地址/stream key 的异常时，快照不泄露敏感正文。
- 尝试违反非空合同返回 `null` 时 fail-closed 且不超过预算。
- 首次调用前已取消时不调用传输尝试回调，并回到 `Idle`。

执行命令：

```powershell
dotnet build src/GpAutoLive.Windows/GpAutoLive.Windows.csproj -c Release --no-restore -p:Platform=x64
dotnet test tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj -c Release --no-restore -p:Platform=x64 --filter FullyQualifiedName~WindowsRtmpReconnectCoordinatorTests
```

这些测试只证明本地状态和预算，不证明真实 RTMP/RTMPS 网络连接、ZLMediaKit 鉴权、断线检测时序、编码器矩阵或 30 分钟稳定性。

## 接线审计

本轮审计确认协调器暂不自动接入 `WindowsRtmpOutputManager` 或
`WindowsRtmpAudioSession`：`Process.Exited` 不是远端断开证明，而且画面进程与 PCM
解码/混音/分流泵必须由同一上层所有者成组重建。协调器已有的注入回调就是最小显式
调用边界，当前不新增网络探测或单独重启宿主。详细证据、未来接线准入和脱敏约束见
[`C5 RTMP 宿主重连接线审计`](./C5-RTMP宿主重连接线审计.md)。
