using System.Text;

namespace GpAutoLive.Contracts;

/// <summary>媒体效果契约的集中式范围校验，避免 UI 和运行时产生第二套边界。</summary>
public static class MediaEffectValidation
{
    /// <summary>校验完整媒体效果参数树。</summary>
    public static bool TryValidate(
        MediaEffectParams? parameters,
        out IReadOnlyList<MediaEffectValidationError> errors)
    {
        var result = new List<MediaEffectValidationError>();
        if (parameters is null)
        {
            result.Add(InvalidValue("media_effect_params", "missing_value", "无", "媒体效果参数不能为空"));
        }
        else
        {
            Validate(parameters.Audio, result);
            Validate(parameters.Video, result);
            Validate(parameters.Advanced, result);
        }

        errors = result;
        return result.Count == 0;
    }

    /// <summary>校验声音效果参数。</summary>
    public static bool TryValidate(
        AudioEffectParams? parameters,
        out IReadOnlyList<MediaEffectValidationError> errors)
    {
        var result = new List<MediaEffectValidationError>();
        if (parameters is null)
        {
            result.Add(InvalidValue("audio", "missing_value", "无", "音频效果参数不能为空"));
        }
        else
        {
            Validate(parameters, result);
        }

        errors = result;
        return result.Count == 0;
    }

    /// <summary>校验视频效果参数。</summary>
    public static bool TryValidate(
        VideoEffectParams? parameters,
        out IReadOnlyList<MediaEffectValidationError> errors)
    {
        var result = new List<MediaEffectValidationError>();
        if (parameters is null)
        {
            result.Add(InvalidValue("video", "missing_value", "无", "视频效果参数不能为空"));
        }
        else
        {
            Validate(parameters, result);
        }

        errors = result;
        return result.Count == 0;
    }

    /// <summary>校验高级效果参数。</summary>
    public static bool TryValidate(
        AdvancedEffectParams? parameters,
        out IReadOnlyList<MediaEffectValidationError> errors)
    {
        var result = new List<MediaEffectValidationError>();
        if (parameters is null)
        {
            result.Add(InvalidValue("advanced", "missing_value", "无", "高级效果参数不能为空"));
        }
        else
        {
            Validate(parameters, result);
        }

        errors = result;
        return result.Count == 0;
    }

