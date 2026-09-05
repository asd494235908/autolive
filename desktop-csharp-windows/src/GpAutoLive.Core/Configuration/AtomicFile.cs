using System.Text;

namespace GpAutoLive.Core.Configuration;

/// <summary>同目录临时文件 + Flush + 同卷替换，避免写坏唯一配置副本。</summary>
internal static class AtomicFile
{
    public static async Task WriteTextAsync(
        string path,
        ReadOnlyMemory<byte> content,
        CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);

        var fullPath = Path.GetFullPath(path);
        var directory = Path.GetDirectoryName(fullPath)
            ?? throw new ConfigurationValidationException("配置路径必须包含目录。");

        Directory.CreateDirectory(directory);
        var temporaryPath = Path.Combine(
            directory,
            $".{Path.GetFileName(fullPath)}.{Guid.NewGuid():N}.tmp");

        try
        {
            await using (var stream = new FileStream(
                temporaryPath,
                FileMode.CreateNew,
                FileAccess.Write,
                FileShare.None,
                bufferSize: 16 * 1024,
                options: FileOptions.Asynchronous | FileOptions.WriteThrough))
            {
                await stream.WriteAsync(content, cancellationToken).ConfigureAwait(false);
                await stream.FlushAsync(cancellationToken).ConfigureAwait(false);
                stream.Flush(flushToDisk: true);
            }

            ReplaceWithinDirectory(temporaryPath, fullPath);
        }
        finally
        {
            TryDeleteTemporaryFile(temporaryPath);
        }
    }

    public static async Task<string> ReadTextAsync(
        string path,
        long maxBytes,
        CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);

        var fullPath = Path.GetFullPath(path);
        var info = new FileInfo(fullPath);
        if (!info.Exists)
        {
            throw new FileNotFoundException("配置文件不存在。", fullPath);
        }

        if (info.Length > maxBytes)
        {
            throw new ConfigurationValidationException("配置文件超过大小上限。");
        }

        using var stream = new FileStream(
            fullPath,
            FileMode.Open,
            FileAccess.Read,
            FileShare.Read,
            bufferSize: 16 * 1024,
            options: FileOptions.Asynchronous | FileOptions.SequentialScan);
        using var reader = new StreamReader(stream, new UTF8Encoding(encoderShouldEmitUTF8Identifier: false), detectEncodingFromByteOrderMarks: true);
        return await reader.ReadToEndAsync(cancellationToken).ConfigureAwait(false);
    }

    private static void ReplaceWithinDirectory(string temporaryPath, string destinationPath)
    {
        if (File.Exists(destinationPath))
        {
            File.Replace(temporaryPath, destinationPath, destinationBackupFileName: null, ignoreMetadataErrors: true);
            return;
        }

        File.Move(temporaryPath, destinationPath);
    }

    private static void TryDeleteTemporaryFile(string temporaryPath)
    {
        try
        {
            if (File.Exists(temporaryPath))
            {
                File.Delete(temporaryPath);
            }
        }
        catch (IOException)
        {
            // 原始写入/替换错误优先返回；清理失败不会覆盖它。
        }
        catch (UnauthorizedAccessException)
        {
            // 同上，避免在错误路径暴露配置内容。
        }
    }
}
