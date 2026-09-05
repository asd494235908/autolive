using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>高级视觉、固定频段、挂件和切片参数。</summary>
public sealed record AdvancedEffectParams
{
    /// <summary>固定视觉频段权重。</summary>
    [JsonPropertyName("band_weights")]
    public IReadOnlyDictionary<uint, double> BandWeights { get; init; } =
        CreateDefaultBandWeights();

    /// <summary>目标频率。</summary>
    [JsonPropertyName("target_frequency_hz")]
    public double? TargetFrequencyHz { get; init; }

    /// <summary>核心频率。</summary>
    [JsonPropertyName("core_frequency_hz")]
    public double? CoreFrequencyHz { get; init; }

    /// <summary>波形强度。</summary>
    [JsonPropertyName("wave_intensity")]
    public double WaveIntensity { get; init; }

    /// <summary>波形电平。</summary>
    [JsonPropertyName("wave_level")]
    public double WaveLevel { get; init; }

    /// <summary>波形粒子数量。</summary>
    [JsonPropertyName("wave_grain_count")]
    public uint WaveGrainCount { get; init; } = 20;

    /// <summary>动态均衡阈值。</summary>
    [JsonPropertyName("dynamic_eq_threshold")]
    public double DynamicEqThreshold { get; init; } = 10.0;

    /// <summary>通道偏移比例。</summary>
    [JsonPropertyName("channel_offset_percent")]
    public double ChannelOffsetPercent { get; init; }

    /// <summary>空间维度。</summary>
    [JsonPropertyName("space_dimension")]
    public byte SpaceDimension { get; init; } = 2;

    /// <summary>频率空间横向偏移。</summary>
    [JsonPropertyName("frequency_space_x_offset_px")]
    public double FrequencySpaceXOffsetPx { get; init; }

    /// <summary>频率空间纵向偏移。</summary>
    [JsonPropertyName("frequency_space_y_offset_px")]
    public double FrequencySpaceYOffsetPx { get; init; }

    /// <summary>帧扰动概率。</summary>
    [JsonPropertyName("frame_perturbation_probability_percent")]
    public double FramePerturbationProbabilityPercent { get; init; }

    /// <summary>随机图形透明度。</summary>
    [JsonPropertyName("random_graphic_opacity_percent")]
    public double RandomGraphicOpacityPercent { get; init; }

    /// <summary>随机图形尺寸。</summary>
    [JsonPropertyName("random_graphic_size_px")]
    public double RandomGraphicSizePx { get; init; } = 4.0;

    /// <summary>抽象面孔数量。</summary>
    [JsonPropertyName("abstract_face_count")]
    public byte AbstractFaceCount { get; init; }

    /// <summary>抽象面孔尺寸。</summary>
    [JsonPropertyName("abstract_face_size_percent")]
    public double AbstractFaceSizePercent { get; init; } = 2.0;

    /// <summary>抽象面孔透明度。</summary>
    [JsonPropertyName("abstract_face_opacity_percent")]
    public double AbstractFaceOpacityPercent { get; init; }

    /// <summary>叠加层偏移。</summary>
    [JsonPropertyName("overlay_offset_px")]
    public double OverlayOffsetPx { get; init; }

    /// <summary>切片长度。</summary>
    [JsonPropertyName("slice_length_ms")]
    public ulong SliceLengthMs { get; init; } = 5_000;

    /// <summary>切片最小长度。</summary>
    [JsonPropertyName("slice_min_length_ms")]
    public ulong SliceMinLengthMs { get; init; } = 10_000;

    /// <summary>切片触发间隔。</summary>
    [JsonPropertyName("slice_trigger_interval_ms")]
    public ulong SliceTriggerIntervalMs { get; init; } = 15_000;

    /// <summary>是否启用随机图形。</summary>
    [JsonPropertyName("random_graphic_enabled")]
    public bool RandomGraphicEnabled { get; init; }

    /// <summary>随机图形数量。</summary>
    [JsonPropertyName("random_graphic_count")]
    public byte RandomGraphicCount { get; init; } = 4;