    private static void Validate(AudioEffectParams parameters, List<MediaEffectValidationError> errors)
    {
        ULongRange(errors, "audio.random_change_period_ms", "ms", parameters.RandomChangePeriodMs, 500, 60_000);
        Range(errors, "audio.pitch_shift_semitones", "semitone", parameters.PitchShiftSemitones, -2, 2);
        Range(errors, "audio.spectral_perturbation_percent", "%", parameters.SpectralPerturbationPercent, 0, 10);
        Range(errors, "audio.environment_noise_percent", "%", parameters.EnvironmentNoisePercent, 0, 100);
        Range(errors, "audio.environment_noise_dbfs", "dBFS", parameters.EnvironmentNoiseDbfs, -60, -20);
        Range(errors, "audio.mfcc_shift_percent", "%", parameters.MfccShiftPercent, -20, 20);
        Range(errors, "audio.phase_perturbation_percent", "%", parameters.PhasePerturbationPercent, -20, 20);
        Range(errors, "audio.loudness_adjustment_db", "dB", parameters.LoudnessAdjustmentDb, -6, 6);
        Range(errors, "audio.input_gain_db", "dB", parameters.InputGainDb, -6, 6);
        Range(errors, "audio.output_gain_db", "dB", parameters.OutputGainDb, -6, 6);
        Range(errors, "audio.playback_speed", "倍", parameters.PlaybackSpeed, 0.5, 2);
        Range(errors, "audio.low_eq_db", "dB", parameters.LowEqDb, -12, 12);
        Range(errors, "audio.mid_eq_db", "dB", parameters.MidEqDb, -12, 12);
        Range(errors, "audio.high_eq_db", "dB", parameters.HighEqDb, -12, 12);
        Range(errors, "audio.noise_reduction_percent", "%", parameters.NoiseReductionPercent, 0, 100);
        Range(errors, "audio.ambient_sound_mix_percent", "%", parameters.AmbientSoundMixPercent, 0, 100);
        ULongRange(errors, "audio.fade_in_ms", "ms", parameters.FadeInMs, 0, 10_000);
        ULongRange(errors, "audio.fade_out_ms", "ms", parameters.FadeOutMs, 0, 10_000);
        Range(errors, "audio.dry_wet_percent", "%", parameters.DryWetPercent, 0, 100);
        Range(errors, "audio.reverb_wet_percent", "%", parameters.ReverbWetPercent, 0, 20);
        ByteRange(errors, "audio.mfcc_dimensions", "阶", parameters.MfccDimensions, 1, 40);
        Range(errors, "audio.snr_variation_db", "dB", parameters.SnrVariationDb, -6, 6);
        Range(errors, "audio.formant_shift_percent", "%", parameters.FormantShiftPercent, -5, 5);
        Range(errors, "audio.vibrato_frequency_hz", "Hz", parameters.VibratoFrequencyHz, 3, 8);
        Range(errors, "audio.vibrato_depth_percent", "%", parameters.VibratoDepthPercent, 0, 3);
        Range(errors, "audio.spectrum_blind_spot_percent", "%", parameters.SpectrumBlindSpotPercent, 0, 5);
        OptionalRange(errors, "audio.snr_target_db", "dB", parameters.SnrTargetDb, 0, 60);
        OptionalRange(errors, "audio.current_formant_hz", "Hz", parameters.CurrentFormantHz, 20, 10_000);
        Range(errors, "audio.filter_q", "无量纲", parameters.FilterQ, 0.3, 10);

        if (parameters.SampleRateHz is not null && parameters.SampleRateHz is not (44_100 or 48_000))
        {
            errors.Add(new(
                "audio.sample_rate_hz",
                "unsupported_value",
                "Hz",
                parameters.SampleRateHz,
                44_100,
                48_000,
                "采样率只能跟随源素材、44100 Hz 或 48000 Hz"));
        }

        UShortRange(errors, "audio.output_bitrate_kbps", "kbps", parameters.OutputBitrateKbps, 64, 320);
        if (parameters.VoiceLibraryId is not null &&
            (parameters.VoiceLibraryId.Length == 0 || Encoding.UTF8.GetByteCount(parameters.VoiceLibraryId) > 128))
        {
            errors.Add(new(
                "audio.voice_library_id",
                "invalid_identifier",
                "ID",
                null,
                null,
                128,
                "音色库 ID 不能为空且长度不能超过 128 个字节"));
        }

        ULongRange(
            errors,
            "audio.high_frequency_perturbation_interval_ms",
            "ms",
            parameters.HighFrequencyPerturbationIntervalMs,
            500,
            60_000);
        Range(
            errors,
            "audio.high_frequency_perturbation_strength_percent",
            "%",
            parameters.HighFrequencyPerturbationStrengthPercent,
            0,
            20);
        Range(
            errors,
            "audio.high_frequency_perturbation_level_db",
            "dB",
            parameters.HighFrequencyPerturbationLevelDb,
            -60,
            0);
    }

    private static void Validate(VideoEffectParams parameters, List<MediaEffectValidationError> errors)
    {
        Range(errors, "video.brightness_percent", "%", parameters.BrightnessPercent, -100, 100);
        Range(errors, "video.saturation_percent", "%", parameters.SaturationPercent, 0, 200);
        Range(errors, "video.blur_radius_px", "px", parameters.BlurRadiusPx, 0, 8);
        Range(errors, "video.contrast_percent", "%", parameters.ContrastPercent, 0, 200);
        Range(errors, "video.hue_rotation_degrees", "°", parameters.HueRotationDegrees, -180, 180);
        Range(errors, "video.sharpen_percent", "%", parameters.SharpenPercent, 0, 100);
        Range(errors, "video.noise_percent", "%", parameters.NoisePercent, 0, 8);
        Range(errors, "video.detail_enhancement_percent", "%", parameters.DetailEnhancementPercent, 0, 50);
        Range(errors, "video.crop_edge_smoothing", "归一化", parameters.CropEdgeSmoothing, 0, 1);
        Range(errors, "video.frame_rate_jitter_percent", "%", parameters.FrameRateJitterPercent, 0, 2);
        Range(errors, "video.frame_rate_perturbation_frequency_hz", "Hz", parameters.FrameRatePerturbationFrequencyHz, 0.01, 2);
        Range(errors, "video.frame_rate_perturbation_amplitude_fps", "fps", parameters.FrameRatePerturbationAmplitudeFps, 0, 2);
        Range(errors, "video.pixel_scale_percent", "%", parameters.PixelScalePercent, 95, 105);
        Range(errors, "video.pixel_jitter_px", "px", parameters.PixelJitterPx, 0, 2);
        Range(errors, "video.dynamic_crop_percent", "%", parameters.DynamicCropPercent, 0, 4);
        Range(errors, "video.frame_inner_perturbation_percent", "%", parameters.FrameInnerPerturbationPercent, 0, 2);
        Range(errors, "video.frame_inter_perturbation_percent", "%", parameters.FrameInterPerturbationPercent, 0, 20);
        Range(errors, "video.space_x_offset_px", "px", parameters.SpaceXOffsetPx, -4, 4);
        Range(errors, "video.space_y_offset_px", "px", parameters.SpaceYOffsetPx, -4, 4);
        Range(errors, "video.color_space_conversion_strength_percent", "%", parameters.ColorSpaceConversionStrengthPercent, 0, 100);
        Range(errors, "video.rotation_degrees", "°", parameters.RotationDegrees, -180, 180);
        Range(errors, "video.vignette_percent", "%", parameters.VignettePercent, 0, 100);
        Range(errors, "video.highlights_percent", "%", parameters.HighlightsPercent, -100, 100);
        Range(errors, "video.shadows_percent", "%", parameters.ShadowsPercent, -100, 100);
        Range(errors, "video.edge_softness_percent", "%", parameters.EdgeSoftnessPercent, 0, 100);
        Range(errors, "video.image_repair_strength_percent", "%", parameters.ImageRepairStrengthPercent, 0, 100);
    }

