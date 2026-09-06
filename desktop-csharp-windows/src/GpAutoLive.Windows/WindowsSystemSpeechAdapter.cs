using System.Buffers;
using System.Buffers.Binary;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>
/// 固定话术的 Windows 本地语音适配器。它只持有一个活动操作，
/// 由 <see cref="FixedSpeechStateMachine"/> 负责身份和终态，桥接器负责实际 SAPI 输出。
/// </summary>
public sealed class WindowsSystemSpeechAdapter : IAsyncDisposable
{
    /// <summary>沿用参考 Web Speech 行为的启动确认预算。</summary>
    public static TimeSpan DefaultStartupTimeout { get; } = TimeSpan.FromMilliseconds(1_500);

    private static readonly TimeSpan DisposeTimeout = TimeSpan.FromSeconds(2);
    private readonly object _gate = new();
    private readonly IWindowsSpeechBridge _bridge;
    private readonly FixedSpeechStateMachine _stateMachine = new();
    private readonly AudioPriorityCoordinator _audioPriority;
    private readonly Func<FinalPcmBus?>? _finalPcmBusProvider;
    private readonly TimeSpan _startupTimeout;
    private ActiveSpeech? _active;
    private bool _disposed;
    private Task? _disposeTask;

    /// <summary>使用可注入桥接器创建适配器；生产环境传入 <see cref="WindowsSapiSpeechBridge"/>。</summary>
    public WindowsSystemSpeechAdapter(
        IWindowsSpeechBridge bridge,
        TimeSpan? startupTimeout = null,
        AudioPriorityCoordinator? audioPriority = null,
        Func<FinalPcmBus?>? finalPcmBusProvider = null)
    {
        _bridge = bridge ?? throw new ArgumentNullException(nameof(bridge));
        _audioPriority = audioPriority ?? new AudioPriorityCoordinator();
        _finalPcmBusProvider = finalPcmBusProvider;
        _startupTimeout = startupTimeout ?? DefaultStartupTimeout;
        if (_startupTimeout <= TimeSpan.Zero || _startupTimeout > TimeSpan.FromSeconds(30))
        {
            throw new ArgumentOutOfRangeException(nameof(startupTimeout), "启动确认超时必须在 1ms 到 30s 内。");
        }
    }

    /// <summary>读取固定话术对普通声音的脱敏静音策略。</summary>
    public AudioPrioritySnapshot AudioPrioritySnapshot => _audioPriority.Snapshot;

