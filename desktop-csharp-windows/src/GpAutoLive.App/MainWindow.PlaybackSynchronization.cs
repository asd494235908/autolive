using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
    // 调用方持有播放命令串行锁；只同步同一个活动媒体段的真实可听时钟。
    private async Task SynchronizeVideoAudioAsync(MpvPlaybackStateSnapshot video)
    {
        var pool = _mediaPool.Snapshot;
        var audio = _audioPlaybackController.Snapshot;
        if (_isClosing || _mediaPool.CurrentIdentity != video.Identity
            || _audioPlaybackIdentity != video.Identity
            || pool.PlaybackState is not PlaybackState.Playing
            || video.Paused || video.EofReached || video.PlaybackTimeMs is not ulong videoMs
            || audio.State is not WindowsAudioPlaybackState.Playing
            || audio.AudibleClock is not { PlaybackTimeMs: ulong audioMs } clock)
            return;
        var duration = pool.SourceMediaPool[pool.SourceMediaIndex].DurationMs;
        if (duration is ulong end && (videoMs >= end || audioMs >= end)) return;
        var synchronized = await _mpvController.SynchronizeAudioClockAsync(
            video.Identity, videoMs, audioMs, clock.PlaybackRate, _windowCancellation.Token).ConfigureAwait(true);
        if (!synchronized.IsSuccess)
            _state.SetStatus(synchronized.Error?.Message ?? "音画同步未完成");
    }

    // 设备初始化已结束，在首次显示/重建边界一次对齐；稳定播放不循环硬 seek。
    private async Task<bool> AlignStartedVideoAudioAsync(MediaPlaybackIdentity identity)
    {
        if (_audioPlaybackIdentity != identity || _mpvController.Snapshot.ActiveIdentity != identity
            || _audioPlaybackController.Snapshot.AudibleClock?.PlaybackTimeMs is not ulong position)
            return true;
        var result = await _mpvController.SeekAsync(identity, position, _windowCancellation.Token).ConfigureAwait(true);
        if (!result.IsSuccess) _state.SetStatus(result.Error?.Message ?? "声音启动后的画面对齐失败");
        return result.IsSuccess;
    }

    private async Task<WindowsMpvPlaybackControllerResult> SwitchVideoSourcePausedAsync(
        SourceMediaDto source, MediaPlaybackIdentity identity)
    {
        var previous = _mpvController.Snapshot;
        if (previous.State is WindowsMpvPlaybackControllerState.Playing)
        {
            var paused = await _mpvController.TogglePauseAsync(previous.ActiveIdentity, _windowCancellation.Token)
                .ConfigureAwait(true);
            if (!paused.IsSuccess) return paused;
        }
        return await _mpvController.SwitchSourceAsync(source, identity, _windowCancellation.Token).ConfigureAwait(true);
    }

    private async Task<bool> ResumeSwitchedVideoAsync(MediaPlaybackIdentity identity)
    {
        if (!await AlignStartedVideoAudioAsync(identity).ConfigureAwait(true))
        {
            var stopped = await StopVideoAudioForTransitionAsync().ConfigureAwait(true);
            ApplyMediaOperation(_mediaPool.StopPlayback(), stopped
                ? "新视频音画对齐失败，已保留上一帧并停止声音"
                : "新视频音画对齐失败，声音资源尚未完成停止");
            return false;
        }
        var resumed = await _mpvController.TogglePauseAsync(identity, _windowCancellation.Token).ConfigureAwait(true);
        if (resumed.IsSuccess) return true;
        var audioStopped = await StopVideoAudioForTransitionAsync().ConfigureAwait(true);
        ApplyMediaOperation(_mediaPool.StopPlayback(), audioStopped
            ? resumed.Error?.Message ?? "新视频未能恢复播放"
            : "新视频未能恢复播放，声音资源尚未完成停止");
        return false;
    }
}
