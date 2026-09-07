using System.Windows;
using System.Globalization;
using System.Windows.Controls;
using GpAutoLive.App.Features.Effects;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Configuration;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
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
            && await ReconfigureVideoAudioAsync(
                    source,
                    identity,
                    _state.AudioProcessingRevision)
                .ConfigureAwait(true);
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
        var audioProcessingRevision = _state.AudioProcessingRevision;
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
            await ReconfigureVideoAudioAsync(
                    source,
                    _mediaPool.CurrentIdentity,
                    audioProcessingRevision)
                .ConfigureAwait(true);
            return;
        }

        var identity = _mediaPool.CurrentIdentity;
        var sourceStartMs = _audioPlaybackController.Snapshot.AudibleClock?.PlaybackTimeMs ?? 0;
        if (source.DurationMs is ulong durationMs && durationMs > 0)
        {
            sourceStartMs = Math.Min(sourceStartMs, durationMs - 1);
        }
        if (!IsAudioProcessingRequestCurrent(audioProcessingRevision))
        {
            ReportStaleAudioProcessingRequest();
            return;
        }

        if (!await StopVideoAudioForTransitionAsync().ConfigureAwait(true))
        {
            return;
        }

        if (!IsAudioProcessingRequestCurrent(audioProcessingRevision))
        {
            ReportStaleAudioProcessingRequest();
            return;
        }

        var started = await StartAudioPlaybackAsync(source, identity, sourceStartMs).ConfigureAwait(true);
        if (!started.IsSuccess)
        {
            var stopped = _mediaPool.StopPlayback();
            ApplyMediaOperation(stopped, started.Error?.Message ?? "声音处理重启失败");
            return;
        }

        if (!IsAudioProcessingRequestCurrent(audioProcessingRevision))
        {
            if (await StopStaleAudioProcessingSessionAsync().ConfigureAwait(true))
            {
                ReportStaleAudioProcessingRequest();
            }

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

        if (!IsAudioProcessingRequestCurrent(audioProcessingRevision))
        {
            if (await StopStaleAudioProcessingSessionAsync().ConfigureAwait(true))
            {
                ReportStaleAudioProcessingRequest();
            }

            return;
        }

        StartAudioCompletionWatcher(identity);
        await PrepareNextAudioCandidateAsync(identity).ConfigureAwait(true);
        if (!IsAudioProcessingRequestCurrent(audioProcessingRevision))
        {
            if (await StopVideoAudioForTransitionAsync().ConfigureAwait(true))
            {
                ReportStaleAudioProcessingRequest();
            }

            return;
        }

        AudioDeviceStatusText.Text = _state.AudioProcessing
            ? "声音处理已应用 · FFmpeg 音频滤镜"
            : "声音处理已关闭 · 已恢复原始 PCM";
        _state.SetStatus(AudioDeviceStatusText.Text);
    }

    private async Task<bool> ReconfigureVideoAudioAsync(
        SourceMediaDto source,
        MediaPlaybackIdentity? identity,
        long audioProcessingRevision)
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

        if (!IsAudioProcessingRequestCurrent(audioProcessingRevision))
        {
            ReportStaleAudioProcessingRequest();
            return false;
        }

        var wasPaused = _mediaPool.Snapshot.PlaybackState is PlaybackState.Paused;
        if (!await StopVideoAudioForTransitionAsync().ConfigureAwait(true))
        {
            return false;
        }

        if (!IsAudioProcessingRequestCurrent(audioProcessingRevision))
        {
            ReportStaleAudioProcessingRequest();
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

        if (!IsAudioProcessingRequestCurrent(audioProcessingRevision))
        {
            if (await StopStaleAudioProcessingSessionAsync().ConfigureAwait(true))
            {
                ReportStaleAudioProcessingRequest();
            }

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

        if (!IsAudioProcessingRequestCurrent(audioProcessingRevision))
        {
            if (await StopStaleAudioProcessingSessionAsync().ConfigureAwait(true))
            {
                ReportStaleAudioProcessingRequest();
            }

            return false;
        }

        StartAudioCompletionWatcher(identity);
        AudioDeviceStatusText.Text = _state.AudioProcessing
            ? "视频声音处理已应用 · 从当前时间点恢复"
            : "视频声音处理已关闭 · 从当前时间点恢复原始 PCM";
        _state.SetStatus(AudioDeviceStatusText.Text);
        return true;
    }

    private bool IsAudioProcessingRequestCurrent(long revision) =>
        !_isClosing && _state.AudioProcessingRevision == revision;

    private void ReportStaleAudioProcessingRequest() =>
        _state.SetStatus("声音处理开关已再次变化，已跳过过期声音重建");

    private async Task<bool> StopStaleAudioProcessingSessionAsync()
    {
        await StopAudioCompletionWatcherAsync().ConfigureAwait(true);
        var stopped = await _audioPlaybackController.StopAsync().ConfigureAwait(true);
        if (!stopped.IsSuccess)
        {
            _state.SetStatus(stopped.Error?.Message ?? "过期声音会话停止失败");
            return false;
        }

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

        if (VideoPlaybackModeSelector.RequiresSessionRestart(
                controllerSnapshot.ActiveVideoProcessingMode,
                next.Mode))
        {
            await RestartVideoSessionForEffectModeAsync(
                    snapshot.SourceMediaPool[snapshot.SourceMediaIndex],
                    identity,
                    next)
                .ConfigureAwait(true);
            return;
        }

        var result = await _mpvController.UpdateEffectsAsync(
                identity,
                next,
                _windowCancellation.Token,
                waitForNextFrame: true)
            .ConfigureAwait(true);
        UpdateMediaProjection();
        _state.SetStatus(result.IsSuccess
            ? (_state.VideoProcessing
                ? next.Mode is MpvVideoProcessingMode.Gpu83
                    ? "视频处理已提交给 mpv GPU83 实时 shader"
                    : "视频处理已提交给 mpv CPU4 实时滤镜"
                : "视频处理已关闭并提交给 mpv")
            : result.Error?.Message ?? "视频处理运行时更新失败");
        _finalEffectController.Update(CreateFinalEffectSnapshot());
    }

    private async Task RestartVideoSessionForEffectModeAsync(
        SourceMediaDto source,
        MediaPlaybackIdentity identity,
        MpvVideoEffectSnapshot next)
    {
        var sourceStartMs = await ReadCurrentVideoPositionAsync(identity, source.DurationMs)
            .ConfigureAwait(true);
        if (sourceStartMs is null)
        {
            _state.SetStatus("无法读取当前视频位置，视频处理模式未切换；请重新播放后再试");
            return;
        }

        if (!EnsureFinalEffectWindowVisible()
            || _finalEffectWindow is null
            || !_finalEffectWindow.TryGetVideoSurfaceHandle(out var hostWindowId))
        {
            _state.SetStatus("最终效果视频表面尚未创建，视频处理模式未切换");
            return;
        }

        var runtime = await EnsureVerifiedMediaRuntimeAsync().ConfigureAwait(true);
        if (runtime is null)
        {
            return;
        }

        var wasPaused = _mediaPool.Snapshot.PlaybackState is PlaybackState.Paused;
        if (!await StopVideoAudioForTransitionAsync().ConfigureAwait(true))
        {
            return;
        }

        await StopVideoStateWatcherAsync().ConfigureAwait(true);
        var stopped = await _mpvController.ShutdownAsync(_windowCancellation.Token)
            .ConfigureAwait(true);
        if (!stopped.IsSuccess)
        {
            _state.SetStatus(stopped.Error?.Message ?? "切换视频处理模式时停止旧 mpv 会话失败");
            return;
        }

        var requestedMode = VideoPlaybackModeSelector.ToLaunchMode(next.Mode);
        var startupModes = requestedMode switch
        {
            MpvLaunchMode.Gpu83 => new[] { MpvLaunchMode.Gpu83, MpvLaunchMode.Cpu4, MpvLaunchMode.Original },
            MpvLaunchMode.Cpu4 => new[] { MpvLaunchMode.Cpu4, MpvLaunchMode.Original },
            _ => new[] { MpvLaunchMode.Original },
        };
        WindowsMpvPlaybackControllerResult started = default!;
        MpvVideoEffectSnapshot? startedEffects = null;
        MpvLaunchMode startedMode = requestedMode;
        MpvVideoParameterError? lastEffectError = null;
        foreach (var startupMode in startupModes)
        {
            if (!TryCreateRuntimeVideoEffectSnapshot(
                    source,
                    _state.VideoProcessing,
                    out var candidateEffects,
                    out var effectError,
                    modeOverride: startupMode)
                || candidateEffects is null)
            {
                lastEffectError = effectError;
                continue;
            }

            started = await _mpvController.StartAsync(
                    runtime,
                    source,
                    identity,
                    hostWindowId,
                    startupMode,
                    _windowCancellation.Token,
                    candidateEffects,
                    waitForFirstFrame: true,
                    sourceStartMs: sourceStartMs.Value)
                .ConfigureAwait(true);
            if (started.IsSuccess)
            {
                startedEffects = candidateEffects;
                startedMode = startupMode;
                break;
            }
        }

        if (!started.IsSuccess || startedEffects is null)
        {
            _state.SetStatus(lastEffectError?.Message ?? started.Error?.Message ?? "视频处理模式切换失败；请重新播放后再试");
            return;
        }

        var audioStarted = false;
        if (!string.IsNullOrWhiteSpace(source.AudioCodecName))
        {
            var audio = await StartAudioPlaybackAsync(source, identity, sourceStartMs.Value)
                .ConfigureAwait(true);
            audioStarted = audio.IsSuccess;
            AudioDeviceStatusText.Text = audioStarted
                ? "视频声音已从当前时间点重新连接"
                : "视频画面已切换 · 声音输出不可用";
        }

        if (wasPaused)
        {
            var pausedVideo = await _mpvController.TogglePauseAsync(
                    identity,
                    _windowCancellation.Token)
                .ConfigureAwait(true);
            if (!pausedVideo.IsSuccess)
            {
                _state.SetStatus(pausedVideo.Error?.Message ?? "视频处理模式切换后暂停同步失败");
                return;
            }

            if (audioStarted)
            {
                var pausedAudio = await _audioPlaybackController.PauseAsync().ConfigureAwait(true);
                if (!pausedAudio.IsSuccess)
                {
                    _state.SetStatus(pausedAudio.Error?.Message ?? "视频处理模式切换后声音暂停同步失败");
                    return;
                }
            }
        }

        if (_mediaPool.CurrentIdentity != identity)
        {
            _ = await _audioPlaybackController.StopAsync().ConfigureAwait(true);
            _ = await _mpvController.ShutdownAsync(_windowCancellation.Token).ConfigureAwait(true);
            _state.SetStatus("媒体源在视频处理模式切换期间发生变化，已拒绝过期会话");
            return;
        }

        if (!wasPaused)
        {
            StartVideoStateWatcher(identity);
        }

        if (audioStarted && !wasPaused)
        {
            StartAudioCompletionWatcher(identity);
        }

        UpdateMediaProjection();
        _finalEffectController.Update(CreateFinalEffectSnapshot());
        _state.SetStatus(audioStarted || string.IsNullOrWhiteSpace(source.AudioCodecName)
            ? startedMode == requestedMode
                ? $"视频处理已切换到 {VideoPlaybackModeSelector.DescribeActive(startedEffects.Mode)}，已从当前位置恢复"
                : $"视频处理已切换并回退到 {VideoPlaybackModeSelector.DescribeActive(startedEffects.Mode)}，已从当前位置恢复"
            : $"视频处理已切换到 {VideoPlaybackModeSelector.DescribeActive(startedEffects.Mode)}，声音输出不可用");
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

    private async void ApplyEffectCycleButton_Click(object sender, RoutedEventArgs e)
    {
        if (!_login.CanEnterWorkbench || _isClosing)
        {
            return;
        }

        var tag = (sender as FrameworkElement)?.Tag as string;
        var video = string.Equals(tag, "video", StringComparison.Ordinal);
        var minimumTextBox = video ? VideoCycleMinTextBox : AudioCycleMinTextBox;
        var maximumTextBox = video ? VideoCycleMaxTextBox : AudioCycleMaxTextBox;
        if (!TryParseCycleSeconds(minimumTextBox.Text, video ? "视频周期最小值" : "声音周期最小值", out var minimumMs, out var error)
            || !TryParseCycleSeconds(maximumTextBox.Text, video ? "视频周期最大值" : "声音周期最大值", out var maximumMs, out error))
        {
            _state.SetStatus(error ?? "周期输入无效");
            return;
        }

        var next = video
            ? new EffectCycleSettings(minimumMs, maximumMs, _effectCycleSettings.AudioPeriodMinMs, _effectCycleSettings.AudioPeriodMaxMs)
            : new EffectCycleSettings(_effectCycleSettings.VideoPeriodMinMs, _effectCycleSettings.VideoPeriodMaxMs, minimumMs, maximumMs);
        if (!EffectCycleSettings.TryCreate(
                next.VideoPeriodMinMs,
                next.VideoPeriodMaxMs,
                next.AudioPeriodMinMs,
                next.AudioPeriodMaxMs,
                out var validated,
                out error)
            || validated is null)
        {
            _state.SetStatus(error ?? "周期范围无效");
            return;
        }

        _effectCycleSettings = validated;
        UpdateEffectCycleProjection();
        if (_preferences is not null)
        {
            var saveWarning = await _preferences
                .SaveEffectCycleSettingsAsync(_effectCycleSettings, _windowCancellation.Token)
                .ConfigureAwait(true);
            if (saveWarning is not null)
            {
                _state.SetStatus(saveWarning);
                return;
            }
        }

        _state.SetStatus(video ? "视频处理周期已应用；下一周期按新范围生成" : "声音处理周期已应用；下一周期按新范围生成");
    }

    private void UpdateEffectCycleProjection()
    {
        VideoCycleRangeText.Text = FormatCycleRange(
            _effectCycleSettings.VideoPeriodMinMs,
            _effectCycleSettings.VideoPeriodMaxMs);
        AudioCycleRangeText.Text = FormatCycleRange(
            _effectCycleSettings.AudioPeriodMinMs,
            _effectCycleSettings.AudioPeriodMaxMs);
        CycleSummaryText.Text = $"视频 {FormatCycleRange(_effectCycleSettings.VideoPeriodMinMs, _effectCycleSettings.VideoPeriodMaxMs)} · "
            + $"声音 {FormatCycleRange(_effectCycleSettings.AudioPeriodMinMs, _effectCycleSettings.AudioPeriodMaxMs)} · "
            + $"插话 {FormatCycleRange(_interludeConfig.IntervalMinMs, _interludeConfig.IntervalMaxMs)}";
        if (!VideoCycleMinTextBox.IsKeyboardFocusWithin
            && !VideoCycleMaxTextBox.IsKeyboardFocusWithin)
        {
            VideoCycleMinTextBox.Text = FormatCycleSeconds(_effectCycleSettings.VideoPeriodMinMs);
            VideoCycleMaxTextBox.Text = FormatCycleSeconds(_effectCycleSettings.VideoPeriodMaxMs);
        }

        if (!AudioCycleMinTextBox.IsKeyboardFocusWithin
            && !AudioCycleMaxTextBox.IsKeyboardFocusWithin)
        {
            AudioCycleMinTextBox.Text = FormatCycleSeconds(_effectCycleSettings.AudioPeriodMinMs);
            AudioCycleMaxTextBox.Text = FormatCycleSeconds(_effectCycleSettings.AudioPeriodMaxMs);
        }
        ApplyVideoCycleButton.IsEnabled = _login.CanEnterWorkbench;
        ApplyAudioCycleButton.IsEnabled = _login.CanEnterWorkbench;
    }

    private static bool TryParseCycleSeconds(
        string text,
        string label,
        out ulong milliseconds,
        out string? error)
    {
        milliseconds = 0;
        error = null;
        if (!double.TryParse(text, NumberStyles.Float, CultureInfo.CurrentCulture, out var seconds)
            || double.IsNaN(seconds)
            || double.IsInfinity(seconds))
        {
            error = $"{label}必须是数字。";
            return false;
        }

        var value = seconds * 1_000d;
        if (value < EffectCycleSettings.MinimumPeriodMs
            || value > EffectCycleSettings.MaximumPeriodMs)
        {
            error = $"{label}必须在 1 到 60 秒之间。";
            return false;
        }

        milliseconds = checked((ulong)Math.Round(value, MidpointRounding.AwayFromZero));
        return true;
    }

    private static string FormatCycleSeconds(ulong milliseconds) =>
        (milliseconds / 1_000d).ToString("0.###", CultureInfo.CurrentCulture);

    private static string FormatCycleRange(ulong minimum, ulong maximum) =>
        $"{FormatCycleSeconds(minimum)}–{FormatCycleSeconds(maximum)} 秒";

    private void ResetVideoEffectsButton_Click(object sender, RoutedEventArgs e)
    {
        _state.VideoEffects.Reset();
        _state.SetStatus("已恢复四项已接入视频参数的默认草稿");
    }
}
