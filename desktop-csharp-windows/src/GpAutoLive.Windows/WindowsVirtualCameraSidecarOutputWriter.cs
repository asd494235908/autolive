using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Windows;

/// <summary>虚拟摄像头 sidecar 固定帧输出泵的生命周期状态。</summary>
public enum WindowsVirtualCameraSidecarOutputWriterState
{
    Ready,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
    Closed
}

/// <summary>sidecar 固定帧输出泵的稳定错误分类。</summary>
public enum WindowsVirtualCameraSidecarOutputWriterErrorCode
{
    NotWindows,
    InvalidFrameRate,
    NotConnected,
    AlreadyRunning,
    WriteFailed,
    Cancelled,
    StopTimedOut,
    Closed
}

/// <summary>不包含管道名称、令牌或异常正文的输出泵错误。</summary>
public sealed record WindowsVirtualCameraSidecarOutputWriterError(
    WindowsVirtualCameraSidecarOutputWriterErrorCode Code,
    string Message,
    bool Retryable = false);

/// <summary>sidecar 固定帧输出泵脱敏快照。</summary>
public sealed record WindowsVirtualCameraSidecarOutputWriterSnapshot(
    WindowsVirtualCameraSidecarOutputWriterState State,
    ulong FramesAttempted,
    ulong FramesWritten,
    ulong BlackFramesWritten,
    ulong LatestFramesWritten,
    WindowsVirtualCameraSidecarOutputWriterErrorCode? LastErrorCode);

/// <summary>sidecar 固定帧输出泵启动/停止结果。</summary>
public sealed record WindowsVirtualCameraSidecarOutputWriterResult(
    bool IsSuccess,
    WindowsVirtualCameraSidecarOutputWriterSnapshot Snapshot,
    WindowsVirtualCameraSidecarOutputWriterError? Error = null);

/// <summary>
/// 以固定 30fps 将 <see cref="VirtualCameraOutputManager"/> 的 latest-wins 帧写入已经连接的
/// AkVirtualCamera sidecar。该类型不创建进程、管道或设备，也不拥有 GPU/WGC 生命周期。
/// </summary>
public sealed class WindowsVirtualCameraSidecarOutputWriter : IAsyncDisposable
{
    private static readonly TimeSpan DefaultFramePeriod = TimeSpan.FromTicks(TimeSpan.TicksPerSecond / VirtualCameraRules.Fps);
    private static readonly TimeSpan StopTimeout = TimeSpan.FromSeconds(2);

    private readonly object _gate = new();
    private readonly VirtualCameraOutputManager _output;
    private readonly WindowsVirtualCameraSidecarClient _client;
    private readonly TimeSpan _framePeriod;
    private readonly byte[] _blackPayload;
    private CancellationTokenSource? _cancellation;
    private Task? _worker;
    private VirtualCameraFrame? _latestFrame;
    private ulong _sequence;
    private ulong _framesAttempted;
    private ulong _framesWritten;
    private ulong _blackFramesWritten;
    private ulong _latestFramesWritten;
    private WindowsVirtualCameraSidecarOutputWriterErrorCode? _lastErrorCode;
    private WindowsVirtualCameraSidecarOutputWriterState _state = WindowsVirtualCameraSidecarOutputWriterState.Ready;
    private bool _disposed;

    /// <summary>使用固定 30fps 创建输出泵。</summary>
    public WindowsVirtualCameraSidecarOutputWriter(
        VirtualCameraOutputManager output,
        WindowsVirtualCameraSidecarClient client)
        : this(output, client, DefaultFramePeriod)
    {
    }

    /// <summary>使用受控帧周期创建输出泵；生产配置必须保持 30fps。</summary>
    public WindowsVirtualCameraSidecarOutputWriter(
        VirtualCameraOutputManager output,
        WindowsVirtualCameraSidecarClient client,
        TimeSpan framePeriod)
    {
        _output = output ?? throw new ArgumentNullException(nameof(output));
        _client = client ?? throw new ArgumentNullException(nameof(client));
        if (framePeriod <= TimeSpan.Zero)
        {
            throw new ArgumentOutOfRangeException(nameof(framePeriod), "虚拟摄像头输出周期必须大于 0");
        }

        _framePeriod = framePeriod;
        var config = output.Snapshot.Config;
        if (!config.TryGetFrameBytes(out _, out var frameBytes))
        {
            throw new ArgumentException("虚拟摄像头输出配置无效", nameof(output));
        }

        _blackPayload = new byte[frameBytes];
        for (var index = 0; index < _blackPayload.Length; index += 4)
        {
            _blackPayload[index] = 16;
            _blackPayload[index + 1] = 128;
            _blackPayload[index + 2] = 16;
            _blackPayload[index + 3] = 128;
        }
    }

