using System.Buffers;
using System.IO.Pipes;
using System.Text;

namespace GpAutoLive.Media;

/// <summary>mpv IPC 命名管道连接状态。</summary>
public enum MpvIpcPipeState
{
    Disconnected,
    Connecting,
    Connected,
    Faulted,
    Closed,
}

/// <summary>mpv IPC 命名管道连接结果。</summary>
public sealed record MpvIpcPipeConnectionResult(
    bool IsSuccess,
    MpvIpcPipeState State,
    MpvIpcError? Error)
{
    public static MpvIpcPipeConnectionResult Success() =>
        new(true, MpvIpcPipeState.Connected, null);

    public static MpvIpcPipeConnectionResult Failed(
        MpvIpcPipeState state,
        MpvIpcError error) =>
        new(false, state, error);
}

/// <summary>mpv IPC 命名管道客户端的有限配置。</summary>
public sealed record MpvIpcPipeOptions
{
    public static MpvIpcPipeOptions Default { get; } = new();

    public TimeSpan ConnectTimeout { get; init; } = TimeSpan.FromSeconds(5);

    public TimeSpan ResponseTimeout { get; init; } = TimeSpan.FromSeconds(5);

    public int MaxFrameBytes { get; init; } = MpvIpcCommand.MaxJsonLineBytes;

    internal void Validate()
    {
        if (ConnectTimeout <= TimeSpan.Zero || ConnectTimeout > TimeSpan.FromMinutes(1))
        {
            throw new ArgumentOutOfRangeException(
                nameof(ConnectTimeout),
                "mpv IPC 连接超时必须在 0 秒到 60 秒之间。");
        }

        if (ResponseTimeout <= TimeSpan.Zero || ResponseTimeout > TimeSpan.FromMinutes(1))
        {
            throw new ArgumentOutOfRangeException(
                nameof(ResponseTimeout),
                "mpv IPC 响应超时必须在 0 秒到 60 秒之间。");
        }

        if (MaxFrameBytes <= 0 || MaxFrameBytes > MpvIpcCommand.MaxJsonLineBytes)
        {
            throw new ArgumentOutOfRangeException(
                nameof(MaxFrameBytes),
                "mpv IPC 帧大小必须在 1 字节到 64 KiB 之间。");
        }
    }
}

/// <summary>
/// 单一所有者的 mpv Windows 命名管道 IPC 客户端。
///
/// 一个实例只维护一个连接，并以有界互斥逐条发送请求、等待对应响应；调用方
/// 不能注入任意命令字符串，命令和响应均通过现有固定工厂/解析器处理。
/// </summary>
public sealed class MpvNamedPipeClient : IAsyncDisposable
{
    private const int DisposeWaitMilliseconds = 2_000;
    private static readonly UTF8Encoding StrictUtf8 = new(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true);

    private readonly MpvIpcPipeEndpoint _endpoint;
    private readonly MpvIpcPipeOptions _options;
    private readonly object _gate = new();
    private readonly SemaphoreSlim _requestGate = new(1, 1);
    private readonly CancellationTokenSource _lifetimeCancellation = new();
    private NamedPipeClientStream? _pipe;
    private MpvIpcPipeState _state = MpvIpcPipeState.Disconnected;
    private bool _disposed;
    private bool _primitivesDisposed;
    private int _activeOperations;
    private TaskCompletionSource<bool> _operationsIdle = CompletedSource();

    public MpvNamedPipeClient(
        MpvIpcPipeEndpoint endpoint,
        MpvIpcPipeOptions? options = null)
    {
        _endpoint = endpoint ?? throw new ArgumentNullException(nameof(endpoint));
        _options = options ?? MpvIpcPipeOptions.Default;
        _options.Validate();
    }

    public MpvIpcPipeEndpoint Endpoint => _endpoint;

    public MpvIpcPipeState State
    {
        get
        {
            lock (_gate)
            {
                return _state;
            }
        }
    }

