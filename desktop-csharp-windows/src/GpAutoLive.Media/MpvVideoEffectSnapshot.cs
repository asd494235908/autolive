using System.Collections.Immutable;
using System.Globalization;
using System.Text;
using System.Text.Json;
using GpAutoLive.Contracts;

namespace GpAutoLive.Media;

/// <summary>
/// mpv 视频处理的有限模式。GPU83、CPU4 和原始画面之间只允许单向降级，
/// C# 层不把 shader 当作任意脚本入口。
/// </summary>
public enum MpvVideoProcessingMode
{
    Original,
    Gpu83,
    Cpu4,
}

/// <summary>CPU4 只允许更新的四个固定参数。</summary>
public enum MpvCpu4Parameter
{
    Brightness,
    Contrast,
    Saturation,
    Hue,
}

/// <summary>视频参数快照校验失败的稳定分类。</summary>
public enum MpvVideoParameterFailureCode
{
    InvalidMode,
    InvalidCpu4Value,
    InvalidShaderOptions,
    ShaderOptionsNotAllowed,
}

/// <summary>只包含可展示文本的参数校验错误；不保留输入字符串。</summary>
public sealed record MpvVideoParameterError(
    MpvVideoParameterFailureCode Code,
    string Message);

/// <summary>
/// GPU83 使用的受限 shader 选项快照。键和值均为 ASCII 标量，不能包含逗号、
/// 等号、路径、换行或脚本片段；排序后序列化以保证快照可比较。
/// </summary>
public sealed class MpvShaderOptionsSnapshot
{
    public const int MaxEntries = 256;
    public const int MaxKeyBytes = 128;
    public const int MaxValueBytes = 256;
    public const int MaxSerializedBytes = 32 * 1024;

    private readonly ImmutableSortedDictionary<string, string> _values;

    private MpvShaderOptionsSnapshot(ImmutableSortedDictionary<string, string> values) =>
        _values = values;

    /// <summary>无 shader 选项的空快照。</summary>
    public static MpvShaderOptionsSnapshot Empty { get; } =
        new(ImmutableSortedDictionary.Create<string, string>(StringComparer.Ordinal));

    /// <summary>
    /// 创建 C# GPU83 基线 shader 使用的四项颜色参数。
    /// 参数名是固定白名单，不接受调用方传入 shader 代码或任意选项键。
    /// </summary>
    public static bool TryCreateBaselineColor(
        double brightnessPercent,
        double contrastPercent,
        double saturationPercent,
        double hueRotationDegrees,
        out MpvShaderOptionsSnapshot? snapshot,
        out MpvVideoParameterError? error)
    {
        snapshot = null;
        error = null;
        if (!IsFiniteInRange(brightnessPercent, -100, 100)
            || !IsFiniteInRange(contrastPercent, 0, 200)
            || !IsFiniteInRange(saturationPercent, 0, 200)
            || !IsFiniteInRange(hueRotationDegrees, -180, 180))
        {
            error = new(
                MpvVideoParameterFailureCode.InvalidCpu4Value,
                "GPU83 基线颜色参数超出产品范围。");
            return false;
        }

        return TryCreate(
            [
                new KeyValuePair<string, string>("al_brightness_percent", Format(brightnessPercent)),
                new KeyValuePair<string, string>("al_contrast_percent", Format(contrastPercent)),
                new KeyValuePair<string, string>("al_saturation_percent", Format(saturationPercent)),
                new KeyValuePair<string, string>("al_hue_degrees", Format(hueRotationDegrees)),
            ],
            out snapshot,
            out error);
    }

    /// <summary>当前规范化的只读键值集合。</summary>
    public IReadOnlyDictionary<string, string> Values => _values;

    /// <summary>是否没有任何选项。</summary>
    public bool IsEmpty => _values.Count == 0;

    /// <summary>
    /// 确认 mpv 返回的固定属性与本次提交的完整键值相同。
    /// mpv 通常返回规范化字符串，兼容部分版本返回的有界标量对象。
    /// 不把运行时原始正文继续向上层传播。
    /// </summary>
    public bool MatchesMpvReadback(JsonElement data)
    {
        if (!TryCreateReadbackSnapshot(data, out var actual)
            || actual is null)
        {
            return false;
        }

        return _values.SequenceEqual(actual._values);
    }

