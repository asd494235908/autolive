using GpAutoLive.Media;
using System.Runtime.InteropServices;

namespace GpAutoLive.Windows;

/// <summary>PortAudio 原生输出流的健康状态。</summary>
public enum WindowsPortAudioHardwareState
{
    NotCreated,
    Active,
    Stopped,
    Inactive,
    Unknown,
    QueryError,
}

/// <summary>PortAudio 输出流的稳定失败分类。</summary>
public enum WindowsPortAudioStreamFailureCode
{
    NotWindows,
    InvalidPath,
    ResourceMissing,
    NativeLoadFailed,
    InitializeFailed,
    InvalidConfig,
    OpenFailed,
    StartFailed,
    PauseFailed,
    ResumeFailed,
    StopFailed,
    RestartFailed,
    CallbackFailed,
    Cancelled,
    Closed,
}

/// <summary>PortAudio 输出流配置；仅允许固定的交错 float PCM 格式。</summary>
public sealed record WindowsPortAudioOutputConfig(
    int DeviceIndex,
    int Channels,
    double SampleRate,
    int FramesPerBuffer = 256);

/// <summary>不包含音频正文或 DLL 路径的输出流状态。</summary>
public sealed record WindowsPortAudioOutputSnapshot(
    bool IsRunning,
    int? DeviceIndex,
    int Channels,
    double SampleRate,
    int FramesPerBuffer,
    ulong UnderrunFrames,
    ulong CallbackFailures,
    string? ErrorCode,
    string? Error)
{
    public bool IsPaused { get; init; }
    public WindowsPortAudioHardwareState HardwareState { get; init; }
    public ulong CallbackCount { get; init; }
    public ulong CallbackStatusFlagsCount { get; init; }
    public uint LastCallbackStatusFlags { get; init; }
    public ulong XrunCount { get; init; }
    public ulong OutputFramesWritten { get; init; }
    public ulong? BaseFramesWritten { get; init; }
    public ulong MediaFramesWritten => BaseFramesWritten
        ?? (OutputFramesWritten > UnderrunFrames ? OutputFramesWritten - UnderrunFrames : 0);
    public bool HasTimeInfo { get; init; }
    public ulong OutputLatencyMicroseconds { get; init; }
}

