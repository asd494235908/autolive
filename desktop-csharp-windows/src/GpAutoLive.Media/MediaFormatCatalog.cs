using System.Collections.Frozen;
using GpAutoLive.Contracts;

namespace GpAutoLive.Media;

/// <summary>
/// 媒体导入扩展名白名单。扩展名只用于选择过滤，最终可读性必须由 FFprobe 判定。
/// </summary>
public static class MediaFormatCatalog
{
    public static FrozenSet<string> VideoExtensions { get; } =
        MediaPoolRules.VideoExtensions;

    public static FrozenSet<string> AudioExtensions { get; } =
        MediaPoolRules.AudioExtensions;

    public static bool IsSupportedExtension(string? path) =>
        TryGetKind(path, out _);

    public static bool TryGetKind(string? path, out MediaKind kind)
    {
        if (string.IsNullOrEmpty(path))
        {
            kind = default;
            return false;
        }

        string extension;
        try
        {
            extension = Path.GetExtension(path);
        }
        catch (ArgumentException)
        {
            kind = default;
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

        kind = default;
        return false;
    }
}
