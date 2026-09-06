# C5 RTMP 宿主重连接线审计

日期：2026-09-03  
范围：`desktop-csharp-windows`；`desktop/` Rust/Tauri/React 仅作只读参考。

## 审计结论

当前生产入口已由 WPF `MainWindow.ReconnectRtmpCoreAsync` 显式调用
`WindowsRtmpReconnectCoordinator`，并由同一上层拥有 `WindowsRtmpOutputManager`、
`WindowsRtmpAudioSession` 和活动源身份。协调器仍不会自行监听远端或在后台自动重连；
它只负责顺序、退避、取消、并发拒绝和有限终态，尝试回调不携带地址、路径、stream key
或 FFmpeg 原文。

本轮未新增网络探测、RTMP/RTMPS 握手猜测或隐式后台重连；WPF 的“Publishing”门禁仍
只表示受管 FFmpeg 有正向进度，不能替代远端握手、首包或回读证据。

## 2026-09-06 当前远端复核

- `192.168.10.22:1935` 一次 TCP 探测在约 3 秒后不可达，因此没有执行真实推流、回读或停止烟测，也没有反复等待。
- 当前代码路径的状态、取消、无音频轨拒绝、有限重连、源身份失效和资源收敛测试均通过；远端网络门禁保持“待验收”。

## 2026-09-06 默认预算更新

协调器默认预算已与 Rust 对齐为初次启动后最多 5 次重试（共 6 次尝试），退避为
`1/2/4/8/15s`；这不改变本审计的核心结论：只有上层确认断开、校验同一活动源身份，
并成组停止/重建画面与最终 PCM 会话后，才允许调用协调器。

## 证据与缺口

### `WindowsRtmpOutputManager`

- `Process.Exited` 只能证明本地 FFmpeg 进程退出，不能证明远端 RTMP/RTMPS 握手、
  鉴权、首包或网络断开原因。
- 对外 `WindowsRtmpSnapshot.TargetUrl` 来自 `RtmpOutputRules.RedactTargetUrl`；
  完整目标地址只在启动计划和进程参数的短生命周期内使用，不进入重连协调器状态。
- `StartAsync` 本身不做自动重试；它只启动当前一次画面/声音 FFmpeg 会话，启动后
  先投影为 `Starting`，只有受管 stderr 的正向 FFmpeg 输出进度才进入本机 `Publishing`，
  这仍不等于远端发布成功。

### `WindowsRtmpAudioSession`

- 音频会话同时拥有 FFmpeg PCM 解码器、固定容量 `FinalPcmBus`、混音输出源和唯一
  `WindowsRtmpFinalPcmPump`。只重启 `WindowsRtmpOutputManager` 会让原音频泵继续写入
  已关闭或已替换的 `pipe:0`，破坏画面/声音会话的一致性。
- 音频会话当前不保存一份可安全重放的完整启动计划，也不自行判断当前播放池的
  `RtmpSourceIdentity` 是否仍与活动源一致。媒体切换前必须由上层先停止整个输出链。
- 因此，未来生产接线必须由一个拥有“画面宿主 + 声音会话 + 活动源身份”的上层
  所有者，以一次原子尝试完成：身份校验 → 停止旧画面/声音 → 启动同一源的新画面/声音
  组合；不能让协调器分别重启两条链。

## 允许的最小调用边界

`WindowsRtmpReconnectCoordinator.ReconnectAsync` 已提供足够的显式边界：

```csharp
var result = await reconnect.ReconnectAsync(async (attempt, cancellationToken) =>
{
    // 调用方必须先确认远端断开、活动源身份仍匹配，并由同一所有者
    // 原子停止并重新启动画面 + 声音会话。
    var started = await RestartWholeRtmpSessionAsync(cancellationToken);
    return started.IsSuccess
        ? WindowsRtmpReconnectAttempt.Succeeded()
        : WindowsRtmpReconnectAttempt.Failed(
            started.ErrorCode ?? WindowsRtmpFailureCode.StartFailed,
            started.Retryable);
}, cancellationToken);
```

这段形状是契约示意，不是当前生产入口。`attempt` 只表示有限序号；回调不得把
`WindowsRtmpOutputManager` 单独重启，也不得把 `StartAsync` 的本机进程启动成功
解释为远端握手成功。若调用方无法提供远端断开确认、活动源身份门禁和画面/声音组合
所有权，应直接停止并显示失败，不提交重连。

协调器本身满足以下约束：

- 默认最多 6 次尝试，退避 1s、2s、4s、8s、15s；不创建无界队列或无人管理任务；
- 同一协调器并发调用被拒绝；退避与回调均受取消令牌控制；
- 回调异常、空结果和未知失败均映射为固定错误分类，不传播敏感异常正文；
- 首次调用前已取消时不会调用传输回调；取消终态回到 `Idle`，重试耗尽进入
  `Failed/ReconnectExhausted`。

## 后续接线准入

只有同时具备以下证据，才允许新增生产上层适配器：

1. 传输层能提供确定的断开/握手失败事件，而不是仅依赖 `Process.Exited`；
2. 上层持有并校验同一 `RtmpSourceIdentity`，媒体编辑、播放切换和窗口关闭会使旧
   身份失效；
3. 同一次尝试可以成组停止并重建画面、PCM 解码、混音和分流泵，且每个任务均有取消、
   2 秒内回收或明确失败终态；
4. 隔离 Windows ZLMediaKit/RTMPS 测试覆盖音画、单轨、鉴权拒绝、断线恢复和退出
   后无残留进程；
5. 日志和 UI 只显示稳定错误码/脱敏地址，不持久化完整地址或密钥。

在这些条件完成前，C5 状态保持“有限重连策略已接入·真实断开信号和网络恢复待验收”，
不宣称 RTMP/RTMPS 重连已完成。