    private static void Validate(AdvancedEffectParams parameters, List<MediaEffectValidationError> errors)
    {
        if (parameters.BandWeights is null)
        {
            errors.Add(InvalidValue("advanced.band_weights", "missing_value", "Hz", "固定视觉频段权重不能为空"));
        }
        else
        {
            foreach (var pair in parameters.BandWeights.OrderBy(pair => pair.Key))
            {
                if (!MediaEffectContracts.VisualBandFrequenciesHz.Contains(pair.Key))
                {
                    errors.Add(new(
                        "advanced.band_weights",
                        "unknown_frequency_band",
                        "Hz",
                        pair.Key,
                        null,
                        null,
                        $"不支持视觉频段 {pair.Key} Hz"));
                }
            }

            foreach (var frequencyHz in MediaEffectContracts.VisualBandFrequenciesHz)
            {
                if (!parameters.BandWeights.TryGetValue(frequencyHz, out var weight))
                {
                    errors.Add(new(
                        "advanced.band_weights",
                        "missing_frequency_band",
                        "Hz",
                        frequencyHz,
                        null,
                        null,
                        $"缺少固定视觉频段 {frequencyHz} Hz"));
                    continue;
                }

                Range(errors, $"advanced.band_weights.{frequencyHz}", "倍", weight, 0.5, 1.5);
            }
        }

        OptionalRange(errors, "advanced.target_frequency_hz", "Hz", parameters.TargetFrequencyHz, 65, 20_000);
        OptionalRange(errors, "advanced.core_frequency_hz", "Hz", parameters.CoreFrequencyHz, 65, 20_000);
        if (parameters.TargetFrequencyHz is null && parameters.CoreFrequencyHz is not null)
        {
            errors.Add(new(
                "advanced.core_frequency_hz",
                "invalid_relation",
                "Hz",
                parameters.CoreFrequencyHz,
                null,
                null,
                "核心频率不能在目标频率关闭时单独启用"));
        }

        Range(errors, "advanced.wave_intensity", "归一化", parameters.WaveIntensity, 0, 1);
        Range(errors, "advanced.wave_level", "归一化", parameters.WaveLevel, 0, 1);
        ULongOrUIntRange(errors, "advanced.wave_grain_count", "个", parameters.WaveGrainCount, 1, 100);
        Range(errors, "advanced.dynamic_eq_threshold", "归一化刻度", parameters.DynamicEqThreshold, 0, 20);
        Range(errors, "advanced.channel_offset_percent", "%", parameters.ChannelOffsetPercent, -10, 10);
        ByteRange(errors, "advanced.space_dimension", "维", parameters.SpaceDimension, 1, 3);
        Range(errors, "advanced.frequency_space_x_offset_px", "px", parameters.FrequencySpaceXOffsetPx, -10, 10);
        Range(errors, "advanced.frequency_space_y_offset_px", "px", parameters.FrequencySpaceYOffsetPx, -10, 10);
        Range(errors, "advanced.frame_perturbation_probability_percent", "%", parameters.FramePerturbationProbabilityPercent, 0, 20);
        Range(errors, "advanced.random_graphic_opacity_percent", "%", parameters.RandomGraphicOpacityPercent, 0, 50);
        Range(errors, "advanced.random_graphic_size_px", "px", parameters.RandomGraphicSizePx, 1, 64);
        ByteRange(errors, "advanced.abstract_face_count", "个", parameters.AbstractFaceCount, 0, 10);
        Range(errors, "advanced.abstract_face_size_percent", "%/width", parameters.AbstractFaceSizePercent, 1, 10);
        Range(errors, "advanced.abstract_face_opacity_percent", "%", parameters.AbstractFaceOpacityPercent, 0, 30);
        Range(errors, "advanced.overlay_offset_px", "px", parameters.OverlayOffsetPx, -10, 10);
        ULongRange(errors, "advanced.slice_length_ms", "ms", parameters.SliceLengthMs, 500, 10_000);
        ULongRange(errors, "advanced.slice_min_length_ms", "ms", parameters.SliceMinLengthMs, 1_000, 60_000);
        ULongRange(errors, "advanced.slice_trigger_interval_ms", "ms", parameters.SliceTriggerIntervalMs, 5_000, 120_000);
        if (parameters.SliceTriggerIntervalMs < parameters.SliceLengthMs)
        {
            errors.Add(new(
                "advanced.slice_trigger_interval_ms",
                "invalid_relation",
                "ms",
                parameters.SliceTriggerIntervalMs,
                parameters.SliceLengthMs,
                null,
                "切片触发间隔不能短于切片长度"));
        }

        ByteRange(errors, "advanced.random_graphic_count", "个", parameters.RandomGraphicCount, 1, 32);
        Range(errors, "advanced.picture_in_picture_scale_percent", "%", parameters.PictureInPictureScalePercent, 10, 50);
        Range(errors, "advanced.picture_in_picture_opacity_percent", "%", parameters.PictureInPictureOpacityPercent, 0, 100);
        Range(errors, "advanced.picture_in_picture_rotation_degrees", "°", parameters.PictureInPictureRotationDegrees, -15, 15);
        Range(errors, "advanced.picture_in_picture_pixel_jitter_px", "px", parameters.PictureInPicturePixelJitterPx, 0, 4);
        Range(errors, "advanced.local_blur_region_percent", "%", parameters.LocalBlurRegionPercent, 5, 50);
        Range(errors, "advanced.local_blur_radius_px", "px", parameters.LocalBlurRadiusPx, 0.1, 16);
        ULongRange(errors, "advanced.local_blur_interval_ms", "ms", parameters.LocalBlurIntervalMs, 500, 60_000);
        Range(errors, "advanced.edge_feather_percent", "%", parameters.EdgeFeatherPercent, 0, 100);
        ULongRange(errors, "advanced.transform_smoothing_duration_ms", "ms", parameters.TransformSmoothingDurationMs, 50, 5_000);
        ULongRange(errors, "advanced.highlight_perturbation_interval_ms", "ms", parameters.HighlightPerturbationIntervalMs, 500, 60_000);
        Range(errors, "advanced.asynchronous_rotation_min_degrees", "°", parameters.AsynchronousRotationMinDegrees, -15, 15);
        Range(errors, "advanced.asynchronous_rotation_max_degrees", "°", parameters.AsynchronousRotationMaxDegrees, -15, 15);
        if (parameters.AsynchronousRotationMinDegrees > parameters.AsynchronousRotationMaxDegrees)
        {
            errors.Add(new(
                "advanced.asynchronous_rotation_max_degrees",
                "invalid_relation",
                "°",
                parameters.AsynchronousRotationMaxDegrees,
                parameters.AsynchronousRotationMinDegrees,
                15,
                "异步旋转最大角度不能小于最小角度"));
        }
    }

