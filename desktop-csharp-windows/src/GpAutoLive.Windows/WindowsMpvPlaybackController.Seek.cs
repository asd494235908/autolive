using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.Windows;

public sealed partial class WindowsMpvPlaybackController
{
    // 调用方持有既有 controller 串行锁；监视器可以继续通过同一 IPC owner 读取属性。
    private async Task<WindowsMpvPlaybackControllerResult> DispatchSeekAndWaitAsync(
        WindowsMpvPlaybackRuntime runtime,
        MpvSessionOperationResult operation,
        MediaPlaybackIdentity identity,
        bool allowEof,
        CancellationToken cancellationToken)
    {
        using var observation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        observation.CancelAfter(NextFrameObservationTimeout);
        var token = observation.Token;
        try
        {
            // 先消费之前缓冲的事件，防止把上一轮 playback-restart 当作本次完成。
            var barrier = await runtime.DispatchAsync(
                MpvIpcCommand.GetProperty(MpvIpcProperty.Seeking), identity, token).ConfigureAwait(false);
            token.ThrowIfCancellationRequested();
            if (!barrier.IsSuccess)
                return Failure(WindowsMpvPlaybackControllerFailureCode.DispatchFailed, "无法确认 mpv 跳转前状态。", true);
            var previousRestart = runtime.PlaybackRestartSequence;
            var dispatched = await DispatchCommandsAsync(operation, identity, token).ConfigureAwait(false);
            token.ThrowIfCancellationRequested();
            if (!dispatched.IsSuccess) return dispatched;

            while (true)
            {
                var seeking = await runtime.DispatchAsync(
                    MpvIpcCommand.GetProperty(MpvIpcProperty.Seeking), identity, token).ConfigureAwait(false);
                token.ThrowIfCancellationRequested();
                if (!seeking.IsSuccess
                    || !MpvIpcValueReader.TryReadBoolean(seeking.Frame!, out var isSeeking, out _))
                    return Failure(WindowsMpvPlaybackControllerFailureCode.DispatchFailed, "无法确认 mpv 跳转完成状态。", true);

                if (!isSeeking)
                {
                    if (runtime.PlaybackRestartSequence > previousRestart) return Success();
                    // 文件末端可能直接进入 EOF，不会重新开始呈现；只对已知末端目标接受它。
                    if (allowEof)
                    {
                        var eof = await runtime.DispatchAsync(
                            MpvIpcCommand.GetProperty(MpvIpcProperty.EofReached), identity, token).ConfigureAwait(false);
                        token.ThrowIfCancellationRequested();
                        if (!eof.IsSuccess
                            || !MpvIpcValueReader.TryReadBoolean(eof.Frame!, out var reached, out _))
                            return Failure(WindowsMpvPlaybackControllerFailureCode.DispatchFailed, "无法确认 mpv 跳转末端状态。", true);
                        if (reached) return Success();
                    }
                }
                await Task.Delay(NextFrameObservationInterval, token).ConfigureAwait(false);
            }
        }
        catch (OperationCanceledException)
        {
            return cancellationToken.IsCancellationRequested
                ? Failure(WindowsMpvPlaybackControllerFailureCode.Cancelled, "mpv 跳转完成观察已取消。", true)
                : Failure(WindowsMpvPlaybackControllerFailureCode.EffectiveFrameNotObserved,
                    "mpv 已接收跳转，但未在观察预算内确认跳转完成。", true);
        }
    }
}