/// <summary>输出流启动/停止结果。</summary>
public sealed record WindowsPortAudioStreamError(
    WindowsPortAudioStreamFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>输出流启动/停止结果。</summary>
public sealed record WindowsPortAudioOutputResult(
    bool IsSuccess,
    WindowsPortAudioOutputSnapshot Snapshot,
    WindowsPortAudioStreamError? Error = null);

/// <summary>
/// 单一 PortAudio 输出流所有者。流只消费固定容量 PCM 环缓；回调不分配内存、不做文件或网络 I/O，
/// 没有数据时输出静音并统计欠载帧，设备/原生错误会停止并 fail-closed。
/// </summary>
public sealed class WindowsPortAudioOutputStream : IDisposable
{
    private const int MaxChannels = 8;
    private const int MinFramesPerBuffer = 16;
    private const int MaxFramesPerBuffer = 4_096;
    private const int MaxSampleRate = 384_000;
    private const uint OutputUnderflowedStatusFlag = 0x04;
    private const uint OutputOverflowedStatusFlag = 0x08;
    private const uint OutputXrunStatusFlags = OutputUnderflowedStatusFlag | OutputOverflowedStatusFlag;
    private readonly object _gate = new();
    private readonly WindowsPortAudioOperationOwner<WindowsPortAudioOutputResult> _operations;
    private readonly Func<string, IWindowsPortAudioStreamNative?> _nativeLoader;
    private readonly IAudioPcmOutputSource _source;
    private readonly WindowsPortAudioNative.StreamCallback _callback;
    private float[] _callbackBuffer = [];
    private IWindowsPortAudioStreamNative? _native;
    private nint _stream;
    private bool _initialized;
    private bool _running;
    private bool _stopping;
    private GCHandle _callbackOwner;
    private WindowsPortAudioHardwareState _hardwareState;
    private long _lastHealthQueryAt;
    private WindowsPortAudioOutputConfig? _config;
    private bool _paused;
    private bool _disposed;
    // A failed restart closes the native handle before the next bounded
    // attempt; keep retry eligibility separate from the handle state.
    private bool _restartEligible;
    private ulong _underrunFrames;
    private ulong _callbackFailures;
    private ulong _callbackCount;
    private ulong _callbackStatusFlagsCount;
    private ulong _xrunCount;
    private ulong _outputFramesWritten;
    private ulong _lastOutputLatencyMicroseconds;
    private int _hasCallbackTimeInfo;
    private uint _lastCallbackStatusFlags;
    private WindowsPortAudioStreamError? _lastError;

    public WindowsPortAudioOutputStream(AudioPcmRingBuffer buffer)
        : this(new AudioPcmRingBufferOutputSource(buffer))
    {
    }

    /// <summary>创建使用自定义固定 PCM 输出源的 PortAudio 流。</summary>
    public WindowsPortAudioOutputStream(IAudioPcmOutputSource source)
        : this(source, WindowsPortAudioNative.LoadStream, TimeSpan.FromSeconds(2))
    {
    }

    internal WindowsPortAudioOutputStream(
        IAudioPcmOutputSource source,
        Func<string, IWindowsPortAudioStreamNative?> nativeLoader,
        TimeSpan operationBudget)
    {
        _source = source ?? throw new ArgumentNullException(nameof(source));
        _callback = OnAudioCallback;
        _nativeLoader = nativeLoader;
        _operations = new(() => StopCore(), operationBudget);
    }

    internal bool HasPendingCleanup
    {
        get
        {
            var busy = _operations.IsBusy;
            lock (_gate)
            {
                return busy || _native is not null;
            }
        }
    }

    /// <summary>读取脱敏流状态。</summary>
    public WindowsPortAudioOutputSnapshot Snapshot
    {
        get
        {
            RefreshHealthIfDue();
            lock (_gate)
            {
                return CreateSnapshot();
            }
        }
    }

    /// <summary>在后台线程加载 DLL、打开并启动一个输出流；超时后保留原生 owner。</summary>
    public Task<WindowsPortAudioOutputResult> StartAsync(
        string? dllPath,
        WindowsPortAudioOutputConfig config,
        CancellationToken cancellationToken = default) =>
        RunOperationAsync(() =>
        {
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.Closed, "PortAudio 输出流已关闭。", false);
                }

                if (_native is not null)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.InvalidConfig, "PortAudio 输出流已启动或尚未回收。", false);
                }
            }

            return StartCore(dllPath, config);
        }, WindowsPortAudioStreamFailureCode.StartFailed, cancellationToken);

    /// <summary>暂停回调消费但保留流和 PCM；原生调用仍受统一预算约束。</summary>
    public WindowsPortAudioOutputResult Pause() => PauseAsync().GetAwaiter().GetResult();

    public Task<WindowsPortAudioOutputResult> PauseAsync() => RunOperationAsync(() =>
    {
        IWindowsPortAudioStreamNative? native;
        nint stream;
        lock (_gate)
        {
            if (_disposed)
            {
                return Failure(WindowsPortAudioStreamFailureCode.Closed, "PortAudio 输出流已关闭。", false);
            }

            native = _native;
            stream = _stream;
            if (native is null || stream == 0 || _stopping)
            {
                return Failure(WindowsPortAudioStreamFailureCode.InvalidConfig, "PortAudio 输出流尚未启动或正在停止。", false);
            }

            if (_paused)
            {
                return Success();
            }
        }

        if (native.StopStream(stream) != 0)
        {
            return Failure(WindowsPortAudioStreamFailureCode.PauseFailed, "PortAudio 输出流暂停失败。", true);
        }

        lock (_gate)
        {
            _paused = true;
            _running = false;
            _hardwareState = WindowsPortAudioHardwareState.Stopped;
        }

        return Success();
    }, WindowsPortAudioStreamFailureCode.PauseFailed);

    /// <summary>恢复暂停流；超时不会并行关闭仍执行中的原生调用。</summary>
    public WindowsPortAudioOutputResult Resume() => ResumeAsync().GetAwaiter().GetResult();

    public Task<WindowsPortAudioOutputResult> ResumeAsync() => RunOperationAsync(() =>
    {
        IWindowsPortAudioStreamNative? native;
        nint stream;
        lock (_gate)
        {
            if (_disposed)
            {
                return Failure(WindowsPortAudioStreamFailureCode.Closed, "PortAudio 输出流已关闭。", false);
            }

            native = _native;
            stream = _stream;
            if (native is null || stream == 0 || _stopping)
            {
                return Failure(WindowsPortAudioStreamFailureCode.InvalidConfig, "PortAudio 输出流尚未启动或正在停止。", false);
            }

            if (!_paused)
            {
                return Success();
            }
        }

        if (native.StartStream(stream) != 0)
        {
            return Failure(WindowsPortAudioStreamFailureCode.ResumeFailed, "PortAudio 输出流恢复失败。", true);
        }

        lock (_gate)
        {
            _paused = false;
            _running = true;
            _hardwareState = WindowsPortAudioHardwareState.Active;
        }

        return Success();
    }, WindowsPortAudioStreamFailureCode.ResumeFailed);

    public WindowsPortAudioOutputResult Stop() => StopAsync().GetAwaiter().GetResult();

    public async Task<WindowsPortAudioOutputResult> StopAsync()
    {
        lock (_gate)
        {
            _stopping = true;
        }

        try
        {
            return await _operations.StopAsync().ConfigureAwait(false);
        }
        catch (TimeoutException)
        {
            return Failure(WindowsPortAudioStreamFailureCode.StopFailed, "PortAudio 输出停止超时，原生资源仍由原任务持有。", true);
        }
    }

    /// <summary>确认旧流关闭后才重开；不能吞掉 Close/Terminate 失败。</summary>
    public Task<WindowsPortAudioOutputResult> RestartAsync(
        string? dllPath,
        WindowsPortAudioOutputConfig config,
        CancellationToken cancellationToken = default) =>
        RunOperationAsync(() =>
        {
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.Closed, "PortAudio 输出流已关闭。", false);
                }

                if ((_stream == 0 && !_restartEligible) || (_stopping && _native is not null))
                {
                    return Failure(WindowsPortAudioStreamFailureCode.InvalidConfig, "PortAudio 输出流尚未启动或旧资源尚未回收。", false);
                }

                _restartEligible = true;
            }

            var stopped = StopCore(ignoreStopError: true, preserveRestartEligibility: true);
            if (!stopped.IsSuccess)
            {
                return stopped;
            }

            if (_operations.StopRequested)
            {
                return Failure(WindowsPortAudioStreamFailureCode.Cancelled, "PortAudio 输出恢复已取消。", true);
            }

            return StartCore(dllPath, config);
        }, WindowsPortAudioStreamFailureCode.RestartFailed, cancellationToken);

    public void Dispose()
    {
        lock (_gate)
        {
            _disposed = true;
        }

        var stopped = Stop();
        if (!stopped.IsSuccess || HasPendingCleanup)
        {
            if (_operations.TimedOut)
            {
                throw new TimeoutException("PortAudio 输出流尚未回收；原 owner 仍持有资源，可重试 Dispose。");
            }

            throw new InvalidOperationException("PortAudio 输出流释放未确认；保留原 owner 等待重试。");
        }
        GC.SuppressFinalize(this);
    }

    private async Task<WindowsPortAudioOutputResult> RunOperationAsync(
        Func<WindowsPortAudioOutputResult> operation,
        WindowsPortAudioStreamFailureCode failureCode,
        CancellationToken cancellationToken = default)
    {
        try
        {
            return await _operations.RunAsync(operation, cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return Failure(WindowsPortAudioStreamFailureCode.Cancelled, "PortAudio 输出操作已取消，等待原生资源回收。", true);
        }
        catch (TimeoutException)
        {
            return Failure(failureCode, "PortAudio 输出操作超时，原生资源仍由原任务持有。", true);
        }
        catch (Exception exception) when (exception is AccessViolationException or InvalidOperationException
            or MarshalDirectiveException or SEHException)
        {
            return Failure(failureCode, "PortAudio 输出原生操作失败或尚未回收，请停止后重试。", true);
        }
    }
    private WindowsPortAudioOutputResult StartCore(
        string? dllPath,
        WindowsPortAudioOutputConfig config)
    {
        if (!OperatingSystem.IsWindows())
        {
            return Failure(WindowsPortAudioStreamFailureCode.NotWindows, "PortAudio 输出流只支持 Windows。", false);
        }

        if (!WindowsPortAudioDeviceEnumerator.TryValidatePath(dllPath, out var pathError))
        {
            return Failure(WindowsPortAudioStreamFailureCode.InvalidPath, pathError?.Message ?? "PortAudio 资源路径无效。", false);
        }

        if (!TryValidateConfig(config, out var configError))
        {
            return Failure(WindowsPortAudioStreamFailureCode.InvalidConfig, configError!, false);
        }

        if (!File.Exists(dllPath))
        {
            return Failure(WindowsPortAudioStreamFailureCode.ResourceMissing, "PortAudio 运行资源不存在。", true);
        }

        var native = _nativeLoader(dllPath!);
        if (native is null)
        {
            return Failure(WindowsPortAudioStreamFailureCode.NativeLoadFailed, "PortAudio 运行资源无法加载。", true);
        }

        var keepOpen = false;
        lock (_gate)
        {
            _native = native;
            _stopping = false;
            _callbackOwner = GCHandle.Alloc(this);
        }

        try
        {
            if (!native.TryInitialize(out _))
            {
                return Failure(WindowsPortAudioStreamFailureCode.InitializeFailed, "PortAudio 初始化失败。", true);
            }

            _initialized = true;
            var parameters = new WindowsPortAudioNative.StreamParameters
            {
                Device = config.DeviceIndex,
                ChannelCount = config.Channels,
                SampleFormat = WindowsPortAudioNative.Float32SampleFormat,
                SuggestedLatency = 0.05,
                HostApiSpecificStreamInfo = 0,
            };
            lock (_gate)
            {
                if (_disposed || _operations.StopRequested)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.Cancelled, "PortAudio 输出流启动已取消。", true);
                }

                _config = config;
                _paused = false;
                _callbackBuffer = new float[checked(config.Channels * config.FramesPerBuffer)];
            }

            var openError = native.OpenOutputStream(out var stream, ref parameters,
                config.SampleRate, (uint)config.FramesPerBuffer, _callback);
            lock (_gate)
            {
                _stream = stream;
            }

            if (openError != 0 || stream == 0)
            {
                return Failure(WindowsPortAudioStreamFailureCode.OpenFailed, "PortAudio 输出设备无法打开。", true);
            }

            if (_operations.StopRequested)
            {
                return Failure(WindowsPortAudioStreamFailureCode.Cancelled, "PortAudio 输出流启动已取消。", true);
            }

            if (native.StartStream(stream) != 0)
            {
                return Failure(WindowsPortAudioStreamFailureCode.StartFailed, "PortAudio 输出流无法启动。", true);
            }

            RefreshHealthCore();
            lock (_gate)
            {
                if (_disposed || _operations.StopRequested)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.Cancelled, "PortAudio 输出流启动已取消。", true);
                }

                _lastError = null;
                _running = true;
            }

            keepOpen = true;
            return Success();
        }
        catch (Exception exception) when (exception is AccessViolationException
            or ArgumentException or InvalidOperationException or MarshalDirectiveException
            or OverflowException or SEHException)
        {
            return Failure(WindowsPortAudioStreamFailureCode.CallbackFailed, "PortAudio 输出流发生原生错误。", true);
        }
        finally
        {
            if (!keepOpen)
            {
                StopCore(preserveRestartEligibility: _restartEligible);
            }
        }
    }

    private WindowsPortAudioOutputResult StopCore(
        bool ignoreStopError = false,
        bool preserveRestartEligibility = false)
    {
        IWindowsPortAudioStreamNative? native;
        nint stream;
        lock (_gate)
        {
            _stopping = true;
            _running = false;
            native = _native;
            stream = _stream;
            if (!preserveRestartEligibility)
            {
                _restartEligible = false;
            }
        }

        if (native is null)
        {
            return Success();
        }

        try
        {
            var stopError = 0;
            if (stream != 0)
            {
                stopError = native.StopStream(stream);
                if (native.CloseStream(stream) != 0)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.StopFailed, "PortAudio 输出流关闭未确认，保留资源等待重试。", true);
                }

                lock (_gate)
                {
                    _stream = 0;
                    _config = null;
                    _paused = false;
                    _callbackBuffer = [];
                    _hardwareState = WindowsPortAudioHardwareState.NotCreated;
                    if (_callbackOwner.IsAllocated)
                    {
                        _callbackOwner.Free();
                    }
                }
            }

            if (_initialized)
            {
                if (native.Terminate() != 0)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.StopFailed, "PortAudio 输出终止未确认，保留运行库等待重试。", true);
                }

                _initialized = false;
            }

            native.Dispose();
            lock (_gate)
            {
                _native = null;
                _config = null;
                _paused = false;
                _callbackBuffer = [];
                _hardwareState = WindowsPortAudioHardwareState.NotCreated;
                if (_callbackOwner.IsAllocated)
                {
                    _callbackOwner.Free();
                }
            }

            return stopError is 0 or -9983 || ignoreStopError
                ? Success()
                : Failure(WindowsPortAudioStreamFailureCode.StopFailed, "PortAudio 输出流已关闭，但停止阶段返回错误。", true);
        }
        catch (Exception exception) when (exception is AccessViolationException or InvalidOperationException or SEHException)
        {
            return Failure(WindowsPortAudioStreamFailureCode.StopFailed, "PortAudio 输出原生释放失败，保留资源等待重试。", true);
        }
    }
    private int OnAudioCallback(
        nint inputBuffer,
        nint outputBuffer,
        uint frameCount,
        nint timeInfo,
        uint statusFlags,
        nint userData)
    {
        if (Volatile.Read(ref _stopping) || _operations.StopRequested)
        {
            return WindowsPortAudioNative.Abort;
        }

        AddCounter(ref _callbackCount, 1);
        Volatile.Write(ref _lastCallbackStatusFlags, statusFlags);
        if (statusFlags != 0)
        {
            AddCounter(ref _callbackStatusFlagsCount, 1);
        }

        var callbackReportedXrun = (statusFlags & OutputXrunStatusFlags) != 0;
        if (callbackReportedXrun)
        {
            AddCounter(ref _xrunCount, 1);
        }

        if (WindowsPortAudioNative.TryReadCallbackTimeInfo(
                timeInfo,
                out var currentTime,
                out var outputBufferDacTime)
            && outputBufferDacTime >= currentTime)
        {
            var latencySeconds = outputBufferDacTime - currentTime;
            if (double.IsFinite(latencySeconds)
                && latencySeconds >= 0
                && latencySeconds <= 10)
            {
                var latencyMicroseconds = (ulong)Math.Round(
                    latencySeconds * 1_000_000,
                    MidpointRounding.AwayFromZero);
                Volatile.Write(ref _lastOutputLatencyMicroseconds, latencyMicroseconds);
                Volatile.Write(ref _hasCallbackTimeInfo, 1);
            }
        }

        if (outputBuffer == 0 || frameCount == 0)
        {
            return WindowsPortAudioNative.Continue;
        }

        var config = Volatile.Read(ref _config);

        if (config is null || frameCount > MaxFramesPerBuffer)
        {
            AddCounter(ref _callbackFailures, 1);
            return WindowsPortAudioNative.Abort;
        }

        try
        {
            var sampleCount = checked((int)frameCount * config.Channels);
            var callbackBuffer = _callbackBuffer;
            if (sampleCount > callbackBuffer.Length)
            {
                AddCounter(ref _callbackFailures, 1);
                return WindowsPortAudioNative.Abort;
            }

            if (!_source.TryReadRealtime(callbackBuffer.AsSpan(0, sampleCount), out var framesRead, out _))
            {
                framesRead = 0;
            }

            if (framesRead < frameCount)
            {
                var missingFrames = (ulong)(frameCount - (uint)framesRead);
                AddCounter(ref _underrunFrames, missingFrames);
                if (!callbackReportedXrun)
                {
                    AddCounter(ref _xrunCount, 1);
                }
                Array.Clear(callbackBuffer, framesRead * config.Channels, (int)(frameCount - (uint)framesRead) * config.Channels);
            }

            System.Runtime.InteropServices.Marshal.Copy(callbackBuffer, 0, outputBuffer, sampleCount);
            AddCounter(ref _outputFramesWritten, frameCount);
            return WindowsPortAudioNative.Continue;
        }
        catch (Exception exception) when (exception is ArgumentException or OverflowException or InvalidOperationException)
        {
            AddCounter(ref _callbackFailures, 1);
            return WindowsPortAudioNative.Abort;
        }
    }

    private static bool TryValidateConfig(WindowsPortAudioOutputConfig config, out string? error)
    {
        error = null;
        if (config.DeviceIndex < 0)
        {
            error = "PortAudio 输出设备索引无效。";
            return false;
        }

        if (config.Channels is < 1 or > MaxChannels)
        {
            error = "PortAudio 输出声道数必须在 1 到 8 之间。";
            return false;
        }

        if (!double.IsFinite(config.SampleRate) || config.SampleRate is < 8_000 or > MaxSampleRate)
        {
            error = "PortAudio 采样率必须在 8000 到 384000 Hz 之间。";
            return false;
        }

        if (config.FramesPerBuffer is < MinFramesPerBuffer or > MaxFramesPerBuffer)
        {
            error = "PortAudio 回调帧数必须在 16 到 4096 之间。";
            return false;
        }

        return true;
    }

    private WindowsPortAudioOutputResult Success()
    {
        lock (_gate)
        {
            return new(true, CreateSnapshot());
        }
    }

    private WindowsPortAudioOutputResult Failure(
        WindowsPortAudioStreamFailureCode code,
        string message,
        bool retryable)
    {
        var error = new WindowsPortAudioStreamError(code, message, retryable);
        lock (_gate)
        {
            _lastError = error;
            return new(false, CreateSnapshot(), error);
        }
    }

    private WindowsPortAudioOutputSnapshot CreateSnapshot()
    {
        var config = _config;
        return new(
            IsRunning: _running && !_paused && !_stopping && !_operations.StopRequested,
            DeviceIndex: config?.DeviceIndex,
            Channels: config?.Channels ?? 0,
            SampleRate: config?.SampleRate ?? 0,
            FramesPerBuffer: config?.FramesPerBuffer ?? 0,
            UnderrunFrames: _underrunFrames,
            CallbackFailures: _callbackFailures,
            ErrorCode: _lastError?.Code.ToString(),
            Error: _lastError?.Message)
        {
            IsPaused = _stream != 0 && _paused && !_stopping && !_operations.StopRequested,
            HardwareState = (_stopping || _operations.StopRequested) && _stream != 0
                ? WindowsPortAudioHardwareState.QueryError
                : _hardwareState,
            CallbackCount = Volatile.Read(ref _callbackCount),
            CallbackStatusFlagsCount = Volatile.Read(ref _callbackStatusFlagsCount),
            LastCallbackStatusFlags = Volatile.Read(ref _lastCallbackStatusFlags),
            XrunCount = Volatile.Read(ref _xrunCount),
            OutputFramesWritten = Volatile.Read(ref _outputFramesWritten),
            BaseFramesWritten = (_source as AudioPcmMixingOutputSource)?.BaseFramesRead,
            HasTimeInfo = Volatile.Read(ref _hasCallbackTimeInfo) != 0,
            OutputLatencyMicroseconds = Volatile.Read(ref _lastOutputLatencyMicroseconds),
        };
    }

    private void RefreshHealthIfDue()
    {
        if (_operations.CheckHealthTimeout())
        {
            return;
        }

        lock (_gate)
        {
            if (_native is null || _stream == 0 || _disposed || _stopping || _paused
                || Environment.TickCount64 - _lastHealthQueryAt < 250)
            {
                return;
            }
        }

        _operations.TryRefresh(RefreshHealthCore);
    }

    private WindowsPortAudioOutputResult RefreshHealthCore()
    {
        var state = QueryHardwareState();
        lock (_gate)
        {
            _lastHealthQueryAt = Environment.TickCount64;
            if (!_stopping && !_operations.StopRequested)
            {
                _hardwareState = state;
            }

            return Success();
        }
    }

    private WindowsPortAudioHardwareState QueryHardwareState()
    {
        if (_stream == 0 || _native is null)
        {
            return WindowsPortAudioHardwareState.NotCreated;
        }

        if (_paused)
        {
            return WindowsPortAudioHardwareState.Stopped;
        }

        try
        {
            if (!_native.TryQueryStreamState(_stream, out var active, out var stopped))
            {
                return WindowsPortAudioHardwareState.Unknown;
            }

            if (active < 0 || stopped < 0)
            {
                return WindowsPortAudioHardwareState.QueryError;
            }

            return active > 0
                ? WindowsPortAudioHardwareState.Active
                : stopped > 0
                    ? WindowsPortAudioHardwareState.Stopped
                    : WindowsPortAudioHardwareState.Inactive;
        }
        catch (Exception exception) when (exception is AccessViolationException or InvalidOperationException or SEHException)
        {
            return WindowsPortAudioHardwareState.QueryError;
        }
    }

    private static void AddCounter(ref ulong target, ulong value)
    {
        while (true)
        {
            var current = Volatile.Read(ref target);
            var next = ulong.MaxValue - current < value ? ulong.MaxValue : current + value;
            if (Interlocked.CompareExchange(ref target, next, current) == current)
            {
                return;
            }
        }
    }
}
