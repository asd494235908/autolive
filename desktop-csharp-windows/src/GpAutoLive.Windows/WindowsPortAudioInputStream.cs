using GpAutoLive.Media;
using System.Runtime.InteropServices;

namespace GpAutoLive.Windows;

/// <summary>PortAudio 输入流的稳定失败分类。</summary>
public enum WindowsPortAudioInputFailureCode
{
    NotWindows,
    InvalidPath,
    ResourceMissing,
    NativeLoadFailed,
    InitializeFailed,
    InvalidConfig,
    OpenFailed,
    StartFailed,
    StopFailed,
    CallbackFailed,
    HardwareUnavailable,
    OutputBusUnavailable,
    Cancelled,
    Closed,
}

/// <summary>PortAudio 输入流配置；仅采集本地交错 float PCM，不做识别或上传。</summary>
public sealed record WindowsPortAudioInputConfig(
    int DeviceIndex,
    int Channels = 1,
    double SampleRate = 48_000,
    int FramesPerBuffer = 256);

/// <summary>不包含音频正文的输入流状态。</summary>
public sealed record WindowsPortAudioInputSnapshot(
    bool IsRunning,
    int? DeviceIndex,
    int Channels,
    double SampleRate,
    int FramesPerBuffer,
    ulong CapturedFrames,
    ulong DroppedFrames,
    ulong CallbackFailures,
    string? ErrorCode,
    string? Error)
{
    public WindowsPortAudioHardwareState HardwareState { get; init; }
    public ulong CallbackCount { get; init; }
    public uint LastCallbackStatusFlags { get; init; }
}

/// <summary>输入流启动/停止结果。</summary>
public sealed record WindowsPortAudioInputError(
    WindowsPortAudioInputFailureCode Code,
    string Message,
    bool Retryable = false);

public sealed record WindowsPortAudioInputResult(
    bool IsSuccess,
    WindowsPortAudioInputSnapshot Snapshot,
    WindowsPortAudioInputError? Error = null);

/// <summary>
/// 单一 PortAudio 输入流所有者。只将明确启用后的麦克风 PCM 写入固定容量环缓，
/// 不调用 ASR/LLM/TTS，不保存文件；环缓满载按实时策略丢旧帧。
/// </summary>
public sealed class WindowsPortAudioInputStream : IDisposable
{
    private const int MaxChannels = 2;
    private const int MinFramesPerBuffer = 16;
    private const int MaxFramesPerBuffer = 4_096;
    private readonly object _gate = new();
    private readonly SemaphoreSlim _serial = new(1, 1);
    private readonly AudioPcmRingBuffer _destination;
    private readonly MicrophoneInterludeGate? _interludeGate;
    private readonly WindowsPortAudioNative.StreamCallback _callback;
    private WindowsPortAudioInputConfig? _config;
    private float[] _callbackBuffer = [];
    private WindowsPortAudioNative? _native;
    private nint _stream;
    private bool _disposed;
    private ulong _capturedFrames;
    private ulong _droppedFrames;
    private ulong _callbackFailures;
    private ulong _callbackCount;
    private uint _lastCallbackStatusFlags;
    private WindowsPortAudioInputError? _lastError;

    public WindowsPortAudioInputStream(AudioPcmRingBuffer destination)
        : this(destination, gate: null)
    {
    }

    public WindowsPortAudioInputStream(
        AudioPcmRingBuffer destination,
        MicrophoneInterludeGate? gate)
    {
        _destination = destination ?? throw new ArgumentNullException(nameof(destination));
        _interludeGate = gate;
        _callback = OnAudioCallback;
    }

