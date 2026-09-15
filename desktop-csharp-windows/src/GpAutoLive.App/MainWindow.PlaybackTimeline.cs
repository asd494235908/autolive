using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private RtmpOutputConfig? _pausedRtmpConfig;

    private async Task ChangeMediaSourceWithRtmpAsync(Func<Task> changeSource)
    {
        var config = _lastRtmpConfig ?? _pausedRtmpConfig;
        var previousIdentity = _mediaPool.CurrentIdentity;
        if (!await StopRtmpForMediaMutationAsync().ConfigureAwait(true)) return;
        await changeSource().ConfigureAwait(true);
        if (config is null || _isClosing) return;
        if (_mediaPool.CurrentIdentity == previousIdentity)
        {
            if (_mediaPool.Snapshot.PlaybackState is PlaybackState.Paused)
                _pausedRtmpConfig = config;
            return;
        }
        if (_mediaPool.Snapshot.PlaybackState is PlaybackState.Playing)
            await StartRtmpCoreAsync(config).ConfigureAwait(true);
        else if (_mediaPool.Snapshot.PlaybackState is PlaybackState.Paused)
            _pausedRtmpConfig = config;
    }

    private async Task TogglePlaybackCoreAsync()
    {
        var previousState = _mediaPool.Snapshot.PlaybackState;
        if (previousState is PlaybackState.Playing && _lastRtmpConfig is { } publishingConfig)
        {
            if (!await StopRtmpForMediaMutationAsync().ConfigureAwait(true)) return;
            _pausedRtmpConfig = publishingConfig;
        }

        await TransitionPlaybackCoreAsync().ConfigureAwait(true);
        if (!_isClosing
            && _mediaPool.Snapshot.PlaybackState is PlaybackState.Playing
            && _pausedRtmpConfig is { } resumeConfig)
        {
            _pausedRtmpConfig = null;
            await StartRtmpCoreAsync(resumeConfig).ConfigureAwait(true);
        }
    }
}
