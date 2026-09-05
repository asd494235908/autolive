using GpAutoLive.Contracts;
using GpAutoLive.Core.Processes;
using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>FFmpeg 首帧缩略图提取结果；错误不携带路径或外部进程原文。</summary>
public sealed record WindowsFfmpegThumbnailResult(
    bool IsSuccess,
    byte[]? ImageBytes = null,
    string? ErrorMessage = null,
    bool Retryable = false);

/// <summary>
/// 使用已验证 FFmpeg 资源在本地临时文件生成单帧 JPEG。
/// 该适配器不返回 WPF 类型，进程由现有受管执行边界负责。
/// </summary>
public sealed class WindowsFfmpegThumbnailExtractor
{
    private const int MaxImageBytes = 2 * 1024 * 1024;
    private readonly IExternalProcessRunner _processRunner;

    public WindowsFfmpegThumbnailExtractor(IExternalProcessRunner processRunner)
    {
        _processRunner = processRunner ?? throw new ArgumentNullException(nameof(processRunner));
    }

    public async Task<WindowsFfmpegThumbnailResult> ExtractAsync(
        VerifiedMediaRuntime? runtime,
        SourceMediaDto? source,
        CancellationToken cancellationToken = default)
    {
        if (runtime is null || source is null || source.MediaKind is not MediaKind.Video)
        {
            return Failure("缩略图媒体源无效。", retryable: false);
        }

        if (!runtime.TryGetResource("ffmpeg.exe", out var ffmpeg) || ffmpeg is null)
        {
            return Failure("已验证的媒体运行资源缺少 FFmpeg。", retryable: false);
        }

        var outputPath = Path.Combine(Path.GetTempPath(), $"gpautolive-thumbnail-{Guid.NewGuid():N}.jpg");
        try
        {
            var plan = FfmpegThumbnailCommandBuilder.Create(
                ffmpeg.AbsolutePath,
                source.SourcePath,
                outputPath);
            var result = await _processRunner.RunAsync(plan.ToExternalProcessPlan(), cancellationToken)
                .ConfigureAwait(false);
            if (result.Status is ExternalProcessRunStatus.Cancelled
                or ExternalProcessRunStatus.TimedOut)
            {
                return Failure(
                    result.Status is ExternalProcessRunStatus.Cancelled ? "缩略图提取已取消。" : "缩略图提取超时。",
                    retryable: true);
            }

            if (result.Status is not ExternalProcessRunStatus.Completed
                || result.ExitCode is not 0
                || !File.Exists(outputPath))
            {
                return Failure("媒体首帧缩略图提取失败。", retryable: true);
            }

            var fileInfo = new FileInfo(outputPath);
            if (fileInfo.Length is < 16 or > MaxImageBytes)
            {
                return Failure("媒体首帧缩略图大小无效。", retryable: false);
            }

            var bytes = await File.ReadAllBytesAsync(outputPath, cancellationToken).ConfigureAwait(false);
            if (!LooksLikeJpeg(bytes))
            {
                return Failure("媒体首帧缩略图格式无效。", retryable: false);
            }

            return new WindowsFfmpegThumbnailResult(true, bytes);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return Failure("缩略图提取已取消。", retryable: true);
        }
        catch (FfmpegThumbnailCommandValidationException)
        {
            return Failure("缩略图提取参数无效。", retryable: false);
        }
        catch (Exception exception) when (exception is IOException or UnauthorizedAccessException or NotSupportedException)
        {
            return Failure("缩略图文件当前不可访问。", retryable: true);
        }
        finally
        {
            TryDelete(outputPath);
        }
    }

    private static bool LooksLikeJpeg(byte[] bytes) =>
        bytes.Length >= 4
        && bytes[0] == 0xFF
        && bytes[1] == 0xD8
        && bytes[^2] == 0xFF
        && bytes[^1] == 0xD9;

    private static WindowsFfmpegThumbnailResult Failure(string message, bool retryable) =>
        new(false, ErrorMessage: message, Retryable: retryable);

    private static void TryDelete(string path)
    {
        try
        {
            if (File.Exists(path))
            {
                File.Delete(path);
            }
        }
        catch (IOException)
        {
            // 临时缩略图清理失败不覆盖原始提取结果。
        }
        catch (UnauthorizedAccessException)
        {
            // 同上。
        }
    }
}
