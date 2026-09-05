using System.Collections.Immutable;
using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>插话声音预设选择方式，与 Rust/Tauri 端保持同一 JSON 合同。</summary>
public enum InterludeAudioSelectionMode
{
    /// <summary>固定使用 audio_fixed_preset_id。</summary>
    Fixed,

    /// <summary>从 audio_preset_ids 中随机选择。</summary>
    Random
}

/// <summary>预设变化周期；只描述选择策略，不代表 DSP 已完成。</summary>
public enum InterludeAudioVariationMode
{
    /// <summary>每次插话重新选择。</summary>
    EachPlayback,

    /// <summary>在周期窗口内保持同一选择。</summary>
    Periodic
}

/// <summary>插话声音配置的可验证 JSON 数据；不包含凭据、URL 或音频正文。</summary>
public sealed record InterludeAudioConfig
{
    /// <summary>是否启用插话声音调度。</summary>
    [property: JsonPropertyName("enabled")]
    public bool Enabled { get; init; }

    /// <summary>插话文件目录；仅保存路径，不保存音频内容。</summary>
    [property: JsonPropertyName("directory")]
    public string? Directory { get; init; }

    /// <summary>固定或随机预设选择。</summary>
    [property: JsonPropertyName("audio_selection_mode")]
    public InterludeAudioSelectionMode AudioSelectionMode { get; init; } = InterludeAudioSelectionMode.Random;

    /// <summary>固定模式使用的预设 ID。</summary>
    [property: JsonPropertyName("audio_fixed_preset_id")]
    public string AudioFixedPresetId { get; init; } = InterludeAudioRules.DefaultFixedPresetId;

    /// <summary>随机模式的候选预设 ID。</summary>
    [property: JsonPropertyName("audio_preset_ids")]
    public ImmutableArray<string> AudioPresetIds { get; init; } = InterludeAudioRules.DefaultPresetIds;

    /// <summary>是否允许一次选择多个预设轨道。</summary>
    [property: JsonPropertyName("audio_mix_enabled")]
    public bool AudioMixEnabled { get; init; }

    /// <summary>多轨选择数量下限。</summary>
    [property: JsonPropertyName("audio_mix_pick_min")]
    public byte AudioMixPickMin { get; init; } = InterludeAudioRules.DefaultMixPickMin;

    /// <summary>多轨选择数量上限。</summary>
    [property: JsonPropertyName("audio_mix_pick_max")]
    public byte AudioMixPickMax { get; init; } = InterludeAudioRules.DefaultMixPickMax;

    /// <summary>预设选择变化方式。</summary>
    [property: JsonPropertyName("audio_variation_mode")]
    public InterludeAudioVariationMode AudioVariationMode { get; init; } = InterludeAudioVariationMode.EachPlayback;

    /// <summary>预设变化周期下限。</summary>
    [property: JsonPropertyName("audio_variation_period_min_ms")]
    public ulong AudioVariationPeriodMinMs { get; init; } = InterludeAudioRules.DefaultVariationPeriodMinMs;

    /// <summary>预设变化周期上限。</summary>
    [property: JsonPropertyName("audio_variation_period_max_ms")]
    public ulong AudioVariationPeriodMaxMs { get; init; } = InterludeAudioRules.DefaultVariationPeriodMaxMs;

    /// <summary>插话触发间隔下限。</summary>
    [property: JsonPropertyName("interval_min_ms")]
    public ulong IntervalMinMs { get; init; } = InterludeAudioRules.DefaultIntervalMinMs;

    /// <summary>插话触发间隔上限。</summary>
    [property: JsonPropertyName("interval_max_ms")]
    public ulong IntervalMaxMs { get; init; } = InterludeAudioRules.DefaultIntervalMaxMs;

    /// <summary>插话轨音量，单位 dB。</summary>
    [property: JsonPropertyName("volume_db")]
    public double VolumeDb { get; init; }

    /// <summary>基础媒体 duck 深度，单位 dB。</summary>
    [property: JsonPropertyName("ducking_depth_db")]
    public double DuckingDepthDb { get; init; } = -60;

    /// <summary>duck 淡入时间。</summary>
    [property: JsonPropertyName("ducking_attack_ms")]
    public ulong DuckingAttackMs { get; init; } = 50;

    /// <summary>duck 淡出时间。</summary>
    [property: JsonPropertyName("ducking_release_ms")]
    public ulong DuckingReleaseMs { get; init; } = 250;

    /// <summary>与参考端默认值一致的安全配置。</summary>
    public static InterludeAudioConfig Default { get; } = new();
}