    private static bool TryCreateReadbackSnapshot(
        JsonElement data,
        out MpvShaderOptionsSnapshot? snapshot)
    {
        snapshot = null;
        var entries = new List<KeyValuePair<string, string>>();
        if (data.ValueKind is JsonValueKind.String)
        {
            var raw = data.GetString();
            if (raw is null)
            {
                return false;
            }

            if (raw.Length == 0)
            {
                snapshot = Empty;
                return true;
            }

            foreach (var entry in raw.Split(',', StringSplitOptions.None))
            {
                var separator = entry.IndexOf('=');
                if (separator <= 0
                    || separator == entry.Length - 1
                    || !TryNormalizeReadbackEntry(
                        entry[..separator],
                        entry[(separator + 1)..],
                        out var normalized))
                {
                    return false;
                }

                entries.Add(normalized);
            }
        }
        else if (data.ValueKind is JsonValueKind.Object)
        {
            foreach (var property in data.EnumerateObject())
            {
                var rawValue = property.Value.ValueKind switch
                {
                    JsonValueKind.String => property.Value.GetString(),
                    JsonValueKind.Number => property.Value.GetRawText(),
                    _ => null,
                };
                if (rawValue is null
                    || !TryNormalizeReadbackEntry(property.Name, rawValue, out var normalized))
                {
                    return false;
                }

                entries.Add(normalized);
            }
        }
        else
        {
            return false;
        }

        return TryCreate(entries, out snapshot, out _);
    }

    private static bool TryNormalizeReadbackEntry(
        string key,
        string value,
        out KeyValuePair<string, string> normalized)
    {
        normalized = default;
        if (!TryCreate(
                [new KeyValuePair<string, string>(key, value)],
                out _,
                out _))
        {
            return false;
        }

        if (double.TryParse(
                value,
                NumberStyles.Float,
                CultureInfo.InvariantCulture,
                out var number))
        {
            if (!double.IsFinite(number))
            {
                return false;
            }

            value = number.ToString("R", CultureInfo.InvariantCulture);
        }

        normalized = new(key, value);
        return true;
    }

    /// <summary>按 mpv glsl-shader-opts 约定生成稳定的 key=value,key=value 字符串。</summary>
    public string ToMpvValue() =>
        string.Join(",", _values.Select(pair => $"{pair.Key}={pair.Value}"));

    /// <summary>
    /// 从不可信输入创建快照。枚举、数量、字符集、总大小和有限数值均受限。
    /// </summary>
    public static bool TryCreate(
        IEnumerable<KeyValuePair<string, string>>? entries,
        out MpvShaderOptionsSnapshot? snapshot,
        out MpvVideoParameterError? error)
    {
        snapshot = null;
        error = null;
        if (entries is null)
        {
            error = InvalidOptions();
            return false;
        }

        var values = ImmutableSortedDictionary.CreateBuilder<string, string>(StringComparer.Ordinal);
        try
        {
            foreach (var pair in entries)
            {
                if (values.Count >= MaxEntries)
                {
                    error = InvalidOptions();
                    return false;
                }

                if (!IsValidToken(pair.Key, MaxKeyBytes, allowEquals: false)
                    || !IsValidToken(pair.Value, MaxValueBytes, allowEquals: false)
                    || !MpvGpu83ShaderSnapshot.IsSupportedShaderOption(pair.Key))
                {
                    error = InvalidOptions();
                    return false;
                }

                if (double.TryParse(
                        pair.Value,
                        NumberStyles.Float,
                        CultureInfo.InvariantCulture,
                        out var numeric)
                    && !double.IsFinite(numeric))
                {
                    error = InvalidOptions();
                    return false;
                }

                if (!values.TryAdd(pair.Key, pair.Value))
                {
                    error = InvalidOptions();
                    return false;
                }
            }
        }
        catch (Exception exception) when (exception is ArgumentException or InvalidOperationException)
        {
            error = InvalidOptions();
            return false;
        }

        var candidate = new MpvShaderOptionsSnapshot(values.ToImmutable());
        if (Encoding.UTF8.GetByteCount(candidate.ToMpvValue()) > MaxSerializedBytes)
        {
            error = InvalidOptions();
            return false;
        }

        snapshot = candidate;
        return true;
    }

    /// <summary>解析已有的 mpv 选项字符串，不允许空条目或重复键。</summary>
    public static bool TryParse(
        string? value,
        out MpvShaderOptionsSnapshot? snapshot,
        out MpvVideoParameterError? error)
    {
        snapshot = null;
        error = null;
        if (value is null || Encoding.UTF8.GetByteCount(value) > MaxSerializedBytes)
        {
            error = InvalidOptions();
            return false;
        }

        if (value.Length == 0)
        {
            snapshot = Empty;
            return true;
        }

        var entries = new List<KeyValuePair<string, string>>();
        foreach (var entry in value.Split(',', StringSplitOptions.None))
        {
            var separator = entry.IndexOf('=');
            if (separator <= 0 || separator == entry.Length - 1)
            {
                error = InvalidOptions();
                return false;
            }

            entries.Add(new KeyValuePair<string, string>(
                entry[..separator],
                entry[(separator + 1)..]));
        }

        return TryCreate(entries, out snapshot, out error);
    }

