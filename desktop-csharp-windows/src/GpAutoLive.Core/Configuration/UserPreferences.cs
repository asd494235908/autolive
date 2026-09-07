namespace GpAutoLive.Core.Configuration;

/// <summary>
/// 仅包含低敏感、低频写入的桌面偏好。媒体路径、播放位置、URL 和凭据不属于此模型。
/// </summary>
public sealed record UserPreferences
{
    /// <summary>当前偏好文件格式版本。</summary>
    public const int CurrentSchemaVersion = 1;

    /// <summary>偏好文件格式版本。</summary>
    public int SchemaVersion { get; init; } = CurrentSchemaVersion;

    /// <summary>窗口宽度。</summary>
    public int WindowWidth { get; init; } = 1280;

    /// <summary>窗口高度。</summary>
    public int WindowHeight { get; init; } = 800;

    /// <summary>窗口左坐标。</summary>
    public double? WindowLeft { get; init; }

    /// <summary>窗口上坐标。</summary>
    public double? WindowTop { get; init; }

    /// <summary>主题标识。</summary>
    public string Theme { get; init; } = "dark";

    /// <summary>界面语言标识。</summary>
    public string Language { get; init; } = "zh-CN";

    /// <summary>效果面板是否展开。</summary>
    public bool EffectsPanelExpanded { get; init; } = true;

    /// <summary>是否启用性能采样。</summary>
    public bool PerformanceSamplingEnabled { get; init; } = true;

    /// <summary>最近一次非敏感输出模式。</summary>
    public string LastOutputMode { get; init; } = "preview";

    /// <summary>视频处理周期下限，单位毫秒。</summary>
    public ulong VideoCyclePeriodMinMs { get; init; } = DefaultVideoCyclePeriodMinMs;

    /// <summary>视频处理周期上限，单位毫秒。</summary>
    public ulong VideoCyclePeriodMaxMs { get; init; } = DefaultVideoCyclePeriodMaxMs;

    /// <summary>普通声音处理周期下限，单位毫秒。</summary>
    public ulong AudioCyclePeriodMinMs { get; init; } = DefaultAudioCyclePeriodMinMs;

    /// <summary>普通声音处理周期上限，单位毫秒。</summary>
    public ulong AudioCyclePeriodMaxMs { get; init; } = DefaultAudioCyclePeriodMaxMs;

    /// <summary>默认视频周期下限。</summary>
    public const ulong DefaultVideoCyclePeriodMinMs = 5_000;
    /// <summary>默认视频周期上限。</summary>
    public const ulong DefaultVideoCyclePeriodMaxMs = 8_000;
    /// <summary>默认声音周期下限。</summary>
    public const ulong DefaultAudioCyclePeriodMinMs = 3_000;
    /// <summary>默认声音周期上限。</summary>
    public const ulong DefaultAudioCyclePeriodMaxMs = 5_000;

    /// <summary>默认偏好实例。</summary>
    public static UserPreferences Defaults { get; } = new();

    /// <summary>
    /// 在 Core 内完成 UI 偏好白名单和规范化，避免 WPF 页面自行复制配置规则。
    /// </summary>
    public UserPreferences WithUiSettings(
        string theme,
        string language,
        bool effectsPanelExpanded,
        bool performanceSamplingEnabled,
        string lastOutputMode)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(theme);
        ArgumentException.ThrowIfNullOrWhiteSpace(language);
        ArgumentException.ThrowIfNullOrWhiteSpace(lastOutputMode);
        if (!IsAllowedTheme(theme))
        {
            throw new ConfigurationValidationException("主题值不受支持。");
        }

        if (!IsAllowedLanguage(language))
        {
            throw new ConfigurationValidationException("语言值不受支持。");
        }

        if (!IsAllowedOutputMode(lastOutputMode))
        {
            throw new ConfigurationValidationException("输出模式值不受支持。");
        }

        return this with
        {
            Theme = NormalizeTheme(theme),
            Language = NormalizeLanguage(language),
            EffectsPanelExpanded = effectsPanelExpanded,
            PerformanceSamplingEnabled = performanceSamplingEnabled,
            LastOutputMode = NormalizeOutputMode(lastOutputMode),
        };
    }

    /// <summary>更新视频和普通声音周期规则；周期结果快照不属于此配置。</summary>
    public UserPreferences WithEffectCycleSettings(EffectCycleSettings settings) =>
        (settings ?? throw new ArgumentNullException(nameof(settings))).ApplyTo(this);

    internal void Validate()
    {
        if (SchemaVersion != CurrentSchemaVersion)
        {
            throw new ConfigurationValidationException("桌面偏好版本不受支持。");
        }

        if (WindowWidth is < 960 or > 7680)
        {
            throw new ConfigurationValidationException("窗口宽度超出允许范围。");
        }

        if (WindowHeight is < 680 or > 4320)
        {
            throw new ConfigurationValidationException("窗口高度超出允许范围。");
        }

        ValidateCoordinate(WindowLeft, "窗口左坐标");
        ValidateCoordinate(WindowTop, "窗口上坐标");

        if (!AllowedThemes.Contains(Theme, StringComparer.OrdinalIgnoreCase))
        {
            throw new ConfigurationValidationException("主题值不受支持。");
        }

        if (!AllowedLanguages.Contains(Language, StringComparer.OrdinalIgnoreCase))
        {
            throw new ConfigurationValidationException("语言值不受支持。");
        }

        if (!AllowedOutputModes.Contains(LastOutputMode, StringComparer.OrdinalIgnoreCase))
        {
            throw new ConfigurationValidationException("输出模式值不受支持。");
        }

        if (!EffectCycleSettings.TryCreate(
                VideoCyclePeriodMinMs,
                VideoCyclePeriodMaxMs,
                AudioCyclePeriodMinMs,
                AudioCyclePeriodMaxMs,
                out _,
                out var cycleError))
        {
            throw new ConfigurationValidationException(cycleError ?? "效果周期配置无效。");
        }
    }

    internal static bool IsAllowedTheme(string value) => AllowedThemes.Contains(value, StringComparer.OrdinalIgnoreCase);

    internal static bool IsAllowedLanguage(string value) => AllowedLanguages.Contains(value, StringComparer.OrdinalIgnoreCase);

    internal static bool IsAllowedOutputMode(string value) => AllowedOutputModes.Contains(value, StringComparer.OrdinalIgnoreCase);

    internal static string NormalizeTheme(string value) => AllowedThemes.First(item => item.Equals(value, StringComparison.OrdinalIgnoreCase));

    internal static string NormalizeLanguage(string value) => AllowedLanguages.First(item => item.Equals(value, StringComparison.OrdinalIgnoreCase));

    internal static string NormalizeOutputMode(string value) => AllowedOutputModes.First(item => item.Equals(value, StringComparison.OrdinalIgnoreCase));

    private static readonly string[] AllowedThemes = ["dark", "light", "system"];

    private static readonly string[] AllowedLanguages = ["zh-CN", "en-US"];

    private static readonly string[] AllowedOutputModes = ["preview", "rtmp", "virtual_camera"];

    private static void ValidateCoordinate(double? value, string label)
    {
        if (value is null)
        {
            return;
        }

        if (double.IsNaN(value.Value) || double.IsInfinity(value.Value) || value is < -32768 or > 32767)
        {
            throw new ConfigurationValidationException($"{label}超出允许范围。");
        }
    }
}