/// <summary>插话声音配置的稳定校验错误。</summary>
public enum InterludeAudioConfigFailureCode
{
    /// <summary>候选预设数量超限。</summary>
    AudioPresetCountOutOfRange,
    /// <summary>预设 ID 无效。</summary>
    AudioPresetInvalid,
    /// <summary>预设 ID 重复。</summary>
    AudioPresetDuplicate,
    /// <summary>混合轨数下限无效。</summary>
    AudioMixPickMinOutOfRange,
    /// <summary>混合轨数上限无效。</summary>
    AudioMixPickMaxOutOfRange,
    /// <summary>混合轨数范围倒置。</summary>
    AudioMixPickOrderInvalid,
    /// <summary>预设变化周期下限无效。</summary>
    AudioVariationPeriodMinOutOfRange,
    /// <summary>预设变化周期上限无效。</summary>
    AudioVariationPeriodMaxOutOfRange,
    /// <summary>预设变化周期范围倒置。</summary>
    AudioVariationPeriodOrderInvalid,
    /// <summary>插话间隔下限无效。</summary>
    IntervalMinOutOfRange,
    /// <summary>插话间隔上限无效。</summary>
    IntervalMaxOutOfRange,
    /// <summary>插话间隔范围倒置。</summary>
    IntervalOrderInvalid,
    /// <summary>插话音量无效。</summary>
    VolumeOutOfRange,
    /// <summary>duck 深度无效。</summary>
    DuckingDepthOutOfRange,
    /// <summary>duck 淡入时间无效。</summary>
    DuckingAttackOutOfRange,
    /// <summary>duck 淡出时间无效。</summary>
    DuckingReleaseOutOfRange,
    /// <summary>启用时没有配置目录。</summary>
    MissingDirectory
}

/// <summary>不泄露系统异常正文的配置校验结果。</summary>
public sealed record InterludeAudioConfigError(
    InterludeAudioConfigFailureCode Code,
    string Message);

/// <summary>插话声音预设、周期和 duck 边界。</summary>
public static class InterludeAudioRules
{
    /// <summary>候选预设最大数量。</summary>
    public const int MaxPresetCount = 22;
    /// <summary>默认多轨选择下限。</summary>
    public const byte DefaultMixPickMin = 1;
    /// <summary>默认多轨选择上限。</summary>
    public const byte DefaultMixPickMax = 2;
    /// <summary>默认预设变化周期下限。</summary>
    public const ulong DefaultVariationPeriodMinMs = 8_000;
    /// <summary>默认预设变化周期上限。</summary>
    public const ulong DefaultVariationPeriodMaxMs = 15_000;
    /// <summary>默认插话间隔下限。</summary>
    public const ulong DefaultIntervalMinMs = 8_000;
    /// <summary>默认插话间隔上限。</summary>
    public const ulong DefaultIntervalMaxMs = 13_000;
    /// <summary>插话音量最小 dB。</summary>
    public const double MinVolumeDb = -60;
    /// <summary>插话音量最大 dB。</summary>
    public const double MaxVolumeDb = 12;
    /// <summary>duck 深度最小 dB。</summary>
    public const double MinDuckingDepthDb = -60;
    /// <summary>duck 深度最大 dB。</summary>
    public const double MaxDuckingDepthDb = 0;
    /// <summary>duck 淡入最大毫秒数。</summary>
    public const ulong MaxDuckingAttackMs = 1_000;
    /// <summary>duck 淡出最大毫秒数。</summary>
    public const ulong MaxDuckingReleaseMs = 3_000;
    /// <summary>插话间隔最小毫秒数。</summary>
    public const ulong MinIntervalMs = 500;
    /// <summary>插话间隔最大毫秒数。</summary>
    public const ulong MaxIntervalMs = 60_000;
    /// <summary>预设变化周期最小毫秒数。</summary>
    public const ulong MinVariationPeriodMs = 1_000;
    /// <summary>预设变化周期最大毫秒数。</summary>
    public const ulong MaxVariationPeriodMs = 600_000;
    /// <summary>默认固定预设 ID。</summary>
    public const string DefaultFixedPresetId = "p01";

    /// <summary>低感知默认池，保持参考端 p01 固定、p01～p20 随机候选。</summary>
    public static ImmutableArray<string> DefaultPresetIds { get; } =
        Enumerable.Range(1, 20).Select(static number => $"p{number:00}").ToImmutableArray();

    /// <summary>所有可识别的 22 个预设 ID。</summary>
    public static ImmutableArray<string> AllPresetIds { get; } =
        Enumerable.Range(1, MaxPresetCount).Select(static number => $"p{number:00}").ToImmutableArray();