    /// <summary>是否启用画中画。</summary>
    [JsonPropertyName("picture_in_picture_enabled")]
    public bool PictureInPictureEnabled { get; init; }

    /// <summary>画中画缩放比例。</summary>
    [JsonPropertyName("picture_in_picture_scale_percent")]
    public double PictureInPictureScalePercent { get; init; } = 24.0;

    /// <summary>画中画透明度。</summary>
    [JsonPropertyName("picture_in_picture_opacity_percent")]
    public double PictureInPictureOpacityPercent { get; init; } = 100.0;

    /// <summary>画中画旋转角度。</summary>
    [JsonPropertyName("picture_in_picture_rotation_degrees")]
    public double PictureInPictureRotationDegrees { get; init; }

    /// <summary>画中画像素抖动。</summary>
    [JsonPropertyName("picture_in_picture_pixel_jitter_px")]
    public double PictureInPicturePixelJitterPx { get; init; }

    /// <summary>画中画是否锁定时间轴。</summary>
    [JsonPropertyName("picture_in_picture_timeline_locked")]
    public bool PictureInPictureTimelineLocked { get; init; } = true;

    /// <summary>是否启用局部模糊。</summary>
    [JsonPropertyName("local_blur_enabled")]
    public bool LocalBlurEnabled { get; init; }

    /// <summary>局部模糊区域比例。</summary>
    [JsonPropertyName("local_blur_region_percent")]
    public double LocalBlurRegionPercent { get; init; } = 20.0;

    /// <summary>局部模糊半径。</summary>
    [JsonPropertyName("local_blur_radius_px")]
    public double LocalBlurRadiusPx { get; init; } = 2.0;

    /// <summary>局部模糊间隔。</summary>
    [JsonPropertyName("local_blur_interval_ms")]
    public ulong LocalBlurIntervalMs { get; init; } = 10_000;

    /// <summary>是否启用边缘填充。</summary>
    [JsonPropertyName("edge_fill_enabled")]
    public bool EdgeFillEnabled { get; init; }

    /// <summary>边缘羽化比例。</summary>
    [JsonPropertyName("edge_feather_percent")]
    public double EdgeFeatherPercent { get; init; }

    /// <summary>是否启用变换平滑。</summary>
    [JsonPropertyName("transform_smoothing_enabled")]
    public bool TransformSmoothingEnabled { get; init; }

    /// <summary>变换平滑时长。</summary>
    [JsonPropertyName("transform_smoothing_duration_ms")]
    public ulong TransformSmoothingDurationMs { get; init; } = 800;

    /// <summary>是否启用高光扰动。</summary>
    [JsonPropertyName("highlight_perturbation_enabled")]
    public bool HighlightPerturbationEnabled { get; init; }

    /// <summary>高光扰动间隔。</summary>
    [JsonPropertyName("highlight_perturbation_interval_ms")]
    public ulong HighlightPerturbationIntervalMs { get; init; } = 10_000;

    /// <summary>是否启用异步旋转。</summary>
    [JsonPropertyName("asynchronous_rotation_enabled")]
    public bool AsynchronousRotationEnabled { get; init; }

    /// <summary>异步旋转最小角度。</summary>
    [JsonPropertyName("asynchronous_rotation_min_degrees")]
    public double AsynchronousRotationMinDegrees { get; init; } = -1.0;

    /// <summary>异步旋转最大角度。</summary>
    [JsonPropertyName("asynchronous_rotation_max_degrees")]
    public double AsynchronousRotationMaxDegrees { get; init; } = 1.0;

    /// <summary>返回一份新的正式默认参数快照。</summary>
    public static AdvancedEffectParams Default => new();

    /// <summary>校验本组参数。</summary>
    public bool TryValidate(out IReadOnlyList<MediaEffectValidationError> errors) =>
        MediaEffectValidation.TryValidate(this, out errors);

    private static Dictionary<uint, double> CreateDefaultBandWeights() =>
        MediaEffectContracts.VisualBandFrequenciesHz.ToDictionary(frequencyHz => frequencyHz, _ => 1.0);
}
