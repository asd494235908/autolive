using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>本地麦克风门控会话的有限状态。</summary>
public enum WindowsMicrophoneInterludeState
{
    Idle,
    Starting,
    Listening,
    Stopping,
    Failed,
    Closed,
}

/// <summary>不包含麦克风 PCM 正文的门控会话快照。</summary>
public sealed record WindowsMicrophoneInterludeSnapshot(
    WindowsMicrophoneInterludeState State,
    WindowsPortAudioInputSnapshot Input,
    MicrophoneInterludeGateSnapshot Gate,
    AudioPrioritySnapshot Priority,
    string? ErrorCode,
    string? Error);

/// <summary>本地麦克风门控会话操作结果。</summary>
public sealed record WindowsMicrophoneInterludeError(
    WindowsPortAudioInputFailureCode Code,
    string Message,
    bool Retryable = false);

public sealed record WindowsMicrophoneInterludeResult(
    bool IsSuccess,
    WindowsMicrophoneInterludeSnapshot Snapshot,
    WindowsMicrophoneInterludeError? Error = null);

/// <summary>
/// 组合 PortAudio 输入、能量门控和音频优先级。此类只负责本地门控与抢占通知，
/// 不做 AEC、降噪、AGC、识别、上传或变声；所有后台观察都有取消和有界 Join。
/// </summary>
public sealed class WindowsMicrophoneInterludeController : IAsyncDisposable
{
    private static readonly TimeSpan StopTimeout = TimeSpan.FromSeconds(2);
    private static readonly TimeSpan PollInterval = TimeSpan.FromMilliseconds(50);
    private readonly object _gate = new();
    private readonly SemaphoreSlim _lifecycle = new(1, 1);
    private readonly AudioPriorityCoordinator _audioPriority;
    private readonly MicrophoneInterludeGate _interludeGate;
    private readonly AudioPcmRingBuffer _inputBuffer;
    private readonly WindowsPortAudioInputStream _input;
    private readonly WindowsMicrophonePcmBridge _pcmBridge;
    private readonly Func<FinalPcmBus?>? _finalPcmBusProvider;
    private readonly CancellationTokenSource _disposeCancellation = new();
    private WindowsMicrophoneInterludeState _state = WindowsMicrophoneInterludeState.Idle;
    private WindowsPortAudioInputError? _lastError;
    private CancellationTokenSource? _sessionCancellation;
    private Task? _monitorTask;
    private Task<WindowsPortAudioInputResult>? _inputStopTask;
    private bool _disposed;
    private bool _disposeCompleted;
    private bool _stopRequested;
    private bool _prioritySpeaking;

    public WindowsMicrophoneInterludeController(
        AudioPriorityCoordinator audioPriority,
        int channels = 1,
        int capacityFrames = 48_000,
        float startThresholdDb = -42,
        float stopThresholdDb = -48,
        long hangoverMs = 250,
        Func<FinalPcmBus?>? finalPcmBusProvider = null)
    {
        _audioPriority = audioPriority ?? throw new ArgumentNullException(nameof(audioPriority));
        _interludeGate = new(channels, startThresholdDb, stopThresholdDb, hangoverMs);
        _inputBuffer = new(capacityFrames, channels);
        _input = new(_inputBuffer, _interludeGate);
        _pcmBridge = new(channels);
        _finalPcmBusProvider = finalPcmBusProvider;
    }

    public WindowsMicrophoneInterludeSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshotUnsafe();
            }
        }
    }

    /// <summary>门控快照变化通知；回调不携带 PCM 正文。</summary>
    public event Action<WindowsMicrophoneInterludeSnapshot>? SnapshotChanged;

    public async Task<WindowsMicrophoneInterludeResult> StartAsync(
        string? portAudioDllPath,
        WindowsPortAudioInputConfig config,
        CancellationToken cancellationToken = default)
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return Failure(WindowsPortAudioInputFailureCode.Closed, "麦克风门控已关闭。", false);
            }
        }

        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(
                WindowsPortAudioInputFailureCode.Cancelled,
                "麦克风门控启动已取消。",
                retryable: true);
        }

        try
        {
            if (!await _lifecycle.WaitAsync(StopTimeout, cancellationToken).ConfigureAwait(false))
            {
                return Failure(WindowsPortAudioInputFailureCode.StartFailed, "麦克风上一操作尚未回收。", true);
            }
        }
        catch (ObjectDisposedException)
        {
            return Failure(WindowsPortAudioInputFailureCode.Closed, "麦克风门控已关闭。", false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsPortAudioInputFailureCode.Cancelled, "麦克风门控启动已取消。", true);
        }

        try
        {
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioInputFailureCode.Closed, "麦克风门控已关闭。", false);
                }

                if (_monitorTask is not null || _inputStopTask is not null || _input.HasPendingCleanup
                    || _state is WindowsMicrophoneInterludeState.Starting or WindowsMicrophoneInterludeState.Listening)
                {
                    return Failure(WindowsPortAudioInputFailureCode.InvalidConfig, "麦克风门控已经启动。", false);
                }

                _state = WindowsMicrophoneInterludeState.Starting;
                _stopRequested = false;
                _lastError = null;
            }

            if (_finalPcmBusProvider is not null
                && !TryGetUsableFinalPcmBus(out var busError))
            {
                return Failure(
                    WindowsPortAudioInputFailureCode.OutputBusUnavailable,
                    busError ?? "麦克风最终 PCM 输出总线不可用。",
                    retryable: true);
            }

            var started = await _input
                .StartAsync(portAudioDllPath, config, cancellationToken)
                .ConfigureAwait(false);
            if (!started.IsSuccess)
            {
                lock (_gate)
                {
                    _lastError = started.Error;
                    _state = WindowsMicrophoneInterludeState.Failed;
                }

                PublishSnapshot();
                return new(false, Snapshot, ToError(started.Error));
            }

            bool stopRequested;
            lock (_gate)
            {
                stopRequested = _stopRequested || _disposed || cancellationToken.IsCancellationRequested;
            }

            if (stopRequested)
            {
                await _input.StopAsync().ConfigureAwait(false);
                return Failure(WindowsPortAudioInputFailureCode.Cancelled, "麦克风门控启动已取消。", true);
            }

            var sessionCancellation = CancellationTokenSource.CreateLinkedTokenSource(
                cancellationToken,
                _disposeCancellation.Token);
            lock (_gate)
            {
                if (_stopRequested || _disposed)
                {
                    sessionCancellation.Dispose();
                    return Failure(WindowsPortAudioInputFailureCode.Cancelled, "麦克风门控启动已取消。", true);
                }

                _sessionCancellation = sessionCancellation;
                _prioritySpeaking = false;
                _audioPriority.SetMicrophoneSpeaking(false);
                _state = WindowsMicrophoneInterludeState.Listening;
                _monitorTask = MonitorGateAsync(sessionCancellation.Token);
            }

            PublishSnapshot();
            return new(true, Snapshot);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>停止输入和观察；预算到期保留任务及取消源，后续停止可继续 Join。</summary>
    public async Task<WindowsMicrophoneInterludeResult> StopAsync()
    {
        var deadline = Environment.TickCount64 + (long)StopTimeout.TotalMilliseconds;
        lock (_gate)
        {
            if (_disposeCompleted)
            {
                return new(true, CreateSnapshotUnsafe());
            }

            _stopRequested = true;
            _sessionCancellation?.Cancel();
            _interludeGate.Disable();
            _audioPriority.SetMicrophoneSpeaking(false);
            _prioritySpeaking = false;
        }

        if (!await _lifecycle.WaitAsync(StopTimeout).ConfigureAwait(false))
        {
            return Failure(WindowsPortAudioInputFailureCode.StopFailed, "麦克风生命周期操作未能在停止预算内结束。", true);
        }

        try
        {
            Task? monitorTask;
            WindowsPortAudioInputError? monitorFailure = null;
            Task<WindowsPortAudioInputResult> inputStopTask;
            lock (_gate)
            {
                _state = WindowsMicrophoneInterludeState.Stopping;
                monitorTask = _monitorTask;
                // 重试只等待同一条停止任务；完成但失败时才允许再尝试关闭。
                if (_inputStopTask is null || _inputStopTask.IsCompleted)
                {
                    _inputStopTask = _input.StopAsync();
                }

                inputStopTask = _inputStopTask;
            }

            try
            {
                if (monitorTask is not null)
                {
                    try
                    {
                        await monitorTask.WaitAsync(RemainingStopBudget(deadline)).ConfigureAwait(false);
                    }
                    catch (OperationCanceledException)
                    {
                    }
                    catch (Exception) when (monitorTask.IsFaulted)
                    {
                        monitorFailure = new(WindowsPortAudioInputFailureCode.CallbackFailed,
                            "麦克风观察任务异常结束，正在回收输入资源。", Retryable: true);
                        lock (_gate)
                        {
                            _lastError = monitorFailure;
                        }
                    }
                }

                var stopped = await inputStopTask.WaitAsync(RemainingStopBudget(deadline)).ConfigureAwait(false);
                if (!stopped.IsSuccess || _input.HasPendingCleanup)
                {
                    return Failure(WindowsPortAudioInputFailureCode.StopFailed,
                        stopped.Error?.Message ?? "麦克风输入原生资源尚未回收。", true);
                }
            }
            catch (TimeoutException)
            {
                return Failure(WindowsPortAudioInputFailureCode.StopFailed,
                    "麦克风停止预算已到，保留观察任务、取消源和输入 owner 等待回收。", true);
            }

            lock (_gate)
            {
                _sessionCancellation?.Dispose();
                _sessionCancellation = null;
                _monitorTask = null;
                _inputStopTask = null;
                if (_lastError?.Code is WindowsPortAudioInputFailureCode.StopFailed)
                {
                    _lastError = null;
                }

                _state = _disposed
                    ? WindowsMicrophoneInterludeState.Closed
                    : _lastError is null
                        ? WindowsMicrophoneInterludeState.Idle
                        : WindowsMicrophoneInterludeState.Failed;
            }

            PublishSnapshot();
            return monitorFailure is null
                ? new(true, Snapshot)
                : new(false, Snapshot, ToError(monitorFailure));
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    public async ValueTask DisposeAsync()
    {
        lock (_gate)
        {
            if (_disposeCompleted)
            {
                return;
            }

            _disposed = true;
            _disposeCancellation.Cancel();
        }

        var stopped = await StopAsync().ConfigureAwait(false);
        if (_input.HasPendingCleanup
            || (!stopped.IsSuccess && stopped.Error?.Code is not WindowsPortAudioInputFailureCode.CallbackFailed))
        {
            // 未 Join 的对象不能释放同步原语；再次 Dispose/Stop 可继续回收。
            throw new TimeoutException("麦克风会话释放未确认；保留任务和输入 owner，可重试 DisposeAsync。");
        }

        lock (_gate)
        {
            if (!_disposeCompleted)
            {
                _disposeCompleted = true;
                _disposeCancellation.Dispose();
            }
        }

        // SemaphoreSlim 未使用 WaitHandle，仅含受管状态；不与已排队的启动/停止者竞争 Dispose。
        GC.SuppressFinalize(this);
        if (!stopped.IsSuccess)
        {
            throw new InvalidOperationException("麦克风观察任务异常，输入资源已完成回收。");
        }
    }

    private static TimeSpan RemainingStopBudget(long deadline) =>
        TimeSpan.FromMilliseconds(Math.Max(0, deadline - Environment.TickCount64));
    private async Task MonitorGateAsync(CancellationToken cancellationToken)
    {
        try
        {
            while (!cancellationToken.IsCancellationRequested)
            {
                await Task.Delay(PollInterval, cancellationToken).ConfigureAwait(false);
                var input = _input.Snapshot;
                if (input.HardwareState is WindowsPortAudioHardwareState.Stopped
                    or WindowsPortAudioHardwareState.Inactive
                    or WindowsPortAudioHardwareState.QueryError)
                {
                    if (cancellationToken.IsCancellationRequested)
                    {
                        return;
                    }

                    var healthError = new WindowsPortAudioInputError(
                        WindowsPortAudioInputFailureCode.HardwareUnavailable,
                        "PortAudio 麦克风输入流已停止或健康状态不可用。",
                        Retryable: true);
                    lock (_gate)
                    {
                        _lastError = healthError;
                        _state = WindowsMicrophoneInterludeState.Failed;
                        _sessionCancellation?.Cancel();
                    }

                    _audioPriority.SetMicrophoneSpeaking(false);
                    Volatile.Write(ref _prioritySpeaking, false);
                    await _input.StopAsync().ConfigureAwait(false);
                    PublishSnapshot();
                    return;
                }

                var gate = _interludeGate.Snapshot;
                var speaking = gate.State is
                    MicrophoneInterludeGateState.Speaking or MicrophoneInterludeGateState.Hangover;

                if (_finalPcmBusProvider is not null
                    && !_pcmBridge.TryDrain(
                        _inputBuffer,
                        _finalPcmBusProvider(),
                        speaking,
                        out var outputError))
                {
                    if (cancellationToken.IsCancellationRequested)
                    {
                        return;
                    }

                    lock (_gate)
                    {
                        _lastError = outputError;
                        _state = WindowsMicrophoneInterludeState.Failed;
                        _sessionCancellation?.Cancel();
                    }

                    _audioPriority.SetMicrophoneSpeaking(false);
                    Volatile.Write(ref _prioritySpeaking, false);
                    await _input.StopAsync().ConfigureAwait(false);
                    PublishSnapshot();
                    return;
                }

                if (speaking == Volatile.Read(ref _prioritySpeaking))
                {
                    continue;
                }

                var decision = _audioPriority.SetMicrophoneSpeaking(speaking);
                Volatile.Write(ref _prioritySpeaking, speaking);
                if (decision.Kind is not AudioPriorityDecisionKind.Ignored)
                {
                    PublishSnapshot();
                }
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
    }

    private WindowsMicrophoneInterludeSnapshot CreateSnapshotUnsafe() =>
        new(
            _state,
            _input.Snapshot,
            _interludeGate.Snapshot,
            _audioPriority.Snapshot,
            _lastError?.Code.ToString(),
            _lastError?.Message);

    private void PublishSnapshot()
    {
        var handler = SnapshotChanged;
        if (handler is null)
        {
            return;
        }

        try
        {
            handler(Snapshot);
        }
        catch (Exception)
        {
            // 观察通知不能破坏 PortAudio 的停止或门控线程。
        }
    }

    private WindowsMicrophoneInterludeResult Failure(
        WindowsPortAudioInputFailureCode code,
        string message,
        bool retryable)
    {
        var error = new WindowsPortAudioInputError(code, message, retryable);
        lock (_gate)
        {
            _lastError = error;
            if (_state is not WindowsMicrophoneInterludeState.Closed)
            {
                _state = WindowsMicrophoneInterludeState.Failed;
            }

            return new(false, CreateSnapshotUnsafe(), ToError(error));
        }
    }

    private static WindowsMicrophoneInterludeError? ToError(WindowsPortAudioInputError? error) =>
        error is null
            ? null
            : new(error.Code, error.Message, error.Retryable);

    private bool TryGetUsableFinalPcmBus(out string? error)
    {
        var bus = _finalPcmBusProvider?.Invoke();
        if (bus is null || bus.Snapshot.IsClosed)
        {
            error = "请先播放带声音媒体，麦克风没有可用的最终 PCM 输出总线。";
            return false;
        }

        if (bus.Channels is not (1 or 2))
        {
            error = "当前音频输出只支持 1 或 2 声道麦克风插话。";
            return false;
        }

        error = null;
        return true;
    }
}
