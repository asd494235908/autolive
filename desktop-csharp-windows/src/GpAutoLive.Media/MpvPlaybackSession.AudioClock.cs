using GpAutoLive.Core;

namespace GpAutoLive.Media;

public sealed partial class MpvPlaybackSession
{
    /// <summary>以当前音轨的源内可听 PTS 和实际倍速纠正视频；普通漂移不 seek。</summary>
    public MpvSessionOperationResult SynchronizeAudioClock(
        MediaPlaybackIdentity expectedIdentity,
        ulong videoPositionMs,
        ulong audioPositionMs,
        double audioPlaybackRate)
    {
        lock (_gate)
        {
            if (_snapshot.ActiveSource is not { } source)
                return Failure(MpvSessionFailureCode.NoActiveSource, "mpv 没有活动源。");
            if (source.Identity != expectedIdentity)
                return Failure(MpvSessionFailureCode.StalePlaybackIdentity, "mpv 播放身份已过期。");
            if (!double.IsFinite(audioPlaybackRate) || audioPlaybackRate is < 0.5 or > 2.0
                || audioPositionMs > 9_007_199_254_740_991UL
                || videoPositionMs > 9_007_199_254_740_991UL
                || source.DurationMs is ulong duration && (audioPositionMs >= duration || videoPositionMs >= duration))
                return Failure(MpvSessionFailureCode.InvalidPlaybackIdentity, "音画同步时钟或实际音频倍速无效。");
            if (_snapshot.State is not MpvSessionState.Playing)
                return MpvSessionOperationResult.Success(_snapshot, changed: false);

            var driftMs = (double)audioPositionMs - videoPositionMs;
            if (Math.Abs(driftMs) > 500)
            {
                return MpvSessionOperationResult.Success(_snapshot, changed: false,
                    [MpvIpcCommand.SetPlaybackSpeed(audioPlaybackRate), MpvIpcCommand.SeekAbsoluteMs(audioPositionMs)]);
            }

            var correction = Math.Abs(driftMs) <= 40 ? 0 : Math.Clamp(driftMs / 2_000, -0.05, 0.05);
            return MpvSessionOperationResult.Success(_snapshot, changed: false,
                [MpvIpcCommand.SetPlaybackSpeed(audioPlaybackRate * (1 + correction))]);
        }
    }
}