    /// <summary>连接到已由受管 mpv 进程创建的命名管道。</summary>
    public async Task<MpvIpcPipeConnectionResult> ConnectAsync(
        CancellationToken cancellationToken = default)
    {
        lock (_gate)
        {
            if (_disposed || _state is MpvIpcPipeState.Closed)
            {
                return MpvIpcPipeConnectionResult.Failed(
                    MpvIpcPipeState.Closed,
                    Error(MpvIpcFailureCode.IpcDisconnected, "mpv IPC 客户端已关闭。", retryable: false));
            }

            if (_state is MpvIpcPipeState.Connected)
            {
                return MpvIpcPipeConnectionResult.Success();
            }

            if (_state is MpvIpcPipeState.Connecting)
            {
                return MpvIpcPipeConnectionResult.Failed(
                    _state,
                    Error(MpvIpcFailureCode.IpcNotConnected, "mpv IPC 正在连接中。", retryable: true));
            }

            _state = MpvIpcPipeState.Connecting;
            EnterOperationLocked();
        }

        NamedPipeClientStream? pipe = null;
        using var timeoutCancellation = new CancellationTokenSource(_options.ConnectTimeout);
        using var linkedCancellation = CancellationTokenSource.CreateLinkedTokenSource(
            cancellationToken,
            timeoutCancellation.Token,
            _lifetimeCancellation.Token);
        try
        {
            pipe = new NamedPipeClientStream(
                serverName: ".",
                pipeName: _endpoint.PipeName,
                direction: PipeDirection.InOut,
                options: PipeOptions.Asynchronous);

            lock (_gate)
            {
                if (_disposed)
                {
                    throw new ObjectDisposedException(nameof(MpvNamedPipeClient));
                }

                _pipe = pipe;
            }

            await pipe.ConnectAsync(
                checked((int)Math.Min(_options.ConnectTimeout.TotalMilliseconds, int.MaxValue)),
                linkedCancellation.Token).ConfigureAwait(false);

            lock (_gate)
            {
                if (_disposed)
                {
                    return MpvIpcPipeConnectionResult.Failed(
                        MpvIpcPipeState.Closed,
                        Error(MpvIpcFailureCode.IpcDisconnected, "mpv IPC 客户端已关闭。", retryable: false));
                }

                _state = MpvIpcPipeState.Connected;
            }

            return MpvIpcPipeConnectionResult.Success();
        }
        catch (OperationCanceledException)
        {
            var error = _lifetimeCancellation.IsCancellationRequested || IsDisposed()
                ? Error(MpvIpcFailureCode.IpcDisconnected, "mpv IPC 连接已关闭。", retryable: false)
                : cancellationToken.IsCancellationRequested
                    ? Error(MpvIpcFailureCode.IpcCancelled, "mpv IPC 连接已取消。", retryable: true)
                    : Error(MpvIpcFailureCode.IpcTimeout, "mpv IPC 连接超时。", retryable: true);
            SetDisconnected(pipe, faulted: false);
            return MpvIpcPipeConnectionResult.Failed(State, error);
        }
        catch (TimeoutException)
        {
            SetDisconnected(pipe, faulted: false);
            return MpvIpcPipeConnectionResult.Failed(
                State,
                Error(MpvIpcFailureCode.IpcTimeout, "mpv IPC 连接超时。", retryable: true));
        }
        catch (Exception exception) when (exception is IOException or UnauthorizedAccessException or ArgumentException)
        {
            SetDisconnected(pipe, faulted: false);
            return MpvIpcPipeConnectionResult.Failed(
                State,
                Error(MpvIpcFailureCode.IpcDisconnected, "mpv IPC 命名管道无法连接。", retryable: true));
        }
        catch (ObjectDisposedException)
        {
            SetDisconnected(pipe, faulted: false);
            return MpvIpcPipeConnectionResult.Failed(
                MpvIpcPipeState.Closed,
                Error(MpvIpcFailureCode.IpcDisconnected, "mpv IPC 客户端已关闭。", retryable: false));
        }
        finally
        {
            ExitOperation();
        }
    }

