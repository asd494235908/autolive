using System.Globalization;
using System.Text;

namespace GpAutoLive.Core.Configuration;

/// <summary>
/// 白名单 INI 存储。只接受本类定义的 section/key，防止把凭据或媒体数据混入 app.ini。
/// </summary>
public sealed class IniUserPreferencesStore
{
    /// <summary>偏好文件允许的最大字节数。</summary>
    public const long MaxFileBytes = 64 * 1024;

    private static readonly IReadOnlySet<string> AllowedSections = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
    {
        "meta",
        "window",
        "ui",
        "effects"
    };

    private static readonly IReadOnlyDictionary<string, IReadOnlySet<string>> AllowedKeys =
        new Dictionary<string, IReadOnlySet<string>>(StringComparer.OrdinalIgnoreCase)
        {
            ["meta"] = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { "schema_version" },
            ["window"] = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { "width", "height", "left", "top" },
            ["ui"] = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
            {
                "theme",
                "language",
                "effects_panel_expanded",
                "performance_sampling_enabled",
                "last_output_mode"
            },
            ["effects"] = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
            {
                "video_cycle_period_min_ms",
                "video_cycle_period_max_ms",
                "audio_cycle_period_min_ms",
                "audio_cycle_period_max_ms"
            }
        };

    private readonly string _path;

