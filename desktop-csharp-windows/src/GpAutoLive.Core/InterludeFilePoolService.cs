using System.Collections.Immutable;
using System.IO;
using GpAutoLive.Contracts;

namespace GpAutoLive.Core;

/// <summary>
/// 插话目录扫描的唯一状态所有者。只建立有界、可排序的文件快照，不启动解码或输出设备。
/// </summary>
public sealed class InterludeFilePoolService
{
    private readonly object _gate = new();
    private InterludePoolSnapshot _snapshot = InterludePoolSnapshot.Initial;

    /// <summary>返回当前不可变插话目录快照。</summary>
    public InterludePoolSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return _snapshot;
            }
        }
    }

    /// <summary>
    /// 递归扫描目录。只有完整扫描成功后才替换快照；单项访问失败按参考端语义跳过。
    /// </summary>
    public InterludePoolResult ScanDirectory(
        string? directory,
        CancellationToken cancellationToken = default)
    {
        var input = directory?.Trim();
        if (string.IsNullOrWhiteSpace(input))
        {
            return Fail(
                InterludePoolErrorCode.InvalidDirectory,
                "插话目录不能为空。");
        }

        string canonicalDirectory;
        try
        {
            canonicalDirectory = Path.GetFullPath(input);
        }
        catch (ArgumentException)
        {
            return Fail(InterludePoolErrorCode.InvalidDirectory, "插话目录路径无效。");
        }
        catch (NotSupportedException)
        {
            return Fail(InterludePoolErrorCode.InvalidDirectory, "插话目录路径格式不受支持。");
        }

        if (InterludePoolRules.GetUtf8ByteCount(canonicalDirectory) > InterludePoolRules.MaxPathBytes)
        {
            return Fail(
                InterludePoolErrorCode.PathTooLong,
                $"插话目录路径超过 {InterludePoolRules.MaxPathBytes} 字节上限。");
        }

        var rootInfo = new DirectoryInfo(canonicalDirectory);
        if (!rootInfo.Exists)
        {
            return Fail(InterludePoolErrorCode.DirectoryNotFound, "插话目录不存在或不可访问。");
        }

        try
        {
            if ((rootInfo.Attributes & FileAttributes.ReparsePoint) != 0)
            {
                var resolvedRoot = rootInfo.ResolveLinkTarget(returnFinalTarget: true);
                if (resolvedRoot is null)
                {
                    return Fail(InterludePoolErrorCode.DirectoryReadFailed, "插话目录链接目标不可访问。");
                }

                canonicalDirectory = resolvedRoot.FullName;
                if (InterludePoolRules.GetUtf8ByteCount(canonicalDirectory) > InterludePoolRules.MaxPathBytes)
                {
                    return Fail(
                        InterludePoolErrorCode.PathTooLong,
                        $"插话目录路径超过 {InterludePoolRules.MaxPathBytes} 字节上限。");
                }
            }
        }
        catch (IOException)
        {
            return Fail(InterludePoolErrorCode.DirectoryReadFailed, "插话目录链接目标读取失败。");
        }
        catch (UnauthorizedAccessException)
        {
            return Fail(InterludePoolErrorCode.DirectoryReadFailed, "插话目录链接目标访问被拒绝。");
        }

        var entries = new List<InterludeFileEntry>(capacity: 64);
        var totalPathBytes = InterludePoolRules.GetUtf8ByteCount(canonicalDirectory);
        var visitedDirectories = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        {
            canonicalDirectory,
        };
        var pending = new Stack<DirectoryInfo>();
        pending.Push(new DirectoryInfo(canonicalDirectory));

        try
        {
            while (pending.Count > 0)
            {
                cancellationToken.ThrowIfCancellationRequested();
                var current = pending.Pop();
                try
                {
                    foreach (var child in current.EnumerateFileSystemInfos())
                    {
                        cancellationToken.ThrowIfCancellationRequested();
                        if ((child.Attributes & FileAttributes.ReparsePoint) != 0)
                        {
                            continue;
                        }

                        if ((child.Attributes & FileAttributes.Directory) != 0)
                        {
                            var childDirectory = child.FullName;
                            if (!InterludePoolRules.IsPathWithinDirectory(childDirectory, canonicalDirectory)
                                || !visitedDirectories.Add(childDirectory))
                            {
                                continue;
                            }

                            if (InterludePoolRules.GetUtf8ByteCount(childDirectory) > InterludePoolRules.MaxPathBytes)
                            {
                                return Fail(
                                    InterludePoolErrorCode.PathTooLong,
                                    $"插话子目录路径超过 {InterludePoolRules.MaxPathBytes} 字节上限。");
                            }

                            pending.Push(new DirectoryInfo(childDirectory));
                            continue;
                        }

                        if (!InterludePoolRules.TryGetMediaKind(child.FullName, out var kind))
                        {
                            continue;
                        }

                        var canonicalPath = child.FullName;
                        if (!InterludePoolRules.IsPathWithinDirectory(canonicalPath, canonicalDirectory)
                            || InterludePoolRules.GetUtf8ByteCount(canonicalPath) > InterludePoolRules.MaxPathBytes)
                        {
                            return Fail(
                                InterludePoolErrorCode.PathTooLong,
                                $"插话文件路径超过 {InterludePoolRules.MaxPathBytes} 字节上限。");
                        }

                        if (entries.Count >= InterludePoolRules.MaxItems)
                        {
                            return Fail(
                                InterludePoolErrorCode.TooManyFiles,
                                $"插话目录最多允许 {InterludePoolRules.MaxItems} 个媒体文件。");
                        }

                        ulong fileSize;
                        try
                        {
                            fileSize = checked((ulong)new FileInfo(canonicalPath).Length);
                        }
                        catch (IOException)
                        {
                            continue;
                        }
                        catch (UnauthorizedAccessException)
                        {
                            continue;
                        }

                        totalPathBytes = checked(totalPathBytes + InterludePoolRules.GetUtf8ByteCount(canonicalPath));
                        if (totalPathBytes > InterludePoolRules.MaxCatalogPathBytes)
                        {
                            return Fail(
                                InterludePoolErrorCode.CatalogTooLarge,
                                $"插话目录路径总长度超过 {InterludePoolRules.MaxCatalogPathBytes} 字节上限。");
                        }

                        entries.Add(new InterludeFileEntry(
                            canonicalPath,
                            Path.GetFileName(canonicalPath),
                            kind,
                            fileSize));
                    }
                }
                catch (UnauthorizedAccessException)
                {
                    if (string.Equals(current.FullName, canonicalDirectory, StringComparison.OrdinalIgnoreCase))
                    {
                        throw;
                    }

                    continue;
                }
                catch (IOException)
                {
                    if (string.Equals(current.FullName, canonicalDirectory, StringComparison.OrdinalIgnoreCase))
                    {
                        throw;
                    }

                    continue;
                }

            }
        }
        catch (OperationCanceledException)
        {
            return Fail(InterludePoolErrorCode.OperationCancelled, "插话目录扫描已取消。");
        }
        catch (OverflowException)
        {
            return Fail(InterludePoolErrorCode.CatalogTooLarge, "插话目录路径总长度超出安全上限。");
        }
        catch (UnauthorizedAccessException)
        {
            return Fail(InterludePoolErrorCode.DirectoryReadFailed, "插话目录读取被系统拒绝。");
        }
        catch (IOException)
        {
            return Fail(InterludePoolErrorCode.DirectoryReadFailed, "插话目录读取失败。");
        }

        entries.Sort(static (left, right) =>
            StringComparer.OrdinalIgnoreCase.Compare(left.Path, right.Path));
        var next = new InterludePoolSnapshot(
            canonicalDirectory,
            entries.ToImmutableArray(),
            entries.Count == 0 ? InterludePoolStatus.Empty : InterludePoolStatus.Ready);

        lock (_gate)
        {
            var changed = !_snapshot.Equals(next);
            _snapshot = next;
            return InterludePoolResult.Succeeded(next, changed);
        }
    }

    /// <summary>清空目录快照，不删除磁盘上的任何文件。</summary>
    public InterludePoolResult Clear()
    {
        var next = InterludePoolSnapshot.Initial;
        lock (_gate)
        {
            var changed = !_snapshot.Equals(next);
            _snapshot = next;
            return InterludePoolResult.Succeeded(next, changed);
        }
    }

    private InterludePoolResult Fail(InterludePoolErrorCode code, string message)
    {
        lock (_gate)
        {
            return InterludePoolResult.Failed(_snapshot, new InterludePoolError(code, message));
        }
    }
}
