using System.Collections.Frozen;
using System.IO;

namespace GpAutoLive.Contracts;

/// <summary>
/// 本地播放池的稳定输入规则。扩展名只用于选择过滤；媒体能否播放仍由后续 FFprobe 探测决定。
/// </summary>
public static class MediaPoolRules
{
    /// <summary>播放池允许的最大条目数。</summary>
    public const int MaxItems = 100;

    /// <summary>规范化源路径的 UTF-8 字节上限，与现有桌面端命令边界一致。</summary>
    public const int MaxSourcePathBytes = 32 * 1024;

    /// <summary>允许的视频扩展名（小写，含点号）。</summary>
    public static FrozenSet<string> VideoExtensions { get; } =
        new[] { ".mp4", ".mov", ".mkv", ".avi", ".webm", ".m4v", ".ts", ".m2ts", ".flv", ".wmv", ".3gp" }
            .ToFrozenSet(StringComparer.OrdinalIgnoreCase);

    /// <summary>允许的纯音频扩展名（小写，含点号）。</summary>
    public static FrozenSet<string> AudioExtensions { get; } =
        new[] { ".mp3", ".wav", ".m4a", ".aac", ".ogg", ".flac" }
            .ToFrozenSet(StringComparer.OrdinalIgnoreCase);

    /// <summary>允许的全部媒体扩展名。</summary>
    public static FrozenSet<string> SupportedExtensions { get; } =
        VideoExtensions.Union(AudioExtensions).ToFrozenSet(StringComparer.OrdinalIgnoreCase);

    /// <summary>按文件名扩展名判断选择过滤使用的媒体类别。</summary>
    public static bool TryGetMediaKind(string path, out MediaKind kind)
    {
        kind = default;
        if (string.IsNullOrWhiteSpace(path))
        {
            return false;
        }

        string extension;
        try
        {
            extension = Path.GetExtension(path);
        }
        catch (ArgumentException)
        {
            return false;
        }

        if (VideoExtensions.Contains(extension))
        {
            kind = MediaKind.Video;
            return true;
        }

        if (AudioExtensions.Contains(extension))
        {
            kind = MediaKind.Audio;
            return true;
        }

        return false;
    }
}
