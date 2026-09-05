using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>普通声音效果参数。只描述配置和边界，不执行媒体处理。</summary>
public sealed record AudioEffectParams
{
    /// <summary>自然真人声音模式。</summary>
    [JsonPropertyName("natural_voice_mode")]
    public NaturalVoiceMode NaturalVoiceMode { get; init; } = NaturalVoiceMode.Original;

    /// <summary>随机变声周期，单位为毫秒，范围 500–60000。</summary>
    [JsonPropertyName("random_change_period_ms")]
    public ulong RandomChangePeriodMs { get; init; } = 4_000;

    /// <summary>音高微移，单位为半音，范围 -2–2。</summary>
    [JsonPropertyName("pitch_shift_semitones")]
    public double PitchShiftSemitones { get; init; }

    /// <summary>频谱扰动幅度，单位为百分比，范围 0–10。</summary>
    [JsonPropertyName("spectral_perturbation_percent")]
    public double SpectralPerturbationPercent { get; init; }

    /// <summary>环境噪声混入比例，单位为百分比，范围 0–100。</summary>
    [JsonPropertyName("environment_noise_percent")]
    public double EnvironmentNoisePercent { get; init; }

    /// <summary>源环境底噪电平，单位为 dBFS，范围 -60–-20。</summary>
    [JsonPropertyName("environment_noise_dbfs")]
    public double EnvironmentNoiseDbfs { get; init; } = -40.0;

    /// <summary>MFCC 相对偏移，单位为百分比，范围 -20–20。</summary>
    [JsonPropertyName("mfcc_shift_percent")]
    public double MfccShiftPercent { get; init; }

    /// <summary>相位扰动幅度，单位为百分比，范围 -20–20。</summary>
    [JsonPropertyName("phase_perturbation_percent")]
    public double PhasePerturbationPercent { get; init; }

    /// <summary>响度调整，单位为 dB，范围 -6–6。</summary>
    [JsonPropertyName("loudness_adjustment_db")]
    public double LoudnessAdjustmentDb { get; init; }

    /// <summary>输入增益，单位为 dB，范围 -6–6。</summary>
    [JsonPropertyName("input_gain_db")]
    public double InputGainDb { get; init; }

    /// <summary>输出增益，单位为 dB，范围 -6–6。</summary>
    [JsonPropertyName("output_gain_db")]
    public double OutputGainDb { get; init; }

    /// <summary>播放速度，单位为倍速，范围 0.5–2。</summary>
    [JsonPropertyName("playback_speed")]
    public double PlaybackSpeed { get; init; } = 1.0;

    /// <summary>低频均衡增益，单位为 dB，范围 -12–12。</summary>
    [JsonPropertyName("low_eq_db")]
    public double LowEqDb { get; init; }

    /// <summary>中频均衡增益，单位为 dB，范围 -12–12。</summary>
    [JsonPropertyName("mid_eq_db")]
    public double MidEqDb { get; init; }

    /// <summary>高频均衡增益，单位为 dB，范围 -12–12。</summary>
    [JsonPropertyName("high_eq_db")]
    public double HighEqDb { get; init; }

    /// <summary>降噪强度，单位为百分比，范围 0–100。</summary>
    [JsonPropertyName("noise_reduction_percent")]
    public double NoiseReductionPercent { get; init; }

    /// <summary>环境声混合比例，单位为百分比，范围 0–100。</summary>
    [JsonPropertyName("ambient_sound_mix_percent")]
    public double AmbientSoundMixPercent { get; init; }

    /// <summary>淡入时长，单位为毫秒，范围 0–10000。</summary>
    [JsonPropertyName("fade_in_ms")]
    public ulong FadeInMs { get; init; }

    /// <summary>淡出时长，单位为毫秒，范围 0–10000。</summary>
    [JsonPropertyName("fade_out_ms")]
    public ulong FadeOutMs { get; init; }

