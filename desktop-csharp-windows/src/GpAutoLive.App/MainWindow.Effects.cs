using System.Windows;
using System.Windows.Controls;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private bool _parameterPageExpanded;

    private async void RegenerateEffectParametersButton_Click(object sender, RoutedEventArgs e)
    {
        _state.RegenerateParameterSnapshots();
        await RunPlaybackCommandAsync(
                ApplyRegeneratedEffectSnapshotsAsync,
                _windowCancellation.Token)
            .ConfigureAwait(true);
    }

    private async Task ApplyRegeneratedEffectSnapshotsAsync()
    {
        var snapshot = _mediaPool.Snapshot;
        if (snapshot.SourceMediaPool.IsEmpty)
        {
            _state.SetStatus("本周期参数已重新生成，将在播放时应用");
            return;
        }

        var source = snapshot.SourceMediaPool[snapshot.SourceMediaIndex];
        if (source.MediaKind is MediaKind.Audio)
        {
            if (snapshot.PlaybackState is not (PlaybackState.Playing or PlaybackState.Paused))
            {
                _state.SetStatus("本周期参数已重新生成，将在下次播放时应用");
                return;
            }

            await ApplyAudioProcessingAsync().ConfigureAwait(true);
            return;
        }

        var identity = _mediaPool.CurrentIdentity;
        var controllerSnapshot = _mpvController.Snapshot;
        if (identity is null
            || snapshot.PlaybackState is not (PlaybackState.Playing or PlaybackState.Paused)
            || controllerSnapshot.ActiveIdentity != identity
            || controllerSnapshot.State is not (WindowsMpvPlaybackControllerState.Playing
                or WindowsMpvPlaybackControllerState.Paused))
        {
            _state.SetStatus("本周期参数已重新生成，将在视频播放时应用");
            return;
        }

        if (!TryCreateRuntimeVideoEffectSnapshot(
                source,
                _state.VideoProcessing,
                out var next,
                out var snapshotError)
            || next is null)
        {
            _state.SetStatus(snapshotError?.Message ?? "视频处理参数快照无效，未提交运行时");
            return;
        }

        var result = await _mpvController.UpdateEffectsAsync(
                identity,
                next,
                _windowCancellation.Token,
                waitForNextFrame: true)
            .ConfigureAwait(true);
        var audioReconfigured = !string.IsNullOrWhiteSpace(source.AudioCodecName)
            && result.IsSuccess
            && await ReconfigureVideoAudioAsync(source, identity).ConfigureAwait(true);
        _finalEffectController.Update(CreateFinalEffectSnapshot());
        if (!result.IsSuccess || (!string.IsNullOrWhiteSpace(source.AudioCodecName) && !audioReconfigured))
        {
            if (!result.IsSuccess)
            {
                _state.SetStatus(result.Error?.Message ?? "本周期视频参数提交失败");
            }

            return;
        }

        _state.SetStatus(result.IsSuccess
            ? _state.VideoProcessing
                ? next.Mode is MpvVideoProcessingMode.Gpu83
                    ? "本周期视频参数已应用到当前 mpv GPU83 画面；视频声音已从当前位置重建"
                    : "本周期视频参数已应用到当前 mpv CPU4 画面；视频声音已从当前位置重建"
                : "本周期视频参数已关闭并应用到当前 mpv 画面"
            : result.Error?.Message ?? "本周期视频参数提交失败");
    }

    private async Task ApplyAudioProcessingAsync()
    {
        var snapshot = _mediaPool.Snapshot;
        if (snapshot.SourceMediaPool.IsEmpty)
        {
            _state.SetStatus("声音处理配置已更新，将在导入媒体后应用");
            return;
        }

        if (snapshot.PlaybackState is not (PlaybackState.Playing or PlaybackState.Paused))
        {
            _state.SetStatus("声音处理配置已更新，将在下次播放时应用");
            return;
        }

        var source = snapshot.SourceMediaPool[snapshot.SourceMediaIndex];
        if (source.MediaKind is MediaKind.Video)
        {
            await ReconfigureVideoAudioAsync(source, _mediaPool.CurrentIdentity).ConfigureAwait(true);
            return;
        }

        var identity = _mediaPool.CurrentIdentity;
        var sourceStartMs = _audioPlaybackController.Snapshot.AudibleClock?.PlaybackTimeMs ?? 0;
        if (source.DurationMs is ulong durationMs && durationMs > 0)
        {
            sourceStartMs = Math.Min(sourceStartMs, durationMs - 1);
        }
        if (!await StopVideoAudioForTransitionAsync().ConfigureAwait(true))
        {
            return;
        }

        var started = await StartAudioPlaybackAsync(source, identity, sourceStartMs).ConfigureAwait(true);
        if (!started.IsSuccess)
        {
            var stopped = _mediaPool.StopPlayback();
            ApplyMediaOperation(stopped, started.Error?.Message ?? "声音处理重启失败");
            return;
        }

        if (snapshot.PlaybackState is PlaybackState.Paused)
        {
            var paused = await _audioPlaybackController.PauseAsync().ConfigureAwait(true);
            if (!paused.IsSuccess)
            {
                await StopAudioCompletionWatcherAsync().ConfigureAwait(true);
                var stopped = await _audioPlaybackController.StopAsync().ConfigureAwait(true);
                var stoppedPool = _mediaPool.StopPlayback();
                ApplyMediaOperation(
                    stoppedPool,
                    stopped.IsSuccess
                        ? paused.Error?.Message ?? "声音处理暂停失败"
                        : stopped.Error?.Message ?? "声音处理重启后停止失败");
                return;
            }
        }

        StartAudioCompletionWatcher(identity);
        await PrepareNextAudioCandidateAsync(identity).ConfigureAwait(true);
        AudioDeviceStatusText.Text = _state.AudioProcessing
            ? "声音处理已应用 · FFmpeg 音频滤镜"
            : "声音处理已关闭 · 已恢复原始 PCM";
        _state.SetStatus(AudioDeviceStatusText.Text);
    }

    private async Task<bool> ReconfigureVideoAudioAsync(
        SourceMediaDto source,
        MediaPlaybackIdentity? identity)
    {
        if (identity is null)
        {
            _state.SetStatus("视频声音处理已更新，将在下次播放时应用");
            return false;
        }

        var sourceStartMs = await ReadCurrentVideoPositionAsync(identity, source.DurationMs)
            .ConfigureAwait(true);
        if (sourceStartMs is null)
        {
            _state.SetStatus("无法读取当前视频位置，声音处理未切换；请重新播放后再试");
            return false;
        }

        var wasPaused = _mediaPool.Snapshot.PlaybackState is PlaybackState.Paused;
        if (!await StopVideoAudioForTransitionAsync().ConfigureAwait(true))
        {
            return false;
        }

        var started = await StartAudioPlaybackAsync(source, identity, sourceStartMs.Value)
            .ConfigureAwait(true);
        if (!started.IsSuccess)
        {
            AudioDeviceStatusText.Text = "视频画面继续运行 · 新声音会话未启动";
            _state.SetStatus(started.Error?.Message ?? "视频声音处理重启失败；视频画面仍在播放");
            return false;
        }

        if (wasPaused)
        {
            var paused = await _audioPlaybackController.PauseAsync().ConfigureAwait(true);
            if (!paused.IsSuccess)
            {
                await _audioPlaybackController.StopAsync().ConfigureAwait(true);
                AudioDeviceStatusText.Text = "视频声音会话已停止";
                _state.SetStatus(paused.Error?.Message ?? "视频声音暂停同步失败；视频画面仍在播放");
                return false;
            }
        }

        StartAudioCompletionWatcher(identity);
        AudioDeviceStatusText.Text = _state.AudioProcessing
            ? "视频声音处理已应用 · 从当前时间点恢复"
            : "视频声音处理已关闭 · 从当前时间点恢复原始 PCM";
        _state.SetStatus(AudioDeviceStatusText.Text);
        return true;
    }

    private async Task<ulong?> ReadCurrentVideoPositionAsync(
        MediaPlaybackIdentity identity,
        ulong? durationMs)
    {
        var result = await _mpvController.ReadPropertyAsync(
                identity,
                MpvIpcProperty.PlaybackTime,
                _windowCancellation.Token)
            .ConfigureAwait(true);
        if (!result.IsSuccess
            || result.Frame is null
            || !MpvIpcValueReader.TryReadFiniteDouble(result.Frame, out var seconds, out _)
            || seconds is not double value
            || value < 0
            || value > 9_007_199_254_740.991)
        {
            return null;
        }

        var milliseconds = Math.Round(value * 1_000, MidpointRounding.AwayFromZero);
        if (!double.IsFinite(milliseconds) || milliseconds < 0)
        {
            return null;
        }

        var positionMs = (ulong)milliseconds;
        return durationMs is ulong duration && (duration == 0 || positionMs >= duration)
            ? null
            : positionMs;
    }

    private async Task ApplyVideoProcessingModeAsync()
    {
        var snapshot = _mediaPool.Snapshot;
        var identity = _mediaPool.CurrentIdentity;
        var controllerSnapshot = _mpvController.Snapshot;
        if (identity is null
            || snapshot.PlaybackState is not (PlaybackState.Playing or PlaybackState.Paused))
        {
            _state.SetStatus("视频处理配置已更新，将在下次播放时应用");
            return;
        }

        if (controllerSnapshot.ActiveIdentity != identity
            || controllerSnapshot.Runtime.State is not WindowsMpvPlaybackRuntimeState.Running)
        {
            _state.SetStatus("视频处理配置已更新，但当前 mpv 会话不可用，将在重新播放时应用");
            return;
        }

        if (!TryCreateRuntimeVideoEffectSnapshot(
                snapshot.SourceMediaPool[snapshot.SourceMediaIndex],
                _state.VideoProcessing,
                out var next,
                out var snapshotError)
            || next is null)
        {
            _state.SetStatus(snapshotError?.Message ?? "视频处理参数快照无效，未提交运行时");
            return;
        }

        var result = await _mpvController.UpdateEffectsAsync(
                identity,
                next,
                _windowCancellation.Token,
                waitForNextFrame: true)
            .ConfigureAwait(true);
        _state.SetStatus(result.IsSuccess
            ? (_state.VideoProcessing
                ? next.Mode is MpvVideoProcessingMode.Gpu83
                    ? "视频处理已提交给 mpv GPU83 实时 shader"
                    : "视频处理已提交给 mpv CPU4 实时滤镜"
                : "视频处理已关闭并提交给 mpv")
            : result.Error?.Message ?? "视频处理运行时更新失败");
        _finalEffectController.Update(CreateFinalEffectSnapshot());
    }

    private async Task ApplyAutomaticVideoEffectCycleAsync(MediaPlaybackIdentity identity)
    {
        if (_isClosing
            || _mediaPool.CurrentIdentity != identity
            || _mediaPool.Snapshot.PlaybackState is not PlaybackState.Playing
            || !_state.VideoProcessing)
        {
            return;
        }

        _state.RegenerateVideoParameterSnapshot();
        await ApplyVideoProcessingModeAsync().ConfigureAwait(true);
    }

    private static bool IsFullGpu83Runtime(VerifiedMediaRuntime runtime) =>
        runtime.TryGetResource("gpu83.hook", out var shader)
        && MpvGpu83ShaderSnapshot.IsFullShaderResource(shader);

    private bool TryCreateRuntimeVideoEffectSnapshot(
        SourceMediaDto source,
        bool enabled,
        out MpvVideoEffectSnapshot? snapshot,
        out MpvVideoParameterError? error,
        MpvLaunchMode? modeOverride = null)
    {
        if (!enabled)
        {
            snapshot = MpvVideoEffectSnapshot.Default;
            error = null;
            return true;
        }

        if (modeOverride is MpvLaunchMode.Original)
        {
            snapshot = MpvVideoEffectSnapshot.Default;
            error = null;
            return true;
        }

        var generated = _state.VideoParameterSnapshot;
        if (_verifiedMediaRuntime is not null
            && _verifiedMediaRuntime.TryGetResource("gpu83.hook", out var shader)
            && MpvGpu83ShaderSnapshot.IsFullShaderResource(shader)
            && modeOverride is not MpvLaunchMode.Cpu4)
        {
            return MpvVideoEffectSnapshot.TryCreateGpu83(
                generated.ToVideoEffectParams(),
                generated.ToAdvancedEffectParams(),
                source.FrameRateFps ?? 30,
                epochStartSeconds: 0,
                randomSeed: (uint)Math.Clamp(generated.Generation, 0, 16_777_215),
                out snapshot,
                out _,
                out error);
        }

        return MpvVideoEffectSnapshot.TryCreate(
            MpvVideoProcessingMode.Cpu4,
            generated.BrightnessPercent,
            generated.ContrastPercent,
            generated.SaturationPercent,
            generated.HueRotationDegrees,
            MpvShaderOptionsSnapshot.Empty,
            out snapshot,
            out error);
    }

    private void ParameterCategoryButton_Click(object sender, RoutedEventArgs e)
    {
        var tag = (sender as FrameworkElement)?.Tag as string;
        var target = tag switch
        {
            "video" => VideoSnapshotSection,
            "audio" => AudioSnapshotSection,
            "advanced" => AdvancedVisualSection,
            _ => null,
        };

        if (target is null)
        {
            return;
        }

        target.BringIntoView();
    }

    private void ParameterScrollViewer_ScrollChanged(object sender, ScrollChangedEventArgs e)
    {
        var expanded = e.VerticalOffset > 8;
        ParameterScrollHintText.Visibility = expanded
            ? Visibility.Visible
            : Visibility.Collapsed;
        if (_parameterPageExpanded == expanded)
        {
            return;
        }

        _parameterPageExpanded = expanded;
        ParameterTabButtons.Visibility = expanded
            ? Visibility.Collapsed
            : Visibility.Visible;
        ParameterScrollHintText.HorizontalAlignment = expanded
            ? HorizontalAlignment.Center
            : HorizontalAlignment.Right;
        ParameterScrollHintText.Margin = expanded
            ? new Thickness(0)
            : new Thickness(0, 0, 8, 0);
        ParameterContentGrid.Margin = expanded
            ? new Thickness(0)
            : new Thickness(0, 8, 0, 0);
        QuickParamsCard.Padding = expanded
            ? new Thickness(8, 0, 8, 8)
            : new Thickness(8);
        ParameterTabRow.Height = expanded
            ? new GridLength(12)
            : new GridLength(42);
        PreviewCard.Visibility = expanded ? Visibility.Collapsed : Visibility.Visible;
        PreviewRow.Height = expanded
            ? new GridLength(0)
            : new GridLength(0.96, GridUnitType.Star);
        PreviewGapRow.Height = expanded
            ? new GridLength(0)
            : new GridLength(10);
        ParameterCategoryNavigation.Visibility = expanded
            ? Visibility.Collapsed
            : Visibility.Visible;
        ParameterCategoryColumn.Width = expanded
            ? new GridLength(0)
            : new GridLength(76);
        ParameterCategoryGapColumn.Width = expanded
            ? new GridLength(0)
            : new GridLength(16);
        QuickParamsRow.Height = expanded
            ? new GridLength(1, GridUnitType.Star)
            : new GridLength(1.04, GridUnitType.Star);
    }

    private void ResetVideoEffectsButton_Click(object sender, RoutedEventArgs e)
    {
        _state.VideoEffects.Reset();
        _state.SetStatus("已恢复四项已接入视频参数的默认草稿");
    }
}
