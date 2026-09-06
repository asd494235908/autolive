using System.Diagnostics;
using System.Windows;
using System.Windows.Threading;
using System.Runtime.InteropServices;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private static readonly TimeSpan RtmpPublishingWaitTimeout = TimeSpan.FromSeconds(10);
    private static readonly TimeSpan RtmpPublishingPollInterval = TimeSpan.FromMilliseconds(100);

    internal static Task<T> RunOnDispatcherAsync<T>(
        Dispatcher dispatcher,
        Func<Task<T>> operation,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(dispatcher);
        ArgumentNullException.ThrowIfNull(operation);
        if (dispatcher.CheckAccess())
        {
            return operation();
        }

        return dispatcher.InvokeAsync(
                operation,
                DispatcherPriority.Send,
                cancellationToken)
            .Task
            .Unwrap();
    }

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

    private void CancelRtmpReconnect()
    {
        try
        {
            _rtmpReconnectCancellation?.Cancel();
        }
        catch (ObjectDisposedException)
        {
            // 重连任务已经完成并释放令牌；停止操作本身仍需继续。
        }
    }

    private async void StartRtmpButton_Click(object sender, RoutedEventArgs e) =>
        await RunPlaybackCommandAsync(() => StartRtmpCoreAsync()).ConfigureAwait(true);

    private async void StopRtmpButton_Click(object sender, RoutedEventArgs e)
    {
        CancelRtmpReconnect();
        await RunPlaybackCommandAsync(() => StopRtmpCoreAsync()).ConfigureAwait(true);
    }

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

        _lastRtmpConfig = config;

        RtmpStatusText.Text = config.AudioEnabled
            ? $"{(config.VideoEnabled ? "音画" : "声音")}连接启动中 · {redactedTargetUrl}"
            : $"画面连接启动中 · {redactedTargetUrl ?? "rtmp://…"}";
        _state.SetStatus(config.AudioEnabled
            ? "RTMP 最终 PCM 声音已启动，等待远端进度"
            : "RTMP 画面已启动，等待远端进度");
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
        if (_rtmpReconnectCoordinator.Snapshot.State == RtmpOutputState.Reconnecting)
        {
            _state.SetStatus("RTMP 已在重连中");
            return;
        }

        var failed = managerSnapshot.State == RtmpOutputState.Failed
            || audioSnapshot.ErrorCode is not null;
        if (!failed)
        {
            _state.SetStatus("当前没有可重连的 RTMP 失败会话");
            UpdateRtmpProjection();
            return;
        }

        var config = SelectRtmpReconnectConfig(_lastRtmpConfig, CreateRtmpOutputConfig());
        if (!RtmpOutputRules.TryValidate(config, out var validationError))
        {
            RtmpStatusText.Text = validationError?.Message ?? "RTMP 配置无效";
            _state.SetStatus("RTMP 重连配置校验失败；未重启推流");
            UpdateRtmpProjection();
            return;
        }

        RtmpStatusText.Text = "RTMP 已断开，正在执行有限重连";
        _state.SetStatus("RTMP 正在有限重连；最多尝试 6 次");
        UpdateRtmpProjection();
        var expectedIdentity = _mediaPool.CurrentIdentity;
        using var reconnectCancellation = CancellationTokenSource.CreateLinkedTokenSource(
            _windowCancellation.Token);
        _rtmpReconnectCancellation = reconnectCancellation;
        try
        {
            var reconnect = await _rtmpReconnectCoordinator
                .ReconnectAsync(
                    (_, cancellationToken) => RunRtmpReconnectAttemptAsync(
                        config,
                        expectedIdentity,
                        cancellationToken),
                    reconnectCancellation.Token)
                .ConfigureAwait(true);

            if (reconnect.IsSuccess)
            {
                RtmpStatusText.Text = "RTMP 有限重连成功";
                _state.SetStatus("RTMP 推流已恢复");
            }
            else if (reconnect.Error?.Code == WindowsRtmpFailureCode.Cancelled)
            {
                RtmpStatusText.Text = "RTMP 重连已取消";
                _state.SetStatus("RTMP 重连已取消");
            }
            else
            {
                RtmpStatusText.Text = reconnect.Error?.Message ?? "RTMP 重连失败";
                _state.SetStatus("RTMP 推流未恢复，可重新开始或检查地址");
            }
            UpdateRtmpProjection();
        }
        finally
        {
            if (ReferenceEquals(_rtmpReconnectCancellation, reconnectCancellation))
            {
                _rtmpReconnectCancellation = null;
            }
        }
    }

    private Task<WindowsRtmpReconnectAttempt> RunRtmpReconnectAttemptAsync(
        RtmpOutputConfig config,
        MediaPlaybackIdentity expectedIdentity,
        CancellationToken cancellationToken)
        => RunOnDispatcherAsync(
            Dispatcher,
            () => RunRtmpReconnectAttemptOnDispatcherAsync(
                config,
                expectedIdentity,
                cancellationToken),
            cancellationToken);

    private async Task<WindowsRtmpReconnectAttempt> RunRtmpReconnectAttemptOnDispatcherAsync(
        RtmpOutputConfig config,
        MediaPlaybackIdentity expectedIdentity,
        CancellationToken cancellationToken)
    {
        if (_mediaPool.CurrentIdentity != expectedIdentity)
        {
            return WindowsRtmpReconnectAttempt.Failed(
                WindowsRtmpFailureCode.StartFailed,
                retryable: false);
        }

        await StopRtmpCoreAsync(cancelReconnect: false, clearLastConfig: false).ConfigureAwait(true);
        if (_rtmpAudioSession.Snapshot.IsRunning
            || _rtmpOutputManager.Snapshot.State is RtmpOutputState.Starting or RtmpOutputState.Publishing)
        {
            return WindowsRtmpReconnectAttempt.Failed(
                WindowsRtmpFailureCode.StopTimedOut,
                retryable: true);
        }

        if (_mediaPool.CurrentIdentity != expectedIdentity)
        {
            return WindowsRtmpReconnectAttempt.Failed(
                WindowsRtmpFailureCode.StartFailed,
                retryable: false);
        }

        await StartRtmpCoreAsync(config).ConfigureAwait(true);
        var ready = await WaitForRtmpTracksReadyAsync(config, expectedIdentity, cancellationToken)
            .ConfigureAwait(true);
        if (_mediaPool.CurrentIdentity != expectedIdentity)
        {
            await StopRtmpCoreAsync(cancelReconnect: false, clearLastConfig: false).ConfigureAwait(true);
            return WindowsRtmpReconnectAttempt.Failed(
                WindowsRtmpFailureCode.StartFailed,
                retryable: false);
        }

        return ready
            ? WindowsRtmpReconnectAttempt.Succeeded()
            : WindowsRtmpReconnectAttempt.Failed(
                _rtmpOutputManager.Snapshot.State == RtmpOutputState.Failed
                    ? WindowsRtmpFailureCode.ProcessExited
                    : WindowsRtmpFailureCode.StartFailed,
                retryable: true);
    }

    private async Task StopRtmpCoreAsync(
        bool cancelReconnect = true,
        bool clearLastConfig = true)
    {
        if (cancelReconnect)
        {
            CancelRtmpReconnect();
        }
        await StopInterludeForPriorityAsync().ConfigureAwait(true);
        bool stopSucceeded;
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
            stopSucceeded = audioResult.IsSuccess;
        }
        else
        {
            var result = await _rtmpOutputManager.StopAsync(_windowCancellation.Token).ConfigureAwait(true);
            RtmpStatusText.Text = result.IsSuccess
                ? "RTMP 推流已停止"
                : result.Error?.Message ?? "RTMP 推流停止失败";
            stopSucceeded = result.IsSuccess;
        }

        if (stopSucceeded && clearLastConfig)
        {
            _lastRtmpConfig = null;
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

    internal static bool AreSelectedRtmpTracksReady(
        RtmpOutputConfig config,
        RtmpOutputState outputState,
        WindowsRtmpAudioSessionSnapshot audioSnapshot)
    {
        ArgumentNullException.ThrowIfNull(config);
        ArgumentNullException.ThrowIfNull(audioSnapshot);
        return (config.VideoEnabled || config.AudioEnabled)
            && outputState == RtmpOutputState.Publishing
            && (!config.AudioEnabled
                || audioSnapshot.IsRunning && audioSnapshot.ErrorCode is null);
    }

    internal static bool CanStopRtmpSession(
        RtmpOutputState managerState,
        RtmpOutputState reconnectState,
        WindowsRtmpAudioSessionSnapshot audioSnapshot,
        bool managerNeedsCleanup = false)
    {
        ArgumentNullException.ThrowIfNull(audioSnapshot);
        var reconnecting = reconnectState == RtmpOutputState.Reconnecting;
        var hasActiveSession = reconnecting
            || managerState is RtmpOutputState.Starting or RtmpOutputState.Publishing
            || audioSnapshot.IsRunning
            || managerNeedsCleanup;
        var failed = !reconnecting && !managerNeedsCleanup
            && (managerState == RtmpOutputState.Failed || audioSnapshot.ErrorCode is not null);
        return hasActiveSession && !failed && managerState != RtmpOutputState.Stopping;
    }

    internal static RtmpOutputConfig SelectRtmpReconnectConfig(
        RtmpOutputConfig? lastStartedConfig,
        RtmpOutputConfig currentEditorConfig)
    {
        ArgumentNullException.ThrowIfNull(currentEditorConfig);
        return lastStartedConfig ?? currentEditorConfig;
    }

    private async Task<bool> WaitForRtmpTracksReadyAsync(
        RtmpOutputConfig config,
        MediaPlaybackIdentity expectedIdentity,
        CancellationToken cancellationToken)
    {
        var startTimestamp = Stopwatch.GetTimestamp();
        while (true)
        {
            if (_mediaPool.CurrentIdentity != expectedIdentity)
            {
                return false;
            }

            var managerSnapshot = _rtmpOutputManager.Snapshot;
            var audioSnapshot = _rtmpAudioSession.Snapshot;
            if (AreSelectedRtmpTracksReady(config, managerSnapshot.State, audioSnapshot))
            {
                return true;
            }

            if (managerSnapshot.State == RtmpOutputState.Failed
                || audioSnapshot.ErrorCode is not null
                || Stopwatch.GetElapsedTime(startTimestamp) >= RtmpPublishingWaitTimeout)
            {
                return false;
            }

            await Task.Delay(RtmpPublishingPollInterval, cancellationToken).ConfigureAwait(true);
        }
    }

    private void UpdateRtmpProjection()
    {
        if (!IsInitialized)
        {
            return;
        }

        var managerSnapshot = _rtmpOutputManager.Snapshot;
        var audioSnapshot = _rtmpAudioSession.Snapshot;
        var reconnectSnapshot = _rtmpReconnectCoordinator.Snapshot;
        var managerPublishing = managerSnapshot.State == RtmpOutputState.Publishing;
        var managerStarting = managerSnapshot.State == RtmpOutputState.Starting;
        var reconnecting = reconnectSnapshot.State == RtmpOutputState.Reconnecting;
        var managerNeedsCleanup = managerSnapshot.ProcessId is not null
            || managerSnapshot.FinalPcmInputOpen;
        var lifecycleBusy = managerSnapshot.State is
            RtmpOutputState.Starting or RtmpOutputState.Reconnecting or RtmpOutputState.Stopping
            || reconnecting;
        var hasActiveSession = managerPublishing
            || managerStarting
            || reconnecting
            || audioSnapshot.IsRunning
            || managerNeedsCleanup;
        var failed = !reconnecting && (managerSnapshot.State == RtmpOutputState.Failed
            || audioSnapshot.ErrorCode is not null);
        var stateLabel = reconnecting || managerSnapshot.State == RtmpOutputState.Reconnecting
            ? "重连中"
            : managerPublishing
                ? "推流中"
                : managerStarting
                    ? "启动中"
                    : managerSnapshot.State == RtmpOutputState.Stopping
                        ? "停止中"
                        : failed
                            ? "失败"
                            : "待机";
        SetStatusPill(RtmpStatePillText, stateLabel, managerPublishing && !reconnecting);
        StartRtmpButton.IsEnabled = _login.CanEnterWorkbench && !hasActiveSession && !lifecycleBusy;
        StopRtmpButton.IsEnabled = _login.CanEnterWorkbench
            && CanStopRtmpSession(
                managerSnapshot.State,
                reconnectSnapshot.State,
                audioSnapshot,
                managerNeedsCleanup);
        ReconnectRtmpButton.IsEnabled = _login.CanEnterWorkbench
            && failed
            && !hasActiveSession
            && !lifecycleBusy;
        ReconnectRtmpButton.ToolTip = ReconnectRtmpButton.IsEnabled
            ? "重新启动当前 RTMP 配置，最多尝试 6 次"
            : "仅在 RTMP 会话明确失败后可用";
        if (failed && !managerPublishing && string.IsNullOrWhiteSpace(RtmpStatusText.Text))
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

                var reconnecting = _rtmpReconnectCoordinator.Snapshot.State == RtmpOutputState.Reconnecting;
                if (!reconnecting
                    && (snapshot.State == RtmpOutputState.Failed
                        || _rtmpAudioSession.Snapshot.ErrorCode is not null))
                {
                    RtmpStatusText.Text = snapshot.Error ?? "RTMP 推流已断开，可重新连接";
                    _state.SetStatus("RTMP 推流已断开，可重新连接");
                }
                else if (snapshot.State == RtmpOutputState.Publishing)
                {
                    RtmpStatusText.Text = $"RTMP 推流中 · {snapshot.TargetUrl ?? "rtmp://…"}";
                    _state.SetStatus("RTMP 推流已建立");
                }
                UpdateRtmpProjection();
            },
            System.Windows.Threading.DispatcherPriority.Background);
    }

    private async Task<bool> StopRtmpForMediaMutationAsync()
    {
        CancelRtmpReconnect();
        var managerSnapshot = _rtmpOutputManager.Snapshot;
        var state = managerSnapshot.State;
        var reconnecting = _rtmpReconnectCoordinator.Snapshot.State == RtmpOutputState.Reconnecting;
        var managerNeedsCleanup = managerSnapshot.ProcessId is not null
            || managerSnapshot.FinalPcmInputOpen;
        if (!_rtmpAudioSession.Snapshot.IsRunning
            && state is not (RtmpOutputState.Starting or RtmpOutputState.Publishing)
            && !reconnecting
            && !managerNeedsCleanup)
        {
            _audioPlaybackController.SetRtmpConsumerAttached(false);
            _lastRtmpConfig = null;
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
        _lastRtmpConfig = null;
        UpdateRtmpProjection();
        return true;
    }
}
