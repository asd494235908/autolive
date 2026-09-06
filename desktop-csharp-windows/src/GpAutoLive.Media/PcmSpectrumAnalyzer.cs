namespace GpAutoLive.Media;

/// <summary>
/// 从实际 PCM 分片计算有界的 16 段频谱。只保留最新结果，不读取或消费任何输出环缓。
/// </summary>
public sealed class PcmSpectrumAnalyzer
{
    public const int BandCount = 16;

    private static readonly double[] CenterFrequenciesHz =
    [
        80, 125, 200, 315, 500, 800, 1_250, 2_000,
        3_150, 5_000, 6_300, 8_000, 10_000, 12_500, 16_000, 20_000,
    ];

    private readonly double[] _coefficients;
    private readonly float[] _levels = new float[BandCount];
    private int _hasSignal;

    public PcmSpectrumAnalyzer(int sampleRateHz = 48_000)
    {
        if (sampleRateHz is < 8_000 or > 192_000)
        {
            throw new ArgumentOutOfRangeException(nameof(sampleRateHz));
        }

        _coefficients = CenterFrequenciesHz
            .Select(frequency => 2d * Math.Cos(2d * Math.PI * frequency / sampleRateHz))
            .ToArray();
    }

    public bool HasSignal => Volatile.Read(ref _hasSignal) != 0;

    /// <summary>分析交错 PCM 的单声道平均能量，不改变调用方缓冲。</summary>
    public void Update(ReadOnlySpan<float> interleavedPcm, int channels)
    {
        if (channels is < 1 or > 8 || interleavedPcm.Length % channels != 0)
        {
            return;
        }

        var frames = interleavedPcm.Length / channels;
        if (frames == 0)
        {
            return;
        }

        Span<double> previous = stackalloc double[BandCount];
        Span<double> previousPrevious = stackalloc double[BandCount];
        double sumSquares = 0;
        for (var frame = 0; frame < frames; frame++)
        {
            var sample = 0d;
            var offset = frame * channels;
            for (var channel = 0; channel < channels; channel++)
            {
                sample += interleavedPcm[offset + channel];
            }

            sample /= channels;
            sumSquares += sample * sample;
            for (var band = 0; band < BandCount; band++)
            {
                var current = sample + (_coefficients[band] * previous[band]) - previousPrevious[band];
                previousPrevious[band] = previous[band];
                previous[band] = current;
            }
        }

        var rms = Math.Sqrt(sumSquares / frames);
        Volatile.Write(ref _hasSignal, rms >= 0.0005d ? 1 : 0);
        for (var band = 0; band < BandCount; band++)
        {
            var power = previous[band] * previous[band]
                + previousPrevious[band] * previousPrevious[band]
                - _coefficients[band] * previous[band] * previousPrevious[band];
            var amplitude = power <= 0
                ? 0d
                : Math.Sqrt(power) * 2d / frames;
            var level = (float)Math.Clamp(amplitude * 2.2d, 0d, 1d);
            var smoothed = (Volatile.Read(ref _levels[band]) * 0.55f) + (level * 0.45f);
            Volatile.Write(ref _levels[band], smoothed);
        }
    }

    public void CopyTo(Span<float> destination)
    {
        if (destination.Length < BandCount)
        {
            throw new ArgumentException($"频谱目标缓冲至少需要 {BandCount} 个元素。", nameof(destination));
        }

        for (var band = 0; band < BandCount; band++)
        {
            destination[band] = Volatile.Read(ref _levels[band]);
        }
    }
}