    /// <summary>校验配置；不访问文件、设备或网络。</summary>
    public static bool TryValidate(InterludeAudioConfig? config, out InterludeAudioConfigError? error)
    {
        error = null;
        if (config is null)
        {
            error = Invalid(InterludeAudioConfigFailureCode.AudioPresetCountOutOfRange, "插话声音配置不能为空。");
            return false;
        }

        if (!IsAllowedPresetId(config.AudioFixedPresetId))
        {
            error = Invalid(InterludeAudioConfigFailureCode.AudioPresetInvalid, "固定声音预设不受支持。");
            return false;
        }

        if (config.AudioPresetIds.IsDefaultOrEmpty || config.AudioPresetIds.Length > MaxPresetCount)
        {
            error = Invalid(InterludeAudioConfigFailureCode.AudioPresetCountOutOfRange, "声音预设数量必须在 1 到 22 之间。");
            return false;
        }

        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (var presetId in config.AudioPresetIds)
        {
            if (!IsAllowedPresetId(presetId))
            {
                error = Invalid(InterludeAudioConfigFailureCode.AudioPresetInvalid, "声音预设 ID 不受支持。");
                return false;
            }

            if (!seen.Add(presetId))
            {
                error = Invalid(InterludeAudioConfigFailureCode.AudioPresetDuplicate, "声音预设不能重复。");
                return false;
            }
        }

        if (config.AudioMixPickMin is < 1 or > 4)
        {
            error = Invalid(InterludeAudioConfigFailureCode.AudioMixPickMinOutOfRange, "混合轨数下限必须在 1 到 4 之间。");
            return false;
        }

        if (config.AudioMixPickMax is < 1 or > 4)
        {
            error = Invalid(InterludeAudioConfigFailureCode.AudioMixPickMaxOutOfRange, "混合轨数上限必须在 1 到 4 之间。");
            return false;
        }

        if (config.AudioMixPickMin > config.AudioMixPickMax)
        {
            error = Invalid(InterludeAudioConfigFailureCode.AudioMixPickOrderInvalid, "混合轨数范围顺序无效。");
            return false;
        }

        if (!IsInRange(config.AudioVariationPeriodMinMs, MinVariationPeriodMs, MaxVariationPeriodMs))
        {
            error = Invalid(InterludeAudioConfigFailureCode.AudioVariationPeriodMinOutOfRange, "预设变化周期下限超出范围。");
            return false;
        }

        if (!IsInRange(config.AudioVariationPeriodMaxMs, MinVariationPeriodMs, MaxVariationPeriodMs))
        {
            error = Invalid(InterludeAudioConfigFailureCode.AudioVariationPeriodMaxOutOfRange, "预设变化周期上限超出范围。");
            return false;
        }

        if (config.AudioVariationPeriodMinMs > config.AudioVariationPeriodMaxMs)
        {
            error = Invalid(InterludeAudioConfigFailureCode.AudioVariationPeriodOrderInvalid, "预设变化周期范围顺序无效。");
            return false;
        }

        if (!IsInRange(config.IntervalMinMs, MinIntervalMs, MaxIntervalMs))
        {
            error = Invalid(InterludeAudioConfigFailureCode.IntervalMinOutOfRange, "插话间隔下限超出范围。");
            return false;
        }

        if (!IsInRange(config.IntervalMaxMs, MinIntervalMs, MaxIntervalMs))
        {
            error = Invalid(InterludeAudioConfigFailureCode.IntervalMaxOutOfRange, "插话间隔上限超出范围。");
            return false;
        }

        if (config.IntervalMinMs > config.IntervalMaxMs)
        {
            error = Invalid(InterludeAudioConfigFailureCode.IntervalOrderInvalid, "插话间隔范围顺序无效。");
            return false;
        }

        if (!double.IsFinite(config.VolumeDb) || config.VolumeDb is < MinVolumeDb or > MaxVolumeDb)
        {
            error = Invalid(InterludeAudioConfigFailureCode.VolumeOutOfRange, "插话音量超出范围。");
            return false;
        }

        if (!double.IsFinite(config.DuckingDepthDb) || config.DuckingDepthDb is < MinDuckingDepthDb or > MaxDuckingDepthDb)
        {
            error = Invalid(InterludeAudioConfigFailureCode.DuckingDepthOutOfRange, "duck 深度超出范围。");
            return false;
        }

        if (config.DuckingAttackMs > MaxDuckingAttackMs)
        {
            error = Invalid(InterludeAudioConfigFailureCode.DuckingAttackOutOfRange, "duck 淡入时间超出范围。");
            return false;
        }

        if (config.DuckingReleaseMs > MaxDuckingReleaseMs)
        {
            error = Invalid(InterludeAudioConfigFailureCode.DuckingReleaseOutOfRange, "duck 淡出时间超出范围。");
            return false;
        }

        if (config.Enabled && string.IsNullOrWhiteSpace(config.Directory))
        {
            error = Invalid(InterludeAudioConfigFailureCode.MissingDirectory, "启用插话声音时必须配置目录。");
            return false;
        }

        return true;
    }

    /// <summary>判断预设 ID 是否属于 p01 到 p22。</summary>
    public static bool IsAllowedPresetId(string? value) =>
        value is { Length: 3 }
        && value[0] == 'p'
        && char.IsAsciiDigit(value[1])
        && char.IsAsciiDigit(value[2])
        && int.Parse(value.AsSpan(1), System.Globalization.CultureInfo.InvariantCulture) is >= 1 and <= MaxPresetCount;

    private static bool IsInRange(ulong value, ulong min, ulong max) => value >= min && value <= max;

    private static InterludeAudioConfigError Invalid(InterludeAudioConfigFailureCode code, string message) => new(code, message);
}
