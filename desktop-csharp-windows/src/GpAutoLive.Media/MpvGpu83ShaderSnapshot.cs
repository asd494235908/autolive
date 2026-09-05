using System.Collections.Immutable;
using System.Globalization;
using GpAutoLive.Contracts;

namespace GpAutoLive.Media;

public enum MpvGpu83ExecutionClass
{
    PixelShader,
    CompositeShader,
    FrameScheduling,
}

public enum MpvGpu83ParameterCapability
{
    ShaderParameter,
    ScheduledParameter,
    Unavailable,
}

public sealed record MpvGpu83ShaderParameterEntry(
    string FieldPath,
    string ShaderOption,
    MpvGpu83ExecutionClass ExecutionClass,
    MpvGpu83ParameterCapability Capability,
    double? Value);

/// <summary>
/// C# 对 Rust GPU83 83 项媒体契约的只读映射结果。
/// 静态像素/合成参数进入一个受限 glsl-shader-opts 快照；调度参数进入同一快照的
/// PTS 运行时输入；需要历史纹理或尚未验证算法的字段显式保留为不可用。
/// </summary>
public sealed record MpvGpu83ShaderSnapshot(
    ImmutableArray<MpvGpu83ShaderParameterEntry> Entries,
    ImmutableArray<string> UnavailableFields,
    MpvShaderOptionsSnapshot ShaderOptions)
{
    public const int ContractParameterCount = 83;

    /// <summary>
    /// 当前 C# 外置完整 shader 的固定 SHA-256。旧基线 shader 仍可用于四项颜色参数，
    /// 但不能被误判为支持完整 GPU83 选项。
    /// </summary>
    public const string FullShaderSha256 =
        "d75917392fc682ff49ec2563d9d16eedfa565b5b8c0cb366b172204c128e9b61";

    public static bool IsFullShaderResource(VerifiedRuntimeResource? resource) =>
        resource is not null
        && string.Equals(resource.Name, "gpu83.hook", StringComparison.OrdinalIgnoreCase)
        && string.Equals(resource.Sha256, FullShaderSha256, StringComparison.OrdinalIgnoreCase);

    public bool IsFullyAvailable => UnavailableFields.IsEmpty;

    public static bool TryCreate(
        VideoEffectParams? video,
        AdvancedEffectParams? advanced,
        double sourceFps,
        double epochStartSeconds,
        uint randomSeed,
        out MpvGpu83ShaderSnapshot? snapshot,
        out MpvVideoParameterError? error)
    {
        snapshot = null;
        error = null;
        if (video is null || advanced is null)
        {
            error = new(
                MpvVideoParameterFailureCode.InvalidShaderOptions,
                "GPU83 视频或高级参数不能为空。");
            return false;
        }

        if (!video.TryValidate(out var videoErrors) || videoErrors.Count > 0
            || !advanced.TryValidate(out var advancedErrors) || advancedErrors.Count > 0)
        {
            error = new(
                MpvVideoParameterFailureCode.InvalidShaderOptions,
                "GPU83 参数超出媒体参数契约范围。");
            return false;
        }

        if (!double.IsFinite(sourceFps) || sourceFps is < 1 or > 240
            || !double.IsFinite(epochStartSeconds) || epochStartSeconds < 0
            || randomSeed > 16_777_215)
        {
            error = new(
                MpvVideoParameterFailureCode.InvalidShaderOptions,
                "GPU83 调度输入无效。");
            return false;
        }

        var entries = ImmutableArray.CreateBuilder<MpvGpu83ShaderParameterEntry>(ContractParameterCount);
        var unavailable = ImmutableArray.CreateBuilder<string>();
        var shaderEntries = new List<KeyValuePair<string, string>>(capacity: 64);
        foreach (var descriptor in Descriptors)
        {
            var value = descriptor.Read(video, advanced);
            if (descriptor.Required && value is null)
            {
                error = new(
                    MpvVideoParameterFailureCode.InvalidShaderOptions,
                    $"GPU83 参数缺少值：{descriptor.FieldPath}。");
                return false;
            }

            if (value is double numeric && !double.IsFinite(numeric))
            {
                error = new(
                    MpvVideoParameterFailureCode.InvalidShaderOptions,
                    "GPU83 参数包含非有限数值。");
                return false;
            }

            if (descriptor.Capability is MpvGpu83ParameterCapability.ShaderParameter)
            {
                shaderEntries.Add(new(descriptor.ShaderOption, Format(value ?? 0)));
            }
            else if (descriptor.Capability is MpvGpu83ParameterCapability.Unavailable)
            {
                unavailable.Add(descriptor.FieldPath);
            }

            entries.Add(new(
                descriptor.FieldPath,
                descriptor.ShaderOption,
                descriptor.ExecutionClass,
                descriptor.Capability,
                value));
        }

        if (entries.Count != ContractParameterCount)
        {
            error = new(
                MpvVideoParameterFailureCode.InvalidShaderOptions,
                "GPU83 参数映射表数量不符合契约。");
            return false;
        }

        shaderEntries.Add(new("al_runtime_epoch_start_seconds", Format(epochStartSeconds)));
        shaderEntries.Add(new("al_runtime_source_fps", Format(sourceFps)));
        shaderEntries.Add(new("al_runtime_random_seed", randomSeed.ToString(CultureInfo.InvariantCulture)));
        shaderEntries.Add(new("al_runtime_frame_inner_percent", Format(video.FrameInnerPerturbationPercent)));
        shaderEntries.Add(new("al_runtime_frame_inter_percent", Format(video.FrameInterPerturbationPercent)));
        shaderEntries.Add(new("al_runtime_frame_probability_percent", Format(advanced.FramePerturbationProbabilityPercent)));
        shaderEntries.Add(new("al_runtime_slice_length_seconds", Format(advanced.SliceLengthMs / 1_000.0)));
        shaderEntries.Add(new("al_runtime_slice_interval_seconds", Format(advanced.SliceTriggerIntervalMs / 1_000.0)));
        shaderEntries.Add(new("al_runtime_pip_jitter_px", Format(advanced.PictureInPicturePixelJitterPx)));
        shaderEntries.Add(new("al_runtime_local_blur_interval_seconds", Format(advanced.LocalBlurIntervalMs / 1_000.0)));
        shaderEntries.Add(new("al_runtime_smoothing_enabled", Format(advanced.TransformSmoothingEnabled ? 1 : 0)));
        shaderEntries.Add(new("al_runtime_smoothing_seconds", Format(advanced.TransformSmoothingDurationMs / 1_000.0)));
        shaderEntries.Add(new("al_runtime_highlight_enabled", Format(advanced.HighlightPerturbationEnabled ? 1 : 0)));
        shaderEntries.Add(new("al_runtime_highlight_interval_seconds", Format(advanced.HighlightPerturbationIntervalMs / 1_000.0)));
        shaderEntries.Add(new("al_runtime_async_rotation_enabled", Format(advanced.AsynchronousRotationEnabled ? 1 : 0)));
        shaderEntries.Add(new("al_runtime_async_rotation_min_degrees", Format(advanced.AsynchronousRotationMinDegrees)));
        shaderEntries.Add(new("al_runtime_async_rotation_max_degrees", Format(advanced.AsynchronousRotationMaxDegrees)));

        if (!MpvShaderOptionsSnapshot.TryCreate(shaderEntries, out var options, out error)
            || options is null)
        {
            return false;
        }

        snapshot = new MpvGpu83ShaderSnapshot(
            entries.ToImmutable(),
            unavailable.ToImmutable(),
            options);
        return true;
    }

    private sealed record Descriptor(
        string FieldPath,
        string ShaderOption,
        MpvGpu83ExecutionClass ExecutionClass,
        MpvGpu83ParameterCapability Capability,
        bool Required,
        Func<VideoEffectParams, AdvancedEffectParams, double?> Read);

    private static readonly Descriptor[] Descriptors =
    [
        Video("video.brightness_percent", "al_brightness_percent", MpvGpu83ExecutionClass.PixelShader, v => v.BrightnessPercent),
        Video("video.saturation_percent", "al_saturation_percent", MpvGpu83ExecutionClass.PixelShader, v => v.SaturationPercent),
        Video("video.blur_radius_px", "al_blur_radius_px", MpvGpu83ExecutionClass.PixelShader, v => v.BlurRadiusPx),
        Video("video.contrast_percent", "al_contrast_percent", MpvGpu83ExecutionClass.PixelShader, v => v.ContrastPercent),
        Video("video.hue_rotation_degrees", "al_hue_degrees", MpvGpu83ExecutionClass.PixelShader, v => v.HueRotationDegrees),
        Video("video.sharpen_percent", "al_sharpen_percent", MpvGpu83ExecutionClass.PixelShader, v => v.SharpenPercent),
        Video("video.noise_percent", "al_noise_percent", MpvGpu83ExecutionClass.PixelShader, v => v.NoisePercent),
        Video("video.detail_enhancement_percent", "al_detail_percent", MpvGpu83ExecutionClass.PixelShader, v => v.DetailEnhancementPercent),
        Video("video.crop_edge_smoothing", "al_crop_edge_smoothing", MpvGpu83ExecutionClass.PixelShader, v => v.CropEdgeSmoothing),
        VideoUnavailableScheduled("video.frame_rate_jitter_percent", "al_frame_rate_jitter_percent", v => v.FrameRateJitterPercent),
        VideoUnavailableScheduled("video.frame_rate_perturbation_frequency_hz", "al_frame_rate_frequency_hz", v => v.FrameRatePerturbationFrequencyHz),
        VideoUnavailableScheduled("video.frame_rate_perturbation_amplitude_fps", "al_frame_rate_amplitude_fps", v => v.FrameRatePerturbationAmplitudeFps),
        Video("video.pixel_scale_percent", "al_pixel_scale_percent", MpvGpu83ExecutionClass.PixelShader, v => v.PixelScalePercent),
        Video("video.pixel_jitter_px", "al_pixel_jitter_px", MpvGpu83ExecutionClass.PixelShader, v => v.PixelJitterPx),
        Video("video.dynamic_crop_percent", "al_dynamic_crop_percent", MpvGpu83ExecutionClass.PixelShader, v => v.DynamicCropPercent),
        VideoScheduled("video.frame_inner_perturbation_percent", "al_frame_inner_percent", v => v.FrameInnerPerturbationPercent),
        VideoScheduled("video.frame_inter_perturbation_percent", "al_frame_inter_percent", v => v.FrameInterPerturbationPercent),
        Video("video.space_x_offset_px", "al_space_x_px", MpvGpu83ExecutionClass.PixelShader, v => v.SpaceXOffsetPx),
        Video("video.space_y_offset_px", "al_space_y_px", MpvGpu83ExecutionClass.PixelShader, v => v.SpaceYOffsetPx),
        VideoUnavailable("video.color_space_conversion_strength_percent", "al_color_space_strength_percent", v => v.ColorSpaceConversionStrengthPercent),
        VideoUnavailable("video.color_space_conversion_enabled", "al_color_space_enabled", v => v.ColorSpaceConversionEnabled ? 1 : 0),
        Video("video.rotation_degrees", "al_rotation_degrees", MpvGpu83ExecutionClass.PixelShader, v => v.RotationDegrees),
        Video("video.vignette_percent", "al_vignette_percent", MpvGpu83ExecutionClass.PixelShader, v => v.VignettePercent),
        Video("video.highlights_percent", "al_highlights_percent", MpvGpu83ExecutionClass.PixelShader, v => v.HighlightsPercent),
        Video("video.shadows_percent", "al_shadows_percent", MpvGpu83ExecutionClass.PixelShader, v => v.ShadowsPercent),
        Video("video.red_channel_lock_enabled", "al_red_lock_enabled", MpvGpu83ExecutionClass.PixelShader, v => v.RedChannelLockEnabled ? 1 : 0),
        Video("video.edge_softness_percent", "al_edge_softness_percent", MpvGpu83ExecutionClass.PixelShader, v => v.EdgeSoftnessPercent),
        Video("video.image_repair_enabled", "al_image_repair_enabled", MpvGpu83ExecutionClass.PixelShader, v => v.ImageRepairEnabled ? 1 : 0),
        Video("video.image_repair_strength_percent", "al_image_repair_strength_percent", MpvGpu83ExecutionClass.PixelShader, v => v.ImageRepairStrengthPercent),
        VideoUnavailableScheduled("video.frame_rate_lock_enabled", "al_frame_rate_lock_enabled", v => v.FrameRateLockEnabled ? 1 : 0),
        AdvancedOptional("advanced.target_frequency_hz", "al_target_frequency_hz", MpvGpu83ExecutionClass.PixelShader, (v, a) => a.TargetFrequencyHz),
        AdvancedOptional("advanced.core_frequency_hz", "al_core_frequency_hz", MpvGpu83ExecutionClass.PixelShader, (v, a) => a.CoreFrequencyHz),
        Advanced("advanced.wave_intensity", "al_wave_intensity", MpvGpu83ExecutionClass.PixelShader, a => a.WaveIntensity),
        Advanced("advanced.wave_level", "al_wave_level", MpvGpu83ExecutionClass.PixelShader, a => a.WaveLevel),
        Advanced("advanced.wave_grain_count", "al_wave_grain_count", MpvGpu83ExecutionClass.PixelShader, a => a.WaveGrainCount),
        Advanced("advanced.dynamic_eq_threshold", "al_dynamic_eq_threshold", MpvGpu83ExecutionClass.PixelShader, a => a.DynamicEqThreshold),
        Advanced("advanced.channel_offset_percent", "al_channel_offset_percent", MpvGpu83ExecutionClass.PixelShader, a => a.ChannelOffsetPercent),
        Advanced("advanced.space_dimension", "al_space_dimension", MpvGpu83ExecutionClass.PixelShader, a => a.SpaceDimension),
        Advanced("advanced.frequency_space_x_offset_px", "al_frequency_space_x_px", MpvGpu83ExecutionClass.PixelShader, a => a.FrequencySpaceXOffsetPx),
        Advanced("advanced.frequency_space_y_offset_px", "al_frequency_space_y_px", MpvGpu83ExecutionClass.PixelShader, a => a.FrequencySpaceYOffsetPx),
        AdvancedScheduled("advanced.frame_perturbation_probability_percent", "al_frame_probability_percent", a => a.FramePerturbationProbabilityPercent),
        Advanced("advanced.random_graphic_opacity_percent", "al_random_graphic_opacity_percent", MpvGpu83ExecutionClass.CompositeShader, a => a.RandomGraphicOpacityPercent),
        Advanced("advanced.random_graphic_size_px", "al_random_graphic_size_px", MpvGpu83ExecutionClass.CompositeShader, a => a.RandomGraphicSizePx),
        Advanced("advanced.abstract_face_count", "al_abstract_face_count", MpvGpu83ExecutionClass.CompositeShader, a => a.AbstractFaceCount),
        Advanced("advanced.abstract_face_size_percent", "al_abstract_face_size_percent", MpvGpu83ExecutionClass.CompositeShader, a => a.AbstractFaceSizePercent),
        Advanced("advanced.abstract_face_opacity_percent", "al_abstract_face_opacity_percent", MpvGpu83ExecutionClass.CompositeShader, a => a.AbstractFaceOpacityPercent),
        Advanced("advanced.overlay_offset_px", "al_overlay_offset_px", MpvGpu83ExecutionClass.CompositeShader, a => a.OverlayOffsetPx),
        AdvancedScheduled("advanced.slice_length_ms", "al_slice_length_ms", a => a.SliceLengthMs),
        AdvancedUnavailable("advanced.slice_min_length_ms", "al_slice_min_length_ms", MpvGpu83ExecutionClass.FrameScheduling, a => a.SliceMinLengthMs),
        AdvancedScheduled("advanced.slice_trigger_interval_ms", "al_slice_interval_ms", a => a.SliceTriggerIntervalMs),
        Advanced("advanced.random_graphic_enabled", "al_random_graphic_enabled", MpvGpu83ExecutionClass.CompositeShader, a => a.RandomGraphicEnabled ? 1 : 0),
        Advanced("advanced.random_graphic_count", "al_random_graphic_count", MpvGpu83ExecutionClass.CompositeShader, a => a.RandomGraphicCount),
        Advanced("advanced.picture_in_picture_enabled", "al_pip_enabled", MpvGpu83ExecutionClass.CompositeShader, a => a.PictureInPictureEnabled ? 1 : 0),
        Advanced("advanced.picture_in_picture_scale_percent", "al_pip_scale_percent", MpvGpu83ExecutionClass.CompositeShader, a => a.PictureInPictureScalePercent),
        Advanced("advanced.picture_in_picture_opacity_percent", "al_pip_opacity_percent", MpvGpu83ExecutionClass.CompositeShader, a => a.PictureInPictureOpacityPercent),
        Advanced("advanced.picture_in_picture_rotation_degrees", "al_pip_rotation_degrees", MpvGpu83ExecutionClass.CompositeShader, a => a.PictureInPictureRotationDegrees),
        AdvancedCompositeScheduled("advanced.picture_in_picture_pixel_jitter_px", "al_pip_jitter_px", a => a.PictureInPicturePixelJitterPx),
        AdvancedUnavailable("advanced.picture_in_picture_timeline_locked", "al_pip_timeline_locked", MpvGpu83ExecutionClass.FrameScheduling, a => a.PictureInPictureTimelineLocked ? 1 : 0),
        Advanced("advanced.local_blur_enabled", "al_local_blur_enabled", MpvGpu83ExecutionClass.CompositeShader, a => a.LocalBlurEnabled ? 1 : 0),
        Advanced("advanced.local_blur_region_percent", "al_local_blur_region_percent", MpvGpu83ExecutionClass.CompositeShader, a => a.LocalBlurRegionPercent),
        Advanced("advanced.local_blur_radius_px", "al_local_blur_radius_px", MpvGpu83ExecutionClass.CompositeShader, a => a.LocalBlurRadiusPx),
        AdvancedScheduled("advanced.local_blur_interval_ms", "al_local_blur_interval_ms", a => a.LocalBlurIntervalMs),
        Advanced("advanced.edge_fill_enabled", "al_edge_fill_enabled", MpvGpu83ExecutionClass.CompositeShader, a => a.EdgeFillEnabled ? 1 : 0),
        Advanced("advanced.edge_feather_percent", "al_edge_feather_percent", MpvGpu83ExecutionClass.CompositeShader, a => a.EdgeFeatherPercent),
        AdvancedScheduled("advanced.transform_smoothing_enabled", "al_transform_smoothing_enabled", a => a.TransformSmoothingEnabled ? 1 : 0),
        AdvancedScheduled("advanced.transform_smoothing_duration_ms", "al_transform_smoothing_ms", a => a.TransformSmoothingDurationMs),
        AdvancedScheduled("advanced.highlight_perturbation_enabled", "al_highlight_perturbation_enabled", a => a.HighlightPerturbationEnabled ? 1 : 0),
        AdvancedScheduled("advanced.highlight_perturbation_interval_ms", "al_highlight_interval_ms", a => a.HighlightPerturbationIntervalMs),
        AdvancedScheduled("advanced.asynchronous_rotation_enabled", "al_async_rotation_enabled", a => a.AsynchronousRotationEnabled ? 1 : 0),
        AdvancedScheduled("advanced.asynchronous_rotation_min_degrees", "al_async_rotation_min_degrees", a => a.AsynchronousRotationMinDegrees),
        AdvancedScheduled("advanced.asynchronous_rotation_max_degrees", "al_async_rotation_max_degrees", a => a.AsynchronousRotationMaxDegrees),
        Band(65, "advanced.band_weights.65", "al_band_65"),
        Band(92, "advanced.band_weights.92", "al_band_92"),
        Band(131, "advanced.band_weights.131", "al_band_131"),
        Band(188, "advanced.band_weights.188", "al_band_188"),
        Band(267, "advanced.band_weights.267", "al_band_267"),
        Band(381, "advanced.band_weights.381", "al_band_381"),
        Band(544, "advanced.band_weights.544", "al_band_544"),
        Band(777, "advanced.band_weights.777", "al_band_777"),
        Band(1_110, "advanced.band_weights.1110", "al_band_1110"),
        Band(1_585, "advanced.band_weights.1585", "al_band_1585"),
        Band(2_263, "advanced.band_weights.2263", "al_band_2263"),
        Band(20_000, "advanced.band_weights.20000", "al_band_20000"),
    ];

    private static readonly ImmutableHashSet<string> SupportedShaderOptions =
        Descriptors
            .Where(static descriptor => descriptor.Capability is MpvGpu83ParameterCapability.ShaderParameter)
            .Select(static descriptor => descriptor.ShaderOption)
            .Concat(
            [
                "al_runtime_epoch_start_seconds",
                "al_runtime_source_fps",
                "al_runtime_random_seed",
                "al_runtime_frame_inner_percent",
                "al_runtime_frame_inter_percent",
                "al_runtime_frame_probability_percent",
                "al_runtime_slice_length_seconds",
                "al_runtime_slice_interval_seconds",
                "al_runtime_pip_jitter_px",
                "al_runtime_local_blur_interval_seconds",
                "al_runtime_smoothing_enabled",
                "al_runtime_smoothing_seconds",
                "al_runtime_highlight_enabled",
                "al_runtime_highlight_interval_seconds",
                "al_runtime_async_rotation_enabled",
                "al_runtime_async_rotation_min_degrees",
                "al_runtime_async_rotation_max_degrees",
            ])
            .ToImmutableHashSet(StringComparer.Ordinal);

    internal static bool IsSupportedShaderOption(string option) =>
        SupportedShaderOptions.Contains(option);

    private static Descriptor Video(
        string fieldPath,
        string option,
        MpvGpu83ExecutionClass executionClass,
        Func<VideoEffectParams, double> read) =>
        new(fieldPath, option, executionClass, MpvGpu83ParameterCapability.ShaderParameter, true, (video, _) => read(video));

    private static Descriptor VideoScheduled(
        string fieldPath,
        string option,
        Func<VideoEffectParams, double> read) =>
        new(fieldPath, option, MpvGpu83ExecutionClass.FrameScheduling, MpvGpu83ParameterCapability.ScheduledParameter, true, (video, _) => read(video));

    private static Descriptor VideoUnavailableScheduled(
        string fieldPath,
        string option,
        Func<VideoEffectParams, double> read) =>
        new(fieldPath, option, MpvGpu83ExecutionClass.FrameScheduling, MpvGpu83ParameterCapability.Unavailable, true, (video, _) => read(video));

    private static Descriptor VideoUnavailable(
        string fieldPath,
        string option,
        Func<VideoEffectParams, double> read) =>
        new(fieldPath, option, MpvGpu83ExecutionClass.PixelShader, MpvGpu83ParameterCapability.Unavailable, true, (video, _) => read(video));

    private static Descriptor Advanced(
        string fieldPath,
        string option,
        MpvGpu83ExecutionClass executionClass,
        Func<AdvancedEffectParams, double> read) =>
        new(fieldPath, option, executionClass, MpvGpu83ParameterCapability.ShaderParameter, true, (_, advanced) => read(advanced));

    private static Descriptor AdvancedScheduled(
        string fieldPath,
        string option,
        Func<AdvancedEffectParams, double> read) =>
        new(fieldPath, option, MpvGpu83ExecutionClass.FrameScheduling, MpvGpu83ParameterCapability.ScheduledParameter, true, (_, advanced) => read(advanced));

    private static Descriptor AdvancedCompositeScheduled(
        string fieldPath,
        string option,
        Func<AdvancedEffectParams, double> read) =>
        new(fieldPath, option, MpvGpu83ExecutionClass.CompositeShader, MpvGpu83ParameterCapability.ScheduledParameter, true, (_, advanced) => read(advanced));

    private static Descriptor AdvancedUnavailable(
        string fieldPath,
        string option,
        MpvGpu83ExecutionClass executionClass,
        Func<AdvancedEffectParams, double> read) =>
        new(fieldPath, option, executionClass, MpvGpu83ParameterCapability.Unavailable, true, (_, advanced) => read(advanced));

    private static Descriptor AdvancedOptional(
        string fieldPath,
        string option,
        MpvGpu83ExecutionClass executionClass,
        Func<VideoEffectParams, AdvancedEffectParams, double?> read) =>
        new(fieldPath, option, executionClass, MpvGpu83ParameterCapability.ShaderParameter, false, read);

    private static Descriptor Band(uint frequencyHz, string fieldPath, string option) =>
        new(
            fieldPath,
            option,
            MpvGpu83ExecutionClass.PixelShader,
            MpvGpu83ParameterCapability.ShaderParameter,
            true,
            (_, advanced) => advanced.BandWeights.TryGetValue(frequencyHz, out var value) ? value : null);

    private static string Format(double value) =>
        (value == 0 ? 0 : value).ToString("0.##########", CultureInfo.InvariantCulture);
}
