# C5 RTMP 有限重连协调器实现记录

日期：2026-09-03  
范围：仅修改 `desktop-csharp-windows`；`desktop/` Rust/Tauri/React 仍为只读参考。

## 本轮取消竞态收口

- 重连尝试回调返回后再次检查调用方取消令牌；即使回调恰好返回成功，只要调用方已取消，也回到 `Idle/Cancelled`，不会错误发布为本机 `Publishing`。
- 新增对应纯逻辑回归测试。协调器仍不访问网络、不接收地址或 FFmpeg 原文，也未接入 `WindowsRtmpOutputManager` 的 `Process.Exited`。

本轮使用项目内 `.tools\dotnet\dotnet.exe` 完成聚焦 RTMP 输出/重连测试 15/15、Windows 测试项目全量 194/194、解决方案格式检查和 `tools/verify-scope.ps1`；真实网络断开信号与音画成组恢复仍未验收。

## 本次实现

- 新增 `GpAutoLive.Windows/WindowsRtmpReconnectCoordinator.cs`，把 RTMP 断开后的有限重连顺序、退避、取消和终态收敛为独立纯逻辑组件。
- 默认最多执行 3 次底层重连尝试，尝试之间使用 250ms、500ms、1s 的有界指数退避；不使用随机抖动，不创建后台任务、网络连接或无界队列。
- 底层尝试通过 `Func<int, CancellationToken, Task<WindowsRtmpReconnectAttempt>>` 注入，1 基序号只表示“第几次重连调用”。协调器不接收地址、路径、命令行、stream key 或 FFmpeg 原始输出。
- `Reconnecting` 只表示当前重连序列正在等待或执行；成功进入 `Publishing`；不可重试失败进入 `Failed`；重试用尽使用固定 `ReconnectExhausted` 错误；调用方取消回到 `Idle`，并返回可重试的 `Cancelled`。
- 未知异常只映射为固定 `StartFailed`，异常正文不进入快照、错误消息或日志。重连计数为已经开始的重连调用数，最大值受策略上限约束。
- 底层尝试若违反非空合同返回 `null`，同样按固定、可重试的 `StartFailed` 处理，最终仍受最大尝试次数约束，不会抛出 `NullReferenceException`。
- `WindowsRtmpFailureCode.ReconnectExhausted` 与现有 Windows RTMP 宿主错误词汇对齐，但当前宿主仍由上层在确认断开后显式调用协调器；本增量不把 `Process.Exited` 直接绑定为自动网络重连，也不声称握手、首包或 ZLMediaKit/RTMPS 已通过。

## 当前不接线到宿主的理由

当前不在 `WindowsRtmpOutputManager` 内自动调用协调器。宿主的 `Process.Exited` 只能证明本地 FFmpeg 进程结束，不能证明远端 RTMP/RTMPS 握手、鉴权或网络断开原因；而且重连所需的配置、媒体源身份、首选编码器和 `WindowsRtmpAudioSession` 所有权由 WPF 上层持有。只重启一个宿主会让音频 `pipe:0` 与画面进程不同步，或在媒体切换后重播旧源。

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

该调用形状暂不作为生产入口；需先由上层补齐断开信号、会话身份校验和画面/声音组合回收，再进行本机隔离网络测试。

## 验证

新增 `WindowsRtmpReconnectCoordinatorTests`，覆盖：

- 成功重连的 `Reconnecting → Publishing` 状态和计数；
- 不可重试失败立即进入 `Failed`；
- 可重试失败最多 3 次、退避顺序和 `ReconnectExhausted` 终态；
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
