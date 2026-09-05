using System.Windows;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using GpAutoLive.App.Features.Effects;
using GpAutoLive.App.Features.Playback;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private sealed record AudioPlaybackPreparation(
        FfmpegPcmDecodePlan Plan,
        string PortAudioPath,
        int DeviceIndex);

    // 通用播放 UI 编排：播放命令串行化、导航、观察者、进度投影与 seek。
    private async void PreviousButton_Click(object sender, RoutedEventArgs e) =>
        await NavigateMediaAsync(next: false).ConfigureAwait(true);

    private async void NextButton_Click(object sender, RoutedEventArgs e) =>
        await NavigateMediaAsync(next: true).ConfigureAwait(true);

    private Task NavigateMediaAsync(bool next) =>
        RunPlaybackCommandAsync(() => NavigateMediaCoreAsync(next));

    private async Task NavigateMediaCoreAsync(bool next)
    {
        if (!_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        var snapshot = _mediaPool.Snapshot;
        var wasPlaying = snapshot.PlaybackState is PlaybackState.Playing;
        if (snapshot.SourceMediaPool.IsEmpty)
        {
            _state.SetStatus("媒体池为空，无法切换媒体项");
            return;
        }

        if (!await StopRtmpForMediaMutationAsync().ConfigureAwait(true))
        {
            return;
        }

        await StopVideoStateWatcherAsync().ConfigureAwait(true);
        await StopInterludeForPriorityAsync().ConfigureAwait(true);

        var audioControllerState = _audioPlaybackController.Snapshot.State;
        var audioSessionWasActive = audioControllerState is WindowsAudioPlaybackState.Starting
            or WindowsAudioPlaybackState.Playing
            or WindowsAudioPlaybackState.Paused
            or WindowsAudioPlaybackState.Stopping;
        if (audioSessionWasActive)
        {
            await StopAudioCompletionWatcherAsync().ConfigureAwait(true);
            var stoppedAudio = await _audioPlaybackController.StopAsync().ConfigureAwait(true);
            if (!stoppedAudio.IsSuccess)
            {
                _state.SetStatus(stoppedAudio.Error?.Message ?? "纯音频切换前停止失败");
                return;
            }

            AudioDeviceStatusText.Text = "PortAudio 输出流已停止，等待新媒体项";
        }

        var result = next
            ? _mediaPool.Next(_mediaPool.CurrentIdentity)
            : _mediaPool.Previous(_mediaPool.CurrentIdentity);
        if (!result.IsSuccess)
        {
            ApplyMediaOperation(result, result.Error?.Message ?? "切换媒体项失败");
            return;
        }

        var currentController = _mpvController.Snapshot;
        var switchedVideoSession = false;
        if (currentController.ActiveIdentity is MediaPlaybackIdentity previousIdentity)
        {
            var target = result.Snapshot.SourceMediaPool[result.Snapshot.SourceMediaIndex];
            if (target.MediaKind is MediaKind.Video
                && snapshot.PlaybackState is PlaybackState.Playing
                && previousIdentity.SourceRevision == result.Snapshot.SourceRevision)
            {
                var switched = await _mpvController.SwitchSourceAsync(
                    target,
                    _mediaPool.CurrentIdentity,
                    _windowCancellation.Token).ConfigureAwait(true);
                if (!switched.IsSuccess)
                {
                    await _mpvController.ShutdownAsync(_windowCancellation.Token).ConfigureAwait(true);
                    var stoppedAfterSwitchFailure = _mediaPool.StopPlayback();
                    ApplyMediaOperation(
                        stoppedAfterSwitchFailure,
                        switched.Error?.Message ?? "视频切换失败，已停止媒体运行时");
                    if (stoppedAfterSwitchFailure.IsSuccess)
                    {
                        MediaListBox.SelectedIndex = stoppedAfterSwitchFailure.Snapshot.SourceMediaIndex;
                    }

                    return;
                }

                switchedVideoSession = true;
            }
            else
            {
                await _mpvController.ShutdownAsync(_windowCancellation.Token).ConfigureAwait(true);
            }
        }

        var nextSource = result.Snapshot.SourceMediaPool[result.Snapshot.SourceMediaIndex];
        if (audioSessionWasActive && wasPlaying)
        {
            // 当前版本不跨媒体项复用解码会话；切到纯音频或带音轨视频时都重新建立
            // 真实设备链路，避免播放池状态先行而没有对应输出会话。
            if (nextSource.MediaKind is MediaKind.Audio)
            {
                var startedAudio = await StartAudioPlaybackAsync(
                    nextSource,
                    _mediaPool.CurrentIdentity).ConfigureAwait(true);
                if (!startedAudio.IsSuccess)
                {
                    var stoppedPool = _mediaPool.StopPlayback();
                    ApplyMediaOperation(stoppedPool, startedAudio.Error?.Message ?? "纯音频切换失败");
                    return;
                }

                StartAudioCompletionWatcher(_mediaPool.CurrentIdentity);
                await PrepareNextAudioCandidateAsync(_mediaPool.CurrentIdentity).ConfigureAwait(true);
            }
            else if (switchedVideoSession && !string.IsNullOrWhiteSpace(nextSource.AudioCodecName))
            {
                var startedAudio = await StartAudioPlaybackAsync(
                    nextSource,
                    _mediaPool.CurrentIdentity).ConfigureAwait(true);
                if (!startedAudio.IsSuccess)
                {
                    await _mpvController.ShutdownAsync(_windowCancellation.Token).ConfigureAwait(true);
                    var stoppedPool = _mediaPool.StopPlayback();
                    ApplyMediaOperation(stoppedPool, startedAudio.Error?.Message ?? "视频切换后的声音启动失败");
                    return;
                }

                AudioDeviceStatusText.Text = "视频已切换 · 新声音已连接";
            }
        }

        if (wasPlaying && nextSource.MediaKind is MediaKind.Video && !switchedVideoSession)
        {
            // 例如从纯音频切到视频时没有可复用的 mpv 会话；保持池状态为 Playing，
            // 复用统一播放入口建立新的画面和声音会话。
            await TogglePlaybackCoreAsync().ConfigureAwait(true);
            return;
        }

        if (switchedVideoSession
            && result.IsSuccess
            && result.Snapshot.PlaybackState is PlaybackState.Playing)
        {
            StartVideoStateWatcher(_mediaPool.CurrentIdentity);
        }

        ApplyMediaOperation(
            result,
            $"已切换到第 {result.Snapshot.SourceMediaIndex + 1} 项");
        if (result.IsSuccess)
        {
            MediaListBox.SelectedIndex = result.Snapshot.SourceMediaIndex;
        }
    }

    private async void PlayPauseButton_Click(object sender, RoutedEventArgs e) =>
        await TogglePlaybackAsync().ConfigureAwait(true);

    private async void StopButton_Click(object sender, RoutedEventArgs e) =>
        await StopPlaybackAsync().ConfigureAwait(true);

    private void Window_PreviewKeyDown(object sender, KeyEventArgs e)
    {
        if (Keyboard.FocusedElement is TextBoxBase)
        {
            return;
        }

        if (e.Key == Key.Space && Keyboard.Modifiers == ModifierKeys.None)
        {
            if (!_login.CanEnterWorkbench)
            {
                _state.SetStatus("请先完成登录与设备授权");
                e.Handled = true;
                return;
            }

            _ = TogglePlaybackAsync();
            e.Handled = true;
            return;
        }

        if (e.Key == Key.O && Keyboard.Modifiers == ModifierKeys.Control)
        {
            if (!_login.CanEnterWorkbench)
            {
                _state.SetStatus("请先完成登录与设备授权");
                e.Handled = true;
                return;
            }

            _ = ImportMediaAsync();
            e.Handled = true;
            return;
        }

        if (e.Key == Key.Delete && Keyboard.Modifiers == ModifierKeys.None && MediaListBox.SelectedIndex >= 0)
        {
            _ = RemoveSelectedMediaAsync();
            e.Handled = true;
            return;
        }

        if (e.Key == Key.S && Keyboard.Modifiers == (ModifierKeys.Control | ModifierKeys.Shift))
        {
            if (!_login.CanEnterWorkbench)
            {
                _state.SetStatus("请先完成登录与设备授权");
                e.Handled = true;
                return;
            }

            _ = StopPlaybackAsync();
            e.Handled = true;
            return;
        }

        if (e.Key == Key.F11 && Keyboard.Modifiers == ModifierKeys.None)
        {
            if (!_login.CanEnterWorkbench)
            {
                _state.SetStatus("请先完成登录与设备授权");
                e.Handled = true;
                return;
            }

            if (EnsureFinalEffectWindowVisible())
            {
                _finalEffectWindow!.ToggleFullscreen();
                _state.SetStatus(_finalEffectWindow.IsFullscreen
                    ? "最终效果已全屏（F11/Esc 退出）"
                    : "最终效果已退出全屏");
            }
            e.Handled = true;
            return;
        }

        if (e.Key == Key.OemComma && Keyboard.Modifiers == ModifierKeys.Control)
        {
            _ = OpenSettingsAsync();
            e.Handled = true;
        }
    }

    private Task TogglePlaybackAsync() => RunPlaybackCommandAsync(TogglePlaybackCoreAsync);

    private async Task TogglePlaybackCoreAsync()
    {
        var snapshot = _mediaPool.Snapshot;
        if (snapshot.SourceMediaPool.IsEmpty)
        {
            _state.SetStatus("播放池为空，无法开始播放");
            return;
        }

        var source = snapshot.SourceMediaPool[snapshot.SourceMediaIndex];
        if (source.MediaKind is not MediaKind.Video)
        {
            var audioSnapshot = _audioPlaybackController.Snapshot;
            if (audioSnapshot.State is WindowsAudioPlaybackState.Playing)
            {
                var cancelledCandidate = await _audioPlaybackController.CancelPreparedNextAsync()
                    .ConfigureAwait(true);
                if (!cancelledCandidate.IsSuccess)
                {
                    _state.SetStatus(cancelledCandidate.Error?.Message ?? "纯音频暂停前清理候选失败");
                    return;
                }

                var paused = await _audioPlaybackController.PauseAsync().ConfigureAwait(true);
                if (!paused.IsSuccess)
                {
                    _state.SetStatus(paused.Error?.Message ?? "纯音频暂停失败");
                    return;
                }

                var pausedPool = _mediaPool.PausePlayback();
                AudioDeviceStatusText.Text = "PortAudio 输出流已暂停；PCM 位置已保留";
                ApplyMediaOperation(pausedPool, pausedPool.IsSuccess ? "纯音频播放已暂停" : pausedPool.Error?.Message ?? "纯音频状态提交失败");
                return;
            }

            if (audioSnapshot.State is WindowsAudioPlaybackState.Paused)
            {
                var resumed = await _audioPlaybackController.ResumeAsync().ConfigureAwait(true);
                if (!resumed.IsSuccess)
                {
                    _state.SetStatus(resumed.Error?.Message ?? "纯音频恢复失败");
                    return;
                }

                var resumedPool = _mediaPool.ResumePlayback();
                AudioDeviceStatusText.Text = "PortAudio 输出流已恢复";
                ApplyMediaOperation(resumedPool, resumedPool.IsSuccess ? "纯音频播放已恢复" : resumedPool.Error?.Message ?? "纯音频状态提交失败");
                return;
            }

            var requestedIdentity = _mediaPool.CurrentIdentity;
            var started = await StartAudioPlaybackAsync(source, requestedIdentity).ConfigureAwait(true);
            if (!started.IsSuccess)
            {
                _state.SetStatus(started.Error?.Message ?? "纯音频播放启动失败");
                return;
            }

            var audioResult = _mediaPool.StartPlayback();
            if (_mediaPool.CurrentIdentity != requestedIdentity)
            {
                await _audioPlaybackController.StopAsync().ConfigureAwait(true);
                var staleStopped = _mediaPool.StopPlayback();
                ApplyMediaOperation(staleStopped, "播放池在音频启动期间已变化，已拒绝过期音频会话");
                return;
            }

            if (!audioResult.IsSuccess)
            {
                await _audioPlaybackController.StopAsync().ConfigureAwait(true);
            }
            else
            {
                StartAudioCompletionWatcher(_mediaPool.CurrentIdentity);
                await PrepareNextAudioCandidateAsync(_mediaPool.CurrentIdentity).ConfigureAwait(true);
                AudioDeviceStatusText.Text = "PortAudio 输出流已启动 · FFmpeg PCM 已连接";
            }

            ApplyMediaOperation(audioResult, audioResult.IsSuccess
                ? "纯音频播放已开始 · PortAudio"
                : audioResult.Error?.Message ?? "纯音频播放状态提交失败");
            return;
        }

        if (!EnsureFinalEffectWindowVisible()
            || _finalEffectWindow is null
            || !_finalEffectWindow.TryGetVideoSurfaceHandle(out var hostWindowId))
        {
            _state.SetStatus("最终效果视频表面尚未创建");
            return;
        }

        var runtime = await EnsureVerifiedMediaRuntimeAsync().ConfigureAwait(true);
        if (runtime is null)
        {
            return;
        }

        var identity = _mediaPool.CurrentIdentity;
        var controllerSnapshot = _mpvController.Snapshot;
        var playbackIntentMatchesController =
            snapshot.PlaybackState is PlaybackState.Playing
                && controllerSnapshot.State is WindowsMpvPlaybackControllerState.Playing
            || snapshot.PlaybackState is PlaybackState.Paused
                && controllerSnapshot.State is WindowsMpvPlaybackControllerState.Paused;
        var reusedController = playbackIntentMatchesController
            && controllerSnapshot.ActiveIdentity == identity
            && controllerSnapshot.Runtime.State is WindowsMpvPlaybackRuntimeState.Running;
        var audioUnavailable = false;
        var audioRecovered = false;
        WindowsMpvPlaybackControllerResult controllerResult;
        var launchMode = MpvLaunchMode.Original;
        var startedMode = MpvLaunchMode.Original;
        if (reusedController)
        {
            var shouldPauseAudio = snapshot.PlaybackState is PlaybackState.Playing;
            controllerResult = await _mpvController.TogglePauseAsync(identity, _windowCancellation.Token)
                .ConfigureAwait(true);
            if (controllerResult.IsSuccess && !string.IsNullOrWhiteSpace(source.AudioCodecName))
            {
                var audioState = _audioPlaybackController.Snapshot.State;
                var audioResult = shouldPauseAudio
                    && audioState is WindowsAudioPlaybackState.Playing
                    ? await PauseVideoAudioAsync().ConfigureAwait(true)
                    : !shouldPauseAudio
                        && audioState is WindowsAudioPlaybackState.Paused
                        ? await _audioPlaybackController.ResumeAsync().ConfigureAwait(true)
                        : null;
                if (audioResult is { IsSuccess: false })
                {
                    _ = await _mpvController.TogglePauseAsync(identity, _windowCancellation.Token)
                        .ConfigureAwait(true);
                    _state.SetStatus(audioResult.Error?.Message ?? "视频声音暂停状态同步失败");
                    return;
                }

                if (!shouldPauseAudio
                    && ShouldRetryVideoAudioOnResume(
                        snapshot.PlaybackState,
                        hasAudioTrack: true,
                        audioState))
                {
                    var sourcePositionMs = await ReadCurrentVideoPositionAsync(
                            identity,
                            source.DurationMs)
                        .ConfigureAwait(true);
                    if (sourcePositionMs is ulong resumePositionMs)
                    {
                        var stoppedFailedAudio = await _audioPlaybackController
                            .StopAsync()
                            .ConfigureAwait(true);
                        if (!stoppedFailedAudio.IsSuccess)
                        {
                            audioUnavailable = true;
                            AudioDeviceStatusText.Text = "视频画面继续运行 · 声音旧会话未能收尾";
                        }
                        else
                        {
                            var restartedAudio = await StartAudioPlaybackAsync(
                                    source,
                                    identity,
                                    resumePositionMs)
                                .ConfigureAwait(true);
                            if (restartedAudio.IsSuccess)
                            {
                                audioRecovered = true;
                                AudioDeviceStatusText.Text = "视频声音已恢复 · 从当前时间点重新连接";
                            }
                            else
                            {
                                audioUnavailable = true;
                                AudioDeviceStatusText.Text = "视频画面继续运行 · 声音输出仍不可用";
                            }
                        }
                    }
                    else
                    {
                        audioUnavailable = true;
                        AudioDeviceStatusText.Text = "视频画面继续运行 · 无法定位声音恢复位置";
                    }
                }
            }
        }
        else
        {
            if (controllerSnapshot.ActiveIdentity is not null)
            {
                _ = await _mpvController.ShutdownAsync(_windowCancellation.Token).ConfigureAwait(true);
            }

            launchMode = VideoPlaybackModeSelector.Select(
                _state.VideoProcessing,
                IsFullGpu83Runtime(runtime));
            var startupModes = launchMode switch
            {
                MpvLaunchMode.Gpu83 => new[] { MpvLaunchMode.Gpu83, MpvLaunchMode.Cpu4, MpvLaunchMode.Original },
                MpvLaunchMode.Cpu4 => new[] { MpvLaunchMode.Cpu4, MpvLaunchMode.Original },
                _ => new[] { MpvLaunchMode.Original },
            };
            controllerResult = default!;
            MpvVideoParameterError? lastEffectError = null;
            var hasStartResult = false;
            startedMode = launchMode;
            foreach (var startupMode in startupModes)
            {
                if (!TryCreateRuntimeVideoEffectSnapshot(
                        source,
                        _state.VideoProcessing,
                        out var initialEffectSnapshot,
                        out var initialEffectError,
                        modeOverride: startupMode)
                    || initialEffectSnapshot is null)
                {
                    lastEffectError = initialEffectError;
                    continue;
                }

                controllerResult = await _mpvController.StartAsync(
                    runtime,
                    source,
                    identity,
                    hostWindowId,
                    startupMode,
                    _windowCancellation.Token,
                    initialEffectSnapshot,
                    waitForFirstFrame: true).ConfigureAwait(true);
                hasStartResult = true;
                if (controllerResult.IsSuccess)
                {
                    startedMode = startupMode;
                    break;
                }
            }

            if (!hasStartResult)
            {
                _state.SetStatus(lastEffectError?.Message ?? "视频处理参数快照无效，未启动播放");
                return;
            }

            if (controllerResult.IsSuccess && startedMode != launchMode)
            {
                _state.SetStatus($"视频处理启动失败，已单向回退到 {DescribeVideoMode(startedMode)}");
            }
        }

        if (!controllerResult.IsSuccess)
        {
            _state.SetStatus(controllerResult.Error?.Message ?? "视频播放运行时启动失败");
            _finalEffectController.Update(CreateFinalEffectSnapshot());
            return;
        }

        if (!reusedController && !string.IsNullOrWhiteSpace(source.AudioCodecName))
        {
            var startedAudio = await StartAudioPlaybackAsync(source, identity).ConfigureAwait(true);
            if (!startedAudio.IsSuccess)
            {
                // mpv 已经成功拥有视频表面；声音是独立输出链，PortAudio 设备失败
                // 不能回滚可用的画面。后续重试播放声音时仍使用同一媒体身份。
                audioUnavailable = true;
                AudioDeviceStatusText.Text = "视频画面已启动 · 声音输出不可用";
            }
            else
            {
                AudioDeviceStatusText.Text = "视频声音已连接 · FFmpeg PCM / PortAudio";
            }
        }

        if (_mediaPool.CurrentIdentity != identity)
        {
            if (!reusedController)
            {
                _ = await _audioPlaybackController.StopAsync().ConfigureAwait(true);
            }
            _ = await _mpvController.ShutdownAsync(_windowCancellation.Token).ConfigureAwait(true);
            _state.SetStatus("播放池在启动期间已变化，已拒绝过期视频会话");
            return;
        }

        var result = reusedController
            ? snapshot.PlaybackState is PlaybackState.Playing
                ? _mediaPool.PausePlayback()
                : _mediaPool.ResumePlayback()
            : _mediaPool.StartPlayback();
        if (!result.IsSuccess && !reusedController)
        {
            _ = await _audioPlaybackController.StopAsync().ConfigureAwait(true);
            _ = await _mpvController.ShutdownAsync(_windowCancellation.Token).ConfigureAwait(true);
        }
        if (result.IsSuccess
            && result.Snapshot.PlaybackState is PlaybackState.Playing
            && !reusedController)
        {
            StartVideoStateWatcher(identity);
        }

        ApplyMediaOperation(result, result.IsSuccess
            ? result.Snapshot.PlaybackState is PlaybackState.Playing
                ? audioUnavailable ? "视频播放已恢复 · 声音输出不可用" : audioRecovered
                    ? "视频播放已恢复 · 声音已重新连接"
                    : startedMode != launchMode
                    ? $"视频播放已开始 · 已回退到 {DescribeVideoMode(startedMode)}"
                    : "视频播放已开始"
                : "视频播放已暂停"
            : result.Error?.Message ?? "播放状态提交失败");
    }

    private static string DescribeVideoMode(MpvLaunchMode mode) => mode switch
    {
        MpvLaunchMode.Gpu83 => "GPU83",
        MpvLaunchMode.Cpu4 => "CPU4",
        _ => "Original",
    };

    internal static bool ShouldRetryVideoAudioOnResume(
        PlaybackState playbackState,
        bool hasAudioTrack,
        WindowsAudioPlaybackState audioState) =>
        playbackState is PlaybackState.Paused
        && hasAudioTrack
        && audioState is (WindowsAudioPlaybackState.Idle
            or WindowsAudioPlaybackState.Failed
            or WindowsAudioPlaybackState.Completed);

    private async Task<WindowsAudioPlaybackResult> PauseVideoAudioAsync()
    {
        var cancelledCandidate = await _audioPlaybackController.CancelPreparedNextAsync()
            .ConfigureAwait(true);
        return !cancelledCandidate.IsSuccess
            ? cancelledCandidate
            : await _audioPlaybackController.PauseAsync().ConfigureAwait(true);
    }

    private Task StopPlaybackAsync() => RunPlaybackCommandAsync(StopPlaybackCoreAsync);

    private async Task StopPlaybackAfterFinalEffectWindowClosedAsync()
    {
        if (!await StopMediaForMutationAsync().ConfigureAwait(true))
        {
            return;
        }

        ApplyMediaSnapshot(
            _mediaPool.Snapshot,
            "最终效果窗口已关闭；播放与视频运行时已释放");
    }

    private async Task StopPlaybackCoreAsync()
    {
        if (!await StopRtmpForMediaMutationAsync().ConfigureAwait(true))
        {
            return;
        }

        await StopVideoStateWatcherAsync().ConfigureAwait(true);
        await StopInterludeForPriorityAsync().ConfigureAwait(true);
        var audioState = _audioPlaybackController.Snapshot.State;
        if (audioState is WindowsAudioPlaybackState.Starting
            or WindowsAudioPlaybackState.Playing
            or WindowsAudioPlaybackState.Paused
            or WindowsAudioPlaybackState.Stopping)
        {
            var stoppedAudio = await _audioPlaybackController.StopAsync().ConfigureAwait(true);
            if (!stoppedAudio.IsSuccess)
            {
                _state.SetStatus(stoppedAudio.Error?.Message ?? "纯音频停止失败");
            }
            else
            {
                AudioDeviceStatusText.Text = "PortAudio 输出流已停止";
            }
        }

        var controllerIdentity = _mpvController.Snapshot.ActiveIdentity;
        var result = _mediaPool.StopPlayback();
        if (controllerIdentity is MediaPlaybackIdentity identity)
        {
            var stopped = await _mpvController.StopPlaybackAsync(identity, _windowCancellation.Token)
                .ConfigureAwait(true);
            if (!stopped.IsSuccess)
            {
                _state.SetStatus(stopped.Error?.Message ?? "视频停止命令失败");
            }
        }

        ApplyMediaOperation(result, result.IsSuccess ? "播放已停止" : result.Error?.Message ?? "停止操作失败");
    }

    private async Task<WindowsAudioPlaybackResult> StartAudioPlaybackAsync(
        SourceMediaDto source,
        MediaPlaybackIdentity identity,
        ulong sourceStartMs = 0)
    {
        if (_audioPlaybackController.Snapshot.State is WindowsAudioPlaybackState.Playing
            or WindowsAudioPlaybackState.Paused
            or WindowsAudioPlaybackState.Starting)
        {
            return new(
                false,
                _audioPlaybackController.Snapshot,
                new WindowsAudioPlaybackError("already_running", "纯音频播放已经在运行。"));
        }

        var preparation = await TryPrepareAudioPlaybackAsync(source, identity, sourceStartMs).ConfigureAwait(true);
        if (preparation.Preparation is null)
        {
            return new(
                false,
                _audioPlaybackController.Snapshot,
                preparation.Error ?? new WindowsAudioPlaybackError(
                    "invalid_plan",
                    "纯音频解码计划无效。"));
        }

        var prepared = preparation.Preparation;
        var finalPcmBus = new FinalPcmBus(capacityFrames: 48_000, prepared.Plan.Channels);
        var result = await _audioPlaybackController.StartAsync(
                prepared.Plan,
                prepared.PortAudioPath,
                new WindowsPortAudioOutputConfig(
                    prepared.DeviceIndex,
                    prepared.Plan.Channels,
                    FfmpegPcmDecodePlanBuilder.DefaultSampleRateHz),
                loop: false,
                _windowCancellation.Token,
                finalPcmBus,
                CreateBaseAudioMixPolicy,
                enableInterludeMix: true,
                mixEnvelopeOptions: new AudioPcmMixEnvelopeOptions(
                    FfmpegPcmDecodePlanBuilder.DefaultSampleRateHz,
                    _interludeConfig.DuckingAttackMs,
                    _interludeConfig.DuckingReleaseMs))
            .ConfigureAwait(true);
        if (!result.IsSuccess)
        {
            finalPcmBus.Dispose();
        }

        if (result.IsSuccess)
        {
            _audioPlaybackIdentity = identity;
        }

        return result;
    }

    private async Task<(
        AudioPlaybackPreparation? Preparation,
        WindowsAudioPlaybackError? Error)> TryPrepareAudioPlaybackAsync(
        SourceMediaDto source,
        MediaPlaybackIdentity identity,
        ulong sourceStartMs = 0)
    {
        var runtime = await EnsureVerifiedMediaRuntimeAsync().ConfigureAwait(true);
        if (runtime is null)
        {
            return (null, new WindowsAudioPlaybackError(
                "runtime_unavailable",
                "媒体运行资源未校验。",
                Retryable: true));
        }

        if (!runtime.TryGetResource("ffmpeg.exe", out var ffmpeg) || ffmpeg is null)
        {
            return (null, new WindowsAudioPlaybackError("ffmpeg_missing", "FFmpeg 运行资源未安装。"));
        }

        if (!runtime.TryGetResource("portaudio_x64.dll", out var portAudio) || portAudio is null)
        {
            return (null, new WindowsAudioPlaybackError("portaudio_missing", "PortAudio 运行资源未安装。"));
        }

        if (AudioOutputDeviceComboBox.SelectedValue is not int)
        {
            await RefreshAudioDevicesAsync().ConfigureAwait(true);
        }

        if (AudioOutputDeviceComboBox.SelectedValue is not int deviceIndex)
        {
            AudioDeviceStatusText.Text = "请先刷新并选择 PortAudio 输出设备";
            return (null, new WindowsAudioPlaybackError(
                "audio_device_unselected",
                "尚未选择 PortAudio 输出设备。"));
        }

        var channels = source.AudioChannelCount == 1 ? 1 : 2;
        if (!FfmpegPcmDecodePlanBuilder.TryCreate(
                ffmpeg.AbsolutePath,
                source,
                FfmpegPcmDecodePlanBuilder.DefaultSampleRateHz,
                channels,
                out var plan,
                out var planError,
                _state.AudioProcessing ? CreateCurrentAudioEffectParameters() : null,
                sourceStartMs)
            || plan is null)
        {
            return (null, new WindowsAudioPlaybackError(
                planError?.Code.ToString().ToLowerInvariant() ?? "invalid_plan",
                planError?.Message ?? "纯音频解码计划无效。",
                planError?.Retryable == true));
        }

        if (_mediaPool.CurrentIdentity != identity)
        {
            return (null, new WindowsAudioPlaybackError(
                "stale_playback_identity",
                "播放池在音频预载期间已变化。"));
        }

        return (new AudioPlaybackPreparation(plan, portAudio.AbsolutePath, deviceIndex), null);
    }

    private async Task PrepareNextAudioCandidateAsync(MediaPlaybackIdentity identity)
    {
        var snapshot = _mediaPool.Snapshot;
        // 声音处理开启时，唯一候选槽由同源周期效果使用；同时预载下一媒体项
        // 会让周期编排把“下一项”误提交成效果候选，导致提前切源且参数不变。
        if (snapshot.PlaybackState is not PlaybackState.Playing
            || _mediaPool.CurrentIdentity != identity
            || snapshot.SourceMediaPool.IsEmpty
            || _state.AudioProcessing
            || _audioPlaybackController.HasPreparedNext)
        {
            return;
        }

        var nextIndex = snapshot.SourceMediaIndex + 1;
        if (nextIndex >= snapshot.SourceMediaPool.Length)
        {
            nextIndex = 0;
        }

        var nextSource = snapshot.SourceMediaPool[nextIndex];
        if (nextSource.MediaKind is not MediaKind.Audio
            || string.IsNullOrWhiteSpace(nextSource.AudioCodecName))
        {
            return;
        }

        var preparation = await TryPrepareAudioPlaybackAsync(nextSource, identity).ConfigureAwait(true);
        if (preparation.Preparation is null)
        {
            return;
        }

        _ = await _audioPlaybackController.PrepareNextAsync(
                preparation.Preparation.Plan,
                _windowCancellation.Token)
            .ConfigureAwait(true);
    }

    private async Task<bool> StopVideoAudioForTransitionAsync()
    {
        await StopAudioCompletionWatcherAsync().ConfigureAwait(true);
        var audioState = _audioPlaybackController.Snapshot.State;
        if (audioState is not (WindowsAudioPlaybackState.Starting
            or WindowsAudioPlaybackState.Playing
            or WindowsAudioPlaybackState.Paused
            or WindowsAudioPlaybackState.Stopping))
        {
            return true;
        }

        var stopped = await _audioPlaybackController.StopAsync().ConfigureAwait(true);
        if (!stopped.IsSuccess)
        {
            _state.SetStatus(stopped.Error?.Message ?? "视频切换前停止声音失败");
            return false;
        }

        AudioDeviceStatusText.Text = "视频切换中 · 已停止上一项声音";
        return true;
    }

    private AudioEffectParams CreateCurrentAudioEffectParameters()
    {
        return _state.AudioParameterSnapshot.ToAudioEffectParams();
    }

    private void StartAudioCompletionWatcher(MediaPlaybackIdentity identity)
    {
        try
        {
            _audioCompletionCancellation?.Cancel();
        }
        catch (ObjectDisposedException)
        {
            // 已完成观察者可能刚刚摘除并释放旧取消源；新观察者仍可独立建立。
        }
        var cancellation = CancellationTokenSource.CreateLinkedTokenSource(_windowCancellation.Token);
        _audioCompletionCancellation = cancellation;
        _audioCompletionTask = ObserveAudioCompletionAsync(identity, cancellation);
    }

    private async Task StopAudioCompletionWatcherAsync()
    {
        var cancellation = _audioCompletionCancellation;
        var task = _audioCompletionTask;
        _audioCompletionCancellation = null;
        _audioCompletionTask = null;
        if (cancellation is null)
        {
            return;
        }

        try
        {
            cancellation.Cancel();
        }
        catch (ObjectDisposedException)
        {
            // 自然 EOF 观察者可能已在字段摘除后完成释放；取消仍保持幂等。
        }
        if (task is not null)
        {
            try
            {
                await task.WaitAsync(TimeSpan.FromSeconds(2)).ConfigureAwait(true);
            }
            catch (OperationCanceledException)
            {
                // 取消只用于唤醒观察者；会话本身由音频控制器负责 Join。
            }
            catch (TimeoutException)
            {
                // 不让 UI 停止命令等待观察者；控制器退出仍有独立预算。
            }
        }
        else
        {
            cancellation.Dispose();
        }

    }

    private void StartVideoStateWatcher(MediaPlaybackIdentity identity)
    {
        try
        {
            _videoStateCancellation?.Cancel();
        }
        catch (ObjectDisposedException)
        {
            // 旧观察者可能刚刚完成释放；新观察者仍可独立建立。
        }

        var cancellation = CancellationTokenSource.CreateLinkedTokenSource(_windowCancellation.Token);
        _videoStateCancellation = cancellation;
        _videoStateTask = ObserveVideoStateAsync(identity, cancellation);
    }

    private async Task StopVideoStateWatcherAsync()
    {
        var cancellation = _videoStateCancellation;
        var task = _videoStateTask;
        _videoStateCancellation = null;
        _videoStateTask = null;
        if (cancellation is null)
        {
            return;
        }

        try
        {
            cancellation.Cancel();
        }
        catch (ObjectDisposedException)
        {
            // 自然 EOF 观察者可能已在字段摘除后完成释放；取消仍保持幂等。
        }

        if (task is not null)
        {
            try
            {
                await task.WaitAsync(TimeSpan.FromSeconds(2)).ConfigureAwait(true);
            }
            catch (OperationCanceledException)
            {
                // 取消只用于唤醒观察者；mpv 会话由调用方负责有界回收。
            }
            catch (TimeoutException)
            {
                // 不让 UI 停止命令等待观察者；mpv 宿主有独立清理预算。
            }
        }
        else
        {
            cancellation.Dispose();
        }
    }

    private async Task ObserveVideoStateAsync(
        MediaPlaybackIdentity identity,
        CancellationTokenSource ownerCancellation)
    {
        var videoCyclePlanner = new VideoEffectCyclePlanner();
        try
        {
            await foreach (var result in _mpvController
                .WatchPlaybackStateAsync(identity, ownerCancellation.Token)
                .ConfigureAwait(true))
            {
                if (!result.IsSuccess || result.Snapshot is null)
                {
                    if (!ownerCancellation.IsCancellationRequested && !_isClosing)
                    {
                        await RunPlaybackCommandAsync(
                                () => StopVideoAfterMonitorFailureAsync(identity, result.Error?.Message),
                                ownerCancellation.Token)
                            .ConfigureAwait(true);
                    }

                    return;
                }

                var playbackTimeMs = result.Snapshot.PlaybackTimeMs;
                if (_audioPlaybackIdentity == identity)
                {
                    var audioSnapshot = _audioPlaybackController.Snapshot;
                    if (audioSnapshot.State is WindowsAudioPlaybackState.Playing
                        or WindowsAudioPlaybackState.Paused)
                    {
                        playbackTimeMs = audioSnapshot.AudibleClock?.PlaybackTimeMs ?? playbackTimeMs;
                    }
                }

                ProjectVideoPlaybackPosition(identity, playbackTimeMs);

                var currentPoolSnapshot = _mediaPool.Snapshot;
                var currentSource = currentPoolSnapshot.SourceMediaPool.IsEmpty
                    ? null
                    : currentPoolSnapshot.SourceMediaPool[currentPoolSnapshot.SourceMediaIndex];
                if (videoCyclePlanner.ShouldRegenerate(
                        identity,
                        playbackTimeMs,
                        currentSource?.DurationMs,
                        _state.VideoProcessing
                        && currentPoolSnapshot.PlaybackState is PlaybackState.Playing))
                {
                    await RunPlaybackCommandAsync(
                            () => ApplyAutomaticVideoEffectCycleAsync(identity),
                            ownerCancellation.Token)
                        .ConfigureAwait(true);
                }

                if (result.Snapshot.EofReached)
                {
                    await RunPlaybackCommandAsync(
                            () => CompleteVideoPlaybackItemAsync(identity),
                            ownerCancellation.Token)
                        .ConfigureAwait(true);
                    return;
                }
            }
        }
        catch (OperationCanceledException) when (ownerCancellation.IsCancellationRequested)
        {
            // 手动停止、切源或窗口关闭会取消观察者；这些不是播放错误。
        }
        finally
        {
            if (ReferenceEquals(_videoStateCancellation, ownerCancellation))
            {
                _videoStateCancellation = null;
                _videoStateTask = null;
            }

            ownerCancellation.Dispose();
        }
    }

    private async Task StopVideoAfterMonitorFailureAsync(
        MediaPlaybackIdentity identity,
        string? reason)
    {
        if (_isClosing
            || _mediaPool.Snapshot.PlaybackState != PlaybackState.Playing
            || _mediaPool.CurrentIdentity != identity)
        {
            return;
        }

        var stopped = await _mpvController.ShutdownAsync(_windowCancellation.Token)
            .ConfigureAwait(true);
        var poolStopped = _mediaPool.StopPlayback();
        ApplyMediaOperation(
            poolStopped,
            stopped.IsSuccess
                ? $"视频状态监视失败，已停止播放：{reason ?? "未知错误"}"
                : stopped.Error?.Message ?? "视频状态监视失败，停止视频会话失败");
    }

    private async Task CompleteVideoPlaybackItemAsync(MediaPlaybackIdentity identity)
    {
        if (_isClosing
            || _mediaPool.Snapshot.PlaybackState != PlaybackState.Playing
            || _mediaPool.CurrentIdentity != identity
            || _mpvController.Snapshot.ActiveIdentity != identity)
        {
            return;
        }

        if (!await StopVideoAudioForTransitionAsync().ConfigureAwait(true))
        {
            var stopped = _mediaPool.StopPlayback();
            ApplyMediaOperation(stopped, "视频声音切换失败，已停止播放");
            return;
        }

        var completed = _mediaPool.CompleteCurrent(
            identity.PlaybackGeneration,
            identity.SourceRevision,
            identity.SourceMediaIndex,
            identity.LoopIndex);
        if (!completed.IsSuccess)
        {
            ApplyMediaOperation(completed, completed.Error?.Message ?? "视频完成回调已拒绝");
            return;
        }

        var target = completed.Snapshot.SourceMediaPool[completed.Snapshot.SourceMediaIndex];
        if (target.MediaKind is MediaKind.Video)
        {
            var switched = await _mpvController.SwitchSourceAsync(
                    target,
                    _mediaPool.CurrentIdentity,
                    _windowCancellation.Token)
                .ConfigureAwait(true);
            if (!switched.IsSuccess)
            {
                await _mpvController.ShutdownAsync(_windowCancellation.Token).ConfigureAwait(true);
                var stopped = _mediaPool.StopPlayback();
                ApplyMediaOperation(stopped, switched.Error?.Message ?? "视频下一项启动失败，已停止播放");
                return;
            }

            if (!string.IsNullOrWhiteSpace(target.AudioCodecName))
            {
                var startedAudio = await StartAudioPlaybackAsync(
                        target,
                        _mediaPool.CurrentIdentity)
                    .ConfigureAwait(true);
                if (!startedAudio.IsSuccess)
                {
                    await _mpvController.ShutdownAsync(_windowCancellation.Token).ConfigureAwait(true);
                    var stopped = _mediaPool.StopPlayback();
                    ApplyMediaOperation(stopped, startedAudio.Error?.Message ?? "下一项视频声音启动失败");
                    return;
                }
            }

            StartVideoStateWatcher(_mediaPool.CurrentIdentity);
            ApplyMediaOperation(completed, $"已自动切换到第 {completed.Snapshot.SourceMediaIndex + 1} 项");
            MediaListBox.SelectedIndex = completed.Snapshot.SourceMediaIndex;
            return;
        }

        await _mpvController.ShutdownAsync(_windowCancellation.Token).ConfigureAwait(true);
        var startedAudioAfterVideo = await StartAudioPlaybackAsync(
                target,
                _mediaPool.CurrentIdentity)
            .ConfigureAwait(true);
        if (!startedAudioAfterVideo.IsSuccess)
        {
            var stopped = _mediaPool.StopPlayback();
            ApplyMediaOperation(stopped, startedAudioAfterVideo.Error?.Message ?? "视频结束后的声音项启动失败");
            return;
        }

        StartAudioCompletionWatcher(_mediaPool.CurrentIdentity);
        await PrepareNextAudioCandidateAsync(_mediaPool.CurrentIdentity).ConfigureAwait(true);
        ApplyMediaOperation(completed, $"已自动切换到第 {completed.Snapshot.SourceMediaIndex + 1} 项");
        MediaListBox.SelectedIndex = completed.Snapshot.SourceMediaIndex;
    }

    private async Task ObserveAudioCompletionAsync(
        MediaPlaybackIdentity identity,
        CancellationTokenSource ownerCancellation)
    {
        var ownerToken = ownerCancellation.Token;
        WindowsFfmpegPcmDecoderResult? candidateResult = null;
        var audioCyclePlanner = new AudioEffectCyclePlanner();
        try
        {
            var candidateCompletion = _audioPlaybackController.CandidateCompletion;
            if (candidateCompletion is null)
            {
                return;
            }

            while (true)
            {
                while (!candidateCompletion.IsCompleted)
                {
                    await Task.Delay(TimeSpan.FromMilliseconds(100), ownerToken)
                        .ConfigureAwait(true);

                    var poolSnapshot = _mediaPool.Snapshot;
                    var source = poolSnapshot.SourceMediaPool.IsEmpty
                        ? null
                        : poolSnapshot.SourceMediaPool[poolSnapshot.SourceMediaIndex];
                    var cycleAudioSnapshot = _audioPlaybackController.Snapshot;
                    var action = audioCyclePlanner.GetAction(
                        identity,
                        cycleAudioSnapshot.AudibleClock?.PlaybackTimeMs,
                        source?.DurationMs,
                        _state.AudioProcessing
                        && poolSnapshot.PlaybackState is PlaybackState.Playing
                        && _mediaPool.CurrentIdentity == identity,
                        _audioPlaybackController.HasPreparedNext,
                        out var targetPositionMs);

                    if (action is AudioEffectCycleAction.Prepare
                        && source is not null)
                    {
                        _state.RegenerateAudioParameterSnapshot();
                        var preparation = await TryPrepareAudioPlaybackAsync(
                                source,
                                identity,
                                targetPositionMs)
                            .ConfigureAwait(true);
                        if (preparation.Preparation is null)
                        {
                            audioCyclePlanner.Reset();
                            continue;
                        }

                        var prepared = await _audioPlaybackController.PrepareNextAsync(
                                preparation.Preparation.Plan,
                                ownerToken)
                            .ConfigureAwait(true);
                        if (!prepared.IsSuccess)
                        {
                            audioCyclePlanner.Reset();
                        }
                    }
                    else if (action is AudioEffectCycleAction.Commit)
                    {
                        var committed = await _audioPlaybackController.CommitPreparedNextAsync(
                                ownerToken,
                                targetPositionMs: targetPositionMs)
                            .ConfigureAwait(true);
                        if (committed.IsSuccess)
                        {
                            audioCyclePlanner.MarkCommitted(identity, targetPositionMs);
                            candidateCompletion = _audioPlaybackController.CandidateCompletion
                                ?? candidateCompletion;
                        }
                        else if (!_audioPlaybackController.HasPreparedNext)
                        {
                            audioCyclePlanner.Reset();
                        }
                    }
                }

                candidateResult = await candidateCompletion
                    .WaitAsync(ownerToken)
                    .ConfigureAwait(true);
                break;
            }
        }
        catch (OperationCanceledException) when (ownerCancellation.IsCancellationRequested)
        {
            return;
        }
        finally
        {
            if (ReferenceEquals(_audioCompletionCancellation, ownerCancellation))
            {
                _audioCompletionCancellation = null;
                _audioCompletionTask = null;
            }

            ownerCancellation.Dispose();
        }

        var audioSnapshot = _audioPlaybackController.Snapshot;
        if (_isClosing)
        {
            return;
        }

        if (_mediaPool.CurrentIdentity != identity)
        {
            return;
        }

        // 主会话自然结束或故障时，控制器会取消插话任务但不持有 UI 层级所有者；
        // 在身份仍匹配时释放插话层，避免下一项音频被永久 duck。
        _audioPriority.End(AudioPriorityLayer.InterludeFile);

        if (candidateResult is null)
        {
            return;
        }

        if (!candidateResult.IsSuccess || audioSnapshot.State is WindowsAudioPlaybackState.Failed)
        {
            if (_rtmpAudioSession.Snapshot.IsRunning)
            {
                var stoppedRtmp = await _rtmpAudioSession.StopAsync(CancellationToken.None).ConfigureAwait(true);
                if (stoppedRtmp.IsSuccess)
                {
                    _audioPlaybackController.SetRtmpConsumerAttached(false);
                }
                if (!stoppedRtmp.IsSuccess)
                {
                    RtmpStatusText.Text = stoppedRtmp.Error?.Message ?? "共享最终 PCM 的 RTMP 会话停止失败";
                }
            }

            await RunPlaybackCommandAsync(
                    () => StopAudioAfterFailureAsync(
                        identity,
                        candidateResult.Error?.Message ?? audioSnapshot.Error),
                    CancellationToken.None)
                .ConfigureAwait(true);
            return;
        }

        if (audioSnapshot.State is not (WindowsAudioPlaybackState.Completed
            or WindowsAudioPlaybackState.Playing))
        {
            return;
        }

        // 观察者的 CTS 已在 finally 中释放；此处只允许通过身份/播放状态门禁
        // 提交自然完成，不再把已释放的 CancellationTokenSource 传入串行闸门。
        await RunPlaybackCommandAsync(
                () => CompleteAudioPlaybackItemAsync(identity),
                CancellationToken.None)
                .ConfigureAwait(true);
    }

    private async Task StopAudioAfterFailureAsync(MediaPlaybackIdentity identity, string? reason)
    {
        if (_isClosing
            || _mediaPool.Snapshot.PlaybackState != PlaybackState.Playing
            || _mediaPool.CurrentIdentity != identity)
        {
            return;
        }

        var stoppedAudio = await _audioPlaybackController.StopAsync().ConfigureAwait(true);
        var stoppedPool = _mediaPool.StopPlayback();
        AudioDeviceStatusText.Text = stoppedAudio.IsSuccess
            ? "PortAudio 输出流已停止"
            : stoppedAudio.Error?.Message ?? "PortAudio 输出流停止失败";
        ApplyMediaOperation(
            stoppedPool,
            $"音频输出故障，已停止播放：{reason ?? "未知错误"}");
    }

    private async Task CompleteAudioPlaybackItemAsync(MediaPlaybackIdentity identity)
    {
        var audioState = _audioPlaybackController.Snapshot.State;
        var hasPreparedNext = _audioPlaybackController.HasPreparedNext;
        if (_isClosing
            || (audioState != WindowsAudioPlaybackState.Completed
                && !(audioState == WindowsAudioPlaybackState.Playing && hasPreparedNext))
            || _mediaPool.Snapshot.PlaybackState != PlaybackState.Playing
            || _mediaPool.CurrentIdentity != identity)
        {
            return;
        }

        var completed = _mediaPool.CompleteCurrent(
            identity.PlaybackGeneration,
            identity.SourceRevision,
            identity.SourceMediaIndex,
            identity.LoopIndex);
        if (!completed.IsSuccess)
        {
            ApplyMediaOperation(completed, completed.Error?.Message ?? "纯音频完成回调已拒绝");
            return;
        }

        var target = completed.Snapshot.SourceMediaPool[completed.Snapshot.SourceMediaIndex];
        if (target.MediaKind is not MediaKind.Audio)
        {
            if (_rtmpAudioSession.Snapshot.IsRunning)
            {
                var stoppedRtmp = await _rtmpAudioSession.StopAsync(CancellationToken.None).ConfigureAwait(true);
                if (stoppedRtmp.IsSuccess)
                {
                    _audioPlaybackController.SetRtmpConsumerAttached(false);
                }
                if (!stoppedRtmp.IsSuccess)
                {
                    RtmpStatusText.Text = stoppedRtmp.Error?.Message ?? "共享最终 PCM 的 RTMP 会话停止失败";
                }
            }

            // 音频→视频必须重新建立 mpv 视频表面；TogglePlaybackCoreAsync 会复用
            // 已保持 Playing 的播放池状态，并统一启动视频与其声音会话。
            MediaListBox.SelectedIndex = completed.Snapshot.SourceMediaIndex;
            await TogglePlaybackCoreAsync().ConfigureAwait(true);
            return;
        }

        if (hasPreparedNext)
        {
            var committed = await _audioPlaybackController.CommitPreparedNextAsync(
                    _windowCancellation.Token)
                .ConfigureAwait(true);
            if (committed.IsSuccess)
            {
                var nextIdentity = _mediaPool.CurrentIdentity;
                StartAudioCompletionWatcher(nextIdentity);
                ApplyMediaOperation(completed, $"已自动切换到第 {completed.Snapshot.SourceMediaIndex + 1} 项");
                MediaListBox.SelectedIndex = completed.Snapshot.SourceMediaIndex;
                await PrepareNextAudioCandidateAsync(nextIdentity).ConfigureAwait(true);
                return;
            }
        }

        if (_rtmpAudioSession.Snapshot.IsRunning)
        {
            var stoppedRtmp = await _rtmpAudioSession.StopAsync(CancellationToken.None).ConfigureAwait(true);
            if (stoppedRtmp.IsSuccess)
            {
                _audioPlaybackController.SetRtmpConsumerAttached(false);
            }
            if (!stoppedRtmp.IsSuccess)
            {
                RtmpStatusText.Text = stoppedRtmp.Error?.Message ?? "共享最终 PCM 的 RTMP 会话停止失败";
            }
        }

        var stoppedCurrent = await _audioPlaybackController.StopAsync().ConfigureAwait(true);
        if (!stoppedCurrent.IsSuccess)
        {
            ApplyMediaOperation(
                completed,
                stoppedCurrent.Error?.Message ?? "纯音频下一项切换前停止失败");
            return;
        }

        var started = await StartAudioPlaybackAsync(
            target,
            _mediaPool.CurrentIdentity).ConfigureAwait(true);
        if (!started.IsSuccess)
        {
            var stopped = _mediaPool.StopPlayback();
            ApplyMediaOperation(stopped, started.Error?.Message ?? "纯音频下一项启动失败");
            return;
        }

        StartAudioCompletionWatcher(_mediaPool.CurrentIdentity);
        await PrepareNextAudioCandidateAsync(_mediaPool.CurrentIdentity).ConfigureAwait(true);
        ApplyMediaOperation(completed, $"已自动切换到第 {completed.Snapshot.SourceMediaIndex + 1} 项");
        MediaListBox.SelectedIndex = completed.Snapshot.SourceMediaIndex;
    }

    private Task RunPlaybackCommandAsync(Func<Task> command) =>
        RunPlaybackCommandAsync(command, _windowCancellation.Token);

    private async Task RunPlaybackCommandAsync(
        Func<Task> command,
        CancellationToken cancellationToken)
    {
        try
        {
            await _playbackCommandSerial.WaitAsync(cancellationToken).ConfigureAwait(true);
        }
        catch (OperationCanceledException)
        {
            return;
        }

        try
        {
            await command().ConfigureAwait(true);
        }
        finally
        {
            _playbackCommandSerial.Release();
        }
    }

    private void UpdateMediaProjection()
    {
        var hasMedia = _state.HasMedia;
        var snapshot = _mediaPool.Snapshot;
        var currentSource = snapshot.SourceMediaPool.IsEmpty
            ? null
            : snapshot.SourceMediaPool[snapshot.SourceMediaIndex];
        var currentItem = snapshot.SourceMediaPool.IsEmpty
            || snapshot.SourceMediaIndex < 0
            || snapshot.SourceMediaIndex >= _state.MediaItems.Count
            ? null
            : _state.MediaItems[snapshot.SourceMediaIndex];
        var currentIdentity = _mediaPool.CurrentIdentity;
        var isVideo = currentSource?.MediaKind is MediaKind.Video;
        var hasPreviewThumbnail = isVideo && currentItem?.Thumbnail is not null;
        PreviewThumbnailImage.Source = currentItem?.Thumbnail;
        PreviewThumbnailImage.Visibility = hasPreviewThumbnail ? Visibility.Visible : Visibility.Collapsed;
        PreviewPlaceholderPanel.Visibility = hasPreviewThumbnail ? Visibility.Collapsed : Visibility.Visible;
        PreviewCurrentMediaText.Text = currentSource?.FileName ?? "无活动源";
        CurrentMediaBarText.Text = currentSource is null
            ? "当前媒体：无活动源"
            : $"当前媒体：{currentSource.FileName}";
        PreviewSurfaceStatusText.Text = currentSource is null
            ? "等待媒体 · 不启动 mpv / FFmpeg"
            : currentSource.MediaKind is MediaKind.Audio
                ? "纯音频媒体 · 预览窗口保持黑色"
                : snapshot.PlaybackState switch
                {
                    PlaybackState.Playing => "正在播放 · 单一视频表面由最终效果窗口承载",
                    PlaybackState.Paused => "已暂停 · 单一视频表面保留当前帧",
                    _ => "已就绪 · 点击播放启动单一视频表面",
                };
        PreviewTimeText.Text = currentSource is null
            ? "00:00:00 / —"
            : $"{PlaybackTimeFormatter.Format(
                isVideo && _projectedPositionIdentity == currentIdentity
                    ? _projectedPositionMs
                    : 0)} / {PlaybackTimeFormatter.Format(currentSource.DurationMs)}";
        PreviewPlaybackStateText.Text = snapshot.PlaybackState switch
        {
            PlaybackState.Playing => "正在播放",
            PlaybackState.Paused => "已暂停",
            _ => currentSource is null ? "就绪" : "已就绪",
        };
        PreviewPlaybackStateText.Foreground = snapshot.PlaybackState is PlaybackState.Playing
            ? (System.Windows.Media.Brush)FindResource("AccentBrush")
            : (System.Windows.Media.Brush)FindResource("MutedTextBrush");
        PreviewVideoInfoText.Text = isVideo
            ? $"视频：{currentSource?.Width ?? 0}×{currentSource?.Height ?? 0} {currentSource?.FrameRateFps ?? 0:0.#}fps"
            : "视频：—";
        PreviewAudioInfoText.Text = currentSource?.AudioSampleRateHz is uint sampleRate
            ? $"音频：{sampleRate / 1000.0:0.#}kHz {currentSource.AudioChannelCount ?? 0}ch"
            : "音频：—";
        if (!isVideo || _projectedPositionIdentity != currentIdentity)
        {
            _projectedPositionIdentity = null;
            _projectedPositionMs = null;
            _state.PlaybackProgress = 0;
            PlaybackPositionText.Text = "00:00";
        }

        PlaybackDurationText.Text = isVideo
            ? PlaybackTimeFormatter.Format(currentSource?.DurationMs)
            : "—";
        EmptyMediaPanel.Visibility = hasMedia ? Visibility.Collapsed : Visibility.Visible;
        MediaListBox.Visibility = hasMedia ? Visibility.Visible : Visibility.Collapsed;
        PlaybackSlider.IsEnabled = hasMedia
            && isVideo
            && snapshot.PlaybackState is PlaybackState.Playing or PlaybackState.Paused;
        MediaCountText.Text = hasMedia
            ? $"共 {_state.MediaItems.Count} 个媒体（拖拽排序）"
            : "共 0 个媒体";
        var selectedIndex = MediaListBox.SelectedIndex;
        MoveUpButton.IsEnabled = hasMedia && selectedIndex > 0;
        MoveDownButton.IsEnabled = hasMedia && selectedIndex >= 0 && selectedIndex < _state.MediaItems.Count - 1;
        RemoveMediaButton.IsEnabled = hasMedia && selectedIndex >= 0;
        ClearMediaButton.IsEnabled = hasMedia;
        UpdateInterludeProjection();
    }

    private FinalEffectSnapshot CreateFinalEffectSnapshot()
    {
        var snapshot = _mediaPool.Snapshot;
        if (snapshot.SourceMediaPool.IsEmpty)
        {
            return FinalEffectSnapshot.Empty;
        }

        var source = snapshot.SourceMediaPool[snapshot.SourceMediaIndex];
        var surfaceKind = source.MediaKind is MediaKind.Video
            ? FinalEffectSurfaceKind.VideoHwndReserved
            : FinalEffectSurfaceKind.AudioBlack;
        return FinalEffectSnapshot.Create(surfaceKind);
    }

    private void ProjectVideoPlaybackPosition(
        MediaPlaybackIdentity identity,
        ulong? positionMs)
    {
        if (_isClosing || _mediaPool.CurrentIdentity != identity)
        {
            return;
        }

        var snapshot = _mediaPool.Snapshot;
        if (snapshot.SourceMediaPool.IsEmpty
            || snapshot.SourceMediaPool[snapshot.SourceMediaIndex].MediaKind is not MediaKind.Video)
        {
            return;
        }

        var durationMs = snapshot.SourceMediaPool[snapshot.SourceMediaIndex].DurationMs;
        var resolvedPositionMs = positionMs
            ?? (_projectedPositionIdentity == identity ? _projectedPositionMs : null)
            ?? 0;
        if (durationMs is ulong duration)
        {
            resolvedPositionMs = Math.Min(resolvedPositionMs, duration);
            _state.PlaybackProgress = duration == 0
                ? 0
                : Math.Clamp((double)resolvedPositionMs / duration, 0, 1);
        }
        else
        {
            _state.PlaybackProgress = 0;
        }

        _projectedPositionIdentity = identity;
        _projectedPositionMs = resolvedPositionMs;
        PlaybackPositionText.Text = PlaybackTimeFormatter.Format(resolvedPositionMs);
        PlaybackDurationText.Text = PlaybackTimeFormatter.Format(durationMs);
        PreviewTimeText.Text = $"{PlaybackTimeFormatter.Format(resolvedPositionMs)} / {PlaybackTimeFormatter.Format(durationMs)}";
        PreviewPlaybackStateText.Text = snapshot.PlaybackState is PlaybackState.Playing ? "正在播放" : "已暂停";
        PreviewPlaybackStateText.Foreground = snapshot.PlaybackState is PlaybackState.Playing
            ? (System.Windows.Media.Brush)FindResource("AccentBrush")
            : (System.Windows.Media.Brush)FindResource("MutedTextBrush");
        _finalEffectController.Update(CreateFinalEffectSnapshot());
    }

    private async void PlaybackSlider_PreviewMouseLeftButtonUp(
        object sender,
        MouseButtonEventArgs e) =>
        await SeekVideoFromSliderAsync().ConfigureAwait(true);

    private async void PlaybackSlider_KeyUp(object sender, KeyEventArgs e)
    {
        if (e.Key is not (Key.Left or Key.Right or Key.Home or Key.End))
        {
            return;
        }

        await SeekVideoFromSliderAsync().ConfigureAwait(true);
        e.Handled = true;
    }

    private Task SeekVideoFromSliderAsync() =>
        RunPlaybackCommandAsync(SeekVideoFromSliderCoreAsync);

    private async Task SeekVideoFromSliderCoreAsync()
    {
        var snapshot = _mediaPool.Snapshot;
        if (!_login.CanEnterWorkbench || snapshot.SourceMediaPool.IsEmpty)
        {
            return;
        }

        var source = snapshot.SourceMediaPool[snapshot.SourceMediaIndex];
        if (source.MediaKind is not MediaKind.Video
            || source.DurationMs is not ulong durationMs
            || durationMs == 0)
        {
            return;
        }

        if (!double.IsFinite(PlaybackSlider.Value))
        {
            return;
        }

        var fraction = Math.Clamp(PlaybackSlider.Value, 0, 1);
        var positionMs = (ulong)Math.Clamp(
            decimal.Round((decimal)fraction * durationMs, MidpointRounding.AwayFromZero),
            0,
            (decimal)durationMs);
        var identity = _mediaPool.CurrentIdentity;
        var result = await _mpvController
            .SeekAsync(identity, positionMs, _windowCancellation.Token)
            .ConfigureAwait(true);
        if (!result.IsSuccess)
        {
            _state.SetStatus(result.Error?.Message ?? "视频跳转失败");
            return;
        }

        ProjectVideoPlaybackPosition(identity, positionMs);
        _state.SetStatus($"已跳转到 {PlaybackTimeFormatter.Format(positionMs)}");
    }
}
