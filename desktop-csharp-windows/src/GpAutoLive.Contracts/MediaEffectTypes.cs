using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>自然真人声音模式。</summary>
[JsonConverter(typeof(JsonStringEnumConverter))]
public enum NaturalVoiceMode
{
    /// <summary>保持原声。</summary>
    Original,

    /// <summary>允许本地处理器使用自然动态参数。</summary>
    NaturalDynamic
}

/// <summary>单个媒体参数校验失败的稳定结构。</summary>
public sealed record MediaEffectValidationError(
    [property: JsonPropertyName("field")] string Field,
    [property: JsonPropertyName("code")] string Code,
    [property: JsonPropertyName("unit")] string Unit,
    [property: JsonPropertyName("value")] double? Value,
    [property: JsonPropertyName("min")] double? Min,
    [property: JsonPropertyName("max")] double? Max,
    [property: JsonPropertyName("message")] string Message);