    private static void Range(
        List<MediaEffectValidationError> errors,
        string field,
        string unit,
        double value,
        double min,
        double max)
    {
        if (!double.IsFinite(value) || value < min || value > max)
        {
            errors.Add(new(
                field,
                "out_of_range",
                unit,
                double.IsFinite(value) ? value : null,
                min,
                max,
                $"参数必须位于 {min}–{max} {unit} 范围内"));
        }
    }

    private static void OptionalRange(
        List<MediaEffectValidationError> errors,
        string field,
        string unit,
        double? value,
        double min,
        double max)
    {
        if (value is not null)
        {
            Range(errors, field, unit, value.Value, min, max);
        }
    }

    private static void ULongRange(
        List<MediaEffectValidationError> errors,
        string field,
        string unit,
        ulong value,
        ulong min,
        ulong max) => Range(errors, field, unit, value, min, max);

    private static void ULongOrUIntRange(
        List<MediaEffectValidationError> errors,
        string field,
        string unit,
        uint value,
        uint min,
        uint max) => Range(errors, field, unit, value, min, max);

    private static void ByteRange(
        List<MediaEffectValidationError> errors,
        string field,
        string unit,
        byte value,
        byte min,
        byte max) => Range(errors, field, unit, value, min, max);

    private static void UShortRange(
        List<MediaEffectValidationError> errors,
        string field,
        string unit,
        ushort value,
        ushort min,
        ushort max) => Range(errors, field, unit, value, min, max);

    private static MediaEffectValidationError InvalidValue(
        string field,
        string code,
        string unit,
        string message) => new(field, code, unit, null, null, null, message);
}
