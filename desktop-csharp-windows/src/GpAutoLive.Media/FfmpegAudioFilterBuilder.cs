using System.Globalization;
using GpAutoLive.Contracts;

namespace GpAutoLive.Media;

/// <summary>
/// 将已校验的普通声音参数转换为 FFmpeg 的受管音频滤镜链。
/// 当前实时 PCM 路径复用 Rust 已验证的 FFmpeg 原生子集，并消费自然动态模式和
/// 本地音色预设；需要完整输入缓存或专用 DSP 的字段不会被伪装成已生效。
/// </summary>
public static class FfmpegAudioFilterBuilder
{
    /// <summary>
    /// 根据参数生成安全的 FFmpeg 音频滤镜；没有已接入变化时返回空字符串。
    /// 末尾淡出只有在调用方提供可靠源时长时才加入。
    /// </summary>
    public static string Create(
        AudioEffectParams parameters,
        ulong? sourceDurationMs = null,
        int sampleRateHz = FfmpegPcmDecodePlanBuilder.DefaultSampleRateHz)
    {
        ArgumentNullException.ThrowIfNull(parameters);

        var filters = new List<string>(capacity: 14);
        var totalGainDb = parameters.InputGainDb
            + parameters.OutputGainDb
            + parameters.LoudnessAdjustmentDb;
        if (Math.Abs(totalGainDb) > 0.001)
        {
            filters.Add($"volume={Format(totalGainDb)}dB");
        }

        if (parameters.NaturalVoiceMode is NaturalVoiceMode.NaturalDynamic)
        {
            var periodSeconds = parameters.RandomChangePeriodMs / 1_000.0;
            filters.Add(
                $"volume='1+0.012000*sin(2*PI*t/{FormatFixed(periodSeconds)})':eval=frame");
        }

        if (parameters.VoiceLibraryId is { Length: > 0 } voiceLibraryId)
        {
            var preset = CreateLocalVoicePreset(voiceLibraryId);
            filters.Add(
                $"equalizer=f={FormatFixed(preset.CenterFrequencyHz)}:t=q:w={FormatFixed(preset.Q)}:g={FormatFixed(preset.GainDb)}");
        }

        if (parameters.SpectralPerturbationPercent > 0.001)
        {
            filters.Add(CreateSpectralPerturbationFilter(parameters.SpectralPerturbationPercent));
        }

        if (parameters.HighFrequencyPerturbationEnabled
            && parameters.HighFrequencyPerturbationStrengthPercent > 0.001)
        {
            filters.Add(CreateHighFrequencyPerturbationFilter(parameters));
        }

        if (parameters.SpectrumBlindSpotPercent > 0.001)
        {
            var bandwidthHz = 20_000.0 * parameters.SpectrumBlindSpotPercent / 100.0;
            filters.Add(
                $"bandreject=f=8000.000000:t=h:w={FormatFixed(bandwidthHz)}");
        }

        AddEqualizer(filters, 200, parameters.LowEqDb, parameters.FilterQ);
        AddEqualizer(filters, 1_000, parameters.MidEqDb, parameters.FilterQ);
        AddEqualizer(filters, 8_000, parameters.HighEqDb, parameters.FilterQ);

        if (Math.Abs(parameters.PitchShiftSemitones) > 0.001)
        {
            var pitchRatio = Math.Pow(2, parameters.PitchShiftSemitones / 12.0);
            var shiftedRate = Math.Clamp(
                (int)Math.Round(sampleRateHz * pitchRatio, MidpointRounding.AwayFromZero),
                8_000,
                384_000);
            filters.Add($"asetrate={shiftedRate}");
            filters.Add($"aresample={sampleRateHz}");
            filters.Add($"atempo={Format(1.0 / pitchRatio)}");
        }

        if (Math.Abs(parameters.PlaybackSpeed - 1.0) > 0.001)
        {
            filters.Add($"atempo={Format(parameters.PlaybackSpeed)}");
        }

        if (parameters.FadeInMs > 0)
        {
            filters.Add($"afade=t=in:st=0:d={Format(parameters.FadeInMs / 1_000.0)}");
        }

        if (parameters.FadeOutMs > 0
            && sourceDurationMs is ulong durationMs
            && durationMs > 0)
        {
            var fadeOutMs = Math.Min(parameters.FadeOutMs, durationMs);
            var fadeOutStartMs = durationMs - fadeOutMs;
            filters.Add(
                $"afade=t=out:st={Format(fadeOutStartMs / 1_000.0)}:d={Format(fadeOutMs / 1_000.0)}");
        }

        if (parameters.ReverbWetPercent > 0.001)
        {
            var decay = Math.Clamp(parameters.ReverbWetPercent / 100.0, 0, 1);
            filters.Add($"aecho=1:1:80:{Format(decay)}");
        }

        if (parameters.NoiseReductionPercent > 0.001)
        {
            var reductionDb = Math.Clamp(parameters.NoiseReductionPercent * 0.97, 0.01, 97);
            filters.Add($"afftdn=nr={Format(reductionDb)}");
        }

        if (Math.Abs(parameters.PhasePerturbationPercent) > 0.001)
        {
            var magnitude = Math.Abs(parameters.PhasePerturbationPercent);
            var depth = Math.Clamp(magnitude / 20.0, 0, 0.99);
            var inputGain = magnitude <= 1.0 ? "1" : "0.4";
            var outputGain = magnitude <= 1.0 ? "1" : "0.74";
            filters.Add(
                $"aphaser=in_gain={inputGain}:out_gain={outputGain}:delay=3:decay={Format(depth)}:speed=0.5");
        }

        if (parameters.VibratoDepthPercent > 0.001)
        {
            var depth = Math.Clamp(parameters.VibratoDepthPercent / 100.0, 0, 1);
            filters.Add($"vibrato=f={Format(parameters.VibratoFrequencyHz)}:d={Format(depth)}");
        }

        // 与 Rust realtime=true 的边界保持一致：需要整轮反向缓冲的复杂变换，
        // 不能直接放进实时 PCM 管道，否则会造成启动静音或音画时钟漂移。
        return string.Join(',', filters);
    }

