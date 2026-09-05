using System.Windows;
using System.Runtime.InteropServices;
using GpAutoLive.Contracts;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private void CopyRtmpTargetUrlButton_Click(object sender, RoutedEventArgs e)
    {
        if (string.IsNullOrWhiteSpace(RtmpTargetUrlTextBox.Text))
        {
            _state.SetStatus("RTMP 推流地址为空，未复制");
            return;
        }

        try
        {
            Clipboard.SetText(RtmpTargetUrlTextBox.Text);
            _state.SetStatus("RTMP 推流地址已复制到剪贴板");
        }
        catch (ExternalException)
        {
            _state.SetStatus("系统剪贴板当前不可用，未复制 RTMP 地址");
        }
        catch (System.Threading.ThreadStateException)
        {
            _state.SetStatus("系统剪贴板当前不可用，未复制 RTMP 地址");
        }
    }

    private void ToggleRtmpStreamKeyButton_Click(object sender, RoutedEventArgs e)
    {
        if (RtmpStreamKeyPasswordBox.Visibility == Visibility.Visible)
        {
            RtmpStreamKeyTextBox.Text = RtmpStreamKeyPasswordBox.Password;
            RtmpStreamKeyPasswordBox.Visibility = Visibility.Collapsed;
            RtmpStreamKeyTextBox.Visibility = Visibility.Visible;
            RtmpStreamKeyVisibilityButton.Content = "隐藏";
            return;
        }

        RtmpStreamKeyPasswordBox.Password = RtmpStreamKeyTextBox.Text;
        RtmpStreamKeyTextBox.Visibility = Visibility.Collapsed;
        RtmpStreamKeyPasswordBox.Visibility = Visibility.Visible;
        RtmpStreamKeyVisibilityButton.Content = "显示";
    }

    private async void ReconnectRtmpButton_Click(object sender, RoutedEventArgs e) =>
        await RunPlaybackCommandAsync(ReconnectRtmpCoreAsync).ConfigureAwait(true);

    private void OutputButton_Click(object sender, RoutedEventArgs e)
    {
        var config = RtmpOutputConfig.Default with
        {
            TargetUrl = RtmpTargetUrlTextBox.Text,
            VideoEnabled = RtmpVideoCheckBox.IsChecked == true,
            AudioEnabled = RtmpAudioCheckBox.IsChecked == true
        };

        if (!RtmpOutputRules.TryValidate(config, out var error))
        {
            RtmpStatusText.Text = error?.Message ?? "RTMP 配置无效";
            _state.SetStatus("RTMP 配置校验失败；未启动推流");
            return;
        }

        RtmpStatusText.Text = $"配置有效 · {RtmpOutputRules.RedactTargetUrl(config.TargetUrl)} · 可开始画面推流";
        _state.SetStatus("RTMP 配置校验通过；可开始画面直推");
        UpdateRtmpProjection();
    }

    private async void StartRtmpButton_Click(object sender, RoutedEventArgs e) =>
        await RunPlaybackCommandAsync(() => StartRtmpCoreAsync()).ConfigureAwait(true);

    private async void StopRtmpButton_Click(object sender, RoutedEventArgs e) =>
        await RunPlaybackCommandAsync(StopRtmpCoreAsync).ConfigureAwait(true);

    private async Task StartRtmpCoreAsync(RtmpOutputConfig? requestedConfig = null)
    {
        if (!_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        var currentRtmpState = _rtmpOutputManager.Snapshot.State;
        if (_rtmpAudioSession.Snapshot.IsRunning
            || currentRtmpState is RtmpOutputState.Starting or RtmpOutputState.Publishing)
        {
            _state.SetStatus("RTMP 推流已经在运行");
            return;
        }

        var staleAudioSnapshot = _rtmpAudioSession.Snapshot;
        if (!staleAudioSnapshot.IsRunning && staleAudioSnapshot.ErrorCode is not null)
        {
            var cleanup = await _rtmpAudioSession.StopAsync(_windowCancellation.Token).ConfigureAwait(true);
            _audioPlaybackController.SetRtmpConsumerAttached(false);
            if (!cleanup.IsSuccess)
            {
                RtmpStatusText.Text = cleanup.Error?.Message ?? "上一次 RTMP 会话清理失败";
                _state.SetStatus("RTMP 旧会话未清理，未启动新推流");
                UpdateRtmpProjection();
                return;
            }
        }

        var config = requestedConfig ?? CreateRtmpOutputConfig();
        if (!RtmpOutputRules.TryValidate(config, out var validationError))
        {
            RtmpStatusText.Text = validationError?.Message ?? "RTMP 配置无效";
            _state.SetStatus("RTMP 配置校验失败；未启动推流");
            return;
        }

        var snapshot = _mediaPool.Snapshot;
        if (snapshot.SourceMediaPool.IsEmpty)
        {
            _state.SetStatus("播放池为空，无法开始 RTMP 推流");
            return;
        }

        var source = snapshot.SourceMediaPool[snapshot.SourceMediaIndex];
        if (config.VideoEnabled && source.MediaKind is not MediaKind.Video)
        {
            _state.SetStatus("当前媒体没有画面，无法开始 RTMP 画面推流");
            return;
        }

        if (config.AudioEnabled
            && (source.AudioChannelCount is null or 0
                || string.IsNullOrWhiteSpace(source.AudioCodecName)))
        {
            RtmpStatusText.Text = "当前媒体没有可用声音轨道";
            _state.SetStatus("当前媒体没有可用声音，无法开始 RTMP 声音推流");
            return;
        }

        var runtime = await EnsureVerifiedMediaRuntimeAsync().ConfigureAwait(true);
        if (runtime is null)
        {
            return;
        }

        if (!runtime.TryGetResource("ffmpeg.exe", out var ffmpeg) || ffmpeg is null)
        {
            RtmpStatusText.Text = "FFmpeg 运行资源未安装；不会回退 PATH";
            _state.SetStatus("RTMP 运行资源不可用");
            return;
        }

        MpvVideoEffectSnapshot? rtmpVideoEffects = null;
        if (config.VideoEnabled && _state.VideoProcessing)
        {
            if (!TryCreateRuntimeVideoEffectSnapshot(
                    source,
                    enabled: true,
                    out rtmpVideoEffects,
                    out var videoEffectError,
                    modeOverride: MpvLaunchMode.Cpu4)
                || rtmpVideoEffects is null)
            {
                RtmpStatusText.Text = videoEffectError?.Message ?? "RTMP 视频效果参数无效";
                _state.SetStatus("RTMP 视频效果无效；未启动推流");
                UpdateRtmpProjection();
                return;
            }
        }

        string? preferredEncoder = null;
        if (config.VideoEnabled)
        {
            var encoderProbe = await RtmpEncoderProbe.ProbeAsync(
                    ffmpeg.AbsolutePath,
                    preferredEncoder: null,
                    new WindowsExternalProcessRunner(),
                    _windowCancellation.Token)
                .ConfigureAwait(true);
            if (!encoderProbe.IsSuccess || string.IsNullOrWhiteSpace(encoderProbe.Snapshot.SelectedEncoder))
            {
                RtmpStatusText.Text = encoderProbe.Error?.Message ?? "本机没有可用的 H.264 编码器";
                _state.SetStatus("RTMP 编码器探测失败；未启动推流");
                UpdateRtmpProjection();
                return;
            }

            preferredEncoder = encoderProbe.Snapshot.SelectedEncoder;
        }

        var identity = _mediaPool.CurrentIdentity;
        var sourceIdentity = new RtmpSourceIdentity(
            identity.PlaybackGeneration,
            identity.SourceRevision,
            identity.SourceMediaIndex,
            identity.LoopIndex,
            0,
            source.DurationMs);
        bool startSucceeded;
        string? startError;
        string? redactedTargetUrl;
        if (config.AudioEnabled)
        {
            var audioResult = await _rtmpAudioSession.StartAsync(
                    config,
                    source,
                    ffmpeg.AbsolutePath,
                    sourceIdentity,
                    preferredEncoder,
                    _windowCancellation.Token,
                    CreateBaseAudioMixPolicy,
                     new AudioPcmMixEnvelopeOptions(
                         FfmpegPcmDecodePlanBuilder.DefaultSampleRateHz,
                         _interludeConfig.DuckingAttackMs,
                         _interludeConfig.DuckingReleaseMs),
                      sharedFinalPcmBus: _audioPlaybackController.ActiveFinalPcmBus,
                      sharedRtmpOutputSource: _audioPlaybackController.ActiveFinalPcmRtmpOutputSource,
                      sharedRtmpOverlayOutputSource: _audioPlaybackController.ActiveFinalPcmRtmpOverlayOutputSource,
                      sharedFinalPcmBusProvider: () => _audioPlaybackController.ActiveFinalPcmBus,
                      audioEffects: _state.AudioProcessing
                          ? CreateCurrentAudioEffectParameters()
                          : null,
                      videoEffects: rtmpVideoEffects)
                .ConfigureAwait(true);
            startSucceeded = audioResult.IsSuccess;
            startError = audioResult.Error?.Message;
            redactedTargetUrl = RtmpOutputRules.RedactTargetUrl(config.TargetUrl);
        }
        else
        {
            var videoResult = await _rtmpOutputManager.StartAsync(
                    config,
                    source,
                    ffmpeg.AbsolutePath,
                    preferredEncoder: preferredEncoder,
                    sourceIdentity: sourceIdentity,
                    videoEffects: rtmpVideoEffects,
                    cancellationToken: _windowCancellation.Token)
                .ConfigureAwait(true);
            startSucceeded = videoResult.IsSuccess;
            startError = videoResult.Error?.Message;
            redactedTargetUrl = videoResult.Snapshot.TargetUrl;
        }

        if (!startSucceeded)
        {
            RtmpStatusText.Text = startError ?? "RTMP 推流启动失败";
            _state.SetStatus("RTMP 推流未启动");
            UpdateRtmpProjection();
            return;
        }

        if (config.AudioEnabled)
        {
            _audioPlaybackController.SetRtmpConsumerAttached(true);
        }

        RtmpStatusText.Text = config.AudioEnabled
            ? $"{(config.VideoEnabled ? "音画" : "声音")}推流中 · {redactedTargetUrl}"
            : $"画面推流中 · {redactedTargetUrl ?? "rtmp://…"}";
        _state.SetStatus(config.AudioEnabled ? "RTMP 最终 PCM 声音推流已启动" : "RTMP 画面直推已启动");
        UpdateRtmpProjection();
    }

    private async Task ReconnectRtmpCoreAsync()
    {
        if (!_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        var managerSnapshot = _rtmpOutputManager.Snapshot;
        var audioSnapshot = _rtmpAudioSession.Snapshot;
        var failed = managerSnapshot.State == RtmpOutputState.Failed
            || audioSnapshot.ErrorCode is not null;
        if (!failed)
        {
            _state.SetStatus("当前没有可重连的 RTMP 失败会话");
            UpdateRtmpProjection();
            return;
        }

        var config = CreateRtmpOutputConfig();
        if (!RtmpOutputRules.TryValidate(config, out var validationError))
        {
            RtmpStatusText.Text = validationError?.Message ?? "RTMP 配置无效";
            _state.SetStatus("RTMP 重连配置校验失败；未重启推流");
            UpdateRtmpProjection();
            return;
        }

        RtmpStatusText.Text = "RTMP 已断开，正在执行有限重连";
        _state.SetStatus("RTMP 正在有限重连；最多尝试 3 次");
        UpdateRtmpProjection();
        var reconnect = await _rtmpReconnectCoordinator
            .ReconnectAsync(
                async (_, cancellationToken) =>
                {
                    await StopRtmpCoreAsync().ConfigureAwait(true);
                    if (_rtmpAudioSession.Snapshot.IsRunning
                        || _rtmpOutputManager.Snapshot.State is RtmpOutputState.Starting or RtmpOutputState.Publishing)
                    {
                        return WindowsRtmpReconnectAttempt.Failed(
                            WindowsRtmpFailureCode.StopTimedOut,
                            retryable: true);
                    }

                    await StartRtmpCoreAsync(config).ConfigureAwait(true);
                    var audioRunning = _rtmpAudioSession.Snapshot.IsRunning;
                    var videoRunning = _rtmpOutputManager.Snapshot.State is RtmpOutputState.Starting or RtmpOutputState.Publishing;
                    return audioRunning || videoRunning
                        ? WindowsRtmpReconnectAttempt.Succeeded()
                        : WindowsRtmpReconnectAttempt.Failed(
                            WindowsRtmpFailureCode.StartFailed,
                            retryable: true);
                },
                _windowCancellation.Token)
            .ConfigureAwait(true);

        if (reconnect.IsSuccess)
        {
            RtmpStatusText.Text = "RTMP 有限重连成功";
            _state.SetStatus("RTMP 推流已恢复");
        }
        else
        {
            RtmpStatusText.Text = reconnect.Error?.Message ?? "RTMP 重连失败";
            _state.SetStatus("RTMP 推流未恢复，可重新开始或检查地址");
        }
        UpdateRtmpProjection();
    }

    private async Task StopRtmpCoreAsync()
    {
        await StopInterludeForPriorityAsync().ConfigureAwait(true);
        if (_rtmpAudioSession.Snapshot.IsRunning)
        {
            var audioResult = await _rtmpAudioSession.StopAsync(_windowCancellation.Token).ConfigureAwait(true);
            if (audioResult.IsSuccess)
            {
                _audioPlaybackController.SetRtmpConsumerAttached(false);
            }
            RtmpStatusText.Text = audioResult.IsSuccess
                ? "RTMP 推流已停止"
                : audioResult.Error?.Message ?? "RTMP 推流停止失败";
        }
        else
        {
            var result = await _rtmpOutputManager.StopAsync(_windowCancellation.Token).ConfigureAwait(true);
            RtmpStatusText.Text = result.IsSuccess
                ? "RTMP 推流已停止"
                : result.Error?.Message ?? "RTMP 推流停止失败";
        }
        _state.SetStatus(RtmpStatusText.Text);
        UpdateRtmpProjection();
    }

    private RtmpOutputConfig CreateRtmpOutputConfig() => RtmpOutputConfig.Default with
    {
        TargetUrl = RtmpTargetUrlTextBox.Text,
        VideoEnabled = RtmpVideoCheckBox.IsChecked == true,
        AudioEnabled = RtmpAudioCheckBox.IsChecked == true,
    };

    private void UpdateRtmpProjection()
    {
        if (!IsInitialized)
        {
            return;
        }

        var managerSnapshot = _rtmpOutputManager.Snapshot;
        var audioSnapshot = _rtmpAudioSession.Snapshot;
        var managerPublishing = managerSnapshot.State is RtmpOutputState.Starting or RtmpOutputState.Publishing;
        var publishing = managerPublishing
            || (audioSnapshot.IsRunning && managerSnapshot.State != RtmpOutputState.Failed);
        var failed = managerSnapshot.State == RtmpOutputState.Failed
            || audioSnapshot.ErrorCode is not null;
        SetStatusPill(RtmpStatePillText, publishing ? "推流中" : "待机", publishing);
        StartRtmpButton.IsEnabled = _login.CanEnterWorkbench && !publishing;
        StopRtmpButton.IsEnabled = _login.CanEnterWorkbench && publishing;
        ReconnectRtmpButton.IsEnabled = _login.CanEnterWorkbench && failed && !publishing;
        ReconnectRtmpButton.ToolTip = ReconnectRtmpButton.IsEnabled
            ? "重新启动当前 RTMP 配置，最多尝试 3 次"
            : "仅在 RTMP 会话明确失败后可用";
        if (failed && !publishing && string.IsNullOrWhiteSpace(RtmpStatusText.Text))
        {
            RtmpStatusText.Text = managerSnapshot.Error
                ?? audioSnapshot.Error
                ?? "RTMP 推流已断开，可重新连接";
        }
    }

    private void RtmpOutputManager_SnapshotChanged(WindowsRtmpSnapshot snapshot)
    {
        if (_isClosing)
        {
            return;
        }

        _ = Dispatcher.InvokeAsync(
            () =>
            {
                if (_isClosing)
                {
                    return;
                }

                if (snapshot.State == RtmpOutputState.Failed
                    || _rtmpAudioSession.Snapshot.ErrorCode is not null)
                {
                    RtmpStatusText.Text = snapshot.Error ?? "RTMP 推流已断开，可重新连接";
                    _state.SetStatus("RTMP 推流已断开，可重新连接");
                }
                UpdateRtmpProjection();
            },
            System.Windows.Threading.DispatcherPriority.Background);
    }

    private async Task<bool> StopRtmpForMediaMutationAsync()
    {
        var state = _rtmpOutputManager.Snapshot.State;
        if (!_rtmpAudioSession.Snapshot.IsRunning
            && state is not (RtmpOutputState.Starting or RtmpOutputState.Publishing))
        {
            _audioPlaybackController.SetRtmpConsumerAttached(false);
            return true;
        }

        await StopInterludeForPriorityAsync().ConfigureAwait(true);

        bool stopSucceeded;
        string? stopError;
        if (_rtmpAudioSession.Snapshot.IsRunning)
        {
            var audioStopped = await _rtmpAudioSession.StopAsync(_windowCancellation.Token).ConfigureAwait(true);
            if (audioStopped.IsSuccess)
            {
                _audioPlaybackController.SetRtmpConsumerAttached(false);
            }
            stopSucceeded = audioStopped.IsSuccess;
            stopError = audioStopped.Error?.Message;
        }
        else
        {
            var videoStopped = await _rtmpOutputManager.StopAsync(_windowCancellation.Token).ConfigureAwait(true);
            stopSucceeded = videoStopped.IsSuccess;
            stopError = videoStopped.Error?.Message;
        }

        if (!stopSucceeded)
        {
            RtmpStatusText.Text = stopError ?? "RTMP 源切换前停止失败";
            _state.SetStatus("媒体源未变化，RTMP 推流停止失败");
            UpdateRtmpProjection();
            return false;
        }

        RtmpStatusText.Text = "媒体源即将变化，RTMP 推流已停止";
        UpdateRtmpProjection();
        return true;
    }
}