    private static bool IsValidToken(string value, int maxBytes, bool allowEquals)
    {
        if (string.IsNullOrEmpty(value) || Encoding.UTF8.GetByteCount(value) > maxBytes)
        {
            return false;
        }

        return value.All(character =>
            character is >= 'a' and <= 'z'
                or >= 'A' and <= 'Z'
                or >= '0' and <= '9'
                or '_'
                or '-'
                or '+'
                or '.'
                || (allowEquals && character == '='));
    }

    private static MpvVideoParameterError InvalidOptions() => new(
        MpvVideoParameterFailureCode.InvalidShaderOptions,
        "GPU83 参数快照无效或超过限制。");

    private static bool IsFiniteInRange(double value, double minimum, double maximum) =>
        double.IsFinite(value) && value >= minimum && value <= maximum;

    private static string Format(double value) =>
        value.ToString("0.###", CultureInfo.InvariantCulture);
}

/// <summary>
/// 一次视频效果更新的不可变快照。快照通过 TryCreate 或 Default 创建，
/// 生成 IPC 命令前仍会再次校验，以防 record with 绕过入口校验。
/// </summary>
public sealed record MpvVideoEffectSnapshot
{
    private MpvVideoEffectSnapshot(
        MpvVideoProcessingMode mode,
        double brightnessPercent,
        double contrastPercent,
        double saturationPercent,
        double hueRotationDegrees,
        MpvShaderOptionsSnapshot shaderOptions)
    {
        Mode = mode;
        BrightnessPercent = brightnessPercent;
        ContrastPercent = contrastPercent;
        SaturationPercent = saturationPercent;
        HueRotationDegrees = hueRotationDegrees;
        ShaderOptions = shaderOptions;
    }

    public MpvVideoProcessingMode Mode { get; init; }

    public double BrightnessPercent { get; init; }

    public double ContrastPercent { get; init; }

    public double SaturationPercent { get; init; }

    public double HueRotationDegrees { get; init; }

    public MpvShaderOptionsSnapshot ShaderOptions { get; init; }

    /// <summary>原始画面默认快照。</summary>
    public static MpvVideoEffectSnapshot Default { get; } = new(
        MpvVideoProcessingMode.Original,
        brightnessPercent: 0,
        contrastPercent: 100,
        saturationPercent: 100,
        hueRotationDegrees: 0,
        MpvShaderOptionsSnapshot.Empty);

    /// <summary>创建经过产品范围校验的参数快照。</summary>
    public static bool TryCreate(
        MpvVideoProcessingMode mode,
        double brightnessPercent,
        double contrastPercent,
        double saturationPercent,
        double hueRotationDegrees,
        MpvShaderOptionsSnapshot? shaderOptions,
        out MpvVideoEffectSnapshot? snapshot,
        out MpvVideoParameterError? error)
    {
        snapshot = null;
        error = null;
        if (!Enum.IsDefined(mode))
        {
            error = new(MpvVideoParameterFailureCode.InvalidMode, "视频处理模式无效。");
            return false;
        }

        if (!IsInRange(brightnessPercent, -100, 100)
            || !IsInRange(contrastPercent, 0, 200)
            || !IsInRange(saturationPercent, 0, 200)
            || !IsInRange(hueRotationDegrees, -180, 180))
        {
            error = new(MpvVideoParameterFailureCode.InvalidCpu4Value, "CPU4 参数超出产品范围。");
            return false;
        }

        if (shaderOptions is null)
        {
            error = new(MpvVideoParameterFailureCode.InvalidShaderOptions, "GPU83 参数快照不能为空。");
            return false;
        }

        if (mode is not MpvVideoProcessingMode.Gpu83 && !shaderOptions.IsEmpty)
        {
            error = new(
                MpvVideoParameterFailureCode.ShaderOptionsNotAllowed,
                "原始画面和 CPU4 模式不能携带 GPU83 参数。");
            return false;
        }

        snapshot = new MpvVideoEffectSnapshot(
            mode,
            brightnessPercent,
            contrastPercent,
            saturationPercent,
            hueRotationDegrees,
            shaderOptions);
        return true;
    }

    /// <summary>创建带 C# GPU83 基线颜色 shader 参数的受限快照。</summary>
    public static bool TryCreateGpu83BaselineColor(
        double brightnessPercent,
        double contrastPercent,
        double saturationPercent,
        double hueRotationDegrees,
        out MpvVideoEffectSnapshot? snapshot,
        out MpvVideoParameterError? error)
    {
        snapshot = null;
        if (!MpvShaderOptionsSnapshot.TryCreateBaselineColor(
            brightnessPercent,
            contrastPercent,
            saturationPercent,
            hueRotationDegrees,
            out var shaderOptions,
            out error)
            || shaderOptions is null)
        {
            return false;
        }

        return TryCreate(
            MpvVideoProcessingMode.Gpu83,
            brightnessPercent,
            contrastPercent,
            saturationPercent,
            hueRotationDegrees,
            shaderOptions,
            out snapshot,
            out error);
    }

