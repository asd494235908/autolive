using Microsoft.Win32;
using System;
using System.IO;
using System.Globalization;
using System.Windows;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Configuration;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
    // 固定话术界面编排保留在此 partial；共享状态仍由 MainWindow.xaml.cs 唯一持有。

    private async void SpeakFixedSpeechButton_Click(object sender, RoutedEventArgs e)
    {
        if (!_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        var text = FixedSpeechTextBox.Text;
        if (!FixedSpeechContractValidation.TryNormalizeText(text, out _, out var validationError))
        {
            FixedSpeechStatusText.Text = validationError?.Message ?? "固定话术文本无效";
            _state.SetStatus("固定话术未启动，文本无效");
            return;
        }

        var operationId = Guid.NewGuid().ToString("N");
        await StopInterludeForPriorityAsync().ConfigureAwait(true);
        var adapter = EnsureSpeechAdapter();
        var result = await adapter.SpeakAsync(
                FixedSpeechCommandDto.Speak(operationId, text),
                cancellationToken: _windowCancellation.Token)
            .ConfigureAwait(true);

        if (!result.IsAccepted || result.Completion is null || result.OperationId is null)
        {
            FixedSpeechStatusText.Text = result.Error?.Message ?? "固定话术未启动";
            _state.SetStatus(FixedSpeechStatusText.Text);
            UpdateFixedSpeechProjection();
            return;
        }

        _speechOperationId = result.OperationId;
        FixedSpeechStatusText.Text = "正在朗读 · SAPI → 最终 PCM（待验收）";
        _state.SetStatus("固定话术已进入最终 PCM；SAPI/声卡仍待验收");
        UpdateFixedSpeechProjection();
        _ = ObserveFixedSpeechCompletionAsync(result.OperationId, result.Completion);
    }

    private async void CancelFixedSpeechButton_Click(object sender, RoutedEventArgs e)
    {
        await CancelFixedSpeechIfActiveAsync().ConfigureAwait(true);
    }

    private async Task CancelFixedSpeechIfActiveAsync()
    {
        if (_speechAdapter is null || _speechOperationId is null)
        {
            return;
        }

        var operationId = _speechOperationId;
        var result = await _speechAdapter.CancelAsync(operationId, _windowCancellation.Token)
            .ConfigureAwait(true);
        if (result.Error is not null)
        {
            _state.SetStatus(result.Error.Message);
        }
        else
        {
            FixedSpeechStatusText.Text = "固定话术已取消";
            _state.SetStatus("固定话术已取消");
        }

        if (string.Equals(_speechOperationId, operationId, StringComparison.Ordinal))
        {
            _speechOperationId = null;
        }

        UpdateFixedSpeechProjection();
    }

    private async Task ObserveFixedSpeechCompletionAsync(
        string operationId,
        Task<WindowsSpeechTerminalResult> completion)
    {
        WindowsSpeechTerminalResult terminal;
        try
        {
            terminal = await completion.ConfigureAwait(true);
        }
        catch (OperationCanceledException)
        {
            terminal = new(
                false,
                operationId,
                new(WindowsSpeechAdapterState.Cancelled, operationId, null, null),
                new(WindowsSpeechFailureCode.Cancelled, "固定话术已取消。", Retryable: true));
        }
        catch (Exception)
        {
            terminal = new(
                false,
                operationId,
                new(WindowsSpeechAdapterState.Failed, operationId, null, null),
                new(WindowsSpeechFailureCode.SpeechFailed, "Windows 本地语音播放失败。"));
        }

        if (_isClosing || !string.Equals(_speechOperationId, operationId, StringComparison.Ordinal))
        {
            return;
        }

        _speechOperationId = null;
        FixedSpeechStatusText.Text = terminal.IsSuccess
            ? "固定话术已完成 · 本地 SAPI"
            : terminal.Error?.Message ?? "固定话术未完成";
        _state.SetStatus(FixedSpeechStatusText.Text);
        UpdateFixedSpeechProjection();
    }

    private void UpdateFixedSpeechProjection()
    {
        if (!IsInitialized)
        {
            return;
        }

        var isAuthorized = _login.CanEnterWorkbench;
        var isActive = _speechAdapter?.Snapshot.State is
            WindowsSpeechAdapterState.Starting or WindowsSpeechAdapterState.Playing;
        SetStatusPill(FixedSpeechStatePillText, isActive ? "朗读中" : "待机", isActive);
        SpeakFixedSpeechButton.IsEnabled = isAuthorized && !isActive;
        CancelFixedSpeechButton.IsEnabled = isAuthorized && isActive;
    }

    private WindowsSystemSpeechAdapter EnsureSpeechAdapter() =>
        _speechAdapter ??= new WindowsSystemSpeechAdapter(
            new WindowsSapiSpeechBridge(),
            audioPriority: _audioPriority,
            finalPcmBusProvider: () => _audioPlaybackController.ActiveFinalPcmBus);

    // 插话文件池界面编排保留在此 partial；共享状态仍由 MainWindow.xaml.cs 唯一持有。

    private async void InterludeScheduleTimer_Tick(object? sender, EventArgs e)
    {
        if (_isClosing || _interludeScheduleStartInFlight)
        {
            return;
        }

        var mediaSnapshot = _mediaPool.Snapshot;
        var pool = _interludePool.Snapshot;
        var scheduleKey = BuildInterludeScheduleKey(mediaSnapshot);
        var audioSnapshot = _audioPlaybackController.Snapshot;
        var rtmpAudioActive = _rtmpAudioSession.Snapshot.IsRunning;
        var audioHostReady = audioSnapshot.State is WindowsAudioPlaybackState.Playing
            or WindowsAudioPlaybackState.Paused
            || rtmpAudioActive;
        var priority = _audioPriority.Snapshot;
        var playbackPaused = mediaSnapshot.PlaybackState is not PlaybackState.Playing
            || priority.FixedSpeechActive
            || priority.MicrophoneSpeaking;
        var scheduleEnabled = _interludeConfig.Enabled
            && pool.Status is InterludePoolStatus.Ready
            && pool.Files.Length > 0
            && mediaSnapshot.PlaybackState is PlaybackState.Playing or PlaybackState.Paused;

        if (!scheduleEnabled)
        {
            _interludeSchedulePlanner.Reset(scheduleKey, resetLastFileIndex: false);
            _state.SetInterludeEffectCycleProgress(0);
            return;
        }

        var decision = _interludeSchedulePlanner.Observe(
            scheduleKey,
            Environment.TickCount64,
            pool.Files.Length,
            _interludeConfig.IntervalMinMs,
            _interludeConfig.IntervalMaxMs,
            enabled: true,
            playbackActive: _interludeObservationTask is not null || priority.InterludeActive,
            playbackStarting: _interludeScheduleStartInFlight,
            paused: playbackPaused,
            canStartPlayback: audioHostReady);
        _state.SetInterludeEffectCycleProgress(
            decision.ShouldStart ? 100 : decision.ProgressPercent);
        if (!decision.ShouldStart || decision.FileIndex is not int fileIndex)
        {
            return;
        }

        _interludeScheduleStartInFlight = true;
        try
        {
            await RunPlaybackCommandAsync(
                    () => StartInterludeFileAsync(fileIndex, fromScheduler: true),
                    _windowCancellation.Token)
                .ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (_isClosing || _windowCancellation.IsCancellationRequested)
        {
            // 关闭时由窗口取消路径回收插话解码器。
        }
        finally
        {
            _interludeScheduleStartInFlight = false;
        }
    }

    private static string? BuildInterludeScheduleKey(AppState snapshot) =>
        snapshot.SourceMediaPool.IsEmpty
            ? null
            : $"{snapshot.PlaybackGeneration}:{snapshot.SourceRevision}:{snapshot.SourceMediaIndex}";

    private async void SelectInterludeDirectoryButton_Click(object sender, RoutedEventArgs e)
    {
        if (!_login.CanEnterWorkbench || _importBusy || _isClosing)
        {
            return;
        }

        var dialog = new OpenFolderDialog
        {
            Title = "选择插话文件目录（递归扫描）",
            Multiselect = false,
        };
        if (dialog.ShowDialog(this) != true || string.IsNullOrWhiteSpace(dialog.FolderName))
        {
            return;
        }

        _importBusy = true;
        UpdateInterludeProjection();
        UpdateMicrophoneProjection();
        SetImportButtonsEnabled(false);
        InterludePoolStatusText.Text = "正在递归扫描插话目录…";
        string? scanError = null;
        try
        {
            var result = await Task.Run(
                () => _interludePool.ScanDirectory(dialog.FolderName, _windowCancellation.Token),
                _windowCancellation.Token).ConfigureAwait(true);
            if (_isClosing)
            {
                return;
            }

            UpdateInterludeProjection(result.Snapshot);
            if (!result.IsSuccess)
            {
                scanError = result.Error?.Message ?? "插话目录扫描失败";
                InterludePoolStatusText.Text = scanError;
                _state.SetStatus("插话目录扫描失败；旧快照已保留");
                return;
            }

            _interludeConfig = _interludeConfig with
            {
                Enabled = result.Snapshot.Files.Length > 0,
                Directory = result.Snapshot.Directory,
            };
            await PersistInterludeAudioConfigAsync().ConfigureAwait(true);

            _state.SetStatus(result.Snapshot.Files.Length == 0
                ? "插话目录为空；未启动解码或音频流"
                : $"插话目录扫描完成；已发现 {result.Snapshot.Files.Length} 个候选文件");
        }
        catch (OperationCanceledException) when (_isClosing || _windowCancellation.IsCancellationRequested)
        {
            // 关闭窗口时取消扫描；不再更新 UI。
        }
        finally
        {
            _importBusy = false;
            if (!_isClosing)
            {
                UpdateInterludeProjection();
                UpdateMicrophoneProjection();
                SetImportButtonsEnabled(_login.CanEnterWorkbench);
                if (scanError is not null)
                {
                    InterludePoolStatusText.Text = scanError;
                }
            }
        }
    }

    private async void ClearInterludePoolButton_Click(object sender, RoutedEventArgs e)
    {
        if (!_login.CanEnterWorkbench || _isClosing)
        {
            return;
        }

        var confirmation = MessageBox.Show(
            this,
            "确定清空当前插话文件池吗？不会删除磁盘文件。",
            "清空插话文件池",
            MessageBoxButton.YesNo,
            MessageBoxImage.Question);
        if (confirmation != MessageBoxResult.Yes)
        {
            return;
        }

        await StopInterludeForPriorityAsync().ConfigureAwait(true);

        var result = _interludePool.Clear();
        _interludeConfig = _interludeConfig with { Enabled = false, Directory = null };
        await PersistInterludeAudioConfigAsync().ConfigureAwait(true);
        UpdateInterludeProjection(result.Snapshot);
        _state.SetStatus(result.Changed ? "插话文件池已清空" : "插话文件池本来就是空的");
    }

    private async void PlayInterludeButton_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            await RunPlaybackCommandAsync(() => StartInterludeFileAsync()).ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (_isClosing || _windowCancellation.IsCancellationRequested)
        {
            // 关闭窗口时由统一取消路径回收插话解码器。
        }
    }

    private async void StopInterludeButton_Click(object sender, RoutedEventArgs e)
    {
        await RunPlaybackCommandAsync(StopInterludeFileAsync).ConfigureAwait(true);
    }

    private async void ApplyInterludeCycleButton_Click(object sender, RoutedEventArgs e)
    {
        if (!_login.CanEnterWorkbench || _isClosing)
        {
            return;
        }

        if (!TryParseInterludeSeconds(InterludeCycleMinTextBox.Text, "插话间隔最小值", out var minimumMs, out var error)
            || !TryParseInterludeSeconds(InterludeCycleMaxTextBox.Text, "插话间隔最大值", out var maximumMs, out error))
        {
            InterludeCycleValidationText.Text = error ?? "插话周期输入无效";
            _state.SetStatus(InterludeCycleValidationText.Text);
            return;
        }

        if (minimumMs > maximumMs)
        {
            InterludeCycleValidationText.Text = "插话间隔最小值不能大于最大值。";
            _state.SetStatus(InterludeCycleValidationText.Text);
            return;
        }

        var next = _interludeConfig with
        {
            IntervalMinMs = minimumMs,
            IntervalMaxMs = maximumMs,
        };
        if (!InterludeAudioRules.TryValidate(next, out var validationError))
        {
            InterludeCycleValidationText.Text = validationError?.Message ?? "插话周期配置无效";
            _state.SetStatus(InterludeCycleValidationText.Text);
            return;
        }

        _interludeConfig = next;
        _interludeSchedulePlanner.Reset(
            BuildInterludeScheduleKey(_mediaPool.Snapshot),
            resetLastFileIndex: false);
        _state.SetInterludeEffectCycleProgress(0);
        await PersistInterludeAudioConfigAsync().ConfigureAwait(true);
        InterludeCycleValidationText.Text = string.Empty;
        UpdateInterludeProjection();
        UpdateEffectCycleProjection();
        _state.SetStatus("插话触发间隔已应用；下一次插话按新范围等待");
    }

    private static bool TryParseInterludeSeconds(
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
        if (value < InterludeAudioRules.MinIntervalMs
            || value > InterludeAudioRules.MaxIntervalMs)
        {
            error = $"{label}必须在 0.5 到 60 秒之间。";
            return false;
        }

        milliseconds = checked((ulong)Math.Round(value, MidpointRounding.AwayFromZero));
        return true;
    }

    private async Task StartInterludeFileAsync(int? selectedIndex = null, bool fromScheduler = false)
    {
        if (_isClosing || !_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        var pool = _interludePool.Snapshot;
        if (pool.Status is not InterludePoolStatus.Ready || pool.Files.IsDefaultOrEmpty)
        {
            _state.SetStatus("插话文件池为空，无法开始插话");
            UpdateInterludeProjection(pool);
            return;
        }

        var audioState = _audioPlaybackController.Snapshot.State;
        var localAudioActive = audioState is WindowsAudioPlaybackState.Playing or WindowsAudioPlaybackState.Paused;
        var rtmpAudioActive = _rtmpAudioSession.Snapshot.IsRunning;
        if (!localAudioActive && !rtmpAudioActive)
        {
            _state.SetStatus("插话文件仅可叠加到正在播放的纯音频或 RTMP 声音会话");
            return;
        }

        var sessionIdentity = _mediaPool.CurrentIdentity;
        if (sessionIdentity is null)
        {
            _state.SetStatus("当前播放身份不可用，插话未启动");
            return;
        }

        var priority = _audioPriority.BeginInterludeFile();
        if (!priority.IsAccepted)
        {
            _state.SetStatus(priority.Error ?? "插话文件被当前音频层级拒绝");
            UpdateInterludeProjection();
            return;
        }

        var runtime = await EnsureVerifiedMediaRuntimeAsync().ConfigureAwait(true);
        if (runtime is null
            || !runtime.TryGetResource("ffmpeg.exe", out var ffmpeg)
            || ffmpeg is null)
        {
            _audioPriority.End(AudioPriorityLayer.InterludeFile);
            _state.SetStatus("FFmpeg 运行资源未校验，插话未启动");
            UpdateInterludeProjection();
            return;
        }

        var effectiveInterludeConfig = _interludeConfig with
        {
            Enabled = true,
            Directory = pool.Directory,
        };
        if (!_interludeSelector.TrySelect(
                effectiveInterludeConfig,
                DateTimeOffset.UtcNow,
                out var audioSelection,
                out var selectionError)
            || audioSelection is null)
        {
            _audioPriority.End(AudioPriorityLayer.InterludeFile);
            _state.SetStatus(selectionError?.Message ?? "插话声音预设选择失败");
            UpdateInterludeProjection();
            return;
        }

        if (!audioSelection.TryCreateBoundedAudioEffectParams(
                out var interludeAudioEffects,
                out var projectionError)
            || interludeAudioEffects is null)
        {
            _audioPriority.End(AudioPriorityLayer.InterludeFile);
            _state.SetStatus(projectionError ?? "插话声音预设尚未接入当前消费链");
            UpdateInterludeProjection();
            return;
        }

        var entryIndex = selectedIndex ?? 0;
        if (entryIndex < 0 || entryIndex >= pool.Files.Length)
        {
            _audioPriority.End(AudioPriorityLayer.InterludeFile);
            _state.SetStatus("插话文件索引已失效，等待下一次调度");
            UpdateInterludeProjection();
            return;
        }

        var entry = pool.Files[entryIndex];
        var interludeSource = CreateInterludeSource(entry);
        if (_mediaPool.CurrentIdentity != sessionIdentity)
        {
            _audioPriority.End(AudioPriorityLayer.InterludeFile);
            _state.SetStatus("播放项已变化，已拒绝过期插话");
            UpdateInterludeProjection();
            return;
        }

        var currentPool = _interludePool.Snapshot;
        if (currentPool.Status is not InterludePoolStatus.Ready
            || entryIndex >= currentPool.Files.Length
            || !string.Equals(currentPool.Files[entryIndex].Path, entry.Path, StringComparison.OrdinalIgnoreCase))
        {
            _audioPriority.End(AudioPriorityLayer.InterludeFile);
            _state.SetStatus("插话文件池已变化，已拒绝过期插话");
            UpdateInterludeProjection(currentPool);
            return;
        }

        if (!FfmpegPcmDecodePlanBuilder.TryCreate(
                ffmpeg.AbsolutePath,
                interludeSource,
                FfmpegPcmDecodePlanBuilder.DefaultSampleRateHz,
                localAudioActive
                    ? _audioPlaybackController.Snapshot.Output?.Channels ?? FinalPcmBus.DefaultChannels
                    : FinalPcmBus.DefaultChannels,
                out var plan,
                out var planError,
                audioEffects: interludeAudioEffects)
            || plan is null)
        {
            _audioPriority.End(AudioPriorityLayer.InterludeFile);
            _state.SetStatus(planError?.Message ?? "插话解码计划无效");
            UpdateInterludeProjection();
            return;
        }

        bool started;
        string? startError;
        Task completion;
        if (localAudioActive)
        {
            var localStarted = await _audioPlaybackController
                .StartInterludeAsync(plan, _windowCancellation.Token)
                .ConfigureAwait(true);
            started = localStarted.IsSuccess;
            startError = localStarted.Error?.Message;
            completion = _audioPlaybackController.InterludeCompletion;
        }
        else
        {
            var rtmpStarted = await _rtmpAudioSession
                .StartInterludeAsync(plan, _windowCancellation.Token)
                .ConfigureAwait(true);
            started = rtmpStarted.IsSuccess;
            startError = rtmpStarted.Error?.Message;
            completion = _rtmpAudioSession.InterludeCompletion;
        }

        if (!started)
        {
            _audioPriority.End(AudioPriorityLayer.InterludeFile);
            _state.SetStatus(startError ?? "插话解码未启动");
            UpdateInterludeProjection();
            return;
        }

        var priorityCompletion = ObserveInterludePriorityCompletionAsync(
            completion,
            _audioPriority);
        _interludeObservationTask = priorityCompletion;
        if (!fromScheduler)
        {
            _interludeSchedulePlanner.MarkPlaybackStarted(
                BuildInterludeScheduleKey(_mediaPool.Snapshot),
                entryIndex);
            _state.SetInterludeEffectCycleProgress(0);
        }
        InterludePoolStatusText.Text = rtmpAudioActive && !localAudioActive
            ? $"正在播放插话 · {entry.FileName} · 与 RTMP PCM 分流共享混音"
            : $"正在播放插话 · {entry.FileName} · 与主音频共享 PortAudio 输出";
        var presetSummary = string.Join(", ", audioSelection.PresetIds);
        _state.SetStatus($"插话已开始：{entry.FileName} · 预设 {presetSummary}（C# 单轨参数已送入 FFmpeg，效果待验收）");
        UpdateInterludeProjection();
        _ = ObserveInterludeCompletionAsync(priorityCompletion, entry.FileName);
    }

    private async Task StopInterludeFileAsync()
    {
        var localResult = await _audioPlaybackController.StopInterludeAsync().ConfigureAwait(true);
        var rtmpResult = await _rtmpAudioSession.StopInterludeAsync().ConfigureAwait(true);
        if (localResult.IsSuccess && rtmpResult.IsSuccess)
        {
            _audioPriority.End(AudioPriorityLayer.InterludeFile);
            _interludeSchedulePlanner.Reset(
                BuildInterludeScheduleKey(_mediaPool.Snapshot),
                resetLastFileIndex: false);
            InterludePoolStatusText.Text = "插话已停止；主音频继续播放";
            _state.SetStatus("插话已停止；主音频继续播放");
        }
        else
        {
            _state.SetStatus(localResult.Error?.Message ?? rtmpResult.Error?.Message ?? "插话停止失败");
        }

        UpdateInterludeProjection();
    }

    private async Task StopInterludeForPriorityAsync()
    {
        if (_interludeObservationTask is null && !_audioPriority.Snapshot.InterludeActive)
        {
            return;
        }

        var localResult = await _audioPlaybackController.StopInterludeAsync().ConfigureAwait(true);
        var rtmpResult = await _rtmpAudioSession.StopInterludeAsync().ConfigureAwait(true);
        if (localResult.IsSuccess && rtmpResult.IsSuccess)
        {
            _audioPriority.End(AudioPriorityLayer.InterludeFile);
            UpdateInterludeProjection();
        }
    }

    private async Task ObserveInterludeCompletionAsync(Task completion, string fileName)
    {
        try
        {
            await completion.ConfigureAwait(true);
        }
        catch (OperationCanceledException)
        {
            // 用户停止或主会话结束均视为正常插话退出。
        }
        catch (Exception)
        {
            if (!_isClosing)
            {
                _state.SetStatus("插话解码异常退出；主音频未受影响");
            }
        }

        if (_isClosing || !ReferenceEquals(_interludeObservationTask, completion))
        {
            return;
        }

        _interludeObservationTask = null;
        InterludePoolStatusText.Text = $"插话已结束 · {fileName} · 主音频继续播放";
        _state.SetStatus("插话已结束；主音频继续播放");
        UpdateInterludeProjection();
    }

    internal static async Task ObserveInterludePriorityCompletionAsync(
        Task completion,
        AudioPriorityCoordinator priority)
    {
        ArgumentNullException.ThrowIfNull(completion);
        ArgumentNullException.ThrowIfNull(priority);
        try
        {
            await completion.ConfigureAwait(false);
        }
        finally
        {
            priority.End(AudioPriorityLayer.InterludeFile);
        }
    }

    private void UpdateInterludeProjection(InterludePoolSnapshot? provided = null)
    {
        if (!IsInitialized)
        {
            return;
        }

        var snapshot = provided ?? _interludePool.Snapshot;
        var authorized = _login.CanEnterWorkbench;
        var audioState = _audioPlaybackController.Snapshot.State;
        var rtmpAudioActive = _rtmpAudioSession.Snapshot.IsRunning;
        var audioCanHostInterlude = audioState is WindowsAudioPlaybackState.Playing or WindowsAudioPlaybackState.Paused
            || rtmpAudioActive;
        var interludeActive = _interludeObservationTask is not null
            || _audioPriority.Snapshot.InterludeActive;
        var persistedDirectory = snapshot.Status is InterludePoolStatus.Disabled
            ? _interludeConfig.Directory
            : null;
        var projectedDirectory = snapshot.Directory ?? persistedDirectory;
        InterludeDirectoryTextBox.Text = projectedDirectory ?? string.Empty;
        InterludeAudioConfigStatusText.Text = FormatInterludeAudioConfig(_interludeConfig);
        if (!InterludeVolumeSlider.IsMouseCaptureWithin
            && !InterludeVolumeSlider.IsKeyboardFocusWithin)
        {
            InterludeVolumeSlider.Value = InterludeVolumeDbToPercent(_interludeConfig.VolumeDb);
        }
        InterludeVolumeText.Text = $"{InterludeVolumeDbToPercent(_interludeConfig.VolumeDb):0}%";
        if (!InterludeCycleMinTextBox.IsKeyboardFocusWithin
            && !InterludeCycleMaxTextBox.IsKeyboardFocusWithin)
        {
            InterludeCycleMinTextBox.Text = FormatSeconds(_interludeConfig.IntervalMinMs);
            InterludeCycleMaxTextBox.Text = FormatSeconds(_interludeConfig.IntervalMaxMs);
        }
        ApplyInterludeCycleButton.IsEnabled = authorized && !_importBusy;
        InterludeVolumeSlider.IsEnabled = authorized && !_importBusy;
        SelectInterludeDirectoryButton.IsEnabled = authorized && !_importBusy;
        ClearInterludePoolButton.IsEnabled = authorized
            && !_importBusy
            && (projectedDirectory is not null || snapshot.Files.Length > 0);
        PlayInterludeButton.IsEnabled = authorized
            && snapshot.Status is InterludePoolStatus.Ready
            && snapshot.Files.Length > 0
            && audioCanHostInterlude
            && !interludeActive;
        StopInterludeButton.IsEnabled = authorized && interludeActive;
        if (interludeActive)
        {
            if (!InterludePoolStatusText.Text.Contains("插话", StringComparison.Ordinal))
            {
                InterludePoolStatusText.Text = "插话正在播放 · 与主音频共享 PortAudio 输出";
            }
        }
        else
        {
            InterludePoolStatusText.Text = snapshot.Status switch
            {
                InterludePoolStatus.Ready => $"已发现 {snapshot.Files.Length} 个候选文件 · 播放中按 {_interludeConfig.IntervalMinMs}–{_interludeConfig.IntervalMaxMs} ms 自动插话",
                InterludePoolStatus.Empty => "目录中没有受支持的媒体文件 · 不启动解码或音频流",
                InterludePoolStatus.Failed => snapshot.Error ?? "插话目录扫描失败；旧快照已保留",
                InterludePoolStatus.Disabled when persistedDirectory is not null => "已保存插话目录；重新选择目录以刷新候选文件",
                _ => "未配置；不会启动解码或音频流",
            };
        }
    }

    private static string FormatInterludeAudioConfig(InterludeAudioConfig config)
    {
        var mode = config.AudioSelectionMode is InterludeAudioSelectionMode.Fixed
            ? $"固定 {config.AudioFixedPresetId}"
            : $"随机 {config.AudioPresetIds.Length} 个预设";
        var mix = config.AudioMixEnabled
            ? $"多轨 {config.AudioMixPickMin}–{config.AudioMixPickMax}"
            : "多轨关闭";
        return $"预设：{mode} · {mix} · 音量 {config.VolumeDb:+0.#;-0.#;0} dB · 插话间隔 {FormatSeconds(config.IntervalMinMs)}–{FormatSeconds(config.IntervalMaxMs)} 秒 · duck {config.DuckingDepthDb:0.#} dB · 淡入/淡出 {config.DuckingAttackMs}/{config.DuckingReleaseMs} ms（曲线待验收）";
    }

    private static string FormatSeconds(ulong milliseconds) =>
        (milliseconds / 1_000d).ToString("0.###", CultureInfo.CurrentCulture);

    private static SourceMediaDto CreateInterludeSource(InterludeFileEntry entry) =>
        new(
            entry.Path,
            entry.Path,
            entry.MediaKind,
            MediaCompatibilityMode.Direct,
            entry.FileName,
            entry.FileSizeBytes,
            null,
            null,
            null,
            null,
            null,
            null,
            FfmpegPcmDecodePlanBuilder.DefaultSampleRateHz,
            FinalPcmBus.DefaultChannels,
            null,
            "unknown",
            null,
            "not-computed");

    private async Task<string?> LoadInterludeAudioConfigAsync()
    {
        if (_interludeConfigStore is null || _isClosing)
        {
            return null;
        }

        try
        {
            _interludeConfig = await _interludeConfigStore
                .ReadAsync(_windowCancellation.Token)
                .ConfigureAwait(true);
            var restoreResult = await Task.Run(
                    () => RestoreConfiguredInterludePool(
                        _interludeConfig,
                        _interludePool,
                        _windowCancellation.Token),
                    _windowCancellation.Token)
                .ConfigureAwait(true);
            if (restoreResult is { IsSuccess: false })
            {
                var message = $"插话候选恢复失败：{restoreResult.Error?.Message ?? "目录扫描失败"}；已保存目录保持不变";
                _state.SetStatus(message);
                return message;
            }
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 关闭时不再更新本地配置状态。
        }
        catch (ConfigurationValidationException)
        {
            _interludeConfig = InterludeAudioConfig.Default;
            _state.SetStatus("插话声音配置无效，已回退到默认值");
        }
        catch (IOException)
        {
            _state.SetStatus("插话声音配置读取失败，使用内存默认值");
        }
        catch (UnauthorizedAccessException)
        {
            _state.SetStatus("插话声音配置不可访问，使用内存默认值");
        }

        return null;
    }

    internal static InterludePoolResult? RestoreConfiguredInterludePool(
        InterludeAudioConfig config,
        InterludeFilePoolService pool,
        CancellationToken cancellationToken) =>
        config.Enabled && !string.IsNullOrWhiteSpace(config.Directory)
            ? pool.ScanDirectory(config.Directory, cancellationToken)
            : null;

    private async Task PersistInterludeAudioConfigAsync()
    {
        if (_interludeConfigStore is null || _isClosing)
        {
            return;
        }

        try
        {
            await _interludeConfigStore
                .WriteAsync(_interludeConfig, _windowCancellation.Token)
                .ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 关闭时取消写入；内存快照仍然有效。
        }
        catch (IOException)
        {
            _state.SetStatus("插话声音配置保存失败；当前会话仍使用内存值");
        }
        catch (UnauthorizedAccessException)
        {
            _state.SetStatus("插话声音配置保存被系统拒绝；当前会话仍使用内存值");
        }
    }

    private static InterludeAudioConfigStore? TryCreateInterludeConfigStore()
    {
        var localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        if (string.IsNullOrWhiteSpace(localAppData))
        {
            return null;
        }

        try
        {
            var path = Path.Combine(
                localAppData,
                "GpAutoLive",
                "profiles",
                "interlude",
                "default.json");
            return new InterludeAudioConfigStore(path);
        }
        catch (ArgumentException)
        {
            return null;
        }
        catch (IOException)
        {
            return null;
        }
        catch (NotSupportedException)
        {
            return null;
        }
    }

    // 麦克风本地门控界面编排保留在此 partial；共享状态仍由 MainWindow.xaml.cs 唯一持有。

    private async void RefreshAudioDevicesButton_Click(object sender, RoutedEventArgs e) =>
        await RefreshAudioDevicesAsync().ConfigureAwait(true);

    /// <summary>
    /// 懒加载并复用一次 PortAudio 设备枚举；播放启动时也调用此入口，
    /// 避免用户必须先手动点击设备刷新才能播放带音轨媒体。
    /// </summary>
    private async Task RefreshAudioDevicesAsync()
    {
        if (!_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        RefreshAudioDevicesButton.IsEnabled = false;
        AudioDeviceStatusText.Text = "正在枚举 PortAudio 输入/输出设备…";
        try
        {
            var runtime = await EnsureVerifiedMediaRuntimeAsync().ConfigureAwait(true);
            if (runtime is null)
            {
                AudioOutputDeviceComboBox.ItemsSource = null;
                AudioInputDeviceComboBox.ItemsSource = null;
                UpdateMicrophoneProjection();
                AudioDeviceStatusText.Text = "媒体运行资源未校验，无法枚举 PortAudio";
                return;
            }

            if (!runtime.TryGetResource("portaudio_x64.dll", out var resource) || resource is null)
            {
                AudioOutputDeviceComboBox.ItemsSource = null;
                AudioInputDeviceComboBox.ItemsSource = null;
                UpdateMicrophoneProjection();
                AudioDeviceStatusText.Text = "PortAudio DLL 未安装；不会回退到系统 PATH";
                _state.SetStatus("PortAudio 运行资源未安装");
                return;
            }

            _portAudioEnumerator ??= new WindowsPortAudioDeviceEnumerator();
            var result = await _portAudioEnumerator
                .ProbeAsync(resource.AbsolutePath, _windowCancellation.Token)
                .ConfigureAwait(true);
            if (_isClosing || _windowCancellation.IsCancellationRequested)
            {
                return;
            }

            if (!result.IsSuccess)
            {
                AudioOutputDeviceComboBox.ItemsSource = null;
                AudioOutputDeviceComboBox.IsEnabled = false;
                AudioInputDeviceComboBox.ItemsSource = null;
                AudioInputDeviceComboBox.IsEnabled = false;
                UpdateMicrophoneProjection();
                AudioDeviceStatusText.Text = result.Error?.Message ?? "PortAudio 设备枚举失败";
                _state.SetStatus("PortAudio 设备枚举失败；未启动音频流");
                return;
            }

            var outputDevices = result.Snapshot.Devices
                .Where(static device => device.MaxOutputChannels > 0)
                .ToArray();
            AudioOutputDeviceComboBox.ItemsSource = outputDevices;
            AudioOutputDeviceComboBox.IsEnabled = outputDevices.Length > 0;
            if (result.Snapshot.DefaultOutputDevice is int defaultIndex
                && outputDevices.Any(device => device.Index == defaultIndex))
            {
                AudioOutputDeviceComboBox.SelectedValue = defaultIndex;
            }
            else if (outputDevices.Length > 0)
            {
                // 某些驱动不会返回 PortAudio 默认设备索引；仍选择第一个
                // 已验证的输出设备，避免“枚举成功但播放永远要求手选”。
                AudioOutputDeviceComboBox.SelectedIndex = 0;
            }

            var inputDevices = result.Snapshot.Devices
                .Where(static device => device.MaxInputChannels > 0)
                .ToArray();
            AudioInputDeviceComboBox.ItemsSource = inputDevices;
            if (inputDevices.Length > 0 && AudioInputDeviceComboBox.SelectedIndex < 0)
            {
                AudioInputDeviceComboBox.SelectedIndex = 0;
            }
            UpdateMicrophoneProjection();

            AudioDeviceStatusText.Text = outputDevices.Length == 0
                ? $"PortAudio 已加载；输入设备 {inputDevices.Length} 个，未发现可用输出设备"
                : $"PortAudio 已发现 {outputDevices.Length} 个输出设备、{inputDevices.Length} 个输入设备 · 尚未启动音频流";
            _state.SetStatus("PortAudio 输入/输出设备枚举完成；尚未启动音频流");
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            AudioDeviceStatusText.Text = "设备枚举已取消";
        }
        finally
        {
            RefreshAudioDevicesButton.IsEnabled = !_isClosing
                && _login.CanEnterWorkbench
                && _microphoneInterludeController?.Snapshot.Input.IsRunning != true;
        }
    }

    private async void StartMicrophoneButton_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            await StartMicrophoneAsync().ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (_isClosing || _windowCancellation.IsCancellationRequested)
        {
            // 关闭窗口时取消资源校验；不把取消显示成启动失败。
        }
    }

    private async Task StartMicrophoneAsync()
    {
        if (_isClosing || !_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        if (AudioInputDeviceComboBox.SelectedValue is not int deviceIndex)
        {
            MicrophoneStatusText.Text = "请先刷新并选择 PortAudio 麦克风输入设备";
            _state.SetStatus("麦克风门控未启动，尚未选择输入设备");
            return;
        }

        var runtime = await EnsureVerifiedMediaRuntimeAsync().ConfigureAwait(true);
        if (runtime is null
            || !runtime.TryGetResource("portaudio_x64.dll", out var resource)
            || resource is null)
        {
            UpdateMicrophoneProjection();
            MicrophoneStatusText.Text = "PortAudio DLL 未校验；麦克风门控保持关闭";
            _state.SetStatus("PortAudio 运行资源未安装，麦克风门控未启动");
            return;
        }

        _microphoneInterludeController ??= new WindowsMicrophoneInterludeController(
            _audioPriority,
            finalPcmBusProvider: () => _audioPlaybackController.ActiveFinalPcmBus);
        _microphoneInterludeController.SnapshotChanged -= MicrophoneInterludeController_SnapshotChanged;
        _microphoneInterludeController.SnapshotChanged += MicrophoneInterludeController_SnapshotChanged;

        var result = await _microphoneInterludeController
            .StartAsync(
                resource.AbsolutePath,
                new WindowsPortAudioInputConfig(deviceIndex, Channels: 1),
                _windowCancellation.Token)
            .ConfigureAwait(true);
        UpdateMicrophoneProjection(result.Snapshot);
        if (!result.IsSuccess)
        {
            MicrophoneStatusText.Text = result.Error?.Message ?? "麦克风门控启动失败";
            _state.SetStatus("麦克风本地门控未启动");
            return;
        }

        MicrophoneStatusText.Text = "麦克风门控已启用 · 仅本地处理，不识别/上传";
        _state.SetStatus("麦克风本地能量门控已启用；AEC/降噪/AGC仍待验收");
    }

    private async void StopMicrophoneButton_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            await StopMicrophoneAsync().ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (_isClosing || _windowCancellation.IsCancellationRequested)
        {
            // 关闭窗口时取消门控停止；由 OnClosed 继续完成有界释放。
        }
    }

    private async Task<bool> StopMicrophoneAsync()
    {
        if (_isClosing)
        {
            return true;
        }

        var controller = _microphoneInterludeController;
        if (controller is null)
        {
            UpdateMicrophoneProjection();
            return true;
        }

        var result = await controller.StopAsync().ConfigureAwait(true);
        if (_isClosing)
        {
            return true;
        }

        UpdateMicrophoneProjection(result.Snapshot);
        if (!result.IsSuccess)
        {
            MicrophoneStatusText.Text = result.Error?.Message ?? "麦克风门控停止失败";
            _state.SetStatus("麦克风门控停止失败；已请求释放输入流");
            return false;
        }

        MicrophoneStatusText.Text = "麦克风门控已停止";
        _state.SetStatus("麦克风本地能量门控已停止");
        return true;
    }

    private void MicrophoneInterludeController_SnapshotChanged(WindowsMicrophoneInterludeSnapshot snapshot)
    {
        if (_isClosing)
        {
            return;
        }

        _microphoneUiUpdates.Post(async () =>
        {
            if (!_isClosing)
            {
                await ApplyMicrophoneSnapshotAsync(snapshot).ConfigureAwait(true);
            }
        });
    }

    private async Task ApplyMicrophoneSnapshotAsync(WindowsMicrophoneInterludeSnapshot snapshot)
    {
        if (_isClosing)
        {
            return;
        }

        UpdateMicrophoneProjection(snapshot);
        if (snapshot.Priority.MicrophoneSpeaking)
        {
            await StopInterludeForPriorityAsync().ConfigureAwait(true);
            await CancelFixedSpeechIfActiveAsync().ConfigureAwait(true);
        }
    }

    private void UpdateMicrophoneProjection(WindowsMicrophoneInterludeSnapshot? provided = null)
    {
        if (!IsInitialized)
        {
            return;
        }

        var snapshot = provided ?? _microphoneInterludeController?.Snapshot;
        var authorized = _login.CanEnterWorkbench;
        var listening = snapshot?.State == WindowsMicrophoneInterludeState.Listening
            && snapshot.Input.IsRunning;
        var finalPcmBusAvailable = _audioPlaybackController.ActiveFinalPcmBus is { Snapshot.IsClosed: false };
        SetStatusPill(MicrophoneStatePillText, listening ? "已启用" : "待机", listening);
        var hasInputDevice = AudioInputDeviceComboBox.Items.Count > 0;
        AudioInputDeviceComboBox.IsEnabled = authorized && !listening && hasInputDevice;
        RefreshAudioDevicesButton.IsEnabled = authorized && !listening && !_importBusy;
        StartMicrophoneButton.IsEnabled = authorized
            && !listening
            && finalPcmBusAvailable
            && AudioInputDeviceComboBox.SelectedValue is int;
        StopMicrophoneButton.IsEnabled = authorized && listening;

        if (snapshot is null)
        {
            MicrophoneStatusText.Text = finalPcmBusAvailable
                ? "未启用；可输出到最终 PCM，总线 DSP 待验收"
                : "未启用；请先播放带声音媒体，AEC/降噪/AGC 待验收";
            return;
        }

        MicrophoneStatusText.Text = snapshot.State switch
        {
            WindowsMicrophoneInterludeState.Listening when snapshot.Priority.MicrophoneSpeaking =>
                "麦克风正在插话 · 本地门控（不识别/上传）",
            WindowsMicrophoneInterludeState.Listening =>
                $"麦克风门控已启用 · 本地电平 {snapshot.Gate.LevelDb:0.0} dB",
            WindowsMicrophoneInterludeState.Failed =>
                snapshot.Error ?? "麦克风门控启动失败",
            WindowsMicrophoneInterludeState.Closed => "麦克风门控已关闭",
            _ => finalPcmBusAvailable
                ? "未启用；可输出到最终 PCM，总线 DSP 待验收"
                : "未启用；请先播放带声音媒体，AEC/降噪/AGC 待验收",
        };
    }
}