    /// <summary>干湿比，单位为百分比，范围 0–100。</summary>
    [JsonPropertyName("dry_wet_percent")]
    public double DryWetPercent { get; init; }

    /// <summary>轻混响湿声比例，单位为百分比，范围 0–20。</summary>
    [JsonPropertyName("reverb_wet_percent")]
    public double ReverbWetPercent { get; init; }

    /// <summary>MFCC 分析维度，单位为阶，范围 1–40。</summary>
    [JsonPropertyName("mfcc_dimensions")]
    public byte MfccDimensions { get; init; } = 13;

    /// <summary>SNR 浮动，单位为 dB，范围 -6–6。</summary>
    [JsonPropertyName("snr_variation_db")]
    public double SnrVariationDb { get; init; }

    /// <summary>共振峰偏移，单位为百分比，范围 -5–5。</summary>
    [JsonPropertyName("formant_shift_percent")]
    public double FormantShiftPercent { get; init; }

    /// <summary>颤音频率，单位为 Hz，范围 3–8。</summary>
    [JsonPropertyName("vibrato_frequency_hz")]
    public double VibratoFrequencyHz { get; init; } = 5.0;

    /// <summary>颤音深度，单位为百分比，范围 0–3。</summary>
    [JsonPropertyName("vibrato_depth_percent")]
    public double VibratoDepthPercent { get; init; }

    /// <summary>频谱盲区宽度，单位为百分比，范围 0–5。</summary>
    [JsonPropertyName("spectrum_blind_spot_percent")]
    public double SpectrumBlindSpotPercent { get; init; }

    /// <summary>目标信噪比，单位为 dB；空值表示自动跟随源素材。</summary>
    [JsonPropertyName("snr_target_db")]
    public double? SnrTargetDb { get; init; }

    /// <summary>当前共振峰测量值，单位为 Hz；空值表示尚未测量。</summary>
    [JsonPropertyName("current_formant_hz")]
    public double? CurrentFormantHz { get; init; }

    /// <summary>滤波器 Q 值，范围 0.3–10。</summary>
    [JsonPropertyName("filter_q")]
    public double FilterQ { get; init; } = 1.0;

    /// <summary>目标采样率，单位为 Hz；空值表示跟随源素材。</summary>
    [JsonPropertyName("sample_rate_hz")]
    public uint? SampleRateHz { get; init; }

    /// <summary>输出音频码率，单位为 kbps，范围 64–320。</summary>
    [JsonPropertyName("output_bitrate_kbps")]
    public ushort OutputBitrateKbps { get; init; } = 192;

    /// <summary>音色库资源 ID；空值表示跟随源音色。</summary>
    [JsonPropertyName("voice_library_id")]
    public string? VoiceLibraryId { get; init; }

    /// <summary>是否启用 6kHz 以上的高频音频扰动。</summary>
    [JsonPropertyName("high_frequency_perturbation_enabled")]
    public bool HighFrequencyPerturbationEnabled { get; init; }

    /// <summary>高频扰动变化间隔，单位为毫秒，范围 500–60000。</summary>
    [JsonPropertyName("high_frequency_perturbation_interval_ms")]
    public ulong HighFrequencyPerturbationIntervalMs { get; init; } = 12_000;

    /// <summary>高频扰动强度，单位为百分比，范围 0–20。</summary>
    [JsonPropertyName("high_frequency_perturbation_strength_percent")]
    public double HighFrequencyPerturbationStrengthPercent { get; init; }

    /// <summary>高频扰动目标电平，单位为 dB，范围 -60–0。</summary>
    [JsonPropertyName("high_frequency_perturbation_level_db")]
    public double HighFrequencyPerturbationLevelDb { get; init; } = -32.0;

    /// <summary>返回一份新的正式默认参数快照。</summary>
    public static AudioEffectParams Default => new();

    /// <summary>校验本组参数。</summary>
    public bool TryValidate(out IReadOnlyList<MediaEffectValidationError> errors) =>
        MediaEffectValidation.TryValidate(this, out errors);
}
