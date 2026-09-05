using GpAutoLive.Contracts;

namespace GpAutoLive.App.Features.Effects;

/// <summary>
/// 当前周期由桌面端生成、只读投影到 UI 的视频结果。
/// 该快照不等于已提交给播放运行时的参数；提交状态由运行时消费者单独确认。
/// </summary>
public sealed record GeneratedVideoEffectSnapshot(
    int Generation,
    double BrightnessPercent,
    double ContrastPercent,
    double SaturationPercent,
    double HueRotationDegrees,
    double SharpnessPercent,
    double Gamma,
    double ExposurePercent,
    string NoiseReduction)
{
    /// <summary>自动生成的完整普通视频参数；旧 positional 构造仍使用正式默认值。</summary>
    public VideoEffectParams VideoParameters { get; init; } = VideoEffectParams.Default;

    /// <summary>自动生成的完整高级视觉参数；旧 positional 构造仍使用正式默认值。</summary>
    public AdvancedEffectParams AdvancedParameters { get; init; } = AdvancedEffectParams.Default;

    /// <summary>
    /// 将当前周期快照转换为正式视频参数契约。
    /// 保留旧 UI 字段作为兼容投影；Gamma、曝光和降噪文案没有对应的正式视频字段，
    /// 因此仍不映射，避免把 UI 展示值冒充为实际生效的媒体参数。
    /// </summary>
    public VideoEffectParams ToVideoEffectParams() =>
        VideoParameters with
        {
            BrightnessPercent = BrightnessPercent,
            ContrastPercent = ContrastPercent,
            SaturationPercent = SaturationPercent,
            HueRotationDegrees = HueRotationDegrees,
            SharpenPercent = SharpnessPercent,
        };

    /// <summary>将自动生成的高级视觉参数交给现有 GPU83 快照消费链。</summary>
    public AdvancedEffectParams ToAdvancedEffectParams() => AdvancedParameters;

    public static GeneratedVideoEffectSnapshot Create(Random? random = null, int generation = 1)
    {
        random ??= Random.Shared;
        var snapshot = new GeneratedVideoEffectSnapshot(
            Generation: generation,
            BrightnessPercent: random.Next(-6, 7),
            ContrastPercent: random.Next(96, 105),
            SaturationPercent: random.Next(96, 107),
            HueRotationDegrees: random.Next(-4, 5),
            SharpnessPercent: random.Next(0, 7),
            Gamma: Math.Round(0.96 + random.NextDouble() * 0.08, 2),
            ExposurePercent: random.Next(-2, 4),
            NoiseReduction: "关闭");

        static double InRange(Random source, double minimum, double maximum) =>
            minimum + source.NextDouble() * (maximum - minimum);

        static double Signed(Random source, double minimum, double maximum) =>
            (source.NextDouble() < 0.5 ? -1 : 1) * InRange(source, minimum, maximum);

        static double Rounded(double value, int digits) =>
            Math.Round(value, digits, MidpointRounding.ToEven);

        var video = VideoEffectParams.Default with
        {
            // These values are submitted through the existing manifest-gated GPU83 IPC path.
            BrightnessPercent = snapshot.BrightnessPercent,
            SaturationPercent = snapshot.SaturationPercent,
            BlurRadiusPx = Rounded(InRange(random, 0.01, 0.05), 3),
            ContrastPercent = snapshot.ContrastPercent,
            HueRotationDegrees = snapshot.HueRotationDegrees,
            SharpenPercent = snapshot.SharpnessPercent,
            NoisePercent = 1.0,
            DetailEnhancementPercent = Rounded(InRange(random, 0.1, 0.35), 3),
            CropEdgeSmoothing = Rounded(InRange(random, 0.7, 0.9), 3),
            FrameRateJitterPercent = Rounded(InRange(random, 0.01, 0.04), 3),
            FrameRatePerturbationFrequencyHz = Rounded(InRange(random, 0.05, 0.18), 3),
            FrameRatePerturbationAmplitudeFps = Rounded(InRange(random, 0.01, 0.04), 3),
            PixelScalePercent = Rounded(100.0 + Signed(random, 0.1, 0.2), 3),
            PixelJitterPx = Rounded(InRange(random, 0.125, 0.375), 3),
            DynamicCropPercent = Rounded(InRange(random, 0.03, 0.1), 3),
            FrameInnerPerturbationPercent = Rounded(InRange(random, 0.01, 0.04), 3),
            FrameInterPerturbationPercent = Rounded(InRange(random, 0.03, 0.1), 3),
            SpaceXOffsetPx = Signed(random, 0.5, 0.5),
            SpaceYOffsetPx = Signed(random, 0.5, 0.5),
            RotationDegrees = Rounded(Signed(random, 0.01, 0.04), 3),
            VignettePercent = Rounded(InRange(random, 0.05, 0.2), 3),
            HighlightsPercent = Rounded(Signed(random, 0.05, 0.2), 3),
            ShadowsPercent = Rounded(Signed(random, 0.05, 0.2), 3),
            RedChannelLockEnabled = true,
            EdgeSoftnessPercent = Rounded(InRange(random, 0.05, 0.2), 3),
            ImageRepairEnabled = true,
            ImageRepairStrengthPercent = Rounded(InRange(random, 0.05, 0.2), 3),
            FrameRateLockEnabled = true,
        };

        var bandWeights = MediaEffectContracts.VisualBandFrequenciesHz
            .Select((frequencyHz, index) =>
                new KeyValuePair<uint, double>(
                    frequencyHz,
                    Rounded(1.0 + (index % 2 == 0 ? -1 : 1) * InRange(random, 0.001, 0.003), 4)))
            .ToDictionary(pair => pair.Key, pair => pair.Value);

        var advanced = AdvancedEffectParams.Default with
        {
            // SliceMinLengthMs and PictureInPictureTimelineLocked intentionally keep their baseline values.
            BandWeights = bandWeights,
            TargetFrequencyHz = random.Next(65, 20_001),
            CoreFrequencyHz = random.Next(65, 20_001),
            WaveIntensity = Rounded(InRange(random, 0.084, 0.1), 4),
            WaveLevel = Rounded(InRange(random, 0.251, 0.3), 4),
            WaveGrainCount = (uint)random.Next(7, 14),
            DynamicEqThreshold = Rounded(InRange(random, 0.1, 0.3), 3),
            ChannelOffsetPercent = Rounded(Signed(random, 0.1, 0.3), 3),
            SpaceDimension = (byte)random.Next(2, 4),
            FrequencySpaceXOffsetPx = Rounded(Signed(random, 0.1, 0.3), 3),
            FrequencySpaceYOffsetPx = Rounded(Signed(random, 0.1, 0.3), 3),
            FramePerturbationProbabilityPercent = Rounded(InRange(random, 0.1, 0.3), 3),
            RandomGraphicOpacityPercent = Rounded(InRange(random, 0.8, 1.2), 3),
            RandomGraphicSizePx = Rounded(InRange(random, 1.0, 2.0), 3),
            AbstractFaceCount = 1,
            AbstractFaceSizePercent = Rounded(InRange(random, 1.0, 1.5), 3),
            AbstractFaceOpacityPercent = Rounded(InRange(random, 0.8, 1.2), 3),
            OverlayOffsetPx = Signed(random, 0.5, 0.5),
            SliceLengthMs = (ulong)(500 + random.Next(4) * 100),
            SliceTriggerIntervalMs = (ulong)(5_000 + random.Next(31) * 100),
            RandomGraphicEnabled = true,
            RandomGraphicCount = 1,
            PictureInPictureEnabled = true,
            PictureInPictureScalePercent = Rounded(InRange(random, 10.0, 12.0), 3),
            PictureInPictureOpacityPercent = Rounded(InRange(random, 0.8, 1.2), 3),
            PictureInPictureRotationDegrees = Rounded(Signed(random, 0.1, 0.2), 3),
            PictureInPicturePixelJitterPx = Rounded(InRange(random, 0.125, 0.375), 3),
            LocalBlurEnabled = true,
            LocalBlurRegionPercent = Rounded(InRange(random, 5.0, 7.0), 3),
            LocalBlurRadiusPx = Rounded(InRange(random, 0.1, 0.2), 3),
            LocalBlurIntervalMs = (ulong)(5_000 + random.Next(31) * 100),
            EdgeFillEnabled = true,
            EdgeFeatherPercent = Rounded(InRange(random, 0.1, 0.4), 3),
            TransformSmoothingEnabled = true,
            TransformSmoothingDurationMs = (ulong)(100 + random.Next(16) * 10),
            HighlightPerturbationEnabled = true,
            HighlightPerturbationIntervalMs = (ulong)(5_000 + random.Next(31) * 100),
            AsynchronousRotationEnabled = true,
            AsynchronousRotationMinDegrees = Rounded(-InRange(random, 0.1, 0.2), 3),
            AsynchronousRotationMaxDegrees = Rounded(InRange(random, 0.1, 0.2), 3),
        };

        return snapshot with
        {
            VideoParameters = video,
            AdvancedParameters = advanced,
        };
    }
}