    private static string CreateSpectralPerturbationFilter(double percent)
    {
        var amplitude = percent / 100.0;
        var scale = $"1+{FormatFixed(amplitude)}*sin(2*PI*b/nb*7+ch*PI/3)";
        return
            $"afftfilt=real='re*({scale})':imag='im*({scale})':win_size=4096:win_func=hann:overlap=0.75";
    }

    private static string CreateHighFrequencyPerturbationFilter(AudioEffectParams parameters)
    {
        var intervalSeconds = parameters.HighFrequencyPerturbationIntervalMs / 1_000.0;
        var level = Math.Pow(10, parameters.HighFrequencyPerturbationLevelDb / 20.0);
        var amplitude = parameters.HighFrequencyPerturbationStrengthPercent / 100.0 * level;
        var scale =
            $"1+gte(b/nb\\,0.25)*{amplitude.ToString("0.00000000", CultureInfo.InvariantCulture)}*sin(2*PI*b/nb*11+ch*PI/5)";
        var activeSeconds = Math.Max(intervalSeconds / 2.0, 0.25);
        return
            $"afftfilt=real='re*({scale})':imag='im*({scale})':win_size=4096:win_func=hann:overlap=0.75:enable='lt(mod(t\\,{FormatFixed(intervalSeconds)})\\,{FormatFixed(activeSeconds)})'";
    }

    private static void AddEqualizer(List<string> filters, int frequencyHz, double gainDb, double q)
    {
        if (Math.Abs(gainDb) <= 0.001)
        {
            return;
        }

        filters.Add($"equalizer=f={frequencyHz}:t=q:w={Format(q)}:g={Format(gainDb)}");
    }

    private static LocalVoicePreset CreateLocalVoicePreset(string voiceLibraryId)
    {
        ReadOnlySpan<double> centerFrequenciesHz =
        [180.0, 260.0, 420.0, 700.0, 1_100.0, 1_700.0, 2_600.0, 3_600.0];
        ulong seed = 0xcbf2_9ce4_8422_2325;
        foreach (var value in System.Text.Encoding.UTF8.GetBytes(voiceLibraryId))
        {
            seed = (seed ^ value) * 0x0000_0100_0000_01b3;
        }

        var centerFrequencyHz = centerFrequenciesHz[(int)(seed % (ulong)centerFrequenciesHz.Length)];
        var magnitudeDb = 0.5 + (seed >> 8) % 5 * 0.25;
        var gainDb = (seed & 1) == 0 ? magnitudeDb : -magnitudeDb;
        var q = 0.8 + (seed >> 16) % 5 * 0.2;
        return new(centerFrequencyHz, gainDb, q);
    }

    private static string Format(double value) =>
        value.ToString("0.###", CultureInfo.InvariantCulture);

    private static string FormatFixed(double value) =>
        value.ToString("0.000000", CultureInfo.InvariantCulture);

    private readonly record struct LocalVoicePreset(
        double CenterFrequencyHz,
        double GainDb,
        double Q);
}
