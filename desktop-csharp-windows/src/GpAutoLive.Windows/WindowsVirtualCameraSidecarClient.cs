using System.Buffers;
using System.IO.Pipes;
using GpAutoLive.Contracts;

namespace GpAutoLive.Windows;

/// <summary>虚拟摄像头 sidecar 传输客户端状态。</summary>
public enum WindowsVirtualCameraSidecarClientState
{
    Stopped,
    Connecting,
    Connected,
    Stopping,
    Failed,
    Closed
}

/// <summary>sidecar 传输客户端的稳定错误码。</summary>
public enum WindowsVirtualCameraSidecarClientErrorCode
{
    NotWindows,
    InvalidPipeName,
    InvalidTimeout,
    Cancelled,
    AlreadyConnected,
    NotConnected,
    Closed,
    ConnectFailed,
    InvalidFrame,
    TimestampOverflow,
    WriteTimedOut,
    WriteFailed
}

/// <summary>sidecar 传输错误；不包含管道令牌、路径或异常详情。</summary>
public sealed record WindowsVirtualCameraSidecarClientError(
    WindowsVirtualCameraSidecarClientErrorCode Code,
    string Message,
    bool Retryable);

/// <summary>sidecar 传输客户端脱敏快照。</summary>
public sealed record WindowsVirtualCameraSidecarClientSnapshot(
    WindowsVirtualCameraSidecarClientState State,
    ulong FramesWritten,
    int? LastPayloadBytes,
    WindowsVirtualCameraSidecarClientErrorCode? LastErrorCode);

/// <summary>sidecar 传输操作结果。</summary>
public sealed record WindowsVirtualCameraSidecarClientResult(
    bool IsSuccess,
    WindowsVirtualCameraSidecarClientSnapshot Snapshot,
    WindowsVirtualCameraSidecarClientError? Error = null);

/// <summary>
/// 连接本机 AkVirtualCamera sidecar 并写入固定大小 YUY2 帧。
/// 该类不启动进程、不创建管道、不执行 WGC/D3D11 捕获；sidecar 生命周期和捕获链由上层所有者负责。
/// </summary>
public sealed class WindowsVirtualCameraSidecarClient : IAsyncDisposable
{
    private static readonly TimeSpan MaxConnectTimeout = TimeSpan.FromSeconds(30);
    private static readonly TimeSpan DefaultWriteTimeout = TimeSpan.FromMilliseconds(500);

    private readonly SemaphoreSlim _lifecycle = new(1, 1);
    private readonly object _snapshotGate = new();
    private readonly TimeSpan _writeTimeout;
    private NamedPipeClientStream? _pipe;
    private byte[]? _frameBuffer;
    private WindowsVirtualCameraSidecarClientState _state = WindowsVirtualCameraSidecarClientState.Stopped;
    private ulong _framesWritten;
    private int? _lastPayloadBytes;
    private WindowsVirtualCameraSidecarClientErrorCode? _lastErrorCode;
    private bool _disposed;

    /// <summary>使用默认 500ms 写入超时创建客户端。</summary>
    public WindowsVirtualCameraSidecarClient()
        : this(DefaultWriteTimeout)
    {
    }

    /// <summary>使用受控写入超时创建客户端。</summary>
    public WindowsVirtualCameraSidecarClient(TimeSpan writeTimeout)
    {
        if (writeTimeout <= TimeSpan.Zero || writeTimeout > MaxConnectTimeout)
        {
            throw new ArgumentOutOfRangeException(nameof(writeTimeout), "sidecar 写入超时必须大于 0 且不超过 30 秒");
        }

        _writeTimeout = writeTimeout;
    }

    /// <summary>读取当前脱敏快照。</summary>
    public WindowsVirtualCameraSidecarClientSnapshot Snapshot
    {
        get
        {
            lock (_snapshotGate)
            {
                return new(_state, _framesWritten, _lastPayloadBytes, _lastErrorCode);
            }
        }
    }

    /// <summary>连接已由上层创建的本机 Named Pipe。</summary>
    public async Task<WindowsVirtualCameraSidecarClientResult> ConnectAsync(
        string? pipeName,
        TimeSpan timeout,
        CancellationToken cancellationToken = default)
    {
        if (!OperatingSystem.IsWindows())
        {
            return Failure(WindowsVirtualCameraSidecarClientErrorCode.NotWindows, "虚拟摄像头 sidecar 仅支持 Windows", false);
        }

        if (!WindowsVirtualCameraSidecarProtocol.TryValidatePipeName(pipeName, out _))
        {
            return Failure(WindowsVirtualCameraSidecarClientErrorCode.InvalidPipeName, "sidecar Named Pipe 名称无效", false);
        }

        if (timeout <= TimeSpan.Zero || timeout > MaxConnectTimeout)
        {
            return Failure(WindowsVirtualCameraSidecarClientErrorCode.InvalidTimeout, "sidecar 连接超时必须大于 0 且不超过 30 秒", false);
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsVirtualCameraSidecarClientErrorCode.Cancelled, "sidecar 连接已取消", true);
        }

