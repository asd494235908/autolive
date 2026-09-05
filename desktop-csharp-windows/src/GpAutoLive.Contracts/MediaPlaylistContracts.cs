using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>本地媒体列表 JSON 的业务数据；只保存路径，不保存运行时探测元数据。</summary>
public sealed record MediaPlaylistData
{
    /// <summary>按播放顺序排列的本地媒体项。</summary>
    [JsonPropertyName("items")]
    public IReadOnlyList<MediaPlaylistItem?>? Items { get; init; }
}

/// <summary>本地媒体列表中的一个媒体路径。</summary>
public sealed record MediaPlaylistItem
{
    /// <summary>本地媒体绝对路径。</summary>
    [JsonPropertyName("path")]
    public string? Path { get; init; }
}
