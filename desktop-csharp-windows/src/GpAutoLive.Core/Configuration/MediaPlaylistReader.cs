using System.Security;
using System.Text;
using GpAutoLive.Contracts;

namespace GpAutoLive.Core.Configuration;

/// <summary>
/// 读取并验证本地媒体列表；文件内容只提供路径，媒体元数据仍由导入协调器重新 FFprobe。
/// </summary>
public sealed class MediaPlaylistReader
{
    /// <summary>当前支持的媒体列表 JSON schema 版本。</summary>
    public const int CurrentSchemaVersion = 1;

    private readonly VersionedJsonStore<MediaPlaylistData> _store;

    /// <summary>创建指定路径的媒体列表读取器。</summary>
    public MediaPlaylistReader(string path)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        _store = new VersionedJsonStore<MediaPlaylistData>(path, CurrentSchemaVersion);
    }

    /// <summary>读取顺序稳定、已规范化的本地媒体路径。</summary>
    public async Task<IReadOnlyList<string>> ReadPathsAsync(CancellationToken cancellationToken = default)
    {
        var document = await _store.ReadAsync(cancellationToken).ConfigureAwait(false);
        if (document?.Data?.Items is not IReadOnlyList<MediaPlaylistItem?> items)
        {
            throw new ConfigurationValidationException("媒体列表必须包含 items 数组。");
        }

        if (items.Count is 0 or > MediaPoolRules.MaxItems)
        {
            throw new ConfigurationValidationException(
                $"媒体列表条目数必须在 1～{MediaPoolRules.MaxItems} 项之间。");
        }

        var comparer = OperatingSystem.IsWindows()
            ? StringComparer.OrdinalIgnoreCase
            : StringComparer.Ordinal;
        var seen = new HashSet<string>(comparer);
        var paths = new string[items.Count];

        for (var index = 0; index < items.Count; index++)
        {
            if (!TryNormalizePath(items[index]?.Path, out var path))
            {
                throw new ConfigurationValidationException($"媒体列表第 {index + 1} 项路径无效。");
            }

            if (!seen.Add(path))
            {
                throw new ConfigurationValidationException($"媒体列表第 {index + 1} 项与其他路径重复。");
            }

            paths[index] = path;
        }

        return paths;
    }

    private static bool TryNormalizePath(string? value, out string path)
    {
        path = string.Empty;
        if (string.IsNullOrWhiteSpace(value))
        {
            return false;
        }

        var trimmed = value.Trim();
        var isUncPath = trimmed.StartsWith("\\\\", StringComparison.Ordinal)
            || trimmed.StartsWith("//", StringComparison.Ordinal);
        if (Encoding.UTF8.GetByteCount(trimmed) > MediaPoolRules.MaxSourcePathBytes
            || isUncPath
            || trimmed.Contains("://", StringComparison.Ordinal))
        {
            return false;
        }

        try
        {
            if (!Path.IsPathFullyQualified(trimmed))
            {
                return false;
            }

            var normalized = Path.TrimEndingDirectorySeparator(Path.GetFullPath(trimmed));
            if (Encoding.UTF8.GetByteCount(normalized) > MediaPoolRules.MaxSourcePathBytes
                || !MediaPoolRules.TryGetMediaKind(normalized, out _))
            {
                return false;
            }

            path = normalized;
            return true;
        }
        catch (Exception exception) when (
            exception is ArgumentException
            or IOException
            or NotSupportedException
            or SecurityException)
        {
            return false;
        }
    }
}
