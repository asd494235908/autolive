using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>普通视频效果参数。只描述配置和边界，不执行媒体处理。</summary>
public sealed record VideoEffectParams
{
    /// <summary>亮度偏移，单位为百分比，范围 -100–100。</summary>
    [JsonPropertyName("brightness_percent")]
    public double BrightnessPercent { get; init; }

    /// <summary>饱和度，单位为百分比，范围 0–200。</summary>
    [JsonPropertyName("saturation_percent")]
    public double SaturationPercent { get; init; } = 100.0;

    /// <summary>模糊半径，单位为像素，范围 0–8。</summary>
    [JsonPropertyName("blur_radius_px")]
    public double BlurRadiusPx { get; init; }

    /// <summary>对比度，单位为百分比，范围 0–200。</summary>
    [JsonPropertyName("contrast_percent")]
    public double ContrastPercent { get; init; } = 100.0;

    /// <summary>色相旋转，单位为度，范围 -180–180。</summary>
    [JsonPropertyName("hue_rotation_degrees")]
    public double HueRotationDegrees { get; init; }

    /// <summary>锐化强度，单位为百分比，范围 0–100。</summary>
    [JsonPropertyName("sharpen_percent")]
    public double SharpenPercent { get; init; }

    /// <summary>噪点强度，单位为百分比，范围 0–8。</summary>
    [JsonPropertyName("noise_percent")]
    public double NoisePercent { get; init; }

    /// <summary>细节增强强度，单位为百分比，范围 0–50。</summary>
    [JsonPropertyName("detail_enhancement_percent")]
    public double DetailEnhancementPercent { get; init; }

    /// <summary>裁剪边缘平滑度，范围 0–1。</summary>
    [JsonPropertyName("crop_edge_smoothing")]
    public double CropEdgeSmoothing { get; init; } = 0.5;

    /// <summary>帧率微扰幅度，单位为百分比，范围 0–2。</summary>
    [JsonPropertyName("frame_rate_jitter_percent")]
    public double FrameRateJitterPercent { get; init; }

    /// <summary>帧率微扰频率，单位为 Hz，范围 0.01–2。</summary>
    [JsonPropertyName("frame_rate_perturbation_frequency_hz")]
    public double FrameRatePerturbationFrequencyHz { get; init; } = 0.1;

    /// <summary>帧率微扰幅度，单位为 fps，范围 0–2。</summary>
    [JsonPropertyName("frame_rate_perturbation_amplitude_fps")]
    public double FrameRatePerturbationAmplitudeFps { get; init; }

    /// <summary>像素级缩放比例，单位为百分比，范围 95–105。</summary>
    [JsonPropertyName("pixel_scale_percent")]
    public double PixelScalePercent { get; init; } = 100.0;

    /// <summary>像素级扰动幅度，单位为像素，范围 0–2。</summary>
    [JsonPropertyName("pixel_jitter_px")]
    public double PixelJitterPx { get; init; }

    /// <summary>动态裁剪幅度，单位为每边百分比，范围 0–4。</summary>
    [JsonPropertyName("dynamic_crop_percent")]
    public double DynamicCropPercent { get; init; }

    /// <summary>帧内微扰幅度，单位为百分比，范围 0–2。</summary>
    [JsonPropertyName("frame_inner_perturbation_percent")]
    public double FrameInnerPerturbationPercent { get; init; }

    /// <summary>帧间微扰概率或幅度，单位为百分比，范围 0–20。</summary>
    [JsonPropertyName("frame_inter_perturbation_percent")]
    public double FrameInterPerturbationPercent { get; init; }

    /// <summary>水平空间偏移，单位为像素，范围 -4–4。</summary>
    [JsonPropertyName("space_x_offset_px")]
    public double SpaceXOffsetPx { get; init; }

    /// <summary>垂直空间偏移，单位为像素，范围 -4–4。</summary>
    [JsonPropertyName("space_y_offset_px")]
    public double SpaceYOffsetPx { get; init; }

    /// <summary>色域转换强度，单位为百分比，范围 0–100。</summary>
    [JsonPropertyName("color_space_conversion_strength_percent")]
    public double ColorSpaceConversionStrengthPercent { get; init; }

    /// <summary>是否启用色域转换。</summary>
    [JsonPropertyName("color_space_conversion_enabled")]
    public bool ColorSpaceConversionEnabled { get; init; }

    /// <summary>是否水平翻转。</summary>
    [JsonPropertyName("horizontal_flip_enabled")]
    public bool HorizontalFlipEnabled { get; init; }

    /// <summary>是否垂直翻转。</summary>
    [JsonPropertyName("vertical_flip_enabled")]
    public bool VerticalFlipEnabled { get; init; }

    /// <summary>画面旋转角度，单位为度，范围 -180–180。</summary>
    [JsonPropertyName("rotation_degrees")]
    public double RotationDegrees { get; init; }

    /// <summary>暗角强度，单位为百分比，范围 0–100。</summary>
    [JsonPropertyName("vignette_percent")]
    public double VignettePercent { get; init; }

    /// <summary>高光调整，单位为百分比，范围 -100–100。</summary>
    [JsonPropertyName("highlights_percent")]
    public double HighlightsPercent { get; init; }

    /// <summary>阴影调整，单位为百分比，范围 -100–100。</summary>
    [JsonPropertyName("shadows_percent")]
    public double ShadowsPercent { get; init; }

    /// <summary>是否锁定红色通道。</summary>
    [JsonPropertyName("red_channel_lock_enabled")]
    public bool RedChannelLockEnabled { get; init; }

    /// <summary>画面边缘柔化强度，单位为百分比，范围 0–100。</summary>
    [JsonPropertyName("edge_softness_percent")]
    public double EdgeSoftnessPercent { get; init; }

    /// <summary>是否启用图像修复。</summary>
    [JsonPropertyName("image_repair_enabled")]
    public bool ImageRepairEnabled { get; init; }

    /// <summary>图像修复强度，单位为百分比，范围 0–100。</summary>
    [JsonPropertyName("image_repair_strength_percent")]
    public double ImageRepairStrengthPercent { get; init; }

    /// <summary>是否锁定输出帧率到源平均帧率。</summary>
    [JsonPropertyName("frame_rate_lock_enabled")]
    public bool FrameRateLockEnabled { get; init; }

    /// <summary>返回一份新的正式默认参数快照。</summary>
    public static VideoEffectParams Default => new();

    /// <summary>校验本组参数。</summary>
    public bool TryValidate(out IReadOnlyList<MediaEffectValidationError> errors) =>
        MediaEffectValidation.TryValidate(this, out errors);
}