/// <summary>当前周期由桌面端选择、只读投影到 UI 的普通声音结果。</summary>
public sealed record GeneratedAudioEffectSnapshot(
    int Generation,
    string PresetId,
    string ProcessingMode,
    double GainDb,
    double DynamicRangeDb,
    double LowEqDb,
    double MidEqDb,
    double HighEqDb,
    double PitchShiftSemitones,
    string Reverb,
    string Compression,
    string NoiseReduction,
    string Tone,
    double SpectralPerturbationPercent,
    double SpectrumBlindSpotPercent,
    bool HighFrequencyPerturbationEnabled,
    ulong HighFrequencyPerturbationIntervalMs,
    double HighFrequencyPerturbationStrengthPercent,
    double HighFrequencyPerturbationLevelDb)
{
    /// <summary>
    /// 将当前快照中已建模且已接入 C# 实时滤镜链的字段映射为正式声音参数。
    /// 动态范围、压缩和音色仍只用于只读展示，避免伪造未建模参数。
    /// </summary>
    public AudioEffectParams ToAudioEffectParams() =>
        new()
        {
            NaturalVoiceMode = string.Equals(ProcessingMode, "随机预设", StringComparison.Ordinal)
                ? NaturalVoiceMode.NaturalDynamic
                : NaturalVoiceMode.Original,
            RandomChangePeriodMs = 4_000,
            VoiceLibraryId = PresetId,
            LoudnessAdjustmentDb = GainDb,
            LowEqDb = LowEqDb,
            MidEqDb = MidEqDb,
            HighEqDb = HighEqDb,
            PitchShiftSemitones = PitchShiftSemitones,
            ReverbWetPercent = Reverb switch
            {
                "低" => 5,
                "中" => 10,
                "高" => 20,
                _ => 0,
            },
            NoiseReductionPercent = NoiseReduction switch
            {
                "低" => 20,
                "中" => 50,
                "高" => 80,
                _ => 0,
            },
            SpectralPerturbationPercent = SpectralPerturbationPercent,
            SpectrumBlindSpotPercent = SpectrumBlindSpotPercent,
            HighFrequencyPerturbationEnabled = HighFrequencyPerturbationEnabled,
            HighFrequencyPerturbationIntervalMs = HighFrequencyPerturbationIntervalMs,
            HighFrequencyPerturbationStrengthPercent = HighFrequencyPerturbationStrengthPercent,
            HighFrequencyPerturbationLevelDb = HighFrequencyPerturbationLevelDb,
            FilterQ = 1.0,
        };

    public static GeneratedAudioEffectSnapshot Create(Random? random = null, int generation = 1)
    {
        random ??= Random.Shared;
        return new GeneratedAudioEffectSnapshot(
            Generation: generation,
            PresetId: $"p{random.Next(1, 21):00}",
            ProcessingMode: "随机预设",
            GainDb: Math.Round(-1 + random.NextDouble() * 3, 1),
            DynamicRangeDb: Math.Round(6 + random.NextDouble() * 6, 1),
            LowEqDb: random.Next(-3, 4),
            MidEqDb: random.Next(-3, 4),
            HighEqDb: random.Next(-3, 4),
            PitchShiftSemitones: Math.Round(-0.5 + random.NextDouble(), 2),
            Reverb: "关闭",
            Compression: "轻度",
            NoiseReduction: "低",
            Tone: "明亮",
            SpectralPerturbationPercent: Math.Round(0.5 + random.NextDouble() * 1.5, 2),
            SpectrumBlindSpotPercent: Math.Round(0.5 + random.NextDouble() * 1.5, 2),
            HighFrequencyPerturbationEnabled: true,
            HighFrequencyPerturbationIntervalMs: (ulong)random.Next(8, 13) * 1_000,
            HighFrequencyPerturbationStrengthPercent: Math.Round(1 + random.NextDouble() * 3, 2),
            HighFrequencyPerturbationLevelDb: -32);
    }
}
