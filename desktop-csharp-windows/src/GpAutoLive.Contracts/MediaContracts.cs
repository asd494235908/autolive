using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>媒体源类型，与现有桌面端的播放池契约保持一致。</summary>
public enum MediaKind
{
    /// <summary>视频媒体。</summary>
    Video,
    /// <summary>纯音频媒体。</summary>
    Audio
}

/// <summary>媒体兼容性模式；媒体探测和播放实现留在后续阶段。</summary>
public enum MediaCompatibilityMode
{
    /// <summary>直接读取原始媒体。</summary>
    Direct,
    /// <summary>经过封装重排。</summary>
    Remuxed,
    /// <summary>经过转码。</summary>
    Transcoded
}

/// <summary>播放池中的一个已探测媒体源。对象使用 record 保持快照不可变。</summary>
public sealed record SourceMediaDto(
    [property: JsonPropertyName("source_path")] string SourcePath,
    [property: JsonPropertyName("playback_reference")] string PlaybackReference,
    [property: JsonPropertyName("media_kind")] MediaKind MediaKind,
    [property: JsonPropertyName("compatibility_mode")] MediaCompatibilityMode CompatibilityMode,
    [property: JsonPropertyName("file_name")] string FileName,
    [property: JsonPropertyName("file_size_bytes")] ulong FileSizeBytes,
    [property: JsonPropertyName("duration_ms")] ulong? DurationMs,
    [property: JsonPropertyName("audio_start_ms")] ulong? AudioStartMs,
    [property: JsonPropertyName("audio_end_ms")] ulong? AudioEndMs,
    [property: JsonPropertyName("width")] uint? Width,
    [property: JsonPropertyName("height")] uint? Height,
    [property: JsonPropertyName("frame_rate_fps")] double? FrameRateFps,
    [property: JsonPropertyName("audio_sample_rate_hz")] uint? AudioSampleRateHz,
    [property: JsonPropertyName("audio_channel_count")] ushort? AudioChannelCount,
    [property: JsonPropertyName("video_codec_name")] string? VideoCodecName,
    [property: JsonPropertyName("audio_codec_name")] string? AudioCodecName,
    [property: JsonPropertyName("mp4_sha256")] string? Mp4Sha256,
    [property: JsonPropertyName("mp4_hash_status")] string Mp4HashStatus);

/// <summary>用户选择媒体时提交的最小探测请求。</summary>
public sealed record MediaProbeRequestDto(
    [property: JsonPropertyName("path")] string Path);

/// <summary>媒体探测成功后的结果。</summary>
public sealed record MediaProbeResultDto(
    [property: JsonPropertyName("canonical_path")] string CanonicalPath,
    [property: JsonPropertyName("source")] SourceMediaDto Source);
