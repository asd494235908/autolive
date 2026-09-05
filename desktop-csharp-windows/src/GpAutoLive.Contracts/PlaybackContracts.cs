using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>播放状态是跨 GUI、核心和本机消息的稳定事实。</summary>
public enum PlaybackState
{
    /// <summary>播放池已准备好但尚未播放。</summary>
    Ready,
    /// <summary>正在播放。</summary>
    Playing,
    /// <summary>已暂停。</summary>
    Paused,
    /// <summary>已停止或播放池为空。</summary>
    Stopped
}

/// <summary>发送给界面的最小播放快照；不包含媒体处理实现细节。</summary>
public sealed record PlaybackSnapshotDto(
    [property: JsonPropertyName("window_id")] string? WindowId,
    [property: JsonPropertyName("playback_generation")] ulong PlaybackGeneration,
    [property: JsonPropertyName("playback_state")] PlaybackState PlaybackState,
    [property: JsonPropertyName("source_revision")] ulong SourceRevision,
    [property: JsonPropertyName("loop_index")] ulong LoopIndex,
    [property: JsonPropertyName("source_media")] SourceMediaDto? SourceMedia,
    [property: JsonPropertyName("source_media_pool")] IReadOnlyList<SourceMediaDto> SourceMediaPool,
    [property: JsonPropertyName("source_media_index")] int SourceMediaIndex);

/// <summary>控制面错误的最小脱敏 DTO。</summary>
public sealed record ApiErrorDto(
    [property: JsonPropertyName("code")] string Code,
    [property: JsonPropertyName("message")] string Message,
    [property: JsonPropertyName("request_id")] string RequestId);