    /// <summary>
    /// 将完整的视频/高级参数契约映射为受限 GPU83 mpv 快照。
    /// 调度输入与参数使用同一原子属性提交；不可验证的历史纹理字段只保留在映射结果中，
    /// 不会被伪装成已进入 shader 的选项。
    /// </summary>
    public static bool TryCreateGpu83(
        VideoEffectParams? video,
        AdvancedEffectParams? advanced,
        double sourceFps,
        double epochStartSeconds,
        uint randomSeed,
        out MpvVideoEffectSnapshot? snapshot,
        out MpvGpu83ShaderSnapshot? gpu83,
        out MpvVideoParameterError? error)
    {
        snapshot = null;
        gpu83 = null;
        if (!MpvGpu83ShaderSnapshot.TryCreate(
                video,
                advanced,
                sourceFps,
                epochStartSeconds,
                randomSeed,
                out gpu83,
                out error)
            || gpu83 is null
            || video is null)
        {
            return false;
        }

        return TryCreate(
            MpvVideoProcessingMode.Gpu83,
            video.BrightnessPercent,
            video.ContrastPercent,
            video.SaturationPercent,
            video.HueRotationDegrees,
            gpu83.ShaderOptions,
            out snapshot,
            out error);
    }

    /// <summary>再次校验快照，避免 record with 形成越界或空引用命令。</summary>
    public bool TryValidate(out MpvVideoParameterError? error)
    {
        error = null;
        if (!Enum.IsDefined(Mode))
        {
            error = new(MpvVideoParameterFailureCode.InvalidMode, "视频处理模式无效。");
            return false;
        }

        if (!IsInRange(BrightnessPercent, -100, 100)
            || !IsInRange(ContrastPercent, 0, 200)
            || !IsInRange(SaturationPercent, 0, 200)
            || !IsInRange(HueRotationDegrees, -180, 180))
        {
            error = new(MpvVideoParameterFailureCode.InvalidCpu4Value, "CPU4 参数超出产品范围。");
            return false;
        }

        if (ShaderOptions is null)
        {
            error = new(MpvVideoParameterFailureCode.InvalidShaderOptions, "GPU83 参数快照不能为空。");
            return false;
        }

        if (Mode is not MpvVideoProcessingMode.Gpu83 && !ShaderOptions.IsEmpty)
        {
            error = new(
                MpvVideoParameterFailureCode.ShaderOptionsNotAllowed,
                "原始画面和 CPU4 模式不能携带 GPU83 参数。");
            return false;
        }

        return true;
    }

    /// <summary>
    /// 确认 mpv 返回的固定 CPU4 滤镜链包含本次提交的四个实际值。
    /// mpv 的 `vf` 结构由外部进程定义，因此这里只匹配受管标签和固定参数文本。
    /// </summary>
    public bool MatchesCpu4Readback(JsonElement data)
    {
        if (Mode is not MpvVideoProcessingMode.Cpu4
            || data.ValueKind is not (JsonValueKind.Array or JsonValueKind.Object))
        {
            return false;
        }

        var raw = data.GetRawText();
        return raw.Contains("autolive_cpu4", StringComparison.Ordinal)
            && raw.Contains(
                $"brightness={Format(MappedCpu4Value(MpvCpu4Parameter.Brightness))}",
                StringComparison.Ordinal)
            && raw.Contains(
                $"contrast={Format(MappedCpu4Value(MpvCpu4Parameter.Contrast))}",
                StringComparison.Ordinal)
            && raw.Contains(
                $"saturation={Format(MappedCpu4Value(MpvCpu4Parameter.Saturation))}",
                StringComparison.Ordinal)
            && raw.Contains(
                $"hue=h={Format(MappedCpu4Value(MpvCpu4Parameter.Hue))}",
                StringComparison.Ordinal);
    }

    /// <summary>将 UI 百分比映射为 CPU4 mpv 滤镜使用的值。</summary>
    public double MappedCpu4Value(MpvCpu4Parameter parameter) => parameter switch
    {
        MpvCpu4Parameter.Brightness => BrightnessPercent / 100,
        MpvCpu4Parameter.Contrast => ContrastPercent / 100,
        MpvCpu4Parameter.Saturation => SaturationPercent / 100,
        MpvCpu4Parameter.Hue => HueRotationDegrees,
        _ => double.NaN,
    };

    private static bool IsInRange(double value, double min, double max) =>
        double.IsFinite(value) && value >= min && value <= max;

    private static string Format(double value) =>
        value.ToString("0.###", CultureInfo.InvariantCulture);
}