    /// <summary>创建指定路径的低敏感偏好存储。</summary>
    public IniUserPreferencesStore(string path)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        _path = Path.GetFullPath(path);
    }

    /// <summary>异步读取并校验偏好；文件不存在时返回默认值。</summary>
    public async Task<UserPreferences> ReadAsync(CancellationToken cancellationToken = default)
    {
        if (!File.Exists(_path))
        {
            return UserPreferences.Defaults;
        }

        string text;
        try
        {
            text = await AtomicFile.ReadTextAsync(_path, MaxFileBytes, cancellationToken).ConfigureAwait(false);
        }
        catch (FileNotFoundException)
        {
            // 文件可能在存在性检查后被并发移除，按首次启动处理。
            return UserPreferences.Defaults;
        }

        return Parse(text);
    }

    /// <summary>异步以原子方式保存偏好。</summary>
    public Task SaveAsync(UserPreferences preferences, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(preferences);
        preferences.Validate();

        var text = Serialize(preferences);
        return AtomicFile.WriteTextAsync(
            _path,
            Encoding.UTF8.GetBytes(text),
            cancellationToken);
    }

    internal static UserPreferences Parse(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        if (Encoding.UTF8.GetByteCount(text) > MaxFileBytes)
        {
            throw new ConfigurationValidationException("配置文件超过大小上限。");
        }

        var values = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        string? section = null;
        using var reader = new StringReader(text);
        string? line;
        while ((line = reader.ReadLine()) is not null)
        {
            if (line.Length > 2048)
            {
                throw new ConfigurationValidationException("配置行超过长度上限。");
            }

            var trimmed = line.Trim();
            if (trimmed.Length == 0 || trimmed.StartsWith(';') || trimmed.StartsWith('#'))
            {
                continue;
            }

            if (trimmed.StartsWith('[') && trimmed.EndsWith(']'))
            {
                section = trimmed[1..^1].Trim();
                if (!AllowedSections.Contains(section))
                {
                    throw new ConfigurationValidationException("配置 section 不受支持。");
                }

                continue;
            }

            if (section is null)
            {
                throw new ConfigurationValidationException("配置键必须位于白名单 section 中。");
            }

            var separator = trimmed.IndexOf('=');
            if (separator <= 0)
            {
                throw new ConfigurationValidationException("配置行格式无效。");
            }

            var key = trimmed[..separator].Trim();
            var value = trimmed[(separator + 1)..].Trim();
            if (!AllowedKeys[section].Contains(key))
            {
                throw new ConfigurationValidationException($"配置键不受支持：{section}.{key}。");
            }

            var qualifiedKey = $"{section}.{key}";
            if (!values.TryAdd(qualifiedKey, value))
            {
                throw new ConfigurationValidationException($"配置键重复：{qualifiedKey}。");
            }
        }

        return CreatePreferences(values);
    }

    internal static string Serialize(UserPreferences preferences)
    {
        ArgumentNullException.ThrowIfNull(preferences);
        preferences.Validate();

        static string FormatCoordinate(double? value) => value?.ToString("R", CultureInfo.InvariantCulture) ?? string.Empty;
        var builder = new StringBuilder(capacity: 512);
        builder.AppendLine("; GpAutoLive desktop preferences - non-sensitive values only");
        builder.AppendLine("[meta]");
        builder.Append("schema_version=").AppendLine(preferences.SchemaVersion.ToString(CultureInfo.InvariantCulture));
        builder.AppendLine();
        builder.AppendLine("[window]");
        builder.Append("width=").AppendLine(preferences.WindowWidth.ToString(CultureInfo.InvariantCulture));
        builder.Append("height=").AppendLine(preferences.WindowHeight.ToString(CultureInfo.InvariantCulture));
        builder.Append("left=").AppendLine(FormatCoordinate(preferences.WindowLeft));
        builder.Append("top=").AppendLine(FormatCoordinate(preferences.WindowTop));
        builder.AppendLine();
        builder.AppendLine("[ui]");
        builder.Append("theme=").AppendLine(preferences.Theme);
        builder.Append("language=").AppendLine(preferences.Language);
        builder.Append("effects_panel_expanded=").AppendLine(preferences.EffectsPanelExpanded.ToString().ToLowerInvariant());
        builder.Append("performance_sampling_enabled=").AppendLine(preferences.PerformanceSamplingEnabled.ToString().ToLowerInvariant());
        builder.Append("last_output_mode=").AppendLine(preferences.LastOutputMode);
        builder.AppendLine();
        builder.AppendLine("[effects]");
        builder.Append("video_cycle_period_min_ms=").AppendLine(preferences.VideoCyclePeriodMinMs.ToString(CultureInfo.InvariantCulture));
        builder.Append("video_cycle_period_max_ms=").AppendLine(preferences.VideoCyclePeriodMaxMs.ToString(CultureInfo.InvariantCulture));
        builder.Append("audio_cycle_period_min_ms=").AppendLine(preferences.AudioCyclePeriodMinMs.ToString(CultureInfo.InvariantCulture));
        builder.Append("audio_cycle_period_max_ms=").AppendLine(preferences.AudioCyclePeriodMaxMs.ToString(CultureInfo.InvariantCulture));
        return builder.ToString();
    }

    private static UserPreferences CreatePreferences(IReadOnlyDictionary<string, string> values)
    {
        var schemaVersion = ParseInt(values, "meta.schema_version", required: true);
        var width = ParseInt(values, "window.width", required: true);
        var height = ParseInt(values, "window.height", required: true);
        var left = ParseCoordinate(values, "window.left");
        var top = ParseCoordinate(values, "window.top");
        var theme = ParseString(values, "ui.theme", required: true);
        var language = ParseString(values, "ui.language", required: true);
        var effectsExpanded = ParseBool(values, "ui.effects_panel_expanded", required: true);
        var performanceSampling = ParseBool(values, "ui.performance_sampling_enabled", required: true);
        var outputMode = ParseString(values, "ui.last_output_mode", required: true);
        var videoCycleMin = ParseULong(values, "effects.video_cycle_period_min_ms", UserPreferences.DefaultVideoCyclePeriodMinMs);
        var videoCycleMax = ParseULong(values, "effects.video_cycle_period_max_ms", UserPreferences.DefaultVideoCyclePeriodMaxMs);
        var audioCycleMin = ParseULong(values, "effects.audio_cycle_period_min_ms", UserPreferences.DefaultAudioCyclePeriodMinMs);
        var audioCycleMax = ParseULong(values, "effects.audio_cycle_period_max_ms", UserPreferences.DefaultAudioCyclePeriodMaxMs);

        if (schemaVersion != UserPreferences.CurrentSchemaVersion)
        {
            throw new ConfigurationValidationException("桌面偏好版本不受支持。");
        }

        if (!UserPreferences.IsAllowedTheme(theme))
        {
            throw new ConfigurationValidationException("主题值不受支持。");
        }

        if (!UserPreferences.IsAllowedLanguage(language))
        {
            throw new ConfigurationValidationException("语言值不受支持。");
        }

        if (!UserPreferences.IsAllowedOutputMode(outputMode))
        {
            throw new ConfigurationValidationException("输出模式值不受支持。");
        }

        var preferences = new UserPreferences
        {
            SchemaVersion = schemaVersion,
            WindowWidth = width,
            WindowHeight = height,
            WindowLeft = left,
            WindowTop = top,
            Theme = UserPreferences.NormalizeTheme(theme),
            Language = UserPreferences.NormalizeLanguage(language),
            EffectsPanelExpanded = effectsExpanded,
            PerformanceSamplingEnabled = performanceSampling,
            LastOutputMode = UserPreferences.NormalizeOutputMode(outputMode),
            VideoCyclePeriodMinMs = videoCycleMin,
            VideoCyclePeriodMaxMs = videoCycleMax,
            AudioCyclePeriodMinMs = audioCycleMin,
            AudioCyclePeriodMaxMs = audioCycleMax,
        };
        preferences.Validate();
        return preferences;
    }

    private static int ParseInt(IReadOnlyDictionary<string, string> values, string key, bool required)
    {
        var value = ParseString(values, key, required);
        if (!int.TryParse(value, NumberStyles.Integer, CultureInfo.InvariantCulture, out var parsed))
        {
            throw new ConfigurationValidationException($"配置值格式无效：{key}。");
        }

        return parsed;
    }

    private static ulong ParseULong(IReadOnlyDictionary<string, string> values, string key, ulong fallback)
    {
        var value = ParseString(values, key, required: false);
        return value.Length == 0
            ? fallback
            : ulong.TryParse(value, NumberStyles.None, CultureInfo.InvariantCulture, out var parsed)
                ? parsed
                : throw new ConfigurationValidationException($"配置值格式无效：{key}。");
    }

    private static double? ParseCoordinate(IReadOnlyDictionary<string, string> values, string key)
    {
        var value = ParseString(values, key, required: false);
        if (value.Length == 0)
        {
            return null;
        }

        if (!double.TryParse(value, NumberStyles.Float, CultureInfo.InvariantCulture, out var parsed)
            || double.IsNaN(parsed)
            || double.IsInfinity(parsed))
        {
            throw new ConfigurationValidationException($"配置值格式无效：{key}。");
        }

        return parsed;
    }

    private static bool ParseBool(IReadOnlyDictionary<string, string> values, string key, bool required)
    {
        var value = ParseString(values, key, required);
        if (!bool.TryParse(value, out var parsed))
        {
            throw new ConfigurationValidationException($"配置值格式无效：{key}。");
        }

        return parsed;
    }

    private static string ParseString(IReadOnlyDictionary<string, string> values, string key, bool required)
    {
        if (!values.TryGetValue(key, out var value))
        {
            if (required)
            {
                throw new ConfigurationValidationException($"缺少配置键：{key}。");
            }

            return string.Empty;
        }

        return value;
    }
}
