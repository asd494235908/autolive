using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>RTMP PCM 分流泵的稳定错误分类。</summary>
public enum WindowsRtmpPcmPumpFailureCode
{
    InvalidArguments,
    AlreadyRunning,
    Cancelled,
    SourceClosed,
    WriteFailed,
    Closed,
}

/// <summary>不包含 PCM 正文的 RTMP PCM 分流错误。</summary>
public sealed record WindowsRtmpPcmPumpError(
    WindowsRtmpPcmPumpFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>RTMP PCM 分流泵状态。</summary>
public sealed record WindowsRtmpPcmPumpSnapshot(
    bool IsRunning,
    ulong ForwardedFrames,
    string? ErrorCode,
    string? Error);

/// <summary>RTMP PCM 分流泵结果。</summary>
public sealed record WindowsRtmpPcmPumpResult(
    bool IsSuccess,
    WindowsRtmpPcmPumpSnapshot Snapshot,
    WindowsRtmpPcmPumpError? Error = null);

/// <summary>
/// 将最终 PCM 总线的 RTMP 环缓串行写入 FFmpeg stdin。泵本身不创建新队列，
/// 没有帧时以固定小间隔让出线程；调用方必须持有并等待返回的任务以完成 Join。
/// </summary>
public sealed class WindowsRtmpFinalPcmPump : IDisposable
{
    private static readonly TimeSpan IdlePollInterval = TimeSpan.FromMilliseconds(10);
    private readonly object _gate = new();
    private readonly IAudioPcmOutputSource _source;
    private readonly int _channels;
    private readonly int _framesPerChunk;
    private readonly CancellationTokenSource _disposeCancellation = new();
    private readonly float[] _chunkBuffer;
    private bool _running;
    private bool _disposed;
    private ulong _forwardedFrames;
    private WindowsRtmpPcmPumpError? _lastError;

    public WindowsRtmpFinalPcmPump(
        AudioPcmRingBuffer source,
        int channels,
        int framesPerChunk = 1_024)
        : this(new AudioPcmRingBufferOutputSource(source), channels, framesPerChunk)
    {
    }

    /// <summary>创建使用自定义固定 PCM 输出源的 RTMP 分流泵。</summary>
    public WindowsRtmpFinalPcmPump(
        IAudioPcmOutputSource source,
        int channels,
        int framesPerChunk = 1_024)
    {
        _source = source ?? throw new ArgumentNullException(nameof(source));
        if (channels is < 1 or > 8)
        {
            throw new ArgumentOutOfRangeException(nameof(channels), "RTMP PCM 分流声道数必须在 1 到 8 之间。");
        }

        if (framesPerChunk is < 16 or > FinalPcmBus.MaxFramesPerPublish)
        {
            throw new ArgumentOutOfRangeException(nameof(framesPerChunk), "RTMP PCM 分流帧数必须在 16 到 4096 之间。");
        }

        _channels = channels;
        _framesPerChunk = framesPerChunk;
        _chunkBuffer = new float[checked(channels * framesPerChunk)];
    }

    public WindowsRtmpPcmPumpSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return new(_running, _forwardedFrames, _lastError?.Code.ToString(), _lastError?.Message);
            }
        }
    }

    /// <summary>启动唯一分流任务；任务结束前不会创建第二个分流任务。</summary>
    public async Task<WindowsRtmpPcmPumpResult> RunAsync(
        WindowsRtmpOutputManager? manager,
        CancellationToken cancellationToken = default)
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return Failure(WindowsRtmpPcmPumpFailureCode.Closed, "RTMP PCM 分流泵已关闭。", retryable: false);
            }

            if (manager is null)
            {
                return Failure(WindowsRtmpPcmPumpFailureCode.InvalidArguments, "RTMP PCM 分流需要有效的输出宿主。", retryable: false);
            }

            if (_running)
            {
                return Failure(WindowsRtmpPcmPumpFailureCode.AlreadyRunning, "RTMP PCM 分流已经在运行。", retryable: false);
            }

            _running = true;
            _lastError = null;
            _forwardedFrames = 0;
        }

        using var linkedCancellation = CancellationTokenSource.CreateLinkedTokenSource(
            cancellationToken,
            _disposeCancellation.Token);
        var runCancellationToken = linkedCancellation.Token;
        try
        {
            while (true)
            {
                runCancellationToken.ThrowIfCancellationRequested();
                var buffer = _chunkBuffer;
                if (!_source.TryRead(buffer.AsSpan(), out var framesRead, out var readError))
                {
                    return Failure(
                        WindowsRtmpPcmPumpFailureCode.SourceClosed,
                        readError?.Message ?? "RTMP PCM 来源不可用。",
                        retryable: true);
                }

                if (framesRead > 0)
                {
                    var writeResult = await manager
                        .WriteFinalPcmAsync(buffer.AsMemory(0, checked(framesRead * _channels)), runCancellationToken)
                        .ConfigureAwait(false);
                    if (!writeResult.IsSuccess)
                    {
                        return Failure(
                            WindowsRtmpPcmPumpFailureCode.WriteFailed,
                            writeResult.Error?.Message ?? "RTMP PCM 写入失败。",
                            retryable: writeResult.Error?.Retryable ?? true);
                    }

                    lock (_gate)
                    {
                        _forwardedFrames = ulong.MaxValue - (ulong)framesRead < _forwardedFrames
                            ? ulong.MaxValue
                            : _forwardedFrames + (ulong)framesRead;
                    }

                    continue;
                }

                if (_source.IsClosed)
                {
                    return Success();
                }

                await Task.Delay(IdlePollInterval, runCancellationToken).ConfigureAwait(false);
            }
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsRtmpPcmPumpFailureCode.Cancelled, "RTMP PCM 分流已取消。", retryable: true);
        }
        finally
        {
            lock (_gate)
            {
                _running = false;
            }
        }
    }

    public void Dispose()
    {
        lock (_gate)
        {
            _disposed = true;
        }

        _disposeCancellation.Cancel();

        GC.SuppressFinalize(this);
    }

    private WindowsRtmpPcmPumpResult Success()
    {
        lock (_gate)
        {
            return new(true, CreateSnapshot());
        }
    }

    private WindowsRtmpPcmPumpResult Failure(
        WindowsRtmpPcmPumpFailureCode code,
        string message,
        bool retryable)
    {
        lock (_gate)
        {
            _lastError = new(code, message, retryable);
            return new(false, CreateSnapshot(), _lastError);
        }
    }

    private WindowsRtmpPcmPumpSnapshot CreateSnapshot() =>
        new(_running, _forwardedFrames, _lastError?.Code.ToString(), _lastError?.Message);
}
