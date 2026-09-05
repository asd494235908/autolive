using System.Collections.Immutable;
using GpAutoLive.Contracts;
using GpAutoLive.Core.Processes;

namespace GpAutoLive.Media;

/// <summary>经过校验的 FFmpeg 首帧缩略图命令计划。</summary>
public sealed record FfmpegThumbnailCommandPlan(
    string ExecutablePath,
    ImmutableArray<string> Arguments,
    ProcessLaunchPolicy LaunchPolicy,
    TimeSpan Timeout,
    int MaxStandardOutputBytes,
    int MaxStandardErrorBytes)
{
    public ExternalProcessPlan ToExternalProcessPlan() => new(
        ExecutablePath,
        Arguments,
        LaunchPolicy,
        Timeout,
        MaxStandardOutputBytes,
        MaxStandardErrorBytes);
}

public sealed class FfmpegThumbnailCommandValidationException : Exception
{
    public FfmpegThumbnailCommandValidationException(MediaProbeFailureCode code, string message)
        : base(message) => Code = code;

    public MediaProbeFailureCode Code { get; }
}

/// <summary>只生成固定单帧提取参数，不启动进程或读取媒体。</summary>
public static class FfmpegThumbnailCommandBuilder
{
    private const string ExpectedExecutableFileName = "ffmpeg.exe";
    private const int MaxArgumentCount = 32;
    private const int MaxArgumentCharacters = 32_000;
    private const int MaxCommandCharacters = 32_767;
    private const int ThumbnailWidth = 192;
    private const int ThumbnailHeight = 108;

    public static FfmpegThumbnailCommandPlan Create(
        string executablePath,
        string mediaPath,
        string outputPath)
    {
        ValidateExecutablePath(executablePath);

        var sourceError = MediaPathPolicy.Validate(mediaPath, requireExistingFile: false, out var validatedSource);
        if (sourceError is not null || validatedSource.Kind is not MediaKind.Video)
        {
            throw new FfmpegThumbnailCommandValidationException(
                sourceError?.Code ?? MediaProbeFailureCode.UnsupportedMediaStream,
                "缩略图源必须是受支持的视频文件。");
        }

        var validatedOutput = ValidateOutputPath(outputPath);
        var arguments = ImmutableArray.Create(
            "-v", "error",
            "-hide_banner",
            "-nostdin",
            "-y",
            "-ss", "0.5",
            "-i", validatedSource.CanonicalPath,
            "-an",
            "-frames:v", "1",
            "-vf", $"scale={ThumbnailWidth}:{ThumbnailHeight}:force_original_aspect_ratio=decrease,pad={ThumbnailWidth}:{ThumbnailHeight}:(ow-iw)/2:(oh-ih)/2:color=black",
            "-q:v", "5",
            "-f", "image2",
            validatedOutput);

        if (arguments.Length > MaxArgumentCount
            || arguments.Any(static argument => argument.Length > MaxArgumentCharacters)
            || arguments.Sum(static argument => (long)argument.Length + 1) + executablePath.Length > MaxCommandCharacters)
        {
            throw new FfmpegThumbnailCommandValidationException(
                MediaProbeFailureCode.PathTooLong,
                "缩略图提取参数超过 Windows 命令长度限制。");
        }

        return new FfmpegThumbnailCommandPlan(
            Path.GetFullPath(executablePath),
            arguments,
            ProcessLaunchPolicy.HiddenNoShellProcessTree,
            TimeSpan.FromSeconds(12),
            MaxStandardOutputBytes: 1024,
            MaxStandardErrorBytes: 64 * 1024);
    }

    private static void ValidateExecutablePath(string executablePath)
    {
        if (string.IsNullOrWhiteSpace(executablePath) || executablePath.Any(char.IsControl))
        {
            throw new FfmpegThumbnailCommandValidationException(
                MediaProbeFailureCode.InvalidExecutable,
                "媒体缩略图程序路径无效。");
        }

        try
        {
            if (!Path.IsPathFullyQualified(executablePath)
                || !string.Equals(Path.GetFileName(Path.GetFullPath(executablePath)), ExpectedExecutableFileName, StringComparison.OrdinalIgnoreCase))
            {
                throw new FfmpegThumbnailCommandValidationException(
                    MediaProbeFailureCode.InvalidExecutable,
                    "媒体缩略图程序不是受支持的 FFmpeg 资源。");
            }
        }
        catch (FfmpegThumbnailCommandValidationException)
        {
            throw;
        }
        catch (Exception exception) when (exception is ArgumentException or NotSupportedException or PathTooLongException)
        {
            throw new FfmpegThumbnailCommandValidationException(
                MediaProbeFailureCode.InvalidExecutable,
                "媒体缩略图程序路径无效。");
        }
    }

    private static string ValidateOutputPath(string outputPath)
    {
        if (string.IsNullOrWhiteSpace(outputPath)
            || outputPath.Any(char.IsControl)
            || !Path.IsPathFullyQualified(outputPath))
        {
            throw new FfmpegThumbnailCommandValidationException(
                MediaProbeFailureCode.InvalidExecutable,
                "缩略图输出路径无效。");
        }

        try
        {
            var fullPath = Path.GetFullPath(outputPath);
            if (fullPath.Length > MediaPathPolicy.MaxPathCharacters
                || !string.Equals(Path.GetExtension(fullPath), ".jpg", StringComparison.OrdinalIgnoreCase))
            {
                throw new FfmpegThumbnailCommandValidationException(
                    MediaProbeFailureCode.PathTooLong,
                    "缩略图输出路径无效。");
            }

            return fullPath;
        }
        catch (FfmpegThumbnailCommandValidationException)
        {
            throw;
        }
        catch (Exception exception) when (exception is ArgumentException or NotSupportedException or PathTooLongException)
        {
            throw new FfmpegThumbnailCommandValidationException(
                MediaProbeFailureCode.PathTooLong,
                "缩略图输出路径无效。");
        }
    }
}
