namespace GpAutoLive.Media;

/// <summary>固定 PCM 输出源的读取边界。</summary>
public interface IAudioPcmOutputSource
{
    /// <summary>输出源的交错 PCM 声道数。</summary>
    int Channels { get; }

    /// <summary>基础输出源是否已关闭并不会再产生帧。</summary>
    bool IsClosed { get; }

    /// <summary>按普通线程语义读取一段 PCM。</summary>
    bool TryRead(Span<float> destination, out int framesRead, out PcmRingBufferError? error);

    /// <summary>按实时回调语义读取一段 PCM；临界区繁忙时立即返回。</summary>
    bool TryReadRealtime(Span<float> destination, out int framesRead, out PcmRingBufferError? error);
}

/// <summary>输出回调使用的插话增益过渡参数；0 表示立即切换。</summary>
public readonly record struct AudioPcmMixEnvelopeOptions(
    int SampleRateHz = 48_000,
    ulong AttackMs = 0,
    ulong ReleaseMs = 0);

/// <summary>将单个 PCM 环缓适配为输出源。</summary>
public sealed class AudioPcmRingBufferOutputSource : IAudioPcmOutputSource
{
    private readonly AudioPcmRingBuffer _buffer;

    public AudioPcmRingBufferOutputSource(AudioPcmRingBuffer buffer)
    {
        _buffer = buffer ?? throw new ArgumentNullException(nameof(buffer));
    }

    /// <inheritdoc />
    public int Channels => _buffer.Snapshot.Channels;

    /// <inheritdoc />
    public bool IsClosed => _buffer.Snapshot.IsClosed;

    /// <inheritdoc />
    public bool TryRead(Span<float> destination, out int framesRead, out PcmRingBufferError? error) =>
        _buffer.TryRead(destination, out framesRead, out error);

    /// <inheritdoc />
    public bool TryReadRealtime(Span<float> destination, out int framesRead, out PcmRingBufferError? error) =>
        _buffer.TryReadRealtime(destination, out framesRead, out error);
}

/// <summary>
/// 在固定输出缓冲内混合基础轨与插话轨。插话轨没有可用帧时按静音处理，
/// 不创建第二个 PortAudio 输出流，也不在回调中分配内存。
/// </summary>
public sealed class AudioPcmMixingOutputSource : IAudioPcmOutputSource
{
    private readonly IAudioPcmOutputSource _baseSource;
    private readonly IAudioPcmOutputSource _overlaySource;
    private readonly int _channels;
    private readonly float[] _overlayScratch;
    private readonly Func<AudioPcmMixPolicy>? _policyProvider;
    private readonly int _attackFrames = 0;
    private readonly int _releaseFrames = 0;
    private float _baseGain = 1F;
    private float _overlayGain;
    private readonly bool _overlayFollowsBase;
    private ulong _baseFramesRead;

    /// <summary>实际混入输出的主源帧数；插话独立输出不推进主源时钟。</summary>
    public ulong BaseFramesRead => Volatile.Read(ref _baseFramesRead);

    public AudioPcmMixingOutputSource(
        AudioPcmRingBuffer baseBuffer,
        AudioPcmRingBuffer overlayBuffer,
        int channels,
        int maxFramesPerRead = 4_096,
        Func<AudioPcmMixPolicy>? policyProvider = null,
        AudioPcmMixEnvelopeOptions? envelopeOptions = null)
        : this(
            new AudioPcmRingBufferOutputSource(baseBuffer ?? throw new ArgumentNullException(nameof(baseBuffer))),
            new AudioPcmRingBufferOutputSource(overlayBuffer ?? throw new ArgumentNullException(nameof(overlayBuffer))),
            channels,
            maxFramesPerRead,
            policyProvider,
            envelopeOptions)
    {
    }

    /// <summary>创建使用可切换 PCM 输出源的混音源。</summary>
    public AudioPcmMixingOutputSource(
        IAudioPcmOutputSource baseSource,
        IAudioPcmOutputSource overlaySource,
        int channels,
        int maxFramesPerRead = 4_096,
        Func<AudioPcmMixPolicy>? policyProvider = null,
        AudioPcmMixEnvelopeOptions? envelopeOptions = null,
        bool overlayFollowsBase = false)
    {
        _baseSource = baseSource ?? throw new ArgumentNullException(nameof(baseSource));
        _overlaySource = overlaySource ?? throw new ArgumentNullException(nameof(overlaySource));
        if (channels is < 1 or > 8)
        {
            throw new ArgumentOutOfRangeException(nameof(channels), "PCM 混音输出声道数必须在 1 到 8 之间。");
        }

        if (maxFramesPerRead is < 1 or > 4_096)
        {
            throw new ArgumentOutOfRangeException(nameof(maxFramesPerRead), "PCM 混音输出读取帧数必须在 1 到 4096 之间。");
        }

        if (_baseSource.Channels != channels || _overlaySource.Channels != channels)
        {
            throw new ArgumentException("PCM 混音基础轨、插话轨和输出声道数必须一致。", nameof(channels));
        }

        _channels = channels;
        _overlayScratch = new float[checked(maxFramesPerRead * channels)];
        _policyProvider = policyProvider;
        _overlayFollowsBase = overlayFollowsBase;
        if (envelopeOptions is { } envelope)
        {
            if (envelope.SampleRateHz is < 1_000 or > 384_000)
            {
                throw new ArgumentOutOfRangeException(nameof(envelopeOptions), "PCM 混音采样率超出范围。");
            }

            _attackFrames = FramesForMilliseconds(envelope.SampleRateHz, envelope.AttackMs);
            _releaseFrames = FramesForMilliseconds(envelope.SampleRateHz, envelope.ReleaseMs);
        }
    }