    /// <summary>不包含话术正文或 COM token 的当前快照。</summary>
    public WindowsSpeechAdapterSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshotUnsafe();
            }
        }
    }

    /// <summary>读取已脱敏的本地 voice 目录；当前机器未提供 SAPI 时失败关闭。</summary>
    public WindowsSpeechVoiceCatalogResult GetVoices()
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return WindowsSpeechVoiceCatalogResult.EmptyUnavailable(
                    Error(WindowsSpeechFailureCode.Closed, "Windows 本地语音适配器已关闭。"));
            }
        }

        try
        {
            var result = _bridge.GetVoices();
            return result.IsAvailable || result.Error is null
                ? result
                : WindowsSpeechVoiceCatalogResult.EmptyUnavailable(
                    SanitizeBridgeError(
                        result.Error,
                        WindowsSpeechFailureCode.SapiUnavailable,
                        "Windows 本地语音目录当前不可用。"));
        }
        catch (ObjectDisposedException)
        {
            return WindowsSpeechVoiceCatalogResult.EmptyUnavailable(
                Error(WindowsSpeechFailureCode.Closed, "Windows 本地语音桥接器已关闭。"));
        }
        catch (Exception)
        {
            return WindowsSpeechVoiceCatalogResult.EmptyUnavailable(
                Error(WindowsSpeechFailureCode.SapiUnavailable, "Windows 本地语音目录当前不可用。", retryable: true));
        }
    }

    /// <summary>
    /// 启动一条固定话术。成功返回只代表 SAPI 在启动预算内确认接受，
    /// 真实终态通过返回结果的 Completion 任务取得。
    /// </summary>
    public async Task<WindowsSpeechOperationResult> SpeakAsync(
        FixedSpeechCommandDto? command,
        string? voiceKey = null,
        bool microphonePriorityActive = false,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Rejected(
                command?.OperationId,
                Error(WindowsSpeechFailureCode.Cancelled, "Windows 本地语音启动已取消。", retryable: true));
        }

        if (command is null)
        {
            return Rejected(null, Error(WindowsSpeechFailureCode.InvalidCommand, "固定话术命令无效。"));
        }

        if (!FixedSpeechContractValidation.TryNormalizeText(command.Text, out var text, out _))
        {
            return Rejected(
                command.OperationId,
                Error(WindowsSpeechFailureCode.InvalidCommand, "固定话术文本无效。"));
        }

        if (!IsVoiceKeyValid(voiceKey))
        {
            return Rejected(
                command.OperationId,
                Error(WindowsSpeechFailureCode.InvalidCommand, "Windows 本地 voice 标识无效。"));
        }

        FinalPcmBus? finalPcmBus;
        try
        {
            finalPcmBus = _finalPcmBusProvider?.Invoke();
        }
        catch (Exception)
        {
            return Rejected(
                command.OperationId,
                Error(WindowsSpeechFailureCode.FinalPcmBusUnavailable, "最终 PCM 总线当前不可用。", retryable: true));
        }

        if (_finalPcmBusProvider is not null
            && (finalPcmBus is null || finalPcmBus.Snapshot.IsClosed))
        {
            return Rejected(
                command.OperationId,
                Error(WindowsSpeechFailureCode.FinalPcmBusUnavailable, "请先启动本地声音播放，再使用固定话术。", retryable: true));
        }

        ActiveSpeech? superseded;
        ActiveSpeech activeSpeech;
        FixedSpeechTransition transition;
        lock (_gate)
        {
            if (_disposed)
            {
                return Rejected(command.OperationId, Error(WindowsSpeechFailureCode.Closed, "Windows 本地语音适配器已关闭。"));
            }

            var priority = _audioPriority.BeginFixedSpeech(microphonePriorityActive);
            if (!priority.IsAccepted)
            {
                return Rejected(
                    command.OperationId,
                    Error(WindowsSpeechFailureCode.MicrophonePriority, priority.Error ?? "麦克风正在插话，固定话术未启动。"));
            }

            transition = _stateMachine.BeginSpeak(command, microphonePriorityActive);
            if (!transition.IsAccepted)
            {
                if (_active is null)
                {
                    _audioPriority.End(AudioPriorityLayer.FixedSpeech);
                }
                var code = microphonePriorityActive
                    && transition.Kind == FixedSpeechTransitionKind.Cancelled
                    ? WindowsSpeechFailureCode.MicrophonePriority
                    : WindowsSpeechFailureCode.InvalidCommand;
                return Rejected(command.OperationId, Error(code, code == WindowsSpeechFailureCode.MicrophonePriority
                    ? "麦克风正在插话，固定话术未启动。"
                    : transition.Error?.Message ?? "固定话术命令被拒绝。"));
            }

            superseded = _active;
            activeSpeech = new ActiveSpeech(
                command.OperationId,
                voiceKey,
                finalPcmBus,
                new(TaskCreationOptions.RunContinuationsAsynchronously));
            _active = activeSpeech;
        }

        finalPcmBus?.DiscardOverlayPending();

        if (superseded is not null)
        {
            await CancelAndDisposeAsync(superseded).ConfigureAwait(false);
            CompleteSuperseded(superseded);
        }

        IWindowsSpeechOperation? operation = null;
        try
        {
            var pcmSink = finalPcmBus is null
                ? null
                : new Func<ReadOnlyMemory<byte>, bool>(pcm => TryPublishPcm16(finalPcmBus, pcm));
            operation = await _bridge.StartAsync(
                    text,
                    voiceKey,
                    finalPcmBus?.Channels ?? 0,
                    pcmSink,
                    cancellationToken)
                .ConfigureAwait(false);
            if (operation is null)
            {
                return await FailCurrentAsync(
                    command.OperationId,
                    Error(WindowsSpeechFailureCode.BridgeUnavailable, "Windows 本地语音桥接器未返回操作。", retryable: true)).ConfigureAwait(false);
            }

            var isCurrent = AttachOperation(command.OperationId, operation);
            if (!isCurrent)
            {
                await CancelAndDisposeAsync(activeSpeech, operation).ConfigureAwait(false);
                return Rejected(command.OperationId, Error(WindowsSpeechFailureCode.Cancelled, "固定话术操作已被更新。", retryable: true));
            }

            WindowsSpeechBridgeStartResult started;
            try
            {
                started = await operation.Started
                    .WaitAsync(_startupTimeout, cancellationToken)
                    .ConfigureAwait(false);
            }
            catch (TimeoutException)
            {
                await CancelAndDisposeAsync(activeSpeech, operation).ConfigureAwait(false);
                return await FailCurrentAsync(
                    command.OperationId,
                    Error(WindowsSpeechFailureCode.StartupTimeout, "Windows 本地语音启动确认超时。", retryable: true)).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                await CancelAndDisposeAsync(activeSpeech, operation).ConfigureAwait(false);
                return await CancelCurrentAsync(command.OperationId).ConfigureAwait(false);
            }

            if (!started.IsSuccess)
            {
                await CancelAndDisposeAsync(activeSpeech, operation).ConfigureAwait(false);
                return await FailCurrentAsync(
                    command.OperationId,
                    SanitizeBridgeError(
                        started.Error,
                        WindowsSpeechFailureCode.SpeechFailed,
                        "Windows 本地语音启动失败。"))
                    .ConfigureAwait(false);
            }

            lock (_gate)
            {
                if (_disposed || _active?.OperationId != command.OperationId)
                {
                    // CancelAsync/DisposeAsync 可能在 Started 等待期间抢占了本操作。
                    isCurrent = false;
                }
                else
                {
                    var playing = _stateMachine.MarkPlaying(command.OperationId);
                    isCurrent = playing.IsAccepted;
                }
            }

            if (!isCurrent)
            {
                await CancelAndDisposeAsync(activeSpeech, operation).ConfigureAwait(false);
                return Rejected(command.OperationId, Error(WindowsSpeechFailureCode.Cancelled, "固定话术操作已被取消。", retryable: true));
            }

            ActiveSpeech current;
            lock (_gate)
            {
                current = _active!;
                current.MonitorTask = MonitorAsync(current);
                return new(
                    true,
                    command.OperationId,
                    CreateSnapshotUnsafe(),
                    Completion: current.Completion.Task);
            }
        }
        catch (OperationCanceledException)
        {
            if (operation is not null)
            {
                await CancelAndDisposeAsync(activeSpeech, operation).ConfigureAwait(false);
            }

            return await CancelCurrentAsync(command.OperationId).ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            if (operation is not null)
            {
                await CancelAndDisposeAsync(activeSpeech, operation).ConfigureAwait(false);
            }

            return await FailCurrentAsync(
                command.OperationId,
                Error(WindowsSpeechFailureCode.Closed, "Windows 本地语音桥接器已关闭。")).ConfigureAwait(false);
        }
        catch (Exception)
        {
            if (operation is not null)
            {
                await CancelAndDisposeAsync(activeSpeech, operation).ConfigureAwait(false);
            }

            return await FailCurrentAsync(
                command.OperationId,
                Error(WindowsSpeechFailureCode.BridgeUnavailable, "Windows 本地语音桥接器当前不可用。", retryable: true)).ConfigureAwait(false);
        }
    }

    /// <summary>取消当前指定操作；过期 ID 保持幂等且不污染新操作。</summary>
    public async Task<WindowsSpeechOperationResult> CancelAsync(
        string? operationId,
        CancellationToken cancellationToken = default)
    {
        if (!FixedSpeechContractValidation.TryValidateOperationId(operationId, out _))
        {
            return Rejected(operationId, Error(WindowsSpeechFailureCode.InvalidCommand, "固定话术操作 ID 无效。"));
        }

        ActiveSpeech? active;
        lock (_gate)
        {
            if (_disposed)
            {
                return Rejected(operationId, Error(WindowsSpeechFailureCode.Closed, "Windows 本地语音适配器已关闭。"));
            }

            if (_active is null || !string.Equals(_active.OperationId, operationId, StringComparison.Ordinal))
            {
                return new(false, operationId, CreateSnapshotUnsafe());
            }

            _stateMachine.Cancel(FixedSpeechCommandDto.Cancel(operationId!));
            active = _active;
        }

        if (active.Operation is not null)
        {
            try
            {
                await active.Operation.CancelAsync(cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                // 即使桥接器的取消等待被调用方取消，状态仍先收敛为 cancelled。
            }
            catch (Exception)
            {
                // 桥接器异常不应让 UI 取消流程失败；下面继续走有界释放。
            }

            if (active.MonitorTask is { } monitorTask)
            {
                try
                {
                    await monitorTask.WaitAsync(DisposeTimeout).ConfigureAwait(false);
                }
                catch (Exception)
                {
                    // 监视器未能在预算内收敛时，最后由适配器执行一次有界释放。
                    await DisposeOperationOnceAsync(active, active.Operation).ConfigureAwait(false);
                }
            }
            else
            {
                await DisposeOperationOnceAsync(active, active.Operation).ConfigureAwait(false);
            }
        }

        CompleteCurrent(active, WindowsSpeechAdapterState.Cancelled, null);
        lock (_gate)
        {
            return new(false, operationId, CreateSnapshotUnsafe());
        }
    }

    /// <summary>释放当前操作和桥接器；释放预算有界，避免关闭流程无限等待 COM。</summary>
    public ValueTask DisposeAsync()
    {
        lock (_gate)
        {
            _disposeTask ??= DisposeCoreAsync();
            return new(_disposeTask);
        }
    }

    private async Task DisposeCoreAsync()
    {
        ActiveSpeech? active;
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            active = _active;
            _active = null;
            _audioPriority.End(AudioPriorityLayer.FixedSpeech);
        }

        if (active is not null)
        {
            await CancelAndDisposeAsync(active).ConfigureAwait(false);
            active.DiscardPendingOverlay();
            active.Completion.TrySetResult(new(
                false,
                active.OperationId,
                new(WindowsSpeechAdapterState.Closed, active.OperationId, active.VoiceKey, null),
                Error(WindowsSpeechFailureCode.Closed, "Windows 本地语音适配器已关闭。")));
        }

        try
        {
            await _bridge.DisposeAsync().ConfigureAwait(false);
        }
        catch (Exception)
        {
            // 退出路径不传播外部 COM 异常；桥接器已进入关闭边界。
        }
    }

    private bool AttachOperation(string operationId, IWindowsSpeechOperation operation)
    {
        lock (_gate)
        {
            if (_disposed || _active?.OperationId != operationId)
            {
                return false;
            }

            _active.Operation = operation;
            return true;
        }
    }

    private async Task MonitorAsync(ActiveSpeech active)
    {
        WindowsSpeechBridgeCompletion completion;
        try
        {
            completion = await active.Operation!.Completion.ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            completion = new(WindowsSpeechBridgeCompletionKind.Cancelled);
        }
        catch (Exception)
        {
            completion = new(
                WindowsSpeechBridgeCompletionKind.Failed,
                Error(WindowsSpeechFailureCode.SpeechFailed, "Windows 本地语音播放失败。"));
        }

        try
        {
            WindowsSpeechAdapterState terminalState;
            WindowsSpeechError? error;
            lock (_gate)
            {
                if (_disposed || !ReferenceEquals(_active, active))
                {
                    return;
                }

                terminalState = completion.Kind switch
                {
                    WindowsSpeechBridgeCompletionKind.Completed => WindowsSpeechAdapterState.Completed,
                    WindowsSpeechBridgeCompletionKind.Cancelled => WindowsSpeechAdapterState.Cancelled,
                    _ => WindowsSpeechAdapterState.Failed
                };
                error = terminalState == WindowsSpeechAdapterState.Failed
                    ? SanitizeBridgeError(
                        completion.Error,
                        WindowsSpeechFailureCode.SpeechFailed,
                        "Windows 本地语音播放失败。")
                    : null;

                switch (terminalState)
                {
                    case WindowsSpeechAdapterState.Completed:
                        _stateMachine.Complete(active.OperationId);
                        break;
                    case WindowsSpeechAdapterState.Cancelled:
                        _stateMachine.Cancel(FixedSpeechCommandDto.Cancel(active.OperationId));
                        break;
                    default:
                        _stateMachine.Fail(active.OperationId, error?.Message);
                        break;
                }

                _active = null;
                _audioPriority.End(AudioPriorityLayer.FixedSpeech);
                active.DiscardPendingOverlay();
                var snapshot = CreateSnapshotUnsafe();
                active.Completion.TrySetResult(new(
                    terminalState is WindowsSpeechAdapterState.Completed,
                    active.OperationId,
                    snapshot,
                    error));
            }
        }
        finally
        {
            await DisposeOperationOnceAsync(active, active.Operation).ConfigureAwait(false);
        }
    }

    private Task<WindowsSpeechOperationResult> FailCurrentAsync(
        string operationId,
        WindowsSpeechError error)
    {
        ActiveSpeech? active;
        lock (_gate)
        {
            active = _active?.OperationId == operationId ? _active : null;
            if (active is not null)
            {
                _stateMachine.Fail(operationId, error.Message);
                _active = null;
                _audioPriority.End(AudioPriorityLayer.FixedSpeech);
                active.DiscardPendingOverlay();
                active.Completion.TrySetResult(new(false, operationId, CreateSnapshotUnsafe(), error));
            }

            return Task.FromResult(Rejected(operationId, error));
        }
    }

    private Task<WindowsSpeechOperationResult> CancelCurrentAsync(string operationId)
    {
        ActiveSpeech? active;
        lock (_gate)
        {
            active = _active?.OperationId == operationId ? _active : null;
            if (active is not null)
            {
                _stateMachine.Cancel(FixedSpeechCommandDto.Cancel(operationId));
                _active = null;
                _audioPriority.End(AudioPriorityLayer.FixedSpeech);
                active.DiscardPendingOverlay();
                active.Completion.TrySetResult(new(
                    false,
                    operationId,
                    CreateSnapshotUnsafe(),
                    Error(WindowsSpeechFailureCode.Cancelled, "Windows 本地语音已取消。", retryable: true)));
            }

            return Task.FromResult(Rejected(operationId, Error(WindowsSpeechFailureCode.Cancelled, "Windows 本地语音已取消。", retryable: true)));
        }
    }

    private void CompleteCurrent(
        ActiveSpeech active,
        WindowsSpeechAdapterState state,
        WindowsSpeechError? error)
    {
        lock (_gate)
        {
            if (!ReferenceEquals(_active, active))
            {
                return;
            }

            _active = null;
            _audioPriority.End(AudioPriorityLayer.FixedSpeech);
            active.DiscardPendingOverlay();
            active.Completion.TrySetResult(new(
                false,
                active.OperationId,
                new(state, active.OperationId, active.VoiceKey, error?.Message),
                error));
        }
    }

    private void CompleteSuperseded(ActiveSpeech active)
    {
        active.DiscardPendingOverlay();
        active.Completion.TrySetResult(new(
            false,
            active.OperationId,
            new(WindowsSpeechAdapterState.Cancelled, active.OperationId, active.VoiceKey, null),
            Error(WindowsSpeechFailureCode.Cancelled, "固定话术被新的操作抢占。", retryable: true)));
    }

    private Task CancelAndDisposeAsync(ActiveSpeech active) =>
        CancelAndDisposeAsync(active, active.Operation);

    private async Task CancelAndDisposeAsync(
        ActiveSpeech? active,
        IWindowsSpeechOperation? operation)
    {
        if (operation is null)
        {
            return;
        }

        try
        {
            using var cancellation = new CancellationTokenSource(DisposeTimeout);
            await operation.CancelAsync(cancellation.Token).ConfigureAwait(false);
        }
        catch (Exception)
        {
            // 随后的有界 Dispose 仍会收拢桥接器资源。
        }

        await DisposeOperationOnceAsync(active, operation).ConfigureAwait(false);
    }

    private static async Task DisposeOperationOnceAsync(
        ActiveSpeech? active,
        IWindowsSpeechOperation? operation)
    {
        if (operation is null || (active is not null && !active.TryClaimOperationDispose()))
        {
            return;
        }

        await DisposeOperationAsync(operation).ConfigureAwait(false);
    }

    private static async Task DisposeOperationAsync(IWindowsSpeechOperation? operation)
    {
        if (operation is null)
        {
            return;
        }

        try
        {
            await operation.DisposeAsync().AsTask().WaitAsync(DisposeTimeout).ConfigureAwait(false);
        }
        catch (Exception)
        {
            // 适配器不能把 COM 异常或超时传播到 UI；生产桥接器必须自行满足有界释放契约。
        }
    }

    private static bool TryPublishPcm16(FinalPcmBus finalPcmBus, ReadOnlyMemory<byte> pcm)
    {
        if (finalPcmBus.Channels is < 1 or > 2)
        {
            return false;
        }

        var bytes = pcm.Span;
        var bytesPerFrame = checked(finalPcmBus.Channels * sizeof(short));
        if (bytes.Length == 0)
        {
            return true;
        }

        if (bytes.Length % bytesPerFrame != 0)
        {
            return false;
        }

        var samples = ArrayPool<float>.Shared.Rent(MaxFramesPerPcmChunk * finalPcmBus.Channels);
        try
        {
            var framesRemaining = bytes.Length / bytesPerFrame;
            var sourceOffset = 0;
            while (framesRemaining > 0)
            {
                var frames = Math.Min(framesRemaining, MaxFramesPerPcmChunk);
                var sampleCount = checked(frames * finalPcmBus.Channels);
                for (var sampleIndex = 0; sampleIndex < sampleCount; sampleIndex++)
                {
                    var sample = BinaryPrimitives.ReadInt16LittleEndian(
                        bytes.Slice(sourceOffset + sampleIndex * sizeof(short), sizeof(short)));
                    samples[sampleIndex] = sample / 32_768F;
                }

                if (!finalPcmBus.TryPublishOverlay(
                        samples.AsSpan(0, sampleCount),
                        out _,
                        out _))
                {
                    return false;
                }

                sourceOffset += checked(sampleCount * sizeof(short));
                framesRemaining -= frames;
            }

            return true;
        }
        finally
        {
            ArrayPool<float>.Shared.Return(samples, clearArray: false);
        }
    }

    private WindowsSpeechAdapterSnapshot CreateSnapshotUnsafe()
    {
        if (_disposed)
        {
            return new(WindowsSpeechAdapterState.Closed, null, null, null);
        }

        var snapshot = _stateMachine.Snapshot;
        return new(
            snapshot.State switch
            {
                FixedSpeechRuntimeState.Starting => WindowsSpeechAdapterState.Starting,
                FixedSpeechRuntimeState.Playing => WindowsSpeechAdapterState.Playing,
                FixedSpeechRuntimeState.Completed => WindowsSpeechAdapterState.Completed,
                FixedSpeechRuntimeState.Cancelled => WindowsSpeechAdapterState.Cancelled,
                FixedSpeechRuntimeState.Failed => WindowsSpeechAdapterState.Failed,
                _ => WindowsSpeechAdapterState.Idle
            },
            snapshot.OperationId,
            _active?.VoiceKey,
            snapshot.Error);
    }

    private WindowsSpeechOperationResult Rejected(string? operationId, WindowsSpeechError error)
    {
        lock (_gate)
        {
            return new(false, operationId, CreateSnapshotUnsafe(), error);
        }
    }

    private static bool IsVoiceKeyValid(string? voiceKey) =>
        voiceKey is null
        || (voiceKey.Length == 64
            && voiceKey.All(static character =>
                character is >= '0' and <= '9'
                or >= 'A' and <= 'F'
                or >= 'a' and <= 'f'));

    private static WindowsSpeechError Error(
        WindowsSpeechFailureCode code,
        string message,
        bool retryable = false) =>
        new(code, message, retryable);

    private static WindowsSpeechError SanitizeBridgeError(
        WindowsSpeechError? error,
        WindowsSpeechFailureCode fallbackCode,
        string fallbackMessage)
    {
        if (error is null)
        {
            return Error(fallbackCode, fallbackMessage);
        }

        var message = error.Code switch
        {
            WindowsSpeechFailureCode.Cancelled => "Windows 本地语音已取消。",
            WindowsSpeechFailureCode.VoiceUnavailable => "选择的 Windows 本地 voice 不可用。",
            WindowsSpeechFailureCode.SapiUnavailable => "Windows SAPI 当前不可用。",
            WindowsSpeechFailureCode.BridgeUnavailable => "Windows 本地语音桥接器当前不可用。",
            WindowsSpeechFailureCode.AlreadyActive => "Windows 本地语音已有活动操作。",
            WindowsSpeechFailureCode.Closed => "Windows 本地语音桥接器已关闭。",
            WindowsSpeechFailureCode.InvalidCommand => "Windows 本地语音命令无效。",
            WindowsSpeechFailureCode.StartupTimeout => "Windows 本地语音启动确认超时。",
            WindowsSpeechFailureCode.MicrophonePriority => "麦克风正在插话，固定话术未启动。",
            WindowsSpeechFailureCode.FinalPcmBusUnavailable => "最终 PCM 总线当前不可用。",
            WindowsSpeechFailureCode.SpeechFailed => "Windows 本地语音播放失败。",
            _ => fallbackMessage
        };
        return new(error.Code, message, error.Retryable);
    }

    private sealed class ActiveSpeech(
        string operationId,
        string? voiceKey,
        FinalPcmBus? finalPcmBus,
        TaskCompletionSource<WindowsSpeechTerminalResult> completion)
    {
        public string OperationId { get; } = operationId;
        public string? VoiceKey { get; } = voiceKey;
        public FinalPcmBus? FinalPcmBus { get; } = finalPcmBus;
        public TaskCompletionSource<WindowsSpeechTerminalResult> Completion { get; } = completion;
        public IWindowsSpeechOperation? Operation { get; set; }
        public Task? MonitorTask { get; set; }

        private int _operationDisposeClaimed;

        public bool TryClaimOperationDispose() =>
            Interlocked.Exchange(ref _operationDisposeClaimed, 1) == 0;

        public void DiscardPendingOverlay() => FinalPcmBus?.DiscardOverlayPending();
    }

    private const int MaxFramesPerPcmChunk = 4_096;
}
