namespace GpAutoLive.Core.Configuration;

/// <summary>
/// 视频和普通声音处理的周期规则。周期以毫秒保存，界面以秒编辑；不包含当前周期生成结果。
/// </summary>
public sealed record EffectCycleSettings(
    ulong VideoPeriodMinMs,
    ulong VideoPeriodMaxMs,
    ulong AudioPeriodMinMs,
    ulong AudioPeriodMaxMs)
{
    /// <summary>与 Rust 周期输入一致的最小周期。</summary>
    public const ulong MinimumPeriodMs = 1_000;

    /// <summary>周期输入的最大周期。</summary>
    public const ulong MaximumPeriodMs = 60_000;

    /// <summary>Rust/C# 共享默认周期规则。</summary>
    public static EffectCycleSettings Default { get; } = new(
        UserPreferences.DefaultVideoCyclePeriodMinMs,
        UserPreferences.DefaultVideoCyclePeriodMaxMs,
        UserPreferences.DefaultAudioCyclePeriodMinMs,
        UserPreferences.DefaultAudioCyclePeriodMaxMs);

    /// <summary>从本地偏好读取周期规则。</summary>
    public static EffectCycleSettings From(UserPreferences preferences)
    {
        ArgumentNullException.ThrowIfNull(preferences);
        return new(
            preferences.VideoCyclePeriodMinMs,
            preferences.VideoCyclePeriodMaxMs,
            preferences.AudioCyclePeriodMinMs,
            preferences.AudioCyclePeriodMaxMs);
    }

    /// <summary>把周期规则写回本地偏好对象。</summary>
    public UserPreferences ApplyTo(UserPreferences preferences)
    {
        ArgumentNullException.ThrowIfNull(preferences);
        if (!TryCreate(
                VideoPeriodMinMs,
                VideoPeriodMaxMs,
                AudioPeriodMinMs,
                AudioPeriodMaxMs,
                out _,
                out var error))
        {
            throw new ConfigurationValidationException(error ?? "效果周期配置无效。");
        }

        return preferences with
        {
            VideoCyclePeriodMinMs = VideoPeriodMinMs,
            VideoCyclePeriodMaxMs = VideoPeriodMaxMs,
            AudioCyclePeriodMinMs = AudioPeriodMinMs,
            AudioCyclePeriodMaxMs = AudioPeriodMaxMs,
        };
    }

    /// <summary>严格创建周期规则；不自动纠正用户输入。</summary>
    public static bool TryCreate(
        ulong videoPeriodMinMs,
        ulong videoPeriodMaxMs,
        ulong audioPeriodMinMs,
        ulong audioPeriodMaxMs,
        out EffectCycleSettings? settings,
        out string? error)
    {
        settings = null;
        error = null;
        if (!IsAllowed(videoPeriodMinMs) || !IsAllowed(videoPeriodMaxMs))
        {
            error = "视频周期必须在 1 到 60 秒之间。";
            return false;
        }

        if (videoPeriodMinMs > videoPeriodMaxMs)
        {
            error = "视频周期最小值不能大于最大值。";
            return false;
        }

        if (!IsAllowed(audioPeriodMinMs) || !IsAllowed(audioPeriodMaxMs))
        {
            error = "声音周期必须在 1 到 60 秒之间。";
            return false;
        }

        if (audioPeriodMinMs > audioPeriodMaxMs)
        {
            error = "声音周期最小值不能大于最大值。";
            return false;
        }

        settings = new(videoPeriodMinMs, videoPeriodMaxMs, audioPeriodMinMs, audioPeriodMaxMs);
        return true;
    }

    /// <summary>为运行时调度器规范化单个周期。</summary>
    public static ulong NormalizePeriod(ulong value, ulong fallbackMs) =>
        value is < MinimumPeriodMs or > MaximumPeriodMs
            ? Math.Clamp(fallbackMs, MinimumPeriodMs, MaximumPeriodMs)
            : value;

    /// <summary>为运行时调度器规范化周期范围。</summary>
    public static (ulong Minimum, ulong Maximum) NormalizeRange(
        ulong minimum,
        ulong maximum,
        ulong fallbackMinimum,
        ulong fallbackMaximum)
    {
        var normalizedMinimum = NormalizePeriod(minimum, fallbackMinimum);
        var normalizedMaximum = NormalizePeriod(maximum, fallbackMaximum);
        return normalizedMinimum <= normalizedMaximum
            ? (normalizedMinimum, normalizedMaximum)
            : (normalizedMaximum, normalizedMinimum);
    }

    private static bool IsAllowed(ulong value) =>
        value is >= MinimumPeriodMs and <= MaximumPeriodMs;
}