        try
        {
            if (_disposed)
            {
                return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.Closed, "sidecar 客户端已关闭", false);
            }

            if (_pipe is not null)
            {
                return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.AlreadyConnected, "sidecar 已连接", false);
            }

            SetStateNoLock(WindowsVirtualCameraSidecarClientState.Connecting, null);
            var suffix = pipeName![WindowsVirtualCameraSidecarProtocol.PipePrefix.Length..];
            var candidate = new NamedPipeClientStream(
                ".",
                suffix,
                PipeDirection.Out,
                PipeOptions.Asynchronous);

            try
            {
                try
                {
                    await candidate.ConnectAsync(ToTimeoutMilliseconds(timeout), cancellationToken).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                    SetStateNoLock(WindowsVirtualCameraSidecarClientState.Stopped, WindowsVirtualCameraSidecarClientErrorCode.Cancelled);
                    return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.Cancelled, "sidecar 连接已取消", true);
                }
                catch (TimeoutException)
                {
                    SetStateNoLock(WindowsVirtualCameraSidecarClientState.Failed, WindowsVirtualCameraSidecarClientErrorCode.ConnectFailed);
                    return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.ConnectFailed, "sidecar 连接超时", true);
                }
                catch (IOException)
                {
                    SetStateNoLock(WindowsVirtualCameraSidecarClientState.Failed, WindowsVirtualCameraSidecarClientErrorCode.ConnectFailed);
                    return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.ConnectFailed, "sidecar 本地管道连接失败", true);
                }
                catch (UnauthorizedAccessException)
                {
                    SetStateNoLock(WindowsVirtualCameraSidecarClientState.Failed, WindowsVirtualCameraSidecarClientErrorCode.ConnectFailed);
                    return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.ConnectFailed, "sidecar 本地管道权限被拒绝", false);
                }

                _pipe = candidate;
                _frameBuffer = ArrayPool<byte>.Shared.Rent(WindowsVirtualCameraSidecarProtocol.EncodedFrameBytes);
                SetStateNoLock(WindowsVirtualCameraSidecarClientState.Connected, null);
                return SuccessNoLock();
            }
            finally
            {
                if (!ReferenceEquals(_pipe, candidate))
                {
                    candidate.Dispose();
                }
            }
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>将一帧转换为固定协议并写入 sidecar；缓冲区在连接期间复用。</summary>
    public async Task<WindowsVirtualCameraSidecarClientResult> WriteFrameAsync(
        VirtualCameraFrame? frame,
        CancellationToken cancellationToken = default)
    {
        var validation = TryValidateFrame(frame, out var sidecarFrame, out var validationError);
        if (!validation)
        {
            return Failure(validationError!.Code, validationError.Message, false);
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsVirtualCameraSidecarClientErrorCode.Cancelled, "sidecar 帧写入已取消", true);
        }

        try
        {
            if (_disposed)
            {
                return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.Closed, "sidecar 客户端已关闭", false);
            }

            if (_pipe is null || _frameBuffer is null)
            {
                return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.NotConnected, "sidecar 尚未连接", true);
            }

            if (!WindowsVirtualCameraSidecarProtocol.TryEncode(
                    sidecarFrame,
                    _frameBuffer,
                    out var written,
                    out _))
            {
                return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.InvalidFrame, "sidecar 帧编码失败", false);
            }

            using var writeTimeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            writeTimeout.CancelAfter(_writeTimeout);
            try
            {
                await _pipe.WriteAsync(_frameBuffer.AsMemory(0, written), writeTimeout.Token).ConfigureAwait(false);
                await _pipe.FlushAsync(writeTimeout.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
                DisposePipeNoLock();
                SetStateNoLock(WindowsVirtualCameraSidecarClientState.Failed, WindowsVirtualCameraSidecarClientErrorCode.WriteTimedOut);
                return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.WriteTimedOut, "sidecar 帧写入超时", true);
            }
            catch (OperationCanceledException)
            {
                DisposePipeNoLock();
                SetStateNoLock(WindowsVirtualCameraSidecarClientState.Failed, WindowsVirtualCameraSidecarClientErrorCode.Cancelled);
                return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.Cancelled, "sidecar 帧写入已取消", true);
            }
            catch (IOException)
            {
                DisposePipeNoLock();
                SetStateNoLock(WindowsVirtualCameraSidecarClientState.Failed, WindowsVirtualCameraSidecarClientErrorCode.WriteFailed);
                return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.WriteFailed, "sidecar 帧写入失败", true);
            }

            _framesWritten = SaturatingIncrement(_framesWritten);
            _lastPayloadBytes = sidecarFrame!.Payload.Length;
            SetStateNoLock(WindowsVirtualCameraSidecarClientState.Connected, null);
            return SuccessNoLock();
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>停止并回收当前本地管道；可重复调用。</summary>
    public async Task<WindowsVirtualCameraSidecarClientResult> StopAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsVirtualCameraSidecarClientErrorCode.Cancelled, "sidecar 停止已取消", true);
        }

        try
        {
            if (_disposed)
            {
                return FailureNoLock(WindowsVirtualCameraSidecarClientErrorCode.Closed, "sidecar 客户端已关闭", false);
            }

            SetStateNoLock(WindowsVirtualCameraSidecarClientState.Stopping, null);
            DisposePipeNoLock();
            SetStateNoLock(WindowsVirtualCameraSidecarClientState.Stopped, null);
            return SuccessNoLock();
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <inheritdoc />
    public async ValueTask DisposeAsync()
    {
        await _lifecycle.WaitAsync().ConfigureAwait(false);
        try
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            SetStateNoLock(WindowsVirtualCameraSidecarClientState.Stopping, null);
            DisposePipeNoLock();
            SetStateNoLock(WindowsVirtualCameraSidecarClientState.Closed, null);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    private static bool TryValidateFrame(
        VirtualCameraFrame? frame,
        out WindowsVirtualCameraSidecarProtocol.SidecarFrame? sidecarFrame,
        out WindowsVirtualCameraSidecarClientError? error)
    {
        sidecarFrame = null;
        error = null;
        if (frame is null || frame.Payload is null || frame.Payload.Length != WindowsVirtualCameraSidecarProtocol.MaxPayloadBytes)
        {
            error = new(WindowsVirtualCameraSidecarClientErrorCode.InvalidFrame, "虚拟摄像头帧 payload 必须固定为 1280×720 YUY2", false);
            return false;
        }

        if (frame.Generation == 0 || frame.Sequence == 0)
        {
            error = new(WindowsVirtualCameraSidecarClientErrorCode.InvalidFrame, "虚拟摄像头帧 generation 和 sequence 不能为 0", false);
            return false;
        }

        if (!TryConvertTimestamp(frame.Timestamp90Khz, out var timestamp100Ns))
        {
            error = new(WindowsVirtualCameraSidecarClientErrorCode.TimestampOverflow, "虚拟摄像头帧时间戳超出 sidecar 100ns 范围", false);
            return false;
        }

        sidecarFrame = new(frame.Generation, frame.Sequence, timestamp100Ns, frame.Payload);
        return true;
    }

    private static bool TryConvertTimestamp(ulong timestamp90Khz, out long timestamp100Ns)
    {
        timestamp100Ns = 0;
        if (timestamp90Khz > ulong.MaxValue / 1_000UL)
        {
            return false;
        }

        var scaled = timestamp90Khz * 1_000UL / 9UL;
        if (scaled > long.MaxValue)
        {
            return false;
        }

        timestamp100Ns = (long)scaled;
        return true;
    }

    private void DisposePipeNoLock()
    {
        _pipe?.Dispose();
        _pipe = null;
        if (_frameBuffer is not null)
        {
            ArrayPool<byte>.Shared.Return(_frameBuffer);
            _frameBuffer = null;
        }
    }

    private WindowsVirtualCameraSidecarClientResult SuccessNoLock() =>
        new(true, Snapshot);

    private WindowsVirtualCameraSidecarClientResult Failure(
        WindowsVirtualCameraSidecarClientErrorCode code,
        string message,
        bool retryable) =>
        new(false, Snapshot, new(code, message, retryable));

    private WindowsVirtualCameraSidecarClientResult FailureNoLock(
        WindowsVirtualCameraSidecarClientErrorCode code,
        string message,
        bool retryable)
    {
        SetStateNoLock(_state, code);
        return new(false, Snapshot, new(code, message, retryable));
    }

    private void SetStateNoLock(
        WindowsVirtualCameraSidecarClientState state,
        WindowsVirtualCameraSidecarClientErrorCode? errorCode)
    {
        lock (_snapshotGate)
        {
            _state = state;
            _lastErrorCode = errorCode;
        }
    }

    private static int ToTimeoutMilliseconds(TimeSpan timeout) =>
        checked((int)Math.Clamp(Math.Ceiling(timeout.TotalMilliseconds), 1, int.MaxValue));

    private static ulong SaturatingIncrement(ulong value) =>
        value == ulong.MaxValue ? value : value + 1;
}