    /// <summary>
    /// 发送一条固定 mpv 命令并等待对应 request_id 的响应。事件帧可被安全跳过，
    /// 任何未知/错配响应都会令当前连接失效，避免将后续响应应用到错误请求。
    /// </summary>
    public async Task<MpvIpcFrameParseResult> ExecuteAsync(
        ulong requestId,
        MpvIpcCommand? command,
        CancellationToken cancellationToken = default)
    {
        if (requestId == 0)
        {
            return Failed(Error(MpvIpcFailureCode.InvalidRequestId, "IPC request_id 必须大于 0。", false));
        }

        if (command is null)
        {
            return Failed(Error(MpvIpcFailureCode.InvalidCommand, "mpv IPC 命令不能为空。", false));
        }

        if (!TryEnterOperation())
        {
            return Failed(Error(MpvIpcFailureCode.IpcNotConnected, "mpv IPC 客户端已关闭。", false, requestId, command.Kind));
        }

        CancellationToken linkedToken;
        using var timeoutCancellation = new CancellationTokenSource(_options.ResponseTimeout);
        using var operationCancellation = CancellationTokenSource.CreateLinkedTokenSource(
            cancellationToken,
            timeoutCancellation.Token,
            _lifetimeCancellation.Token);
        linkedToken = operationCancellation.Token;

        var entered = false;
        NamedPipeClientStream? pipe = null;
        try
        {
            await _requestGate.WaitAsync(linkedToken).ConfigureAwait(false);
            entered = true;

            lock (_gate)
            {
                if (_disposed || _state is not MpvIpcPipeState.Connected || _pipe is null)
                {
                    return Failed(Error(MpvIpcFailureCode.IpcNotConnected, "mpv IPC 当前未连接。", true));
                }

                pipe = _pipe;
            }

            if (!command.TrySerialize(requestId, out var jsonLine, out var commandError)
                || jsonLine is null)
            {
                return Failed(commandError ?? Error(
                    MpvIpcFailureCode.InvalidCommand,
                    "mpv IPC 命令无效。",
                    false,
                    requestId,
                    command.Kind));
            }

            var encoded = Encoding.UTF8.GetBytes(jsonLine);
            if (encoded.Length > _options.MaxFrameBytes)
            {
                return Failed(Error(
                    MpvIpcFailureCode.CommandTooLarge,
                    "mpv IPC 命令超过帧大小限制。",
                    false,
                    requestId,
                    command.Kind));
            }

            await pipe.WriteAsync(encoded.AsMemory(), linkedToken).ConfigureAwait(false);
            await pipe.FlushAsync(linkedToken).ConfigureAwait(false);

            while (true)
            {
                var line = await ReadLineBoundedAsync(pipe, _options.MaxFrameBytes, linkedToken)
                    .ConfigureAwait(false);
                if (line is null)
                {
                    return DisconnectWithError(
                        pipe,
                        Error(MpvIpcFailureCode.IpcDisconnected, "mpv IPC 管道已断开。", true, requestId, command.Kind));
                }

                var parsed = MpvIpcFrameParser.Parse(line, requestId, _options.MaxFrameBytes);
                if (!parsed.IsSuccess)
                {
                    return DisconnectWithError(pipe, parsed.Error! with
                    {
                        RequestId = requestId,
                        CommandKind = command.Kind,
                    });
                }

                if (parsed.Frame?.Kind is MpvIpcFrameKind.Event)
                {
                    continue;
                }

                return parsed;
            }
        }
        catch (OperationCanceledException)
        {
            if (pipe is not null)
            {
                var error = _lifetimeCancellation.IsCancellationRequested || IsDisposed()
                    ? Error(MpvIpcFailureCode.IpcDisconnected, "mpv IPC 客户端已关闭。", false, requestId, command.Kind)
                    : cancellationToken.IsCancellationRequested
                        ? Error(MpvIpcFailureCode.IpcCancelled, "mpv IPC 请求已取消。", true, requestId, command.Kind)
                        : Error(MpvIpcFailureCode.IpcTimeout, "mpv IPC 响应超时。", true, requestId, command.Kind);
                return DisconnectWithError(pipe, error);
            }

            return Failed(_lifetimeCancellation.IsCancellationRequested || IsDisposed()
                ? Error(MpvIpcFailureCode.IpcDisconnected, "mpv IPC 客户端已关闭。", false, requestId, command.Kind)
                : cancellationToken.IsCancellationRequested
                    ? Error(MpvIpcFailureCode.IpcCancelled, "mpv IPC 请求已取消。", true, requestId, command.Kind)
                    : Error(MpvIpcFailureCode.IpcTimeout, "mpv IPC 响应超时。", true, requestId, command.Kind));
        }
        catch (EndOfStreamException)
        {
            return DisconnectWithError(
                pipe,
                Error(MpvIpcFailureCode.IpcDisconnected, "mpv IPC 管道帧不完整。", true, requestId, command.Kind));
        }
        catch (FrameTooLargeException)
        {
            return DisconnectWithError(
                pipe,
                Error(MpvIpcFailureCode.CommandTooLarge, "mpv IPC 响应超过帧大小限制。", false, requestId, command.Kind));
        }
        catch (InvalidDataException)
        {
            return DisconnectWithError(
                pipe,
                Error(MpvIpcFailureCode.MalformedJson, "mpv IPC 响应编码无效。", false, requestId, command.Kind));
        }
        catch (Exception exception) when (exception is IOException or InvalidOperationException or ObjectDisposedException)
        {
            return DisconnectWithError(
                pipe,
                Error(MpvIpcFailureCode.IpcDisconnected, "mpv IPC 管道通信失败。", true, requestId, command.Kind));
        }
        finally
        {
            if (entered)
            {
                _requestGate.Release();
            }

            ExitOperation();
        }
    }