    public WindowsPortAudioInputSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshot();
            }
        }
    }

    /// <summary>后台打开并启动输入流；调用方必须显式传入资源和设备。</summary>
    public async Task<WindowsPortAudioInputResult> StartAsync(
        string? dllPath,
        WindowsPortAudioInputConfig config,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(WindowsPortAudioInputFailureCode.Cancelled, "PortAudio 输入流启动已取消。", true);
        }

        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsPortAudioInputFailureCode.Cancelled, "PortAudio 输入流启动已取消。", true);
        }

        try
        {
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioInputFailureCode.Closed, "PortAudio 输入流已关闭。", false);
                }

                if (_stream != 0)
                {
                    return Failure(WindowsPortAudioInputFailureCode.InvalidConfig, "PortAudio 输入流已经启动。", false);
                }
            }

            var result = await Task.Run(() => StartCore(dllPath, config), CancellationToken.None)
                .ConfigureAwait(false);
            if (cancellationToken.IsCancellationRequested && result.IsSuccess)
            {
                StopCore();
                return Failure(WindowsPortAudioInputFailureCode.Cancelled, "PortAudio 输入流启动已取消。", true);
            }

            return result;
        }
        finally
        {
            _serial.Release();
        }
    }

    /// <summary>有界停止输入流；重复调用安全。</summary>
    public WindowsPortAudioInputResult Stop()
    {
        if (!_serial.Wait(TimeSpan.FromSeconds(2)))
        {
            return Failure(WindowsPortAudioInputFailureCode.StopFailed, "PortAudio 输入流停止超时。", true);
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

    private WindowsPortAudioInputResult StartCore(
        string? dllPath,
        WindowsPortAudioInputConfig config)
    {
        if (!OperatingSystem.IsWindows())
        {
            return Failure(WindowsPortAudioInputFailureCode.NotWindows, "PortAudio 输入流只支持 Windows。", false);
        }

        if (!WindowsPortAudioDeviceEnumerator.TryValidatePath(dllPath, out var pathError))
        {
            return Failure(WindowsPortAudioInputFailureCode.InvalidPath, pathError?.Message ?? "PortAudio 资源路径无效。", false);
        }

        if (!TryValidateConfig(config, out var configError))
        {
            return Failure(WindowsPortAudioInputFailureCode.InvalidConfig, configError!, false);
        }

        if (_destination.Snapshot.Channels != config.Channels)
        {
            return Failure(WindowsPortAudioInputFailureCode.InvalidConfig, "PortAudio 输入环缓声道数与流配置不匹配。", false);
        }

        if (!File.Exists(dllPath))
        {
            return Failure(WindowsPortAudioInputFailureCode.ResourceMissing, "PortAudio 运行资源不存在。", true);
        }

        if (!WindowsPortAudioNative.TryLoad(dllPath!, out var native) || native is null)
        {
            return Failure(WindowsPortAudioInputFailureCode.NativeLoadFailed, "PortAudio 运行资源无法加载。", true);
        }

        var initialized = false;
        var keepOpen = false;
        nint stream = 0;
        try
        {
            if (!native.TryInitialize(out _))
            {
                return Failure(WindowsPortAudioInputFailureCode.InitializeFailed, "PortAudio 初始化失败。", true);
            }

            initialized = true;
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
                if (_disposed)
                {
                    return Failure(WindowsPortAudioInputFailureCode.Closed, "PortAudio 输入流已关闭。", false);
                }

                _config = config;
                _callbackBuffer = new float[checked(config.Channels * config.FramesPerBuffer)];
                if (_interludeGate is not null)
                {
                    _interludeGate.Arm();
                }
            }

            var openError = native.OpenInputStream(
                out stream,
                ref parameters,
                config.SampleRate,
                (uint)config.FramesPerBuffer,
                _callback);
            if (openError != 0 || stream == 0)
            {
                return Failure(WindowsPortAudioInputFailureCode.OpenFailed, "PortAudio 输入设备无法打开。", true);
            }

            lock (_gate)
            {
                _native = native;
                _stream = stream;
            }

            var startError = native.StartStream(stream);
            if (startError != 0)
            {
                return Failure(WindowsPortAudioInputFailureCode.StartFailed, "PortAudio 输入流无法启动。", true);
            }

            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioInputFailureCode.Closed, "PortAudio 输入流已关闭。", false);
                }

                _lastError = null;
            }

            keepOpen = true;
            return Success();
        }
        catch (Exception exception) when (exception is AccessViolationException
            or ArgumentException
            or InvalidOperationException
            or MarshalDirectiveException
            or OverflowException
            or SEHException)
        {
            return Failure(WindowsPortAudioInputFailureCode.CallbackFailed, "PortAudio 输入流发生原生错误。", true);
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
                    }
                }

                lock (_gate)
                {
                    _native = null;
                    _stream = 0;
                    _config = null;
                    _callbackBuffer = [];
                    _interludeGate?.Disable();
                }

                if (initialized)
                {
                    try
                    {
                        native.Terminate();
                    }
                    catch (Exception exception) when (exception is AccessViolationException or InvalidOperationException or SEHException)
                    {
                    }
                }

                native.Dispose();
            }
        }
    }

    private WindowsPortAudioInputResult StopCore()
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
            _callbackBuffer = [];
            _interludeGate?.Disable();
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

        return errorCode == 0
            ? Success()
            : Failure(WindowsPortAudioInputFailureCode.StopFailed, "PortAudio 输入流停止失败。", true);
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

        if (inputBuffer == 0 || frameCount == 0)
        {
            AddCounter(ref _callbackFailures, 1);
            return WindowsPortAudioNative.Abort;
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

            Marshal.Copy(inputBuffer, callbackBuffer, 0, sampleCount);
            _interludeGate?.Process(callbackBuffer.AsSpan(0, sampleCount), Environment.TickCount64);
            if (!_destination.TryWriteRealtime(
                    callbackBuffer.AsSpan(0, sampleCount),
                    out var framesWritten,
                    out var writeError))
            {
                if (writeError?.Code is PcmRingBufferFailureCode.ConsumerBusy)
                {
                    AddCounter(ref _droppedFrames, frameCount);
                    return WindowsPortAudioNative.Continue;
                }

                AddCounter(ref _callbackFailures, 1);
                return WindowsPortAudioNative.Abort;
            }

            AddCounter(ref _capturedFrames, (ulong)framesWritten);
            if (framesWritten < frameCount)
            {
                AddCounter(ref _droppedFrames, (ulong)(frameCount - (uint)framesWritten));
            }

            return WindowsPortAudioNative.Continue;
        }
        catch (Exception exception) when (exception is ArgumentException or OverflowException or InvalidOperationException)
        {
            AddCounter(ref _callbackFailures, 1);
            return WindowsPortAudioNative.Abort;
        }
    }

    private static bool TryValidateConfig(WindowsPortAudioInputConfig config, out string? error)
    {
        error = null;
        if (config.DeviceIndex < 0)
        {
            error = "PortAudio 输入设备索引无效。";
            return false;
        }

        if (config.Channels is < 1 or > MaxChannels)
        {
            error = "PortAudio 输入声道数必须在 1 到 2 之间。";
            return false;
        }

        if (!double.IsFinite(config.SampleRate)
            || config.SampleRate is not (16_000 or 32_000 or 44_100 or 48_000))
        {
            error = "PortAudio 输入采样率必须在 16000 到 48000 Hz 之间。";
            return false;
        }

        if (config.FramesPerBuffer is < MinFramesPerBuffer or > MaxFramesPerBuffer)
        {
            error = "PortAudio 输入回调帧数必须在 16 到 4096 之间。";
            return false;
        }

        return true;
    }

    private WindowsPortAudioInputResult Success() => new(true, CreateSnapshot());

    private WindowsPortAudioInputResult Failure(
        WindowsPortAudioInputFailureCode code,
        string message,
        bool retryable)
    {
        var error = new WindowsPortAudioInputError(code, message, retryable);
        lock (_gate)
        {
            _lastError = error;
            return new(false, CreateSnapshot(), error);
        }
    }

    private WindowsPortAudioInputSnapshot CreateSnapshot()
    {
        var config = _config;
        return new(
            _stream != 0,
            config?.DeviceIndex,
            config?.Channels ?? 0,
            config?.SampleRate ?? 0,
            config?.FramesPerBuffer ?? 0,
            _capturedFrames,
            _droppedFrames,
            _callbackFailures,
            _lastError?.Code.ToString(),
            _lastError?.Message)
        {
            HardwareState = QueryHardwareState(),
            CallbackCount = Volatile.Read(ref _callbackCount),
            LastCallbackStatusFlags = Volatile.Read(ref _lastCallbackStatusFlags),
        };
    }

    private WindowsPortAudioHardwareState QueryHardwareState()
    {
        if (_stream == 0 || _native is null)
        {
            return WindowsPortAudioHardwareState.NotCreated;
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
