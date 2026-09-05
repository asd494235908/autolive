namespace GpAutoLive.Media;

/// <summary>固定容量 PCM 环缓的稳定错误分类。</summary>
public enum PcmRingBufferFailureCode
{
    InvalidFrameShape,
    ConsumerBusy,
    Closed,
    MixFailed,
}

/// <summary>不包含音频正文的环缓错误。</summary>
public sealed record PcmRingBufferError(
    PcmRingBufferFailureCode Code,
    string Message);

/// <summary>PCM 环缓的轻量运行快照。</summary>
public sealed record PcmRingBufferSnapshot(
    int CapacityFrames,
    int Channels,
    int AvailableFrames,
    ulong DroppedFrames,
    bool IsClosed);

/// <summary>
/// 固定容量、交错 float PCM 环形缓冲。
/// 写入满时丢弃最旧帧，优先保持实时延迟有界；不会扩容、阻塞或保留历史正文。
/// </summary>
public sealed class AudioPcmRingBuffer
{
    private readonly object _gate = new();
    private readonly float[] _samples;
    private readonly int _capacityFrames;
    private readonly int _channels;
    private int _readFrame;
    private int _writeFrame;
    private int _availableFrames;
    private ulong _droppedFrames;
    private bool _closed;

    public AudioPcmRingBuffer(int capacityFrames, int channels)
    {
        if (capacityFrames is < 1 or > 480_000)
        {
            throw new ArgumentOutOfRangeException(nameof(capacityFrames), "PCM 环缓容量必须在 1 到 480000 帧内。");
        }

        if (channels is < 1 or > 8)
        {
            throw new ArgumentOutOfRangeException(nameof(channels), "PCM 声道数必须在 1 到 8 之间。");
        }

        _capacityFrames = capacityFrames;
        _channels = channels;
        _samples = new float[checked(capacityFrames * channels)];
    }