    /// <summary>取消当前生命周期、关闭管道并释放本地资源。</summary>
    public async ValueTask DisposeAsync()
    {
        NamedPipeClientStream? pipe;
        Task idleTask;
        lock (_gate)
        {
            if (_disposed)
            {
                idleTask = _operationsIdle.Task;
                pipe = null;
            }
            else
            {
                _disposed = true;
                _state = MpvIpcPipeState.Closed;
                _lifetimeCancellation.Cancel();
                pipe = _pipe;
                _pipe = null;
                idleTask = _activeOperations == 0
                    ? Task.CompletedTask
                    : _operationsIdle.Task;
            }
        }

        pipe?.Dispose();
        if (!idleTask.IsCompleted)
        {
            await Task.WhenAny(idleTask, Task.Delay(DisposeWaitMilliseconds)).ConfigureAwait(false);
        }

        DisposePrimitivesIfIdle();
    }

    private static async Task<string?> ReadLineBoundedAsync(
        Stream stream,
        int maxBytes,
        CancellationToken cancellationToken)
    {
        var buffer = ArrayPool<byte>.Shared.Rent(Math.Min(maxBytes + 1, MpvIpcCommand.MaxJsonLineBytes + 1));
        var count = 0;
        try
        {
            while (true)
            {
                var read = await stream.ReadAsync(
                    buffer.AsMemory(count, 1),
                    cancellationToken).ConfigureAwait(false);
                if (read == 0)
                {
                    if (count == 0)
                    {
                        return null;
                    }

                    throw new EndOfStreamException();
                }

                count += read;
                if (count > maxBytes)
                {
                    throw new FrameTooLargeException();
                }

                if (buffer[count - 1] is (byte)'\n')
                {
                    var length = count > 1 && buffer[count - 2] is (byte)'\r'
                        ? count - 2
                        : count - 1;
                    try
                    {
                        return StrictUtf8.GetString(buffer, 0, length);
                    }
                    catch (DecoderFallbackException exception)
                    {
                        throw new InvalidDataException("mpv IPC 帧不是有效 UTF-8。", exception);
                    }
                }
            }
        }
        finally
        {
            ArrayPool<byte>.Shared.Return(buffer);
        }
    }

    private MpvIpcFrameParseResult DisconnectWithError(
        NamedPipeClientStream? pipe,
        MpvIpcError error)
    {
        SetDisconnected(pipe, faulted: error.Code is not MpvIpcFailureCode.IpcDisconnected
            and not MpvIpcFailureCode.IpcTimeout
            and not MpvIpcFailureCode.IpcCancelled);
        return Failed(error);
    }

    private void SetDisconnected(NamedPipeClientStream? pipe, bool faulted)
    {
        lock (_gate)
        {
            if (ReferenceEquals(_pipe, pipe))
            {
                _pipe = null;
                if (!_disposed)
                {
                    _state = faulted ? MpvIpcPipeState.Faulted : MpvIpcPipeState.Disconnected;
                }
            }
        }

        pipe?.Dispose();
    }

    private bool IsDisposed()
    {
        lock (_gate)
        {
            return _disposed;
        }
    }

    private bool TryEnterOperation()
    {
        lock (_gate)
        {
            if (_disposed || _primitivesDisposed)
            {
                return false;
            }

            EnterOperationLocked();
            return true;
        }
    }

    private void EnterOperationLocked()
    {
        if (_activeOperations == 0)
        {
            _operationsIdle = new TaskCompletionSource<bool>(
                TaskCreationOptions.RunContinuationsAsynchronously);
        }

        _activeOperations++;
    }

    private void ExitOperation()
    {
        lock (_gate)
        {
            if (_activeOperations > 0)
            {
                _activeOperations--;
                if (_activeOperations == 0)
                {
                    _operationsIdle.TrySetResult(true);
                }
            }
        }

        DisposePrimitivesIfIdle();
    }

    private void DisposePrimitivesIfIdle()
    {
        CancellationTokenSource? cancellation = null;
        SemaphoreSlim? gate = null;
        lock (_gate)
        {
            if (_disposed && !_primitivesDisposed && _activeOperations == 0)
            {
                _primitivesDisposed = true;
                cancellation = _lifetimeCancellation;
                gate = _requestGate;
            }
        }

        cancellation?.Dispose();
        gate?.Dispose();
    }

    private static TaskCompletionSource<bool> CompletedSource()
    {
        var source = new TaskCompletionSource<bool>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        source.SetResult(true);
        return source;
    }

    private static MpvIpcFrameParseResult Failed(MpvIpcError error) =>
        MpvIpcFrameParseResult.Failed(error);

    private static MpvIpcError Error(
        MpvIpcFailureCode code,
        string message,
        bool retryable,
        ulong? requestId = null,
        MpvIpcCommandKind? commandKind = null) =>
        new(code, message, retryable, requestId, commandKind);

    private sealed class FrameTooLargeException : Exception;
}
