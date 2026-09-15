using GpAutoLive.Core;

namespace GpAutoLive.Windows;

public sealed partial class WindowsMpvPlaybackController
{
    /// <summary>在既有串行与媒体身份边界内，让 mpv 跟随当前音轨的源内可听时钟。</summary>
    public Task<WindowsMpvPlaybackControllerResult> SynchronizeAudioClockAsync(
        MediaPlaybackIdentity? expectedIdentity,
        ulong videoPositionMs,
        ulong audioPositionMs,
        double audioPlaybackRate,
        CancellationToken cancellationToken = default) =>
        RunSessionOperationAsync(
            expectedIdentity,
            session => session.SynchronizeAudioClock(expectedIdentity!, videoPositionMs, audioPositionMs, audioPlaybackRate),
            cancellationToken);
}