    /// <summary>读取脱敏输出泵快照。</summary>
    public WindowsVirtualCameraSidecarOutputWriterSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshot();
            }
        }
    }

    /// <summary>以固定周期启动输出泵；首帧立即发送。</summary>
    public Task<WindowsVirtualCameraSidecarOutputWriterResult> StartAsync(
        CancellationToken cancellationToken = default)
    {
        if (!OperatingSystem.IsWindows())
        {
            return Task.FromResult(Failure(
                WindowsVirtualCameraSidecarOutputWriterErrorCode.NotWindows,
                "虚拟摄像头 sidecar 仅支持 Windows"));
        }

        var config = _output.Snapshot.Config;
        if (config.Fps != VirtualCameraRules.Fps
            || _framePeriod != DefaultFramePeriod)
        {
            return Task.FromResult(Failure(
                WindowsVirtualCameraSidecarOutputWriterErrorCode.InvalidFrameRate,
                "虚拟摄像头输出泵必须使用固定 30fps"));
        }

        if (cancellationToken.IsCancellationRequested)
        {
            return Task.FromResult(Failure(
                WindowsVirtualCameraSidecarOutputWriterErrorCode.Cancelled,
                "虚拟摄像头输出泵启动已取消",
                retryable: true));
        }

        if (_client.Snapshot.State is not WindowsVirtualCameraSidecarClientState.Connected)
        {
            return Task.FromResult(Failure(
                WindowsVirtualCameraSidecarOutputWriterErrorCode.NotConnected,
                "sidecar 管道尚未连接",
                retryable: true));
        }

        lock (_gate)
        {
            if (_disposed)
            {
                return Task.FromResult(FailureNoLock(
                    WindowsVirtualCameraSidecarOutputWriterErrorCode.Closed,
                    "虚拟摄像头输出泵已关闭"));
            }

            if (_worker is not null)
            {
                return Task.FromResult(FailureNoLock(
                    WindowsVirtualCameraSidecarOutputWriterErrorCode.AlreadyRunning,
                    "虚拟摄像头输出泵已经在运行"));
            }

            _cancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            _state = WindowsVirtualCameraSidecarOutputWriterState.Starting;
            _lastErrorCode = null;
            _worker = RunAsync(_cancellation);
            _state = WindowsVirtualCameraSidecarOutputWriterState.Running;
            return Task.FromResult(SucceededNoLock());
        }
    }

    /// <summary>停止输出泵并等待 worker 有界退出；不停止 sidecar 客户端。</summary>
    public async Task<WindowsVirtualCameraSidecarOutputWriterResult> StopAsync(
        CancellationToken cancellationToken = default)
    {
        Task? worker;
        CancellationTokenSource? cancellation;
        lock (_gate)
        {
            if (_disposed)
            {
                return FailureNoLock(
                    WindowsVirtualCameraSidecarOutputWriterErrorCode.Closed,
                    "虚拟摄像头输出泵已关闭");
            }

            worker = _worker;
            cancellation = _cancellation;
            if (worker is null)
            {
                _state = WindowsVirtualCameraSidecarOutputWriterState.Stopped;
                return SucceededNoLock();
            }

            _state = WindowsVirtualCameraSidecarOutputWriterState.Stopping;
        }

        cancellation?.Cancel();
        try
        {
            await worker.WaitAsync(StopTimeout, cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(
                WindowsVirtualCameraSidecarOutputWriterErrorCode.Cancelled,
                "虚拟摄像头输出泵停止已取消",
                retryable: true);
        }
        catch (TimeoutException)
        {
            lock (_gate)
            {
                _state = WindowsVirtualCameraSidecarOutputWriterState.Failed;
                _lastErrorCode = WindowsVirtualCameraSidecarOutputWriterErrorCode.StopTimedOut;
            }

            return Failure(
                WindowsVirtualCameraSidecarOutputWriterErrorCode.StopTimedOut,
                "虚拟摄像头输出泵未能在停止预算内退出");
        }

        lock (_gate)
        {
            if (ReferenceEquals(_worker, worker))
            {
                _worker = null;
                _cancellation?.Dispose();
                _cancellation = null;
                _latestFrame = null;
                _state = WindowsVirtualCameraSidecarOutputWriterState.Stopped;
            }

            return SucceededNoLock();
        }
    }

    /// <inheritdoc />
    public async ValueTask DisposeAsync()
    {
        Task? worker;
        CancellationTokenSource? cancellation;
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            worker = _worker;
            cancellation = _cancellation;
            _state = WindowsVirtualCameraSidecarOutputWriterState.Stopping;
        }

        cancellation?.Cancel();
        if (worker is not null)
        {
            try
            {
                await worker.WaitAsync(StopTimeout).ConfigureAwait(false);
            }
            catch (TimeoutException)
            {
                lock (_gate)
                {
                    _lastErrorCode = WindowsVirtualCameraSidecarOutputWriterErrorCode.StopTimedOut;
                }
            }
        }

        lock (_gate)
        {
            _worker = null;
            _cancellation?.Dispose();
            _cancellation = null;
            _latestFrame = null;
            _state = WindowsVirtualCameraSidecarOutputWriterState.Closed;
        }
    }

    private async Task RunAsync(CancellationTokenSource cancellation)
    {
        var token = cancellation.Token;
        try
        {
            await WriteOneAsync(token).ConfigureAwait(false);
            using var timer = new PeriodicTimer(_framePeriod);
            while (await timer.WaitForNextTickAsync(token).ConfigureAwait(false))
            {
                await WriteOneAsync(token).ConfigureAwait(false);
            }
        }
        catch (OperationCanceledException) when (token.IsCancellationRequested)
        {
        }
        catch (SidecarOutputWriteException)
        {
            // WriteOneAsync 已把稳定错误码写入快照；worker 以失败终态结束，
            // 由上层决定是否先停止客户端再重新建立输出链。
        }
        catch (Exception exception) when (exception is IOException or InvalidOperationException or ObjectDisposedException)
        {
            lock (_gate)
            {
                _state = WindowsVirtualCameraSidecarOutputWriterState.Failed;
                _lastErrorCode = WindowsVirtualCameraSidecarOutputWriterErrorCode.WriteFailed;
            }
        }
    }

    private async Task WriteOneAsync(CancellationToken cancellationToken)
    {
        WindowsVirtualCameraSidecarOutputWriterErrorCode? errorCode = null;
        VirtualCameraFrame frame;
        var policy = _output.GetOutputPolicy();
        var status = _output.Snapshot;
        lock (_gate)
        {
            if (_latestFrame is not null && _latestFrame.Generation != status.Generation)
            {
                _latestFrame = null;
            }
        }

        if (policy == VirtualCameraOutputPolicy.LatestFrame)
        {
            var candidate = _output.TakeLatestFrame();
            if (candidate is not null && candidate.Generation == status.Generation)
            {
                lock (_gate)
                {
                    _latestFrame = candidate;
                }
            }
        }

        VirtualCameraFrame? latest;
        lock (_gate)
        {
            latest = _latestFrame;
        }

        var useLatest = policy == VirtualCameraOutputPolicy.LatestFrame
            && latest is not null
            && latest.Generation == status.Generation;
        var sequence = NextSequence();
        frame = useLatest
            ? new VirtualCameraFrame(status.Generation, sequence, latest!.Timestamp90Khz, latest.Payload)
            : new VirtualCameraFrame(status.Generation, sequence, 0, _blackPayload);

        lock (_gate)
        {
            _framesAttempted = SaturatingIncrement(_framesAttempted);
        }

        var write = await _client.WriteFrameAsync(frame, cancellationToken).ConfigureAwait(false);
        if (!write.IsSuccess)
        {
            errorCode = MapError(write.Error?.Code);
            lock (_gate)
            {
                _state = WindowsVirtualCameraSidecarOutputWriterState.Failed;
                _lastErrorCode = errorCode;
            }

            throw new SidecarOutputWriteException();
        }

        lock (_gate)
        {
            _framesWritten = SaturatingIncrement(_framesWritten);
            if (useLatest)
            {
                _latestFramesWritten = SaturatingIncrement(_latestFramesWritten);
            }
            else
            {
                _blackFramesWritten = SaturatingIncrement(_blackFramesWritten);
            }
        }
    }

    private ulong NextSequence()
    {
        lock (_gate)
        {
            _sequence = _sequence == ulong.MaxValue ? 1 : _sequence + 1;
            return _sequence;
        }
    }

    private WindowsVirtualCameraSidecarOutputWriterSnapshot CreateSnapshot() => new(
        _state,
        _framesAttempted,
        _framesWritten,
        _blackFramesWritten,
        _latestFramesWritten,
        _lastErrorCode);

    private WindowsVirtualCameraSidecarOutputWriterResult SucceededNoLock() => new(true, CreateSnapshot());

    private WindowsVirtualCameraSidecarOutputWriterResult Failure(
        WindowsVirtualCameraSidecarOutputWriterErrorCode code,
        string message,
        bool retryable = false) => new(false, Snapshot, new(code, message, retryable));

    private WindowsVirtualCameraSidecarOutputWriterResult FailureNoLock(
        WindowsVirtualCameraSidecarOutputWriterErrorCode code,
        string message,
        bool retryable = false)
    {
        _state = code is WindowsVirtualCameraSidecarOutputWriterErrorCode.Cancelled
            ? WindowsVirtualCameraSidecarOutputWriterState.Stopped
            : WindowsVirtualCameraSidecarOutputWriterState.Failed;
        _lastErrorCode = code;
        return new(false, CreateSnapshot(), new(code, message, retryable));
    }

    private static WindowsVirtualCameraSidecarOutputWriterErrorCode MapError(
        WindowsVirtualCameraSidecarClientErrorCode? code) => code switch
        {
            WindowsVirtualCameraSidecarClientErrorCode.Cancelled => WindowsVirtualCameraSidecarOutputWriterErrorCode.Cancelled,
            WindowsVirtualCameraSidecarClientErrorCode.Closed => WindowsVirtualCameraSidecarOutputWriterErrorCode.Closed,
            WindowsVirtualCameraSidecarClientErrorCode.NotWindows => WindowsVirtualCameraSidecarOutputWriterErrorCode.NotWindows,
            _ => WindowsVirtualCameraSidecarOutputWriterErrorCode.WriteFailed
        };

    private static ulong SaturatingIncrement(ulong value) => value == ulong.MaxValue ? value : value + 1;

    private sealed class SidecarOutputWriteException : Exception;
}
