namespace GpAutoLive.Media;

/// <summary>PCM 混音策略的稳定错误分类。</summary>
public enum AudioPcmMixFailureCode
{
    InvalidChannels,
    InvalidFrameShape,
    DestinationTooSmall,
    InvalidGain,
}

/// <summary>不携带音频正文的 PCM 混音错误。</summary>
public sealed record AudioPcmMixError(
    AudioPcmMixFailureCode Code,
    string Message);

/// <summary>
/// 单次有界 PCM 混音策略。ducking 以 dB 表示，静音优先于增益；所有输入均按有限值处理。
/// </summary>
public readonly record struct AudioPcmMixPolicy(
    double BaseGainDb = 0,
    double OverlayGainDb = 0,
    double BaseDuckingDb = 0,
    bool MuteBase = false,
    bool MuteOverlay = false);

/// <summary>
/// 无状态、无分配的交错 float32 PCM 混音器。调用方提供目标缓冲，避免音频回调扩容或生成临时数组。
/// </summary>
public static class AudioPcmMixer
{
    private const double MinGainDb = -120;
    private const double MaxGainDb = 24;

    /// <summary>
    /// 将两段可能不同长度的 PCM 混合到目标缓冲；缺失的一侧按静音处理，返回完整帧数。
    /// </summary>
    public static bool TryMix(
        ReadOnlySpan<float> basePcm,
        ReadOnlySpan<float> overlayPcm,
        Span<float> destination,
        int channels,
        AudioPcmMixPolicy policy,
        out int framesMixed,
        out AudioPcmMixError? error)
    {
        framesMixed = 0;
        error = null;
        if (channels is < 1 or > 8)
        {
            error = new(AudioPcmMixFailureCode.InvalidChannels, "PCM 混音声道数必须在 1 到 8 之间。");
            return false;
        }

        if (basePcm.Length % channels != 0 || overlayPcm.Length % channels != 0)
        {
            error = new(AudioPcmMixFailureCode.InvalidFrameShape, "PCM 混音输入必须完整对齐到交错声道帧。");
            return false;
        }

        if (!TryGain(policy.BaseGainDb, out var baseGain)
            || !TryGain(policy.OverlayGainDb, out var overlayGain)
            || !TryGain(policy.BaseDuckingDb, out var duckGain))
        {
            error = new(AudioPcmMixFailureCode.InvalidGain, "PCM 混音增益必须为有限的 -120 到 24 dB。");
            return false;
        }

        var baseFrames = basePcm.Length / channels;
        var overlayFrames = overlayPcm.Length / channels;
        var outputFrames = Math.Max(baseFrames, overlayFrames);
        if (destination.Length < checked(outputFrames * channels))
        {
            error = new(AudioPcmMixFailureCode.DestinationTooSmall, "PCM 混音目标缓冲容量不足。");
            return false;
        }

        var effectiveBaseGain = policy.MuteBase ? 0F : (float)(baseGain * duckGain);
        var effectiveOverlayGain = policy.MuteOverlay ? 0F : (float)overlayGain;
        MixLinearRamp(
            basePcm,
            overlayPcm,
            destination,
            channels,
            effectiveBaseGain,
            effectiveBaseGain,
            effectiveOverlayGain,
            effectiveOverlayGain,
            outputFrames);
        framesMixed = outputFrames;
        return true;
    }

    /// <summary>使用固定线性增益端点混合一段 PCM；供实时回调做无分配 attack/release 过渡。</summary>
    internal static void MixLinearRamp(
        ReadOnlySpan<float> basePcm,
        ReadOnlySpan<float> overlayPcm,
        Span<float> destination,
        int channels,
        float baseStartGain,
        float baseEndGain,
        float overlayStartGain,
        float overlayEndGain,
        int outputFrames)
    {
        if (outputFrames <= 0)
        {
            return;
        }

        for (var frameIndex = 0; frameIndex < outputFrames; frameIndex++)
        {
            var progress = outputFrames == 1 ? 1F : (float)frameIndex / (outputFrames - 1);
            var baseGain = baseStartGain + ((baseEndGain - baseStartGain) * progress);
            var overlayGain = overlayStartGain + ((overlayEndGain - overlayStartGain) * progress);
            var sampleOffset = frameIndex * channels;
            for (var channelIndex = 0; channelIndex < channels; channelIndex++)
            {
                var sampleIndex = sampleOffset + channelIndex;
                var baseSample = sampleIndex < basePcm.Length ? Sanitize(basePcm[sampleIndex]) : 0F;
                var overlaySample = sampleIndex < overlayPcm.Length ? Sanitize(overlayPcm[sampleIndex]) : 0F;
                destination[sampleIndex] = Math.Clamp(
                    baseSample * baseGain + overlaySample * overlayGain,
                    -1F,
                    1F);
            }
        }
    }

    /// <summary>将 dB 增益转换为有限线性增益。</summary>
    public static bool TryGetLinearGain(double decibels, out float gain)
    {
        if (!TryGain(decibels, out var value))
        {
            gain = 0;
            return false;
        }

        gain = (float)value;
        return true;
    }

    /// <summary>
    /// 对一段基础媒体 PCM 原地应用静音/duck/增益策略。调用方提供已分配的缓冲，
    /// 不创建临时数组；用于解码线程在写入最终总线前收敛共享音频优先级。
    /// </summary>
    public static bool TryApplyBasePolicy(
        Span<float> samples,
        int channels,
        AudioPcmMixPolicy policy,
        out AudioPcmMixError? error)
    {
        error = null;
        if (channels is < 1 or > 8)
        {
            error = new(AudioPcmMixFailureCode.InvalidChannels, "PCM 混音声道数必须在 1 到 8 之间。");
            return false;
        }

        if (samples.Length % channels != 0)
        {
            error = new(AudioPcmMixFailureCode.InvalidFrameShape, "PCM 混音输入必须完整对齐到交错声道帧。");
            return false;
        }

        if (!TryGain(policy.BaseGainDb, out var baseGain)
            || !TryGain(policy.BaseDuckingDb, out var duckGain))
        {
            error = new(AudioPcmMixFailureCode.InvalidGain, "PCM 混音增益必须为有限的 -120 到 24 dB。");
            return false;
        }

        var effectiveGain = policy.MuteBase ? 0F : (float)(baseGain * duckGain);
        for (var sampleIndex = 0; sampleIndex < samples.Length; sampleIndex++)
        {
            samples[sampleIndex] = Math.Clamp(Sanitize(samples[sampleIndex]) * effectiveGain, -1F, 1F);
        }

        return true;
    }

    private static bool TryGain(double decibels, out double gain)
    {
        if (!double.IsFinite(decibels) || decibels is < MinGainDb or > MaxGainDb)
        {
            gain = 0;
            return false;
        }

        gain = Math.Pow(10, decibels / 20);
        return double.IsFinite(gain) && gain >= 0;
    }

    private static float Sanitize(float sample) => float.IsFinite(sample) ? sample : 0F;
}
