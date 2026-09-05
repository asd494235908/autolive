using System.Collections.Frozen;
using System.Collections.Immutable;
using System.IO;
using System.Text;
using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>插话文件池的稳定输入规则。</summary>
public static class InterludePoolRules
{
    /// <summary>递归插话目录允许的最大媒体文件数。</summary>
    public const int MaxItems = 1_000;

    /// <summary>单个规范化路径的 UTF-8 字节上限。</summary>
    public const int MaxPathBytes = 4 * 1024;

    /// <summary>目录快照内所有路径的 UTF-8 总字节上限。</summary>
    public const int MaxCatalogPathBytes = 512 * 1024;

    /// <summary>递归扫描沿用播放池的 17 种媒体扩展名。</summary>
    public static FrozenSet<string> SupportedExtensions => MediaPoolRules.SupportedExtensions;

    /// <summary>按扩展名判定插话媒体类别；视频只作为音频源交给后续混音链。</summary>
    public static bool TryGetMediaKind(string? path, out MediaKind kind) =>
        MediaPoolRules.TryGetMediaKind(path ?? string.Empty, out kind);

    /// <summary>判断路径是否位于目录本身或其子目录内。</summary>
    public static bool IsPathWithinDirectory(string path, string directory)
    {
        var root = directory.EndsWith(Path.DirectorySeparatorChar)
            || directory.EndsWith(Path.AltDirectorySeparatorChar)
            ? directory
            : directory + Path.DirectorySeparatorChar;
        return path.StartsWith(root, StringComparison.OrdinalIgnoreCase)
            || string.Equals(path, directory, StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>返回 UTF-8 字节长度，用于路径边界校验。</summary>
    public static int GetUtf8ByteCount(string value) => Encoding.UTF8.GetByteCount(value);
}

/// <summary>递归插话目录中的一个已确认文件。</summary>
public sealed record InterludeFileEntry(
    [property: JsonPropertyName("path")] string Path,
    [property: JsonPropertyName("file_name")] string FileName,
    [property: JsonPropertyName("media_kind")] MediaKind MediaKind,
    [property: JsonPropertyName("file_size_bytes")] ulong FileSizeBytes);

/// <summary>插话池扫描状态。</summary>
public enum InterludePoolStatus
{
    /// <summary>目录存在但未发现受支持文件。</summary>
    Empty,
    /// <summary>扫描完成并包含候选文件。</summary>
    Ready,
    /// <summary>尚未配置目录或已清空。</summary>
    Disabled,
    /// <summary>保留给持久化/远端状态投影的失败状态。</summary>
    Failed
}

/// <summary>插话目录的不可变快照，供 UI 和后续混音协调器读取。</summary>
public sealed record InterludePoolSnapshot(
    [property: JsonPropertyName("directory")] string? Directory,
    [property: JsonPropertyName("files")] ImmutableArray<InterludeFileEntry> Files,
    [property: JsonPropertyName("status")] InterludePoolStatus Status,
    [property: JsonPropertyName("error")] string? Error = null)
{
    /// <summary>进程启动时的空快照。</summary>
    public static InterludePoolSnapshot Initial { get; } =
        new(null, [], InterludePoolStatus.Disabled);
}

/// <summary>插话目录扫描错误分类。</summary>
public enum InterludePoolErrorCode
{
    /// <summary>输入为空或格式无效。</summary>
    InvalidDirectory,
    /// <summary>目录不存在或不可访问。</summary>
    DirectoryNotFound,
    /// <summary>单个路径超过边界。</summary>
    PathTooLong,
    /// <summary>快照路径总字节数超过边界。</summary>
    CatalogTooLarge,
    /// <summary>候选文件数量超过边界。</summary>
    TooManyFiles,
    /// <summary>根目录读取失败。</summary>
    DirectoryReadFailed,
    /// <summary>扫描被取消。</summary>
    OperationCancelled
}

/// <summary>不向 UI 泄露系统异常、命令行或完整扫描错误正文。</summary>
public sealed record InterludePoolError(
    InterludePoolErrorCode Code,
    string Message);

/// <summary>插话目录扫描或清空结果。</summary>
public sealed record InterludePoolResult(
    bool IsSuccess,
    bool Changed,
    InterludePoolSnapshot Snapshot,
    InterludePoolError? Error = null)
{
    /// <summary>创建成功结果。</summary>
    public static InterludePoolResult Succeeded(InterludePoolSnapshot snapshot, bool changed) =>
        new(true, changed, snapshot);

    /// <summary>创建失败结果；快照保持不变。</summary>
    public static InterludePoolResult Failed(InterludePoolSnapshot snapshot, InterludePoolError error) =>
        new(false, false, snapshot, error);
}