    /// <inheritdoc />
    public int Channels => _channels;

    /// <inheritdoc />
    public bool IsClosed => _baseSource.IsClosed;

    /// <inheritdoc />
    public bool TryRead(Span<float> destination, out int framesRead, out PcmRingBufferError? error) =>
        TryReadCore(destination, realtime: false, out framesRead, out error);

    /// <inheritdoc />
    public bool TryReadRealtime(Span<float> destination, out int framesRead, out PcmRingBufferError? error) =>
        TryReadCore(destination, realtime: true, out framesRead, out error);

    private bool TryReadCore(
        Span<float> destination,
        bool realtime,
        out int framesRead,
        out PcmRingBufferError? error)
    {
        framesRead = 0;
        error = null;
        if (destination.Length % _channels != 0 || destination.Length > _overlayScratch.Length)
        {
            error = new(destination.Length % _channels != 0
                ? PcmRingBufferFailureCode.InvalidFrameShape : PcmRingBufferFailureCode.MixFailed,
                "PCM 混音目标必须对齐且不超过固定容量。");
            return false;
        }
        while (framesRead < destination.Length / _channels)
        {
            if (!TryReadSegment(destination[(framesRead * _channels)..], realtime, out var segmentFrames, out error))
            {
                return false;
            }
            if (segmentFrames == 0)
            {
                return true;
            }
            framesRead += segmentFrames;
        }
        return true;
    }

    private bool TryReadSegment(
        Span<float> destination,
        bool realtime,
        out int framesRead,
        out PcmRingBufferError? error)
    {
        framesRead = 0;
        error = null;

        int baseFrames;
        PcmRingBufferError? baseError;
        var trackSource = _baseSource as AudioPcmTrackSwitchOutputSource;
        var baseReadSucceeded = trackSource is not null
            ? trackSource.TryReadOneCandidate(destination, realtime, out baseFrames, out baseError)
            : realtime
                ? _baseSource.TryReadRealtime(destination, out baseFrames, out baseError)
                : _baseSource.TryRead(destination, out baseFrames, out baseError);
        if (!baseReadSucceeded)
        {
            error = baseError;
            return false;
        }

        // RTMP 的源时间只由基础媒体帧推进。临时欠载保留插话尾部，恢复后继续叠加；
        // 本机输出保留独立 overlay 能力，不受这一发布约束影响。
        if (_overlayFollowsBase && baseFrames == 0)
        {
            return true;
        }
        var overlayLength = _overlayFollowsBase || trackSource is not null && baseFrames > 0
            ? baseFrames * _channels : destination.Length;
        var overlayReadSucceeded = realtime
            ? _overlaySource.TryReadRealtime(_overlayScratch.AsSpan(0, overlayLength), out var overlayFrames, out var overlayError)
            : _overlaySource.TryRead(_overlayScratch.AsSpan(0, overlayLength), out overlayFrames, out overlayError);
        if (!overlayReadSucceeded)
        {
            error = overlayError;
            return false;
        }

        var baseSamples = checked(baseFrames * _channels);
        var overlaySamples = checked(overlayFrames * _channels);
        var policy = _policyProvider?.Invoke() ?? default;
        if (!AudioPcmMixer.TryGetLinearGain(policy.BaseGainDb, out var baseGain)
            || !AudioPcmMixer.TryGetLinearGain(policy.BaseDuckingDb, out var duckGain)
            || !AudioPcmMixer.TryGetLinearGain(policy.OverlayGainDb, out var overlayGain))
        {
            error = new(
                PcmRingBufferFailureCode.MixFailed,
                "PCM 混音增益无效。");
            return false;
        }

        var targetBaseGain = policy.MuteBase ? 0F : baseGain * duckGain;
        var targetOverlayGain = policy.MuteOverlay || overlayFrames == 0 ? 0F : overlayGain;
        var outputFrames = Math.Max(baseFrames, overlayFrames);
        var useEnvelope = _attackFrames > 0 || _releaseFrames > 0;
        var baseStart = useEnvelope ? _baseGain : targetBaseGain;
        var overlayStart = useEnvelope ? _overlayGain : targetOverlayGain;
        var baseEnd = AdvanceGain(baseStart, targetBaseGain, outputFrames, useEnvelope);
        var overlayEnd = AdvanceGain(overlayStart, targetOverlayGain, outputFrames, useEnvelope);
        AudioPcmMixer.MixLinearRamp(
            destination[..baseSamples],
            _overlayScratch.AsSpan(0, overlaySamples),
            destination,
            _channels,
            baseStart,
            baseEnd,
            overlayStart,
            overlayEnd,
            outputFrames);
        _baseGain = baseEnd;
        _overlayGain = overlayEnd;
        Interlocked.Add(ref _baseFramesRead, (ulong)baseFrames);
        framesRead = outputFrames;
        return true;
    }

    private float AdvanceGain(float current, float target, int frames, bool useEnvelope)
    {
        if (!useEnvelope || frames <= 0 || Math.Abs(target - current) < 0.000_001F)
        {
            return target;
        }

        var transitionFrames = target > current ? _attackFrames : _releaseFrames;
        if (transitionFrames <= 0)
        {
            return target;
        }

        var step = Math.Abs(target - current) * Math.Min(1F, (float)frames / transitionFrames);
        return current + MathF.CopySign(step, target - current);
    }

    private static int FramesForMilliseconds(int sampleRateHz, ulong milliseconds)
    {
        if (milliseconds == 0)
        {
            return 0;
        }

        var frames = ((double)sampleRateHz * milliseconds / 1_000D);
        return (int)Math.Clamp(Math.Ceiling(frames), 1D, int.MaxValue);
    }
}
