namespace GpAutoLive.Media;

/// <summary>最终 PCM 总线的稳定失败分类。</summary>
public enum FinalPcmBusFailureCode
{
    InvalidFrameShape,
    TooLarge,
    Closed,
    ConsumerUnavailable,
    MixFailed,
}

/// <summary>不包含音频正文的最终 PCM 总线错误。</summary>
public sealed record FinalPcmBusError(
    FinalPcmBusFailureCode Code,
    string Message);

/// <summary>最终 PCM 总线的有界统计。</summary>
public sealed record FinalPcmBusSnapshot(
    int Channels,
    int CapacityFrames,
    int MaxFramesPerPublish,
    int OutputAvailableFrames,
    int RtmpAvailableFrames,
    ulong PublishedFrames,
    ulong OutputDroppedFrames,
    ulong RtmpDroppedFrames,
    bool IsClosed);

/// <summary>
/// 单一最终 PCM 事实源。每次发布只复制到两个固定容量消费者环缓：本机 PortAudio 和 RTMP。
/// 两个消费者互不阻塞，满载按实时策略丢弃各自最旧帧；不保存音频正文或创建无界队列。
/// </summary>
public sealed class FinalPcmBus : IDisposable
{
    public const int MaxFramesPerPublish = 4_096;
    public const int DefaultChannels = 2;

    private readonly object _gate = new();
    private ulong _publishedFrames;
    private bool _rtmpConsumerAttached;
    private bool _closed;

    public FinalPcmBus(int capacityFrames = 48_000, int channels = 2)
    {
        if (capacityFrames is < 1 or > 480_000)
        {
            throw new ArgumentOutOfRangeException(nameof(capacityFrames), "最终 PCM 总线容量必须在 1 到 480000 帧内。");
        }

        if (channels is < 1 or > 8)
        {
            throw new ArgumentOutOfRangeException(nameof(channels), "最终 PCM 总线声道数必须在 1 到 8 之间。");
        }

        Channels = channels;
        CapacityFrames = capacityFrames;
        OutputBuffer = new AudioPcmRingBuffer(capacityFrames, channels);
        RtmpBuffer = new AudioPcmRingBuffer(capacityFrames, channels);
        OutputOverlayBuffer = new AudioPcmRingBuffer(capacityFrames, channels);
        RtmpOverlayBuffer = new AudioPcmRingBuffer(capacityFrames, channels);
        OutputSpectrum = new();
        OverlaySpectrum = new();
    }

    public int Channels { get; }

    public int CapacityFrames { get; }

    /// <summary>PortAudio 消费的固定容量环缓；消费者不得调用 Close。</summary>
    public AudioPcmRingBuffer OutputBuffer { get; }

    /// <summary>RTMP PCM pump 消费的固定容量环缓；消费者不得调用 Close。</summary>
    public AudioPcmRingBuffer RtmpBuffer { get; }

    /// <summary>PortAudio 消费的插话环缓；由混音输出源与基础轨同步读取。</summary>
    public AudioPcmRingBuffer OutputOverlayBuffer { get; }

    /// <summary>RTMP PCM pump 消费的插话环缓；由混音输出源与基础轨同步读取。</summary>
    public AudioPcmRingBuffer RtmpOverlayBuffer { get; }

    /// <summary>视频/主音频 PCM 的最新频谱；只读诊断结果，不消费输出缓冲。</summary>
    public PcmSpectrumAnalyzer OutputSpectrum { get; }

    /// <summary>插话 PCM 的最新频谱；只读诊断结果，不消费插话缓冲。</summary>
    public PcmSpectrumAnalyzer OverlaySpectrum { get; }