    public PcmRingBufferSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshot();
            }
        }
    }

    /// <summary>写入交错 PCM；满时丢弃最旧帧，返回实际保留的帧数。</summary>
    public bool TryWrite(
        ReadOnlySpan<float> interleavedSamples,
        out int framesWritten,
        out PcmRingBufferError? error)
    {
        framesWritten = 0;
        error = null;
        if (interleavedSamples.Length % _channels != 0)
        {
            error = new(
                PcmRingBufferFailureCode.InvalidFrameShape,
                "PCM 样本数必须完整对齐到交错声道帧。");
            return false;
        }

        var inputFrames = interleavedSamples.Length / _channels;
        if (inputFrames == 0)
        {
            return true;
        }

        lock (_gate)
        {
            return TryWriteLocked(interleavedSamples, inputFrames, out framesWritten, out error);
        }
    }

    /// <summary>实时写入；若普通读写临界区繁忙立即返回，不等待音频回调线程。</summary>
    public bool TryWriteRealtime(
        ReadOnlySpan<float> interleavedSamples,
        out int framesWritten,
        out PcmRingBufferError? error)
    {
        framesWritten = 0;
        error = null;
        if (interleavedSamples.Length % _channels != 0)
        {
            error = new(
                PcmRingBufferFailureCode.InvalidFrameShape,
                "PCM 样本数必须完整对齐到交错声道帧。");
            return false;
        }

        var inputFrames = interleavedSamples.Length / _channels;
        if (inputFrames == 0)
        {
            return true;
        }

        if (!Monitor.TryEnter(_gate))
        {
            error = new(PcmRingBufferFailureCode.ConsumerBusy, "PCM 环缓临界区繁忙。");
            return false;
        }

        try
        {
            return TryWriteLocked(interleavedSamples, inputFrames, out framesWritten, out error);
        }
        finally
        {
            Monitor.Exit(_gate);
        }
    }

    /// <summary>读取最多目标容量的完整 PCM 帧；无数据时返回 0，不阻塞。</summary>
    public bool TryRead(
        Span<float> destination,
        out int framesRead,
        out PcmRingBufferError? error)
    {
        framesRead = 0;
        error = null;
        if (destination.Length % _channels != 0)
        {
            error = new(
                PcmRingBufferFailureCode.InvalidFrameShape,
                "PCM 目标缓冲必须完整对齐到交错声道帧。");
            return false;
        }

        lock (_gate)
        {
            return TryReadLocked(destination, out framesRead, out error);
        }
    }

    /// <summary>实时读取；若普通读写临界区繁忙立即返回，调用方应输出静音或丢弃当前帧。</summary>
    public bool TryReadRealtime(
        Span<float> destination,
        out int framesRead,
        out PcmRingBufferError? error)
    {
        framesRead = 0;
        error = null;
        if (destination.Length % _channels != 0)
        {
            error = new(
                PcmRingBufferFailureCode.InvalidFrameShape,
                "PCM 目标缓冲必须完整对齐到交错声道帧。");
            return false;
        }

        if (!Monitor.TryEnter(_gate))
        {
            error = new(PcmRingBufferFailureCode.ConsumerBusy, "PCM 环缓临界区繁忙。");
            return false;
        }

        try
        {
            return TryReadLocked(destination, out framesRead, out error);
        }
        finally
        {
            Monitor.Exit(_gate);
        }
    }

    /// <summary>关闭写入；已存帧仍可被读取，之后只允许读取或重复关闭。</summary>
    public void Close()
    {
        lock (_gate)
        {
            _closed = true;
        }
    }

    /// <summary>丢弃尚未读取的帧，不改变关闭状态和累计丢帧统计。</summary>
    public int DiscardPending()
    {
        lock (_gate)
        {
            var discarded = _availableFrames;
            _readFrame = _writeFrame;
            _availableFrames = 0;
            return discarded;
        }
    }

    private PcmRingBufferSnapshot CreateSnapshot() => new(
        _capacityFrames,
        _channels,
        _availableFrames,
        _droppedFrames,
        _closed);

    private void AddDroppedFrames(int droppedFrames)
    {
        if (droppedFrames <= 0)
        {
            return;
        }

        var dropped = (ulong)droppedFrames;
        _droppedFrames = ulong.MaxValue - _droppedFrames < dropped
            ? ulong.MaxValue
            : _droppedFrames + dropped;
    }

    private bool TryWriteLocked(
        ReadOnlySpan<float> interleavedSamples,
        int inputFrames,
        out int framesWritten,
        out PcmRingBufferError? error)
    {
        framesWritten = 0;
        error = null;
        if (_closed)
        {
            error = new(PcmRingBufferFailureCode.Closed, "PCM 环缓已关闭。");
            return false;
        }

        var sourceStartFrame = Math.Max(0, inputFrames - _capacityFrames);
        var framesToWrite = inputFrames - sourceStartFrame;
        var overwritten = Math.Max(0, _availableFrames + framesToWrite - _capacityFrames);
        if (overwritten > 0)
        {
            _readFrame = (_readFrame + overwritten) % _capacityFrames;
            _availableFrames -= overwritten;
        }

        AddDroppedFrames(sourceStartFrame + overwritten);
        for (var frame = 0; frame < framesToWrite; frame++)
        {
            var sourceOffset = checked((sourceStartFrame + frame) * _channels);
            var destinationOffset = _writeFrame * _channels;
            interleavedSamples.Slice(sourceOffset, _channels)
                .CopyTo(_samples.AsSpan(destinationOffset, _channels));
            _writeFrame = (_writeFrame + 1) % _capacityFrames;
        }

        _availableFrames += framesToWrite;
        framesWritten = framesToWrite;
        return true;
    }

    private bool TryReadLocked(
        Span<float> destination,
        out int framesRead,
        out PcmRingBufferError? error)
    {
        framesRead = 0;
        error = null;
        var requestedFrames = destination.Length / _channels;
        var framesToRead = Math.Min(requestedFrames, _availableFrames);
        for (var frame = 0; frame < framesToRead; frame++)
        {
            var sourceOffset = _readFrame * _channels;
            var destinationOffset = frame * _channels;
            _samples.AsSpan(sourceOffset, _channels)
                .CopyTo(destination.Slice(destinationOffset, _channels));
            _readFrame = (_readFrame + 1) % _capacityFrames;
        }

        _availableFrames -= framesToRead;
        framesRead = framesToRead;
        return true;
    }
}
