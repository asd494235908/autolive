using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>视频、音频和高级视觉效果的统一参数根模型。</summary>
public sealed record MediaEffectParams
{
    /// <summary>普通声音效果参数。</summary>
    [JsonPropertyName("audio")]
    public AudioEffectParams Audio { get; init; } = AudioEffectParams.Default;

    /// <summary>普通视频效果参数。</summary>
    [JsonPropertyName("video")]
    public VideoEffectParams Video { get; init; } = VideoEffectParams.Default;

    /// <summary>高级视觉效果参数。</summary>
    [JsonPropertyName("advanced")]
    public AdvancedEffectParams Advanced { get; init; } = AdvancedEffectParams.Default;

    /// <summary>返回一份新的正式默认参数快照。</summary>
    public static MediaEffectParams Default => new();

    /// <summary>校验完整参数树。</summary>
    public bool TryValidate(out IReadOnlyList<MediaEffectValidationError> errors) =>
        MediaEffectValidation.TryValidate(this, out errors);
}

/// <summary>媒体契约使用的固定视觉频段。</summary>
public static class MediaEffectContracts
{
    /// <summary>固定视觉频段，单位为 Hz。</summary>
    public static IReadOnlyList<uint> VisualBandFrequenciesHz { get; } =
        [65, 92, 131, 188, 267, 381, 544, 777, 1_110, 1_585, 2_263, 20_000];
}