    public FinalPcmBusSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                var output = OutputBuffer.Snapshot;
                var rtmp = RtmpBuffer.Snapshot;
                return new(
                    Channels,
                    CapacityFrames,
                    MaxFramesPerPublish,
                    output.AvailableFrames,
                    _rtmpConsumerAttached ? rtmp.AvailableFrames : 0,
                    _publishedFrames,
                    output.DroppedFrames,
                    _rtmpConsumerAttached ? rtmp.DroppedFrames : 0,
                    _closed);
            }
        }
    }

    /// <summary>
    /// 设置 RTMP 是否拥有真实消费者。首次接入或断开时丢弃旧尾部，确保 RTMP 只读取接入后的最终 PCM。
    /// </summary>
    public void SetRtmpConsumerAttached(bool attached)
    {
        lock (_gate)
        {
            if (_closed || _rtmpConsumerAttached == attached)
            {
                return;
            }

            RtmpBuffer.DiscardPending();
            RtmpOverlayBuffer.DiscardPending();
            _rtmpConsumerAttached = attached;
        }
    }

    /// <summary>同时向本机和 RTMP 两个消费者发布一段完整交错 PCM。</summary>
    public bool TryPublish(
        ReadOnlySpan<float> interleavedPcm,
        out int framesPublished,
        out FinalPcmBusError? error)
    {
        framesPublished = 0;
        error = null;
        if (interleavedPcm.Length % Channels != 0)
        {
            error = new(FinalPcmBusFailureCode.InvalidFrameShape, "最终 PCM 样本数必须完整对齐到交错声道帧。");
            return false;
        }

        var inputFrames = interleavedPcm.Length / Channels;
        if (inputFrames > MaxFramesPerPublish)
        {
            error = new(FinalPcmBusFailureCode.TooLarge, "最终 PCM 分片超过有界发布容量。");
            return false;
        }

        if (inputFrames == 0)
        {
            return true;
        }

        lock (_gate)
        {
            if (_closed)
            {
                error = new(FinalPcmBusFailureCode.Closed, "最终 PCM 总线已关闭。");
                return false;
            }

            var outputSucceeded = OutputBuffer.TryWrite(interleavedPcm, out var outputFrames, out var outputError);
            var rtmpFrames = 0;
            PcmRingBufferError? rtmpError = null;
            var rtmpSucceeded = !_rtmpConsumerAttached
                || RtmpBuffer.TryWrite(interleavedPcm, out rtmpFrames, out rtmpError);
            if (!outputSucceeded || !rtmpSucceeded)
            {
                error = new(
                    FinalPcmBusFailureCode.ConsumerUnavailable,
                    outputError?.Message ?? rtmpError?.Message ?? "最终 PCM 消费者不可用。");
                return false;
            }

            framesPublished = _rtmpConsumerAttached
                ? Math.Min(outputFrames, rtmpFrames)
                : outputFrames;
            _publishedFrames = ulong.MaxValue - (ulong)framesPublished < _publishedFrames
                ? ulong.MaxValue
                : _publishedFrames + (ulong)framesPublished;
            OutputSpectrum.Update(interleavedPcm, Channels);
            return true;
        }
    }

    /// <summary>向本机和 RTMP 两个插话消费者发布 PCM；输出源负责与基础轨实时混合。</summary>
    public bool TryPublishOverlay(
        ReadOnlySpan<float> interleavedPcm,
        out int framesPublished,
        out FinalPcmBusError? error)
    {
        framesPublished = 0;
        error = null;
        if (interleavedPcm.Length % Channels != 0)
        {
            error = new(FinalPcmBusFailureCode.InvalidFrameShape, "插话 PCM 样本数必须完整对齐到交错声道帧。");
            return false;
        }

        var inputFrames = interleavedPcm.Length / Channels;
        if (inputFrames > MaxFramesPerPublish)
        {
            error = new(FinalPcmBusFailureCode.TooLarge, "插话 PCM 分片超过有界发布容量。");
            return false;
        }

        if (inputFrames == 0)
        {
            return true;
        }

        lock (_gate)
        {
            if (_closed)
            {
                error = new(FinalPcmBusFailureCode.Closed, "最终 PCM 总线已关闭。");
                return false;
            }

            var outputSucceeded = OutputOverlayBuffer.TryWrite(interleavedPcm, out var outputFrames, out var outputError);
            var rtmpFrames = 0;
            PcmRingBufferError? rtmpError = null;
            var rtmpSucceeded = !_rtmpConsumerAttached
                || RtmpOverlayBuffer.TryWrite(interleavedPcm, out rtmpFrames, out rtmpError);
            if (!outputSucceeded || !rtmpSucceeded)
            {
                error = new(
                    FinalPcmBusFailureCode.ConsumerUnavailable,
                    outputError?.Message ?? rtmpError?.Message ?? "插话 PCM 消费者不可用。");
                return false;
            }

            framesPublished = _rtmpConsumerAttached
                ? Math.Min(outputFrames, rtmpFrames)
                : outputFrames;
            OverlaySpectrum.Update(interleavedPcm, Channels);
            return true;
        }
    }

    /// <summary>丢弃插话/固定话术未消费尾部，避免优先级结束后旧语音泄漏到原媒体。</summary>
    public void DiscardOverlayPending()
    {
        lock (_gate)
        {
            OutputOverlayBuffer.DiscardPending();
            RtmpOverlayBuffer.DiscardPending();
        }
    }

    /// <summary>
    /// 先在调用方提供的预分配目标中完成有界混音，再发布到两个消费者；不会创建临时 PCM 数组。
    /// </summary>
    public bool TryPublishMixed(
        ReadOnlySpan<float> basePcm,
        ReadOnlySpan<float> overlayPcm,
        Span<float> mixBuffer,
        AudioPcmMixPolicy policy,
        out int framesPublished,
        out FinalPcmBusError? error)
    {
        framesPublished = 0;
        error = null;
        if (!AudioPcmMixer.TryMix(
                basePcm,
                overlayPcm,
                mixBuffer,
                Channels,
                policy,
                out var framesMixed,
                out var mixError))
        {
            error = new(
                mixError?.Code is AudioPcmMixFailureCode.InvalidFrameShape
                    ? FinalPcmBusFailureCode.InvalidFrameShape
                    : mixError?.Code is AudioPcmMixFailureCode.DestinationTooSmall
                        ? FinalPcmBusFailureCode.TooLarge
                        : FinalPcmBusFailureCode.MixFailed,
                mixError?.Message ?? "最终 PCM 混音失败。");
            return false;
        }

        if (framesMixed > MaxFramesPerPublish)
        {
            error = new(FinalPcmBusFailureCode.TooLarge, "最终 PCM 混音分片超过有界发布容量。");
            return false;
        }

        return TryPublish(
            mixBuffer[..checked(framesMixed * Channels)],
            out framesPublished,
            out error);
    }

    /// <summary>关闭发布；已写入两个消费者的帧仍可排空。</summary>
    public void Close()
    {
        lock (_gate)
        {
            if (_closed)
            {
                return;
            }

            _closed = true;
            OutputBuffer.Close();
            RtmpBuffer.Close();
            OutputOverlayBuffer.Close();
            RtmpOverlayBuffer.Close();
        }
    }

    public void Dispose() => Close();
}
