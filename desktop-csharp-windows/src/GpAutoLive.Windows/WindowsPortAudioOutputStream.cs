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
    private readonly SemaphoreSlim _serial = new(1, 1);
    private readonly IAudioPcmOutputSource _source;
    private readonly WindowsPortAudioNative.StreamCallback _callback;
    private float[] _callbackBuffer = [];
    private WindowsPortAudioNative? _native;
    private nint _stream;
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
    {
        _source = source ?? throw new ArgumentNullException(nameof(source));
        _callback = OnAudioCallback;
    }

    /// <summary>读取脱敏流状态。</summary>
    public WindowsPortAudioOutputSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshot();
            }
        }
    }

    /// <summary>在后台线程加载 DLL、打开并启动一个输出流。</summary>
    public async Task<WindowsPortAudioOutputResult> StartAsync(
        string? dllPath,
        WindowsPortAudioOutputConfig config,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(WindowsPortAudioStreamFailureCode.Cancelled, "PortAudio 输出流启动已取消。", retryable: true);
        }

        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsPortAudioStreamFailureCode.Cancelled, "PortAudio 输出流启动已取消。", retryable: true);
        }

        try
        {
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.Closed, "PortAudio 输出流已关闭。", retryable: false);
                }

                if (_stream != 0)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.InvalidConfig, "PortAudio 输出流已经启动。", retryable: false);
                }
            }

            var result = await Task.Run(() => StartCore(dllPath, config), CancellationToken.None)
                .ConfigureAwait(false);
            if (cancellationToken.IsCancellationRequested && result.IsSuccess)
            {
                StopCore();
                return Failure(WindowsPortAudioStreamFailureCode.Cancelled, "PortAudio 输出流启动已取消。", retryable: true);
            }

            return result;
        }
        finally
        {
            _serial.Release();
        }
    }

    /// <summary>暂停回调消费但保留原生流和当前 PCM 环缓；重复暂停安全。</summary>
    public WindowsPortAudioOutputResult Pause()
    {
        if (!_serial.Wait(TimeSpan.FromSeconds(2)))
        {
            return Failure(WindowsPortAudioStreamFailureCode.PauseFailed, "PortAudio 输出流暂停超时。", retryable: true);
        }

        try
        {
            WindowsPortAudioNative? native;
            nint stream;
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.Closed, "PortAudio 输出流已关闭。", retryable: false);
                }

                native = _native;
                stream = _stream;
                if (native is null || stream == 0)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.InvalidConfig, "PortAudio 输出流尚未启动。", retryable: false);
                }

                if (_paused)
                {
                    return Success();
                }
            }

            var errorCode = native.StopStream(stream);
            if (errorCode != 0)
            {
                return Failure(WindowsPortAudioStreamFailureCode.PauseFailed, "PortAudio 输出流暂停失败。", retryable: true);
            }

            lock (_gate)
            {
                _paused = true;
            }

            return Success();
        }
        catch (Exception exception) when (exception is AccessViolationException or InvalidOperationException or SEHException)
        {
            return Failure(WindowsPortAudioStreamFailureCode.PauseFailed, "PortAudio 输出流暂停失败。", retryable: true);
        }
        finally
        {
            _serial.Release();
        }
    }

    /// <summary>恢复已暂停的原生流；重复恢复安全。</summary>
    public WindowsPortAudioOutputResult Resume()
    {
        if (!_serial.Wait(TimeSpan.FromSeconds(2)))
        {
            return Failure(WindowsPortAudioStreamFailureCode.ResumeFailed, "PortAudio 输出流恢复超时。", retryable: true);
        }

        try
        {
            WindowsPortAudioNative? native;
            nint stream;
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.Closed, "PortAudio 输出流已关闭。", retryable: false);
                }

                native = _native;
                stream = _stream;
                if (native is null || stream == 0)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.InvalidConfig, "PortAudio 输出流尚未启动。", retryable: false);
                }

                if (!_paused)
                {
                    return Success();
                }
            }

            var errorCode = native.StartStream(stream);
            if (errorCode != 0)
            {
                return Failure(WindowsPortAudioStreamFailureCode.ResumeFailed, "PortAudio 输出流恢复失败。", retryable: true);
            }

            lock (_gate)
            {
                _paused = false;
            }

            return Success();
        }
        catch (Exception exception) when (exception is AccessViolationException or InvalidOperationException or SEHException)
        {
            return Failure(WindowsPortAudioStreamFailureCode.ResumeFailed, "PortAudio 输出流恢复失败。", retryable: true);
        }
        finally
        {
            _serial.Release();
        }
    }

    /// <summary>有界停止并释放当前原生流；重复调用安全。</summary>
    public WindowsPortAudioOutputResult Stop()
    {
        if (!_serial.Wait(TimeSpan.FromSeconds(2)))
        {
            return Failure(WindowsPortAudioStreamFailureCode.StopFailed, "PortAudio 输出流停止超时。", retryable: true);
        }

        try
        {
            return StopCore();
        }
        finally
        {
            _serial.Release();
        }
    }

    /// <summary>
    /// 在保留 PCM 环缓的前提下重新打开同一输出配置。只允许一个恢复操作，
    /// 由调用方提供取消令牌和重试上限；失败时保持关闭并由上层 fail-closed。
    /// </summary>
    public async Task<WindowsPortAudioOutputResult> RestartAsync(
        string? dllPath,
        WindowsPortAudioOutputConfig config,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(WindowsPortAudioStreamFailureCode.Cancelled, "PortAudio 输出流恢复已取消。", retryable: true);
        }

        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsPortAudioStreamFailureCode.Cancelled, "PortAudio 输出流恢复已取消。", retryable: true);
        }

        try
        {
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.Closed, "PortAudio 输出流已关闭。", retryable: false);
                }

                if (_stream == 0 && !_restartEligible)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.InvalidConfig, "PortAudio 输出流尚未启动。", retryable: false);
                }

                _restartEligible = true;
            }

            // 设备丢失时原生流可能已经处于 stopped 状态，Pa_StopStream
            // 会返回 paStreamIsStopped；关闭句柄仍然必须继续，不能因此阻断重开。
            _ = StopCore(ignoreStopError: true, preserveRestartEligibility: true);

            var result = await Task.Run(() => StartCore(dllPath, config), CancellationToken.None)
                .ConfigureAwait(false);
            if (cancellationToken.IsCancellationRequested && result.IsSuccess)
            {
                StopCore();
                return Failure(WindowsPortAudioStreamFailureCode.Cancelled, "PortAudio 输出流恢复已取消。", retryable: true);
            }

            return result;
        }
        catch (Exception exception) when (exception is AccessViolationException
            or InvalidOperationException
            or MarshalDirectiveException
            or SEHException)
        {
            return Failure(WindowsPortAudioStreamFailureCode.RestartFailed, "PortAudio 输出流恢复发生原生错误。", retryable: true);
        }
        finally
        {
            _serial.Release();
        }
    }

    public void Dispose()
    {
        lock (_gate)
        {
            _disposed = true;
        }

        if (_serial.Wait(TimeSpan.FromSeconds(2)))
        {
            try
            {
                StopCore();
            }
            finally
            {
                _serial.Release();
            }
        }

        GC.SuppressFinalize(this);
    }

    private WindowsPortAudioOutputResult StartCore(
        string? dllPath,
        WindowsPortAudioOutputConfig config)
    {
        if (!OperatingSystem.IsWindows())
        {
            return Failure(WindowsPortAudioStreamFailureCode.NotWindows, "PortAudio 输出流只支持 Windows。", retryable: false);
        }

        if (!WindowsPortAudioDeviceEnumerator.TryValidatePath(dllPath, out var pathError))
        {
            return Failure(
                WindowsPortAudioStreamFailureCode.InvalidPath,
                pathError?.Message ?? "PortAudio 资源路径无效。",
                retryable: false);
        }

        if (!TryValidateConfig(config, out var configError))
        {
            return Failure(WindowsPortAudioStreamFailureCode.InvalidConfig, configError!, retryable: false);
        }

        if (!File.Exists(dllPath))
        {
            return Failure(WindowsPortAudioStreamFailureCode.ResourceMissing, "PortAudio 运行资源不存在。", retryable: true);
        }

        if (!WindowsPortAudioNative.TryLoad(dllPath!, out var native) || native is null)
        {
            return Failure(WindowsPortAudioStreamFailureCode.NativeLoadFailed, "PortAudio 运行资源无法加载。", retryable: true);
        }

        var initialized = false;
        var keepOpen = false;
        nint stream = 0;
        try
        {
            if (!native.TryInitialize(out _))
            {
                return Failure(WindowsPortAudioStreamFailureCode.InitializeFailed, "PortAudio 初始化失败。", retryable: true);
            }

            initialized = true;

            var callbackSamples = checked(config.FramesPerBuffer * config.Channels);
            var outputParameters = new WindowsPortAudioNative.StreamParameters
            {
                Device = config.DeviceIndex,
                ChannelCount = config.Channels,
                SampleFormat = WindowsPortAudioNative.Float32SampleFormat,
                SuggestedLatency = 0.05,
                HostApiSpecificStreamInfo = 0,
            };
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.Closed, "PortAudio 输出流已关闭。", retryable: false);
                }

                _callbackBuffer = new float[callbackSamples];
                _config = config;
            }

            var openError = native.OpenOutputStream(
                out stream,
                ref outputParameters,
                config.SampleRate,
                (uint)config.FramesPerBuffer,
                _callback);
            if (openError != 0 || stream == 0)
            {
                return Failure(WindowsPortAudioStreamFailureCode.OpenFailed, "PortAudio 输出设备无法打开。", retryable: true);
            }

            lock (_gate)
            {
                _native = native;
                _stream = stream;
                _paused = false;
            }

            var startError = native.StartStream(stream);
            if (startError != 0)
            {
                return Failure(WindowsPortAudioStreamFailureCode.StartFailed, "PortAudio 输出流无法启动。", retryable: true);
            }

            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioStreamFailureCode.Closed, "PortAudio 输出流已关闭。", retryable: false);
                }
            }

            lock (_gate)
            {
                _lastError = null;
            }

            keepOpen = true;
            return Success();
        }
        catch (Exception exception) when (exception is AccessViolationException
            or InvalidOperationException
            or MarshalDirectiveException
            or SEHException)
        {
            return Failure(WindowsPortAudioStreamFailureCode.CallbackFailed, "PortAudio 输出流发生原生错误。", retryable: true);
        }
        finally
        {
            if (!keepOpen)
            {
                if (stream != 0)
                {
                    try
                    {
                        native.CloseStream(stream);
                    }
                    catch (Exception exception) when (exception is AccessViolationException or InvalidOperationException or SEHException)
                    {
                        // 资源释放失败不能覆盖原始启动结果；进程退出时 Job Object 仍会兜底。
                    }
                }

                lock (_gate)
                {
                    _native = null;
                    _stream = 0;
                    _config = null;
                    _paused = false;
                    _callbackBuffer = [];
                }
            }

            if (initialized && !keepOpen)
            {
                try
                {
                    native.Terminate();
                }
                catch (Exception exception) when (exception is AccessViolationException or InvalidOperationException or SEHException)
                {
                    // 同上：不把原生清理异常回显给 UI。
                }
            }

            if (!keepOpen)
            {
                native.Dispose();
            }
        }
    }

    private WindowsPortAudioOutputResult StopCore(
        bool ignoreStopError = false,
        bool preserveRestartEligibility = false)
    {
        WindowsPortAudioNative? native;
        nint stream;
        lock (_gate)
        {
            native = _native;
            stream = _stream;
            _native = null;
            _stream = 0;
            _config = null;
            _paused = false;
            _callbackBuffer = [];
            if (!preserveRestartEligibility)
            {
                _restartEligible = false;
            }
        }

        if (native is null || stream == 0)
        {
            return Success();
        }

        var errorCode = 0;
        try
        {
            errorCode = native.StopStream(stream);
            var closeCode = native.CloseStream(stream);
            errorCode = errorCode != 0 ? errorCode : closeCode;
        }
        catch (Exception exception) when (exception is AccessViolationException or InvalidOperationException or SEHException)
        {
            errorCode = -1;
        }
        finally
        {
            try
            {
                native.Terminate();
            }
            finally
            {
                native.Dispose();
            }
        }

        return errorCode == 0 || ignoreStopError
            ? Success()
            : Failure(WindowsPortAudioStreamFailureCode.StopFailed, "PortAudio 输出流停止失败。", retryable: true);
    }

    private int OnAudioCallback(
        nint inputBuffer,
        nint outputBuffer,
        uint frameCount,
        nint timeInfo,
        uint statusFlags,
        nint userData)
    {
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

    private WindowsPortAudioOutputResult Success() => new(true, CreateSnapshot());

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
            IsRunning: _stream != 0 && !_paused,
            DeviceIndex: config?.DeviceIndex,
            Channels: config?.Channels ?? 0,
            SampleRate: config?.SampleRate ?? 0,
            FramesPerBuffer: config?.FramesPerBuffer ?? 0,
            UnderrunFrames: _underrunFrames,
            CallbackFailures: _callbackFailures,
            ErrorCode: _lastError?.Code.ToString(),
            Error: _lastError?.Message)
        {
            IsPaused = _stream != 0 && _paused,
            HardwareState = QueryHardwareState(),
            CallbackCount = Volatile.Read(ref _callbackCount),
            CallbackStatusFlagsCount = Volatile.Read(ref _callbackStatusFlagsCount),
            LastCallbackStatusFlags = Volatile.Read(ref _lastCallbackStatusFlags),
            XrunCount = Volatile.Read(ref _xrunCount),
            OutputFramesWritten = Volatile.Read(ref _outputFramesWritten),
            HasTimeInfo = Volatile.Read(ref _hasCallbackTimeInfo) != 0,
            OutputLatencyMicroseconds = Volatile.Read(ref _lastOutputLatencyMicroseconds),
        };
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
