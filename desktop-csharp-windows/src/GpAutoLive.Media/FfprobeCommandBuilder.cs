using System.Collections.Immutable;
using GpAutoLive.Core.Processes;

namespace GpAutoLive.Media;

/// <summary>不可变的 FFprobe 参数计划；参数使用数组，不经过命令行字符串或 shell。</summary>
public sealed record FfprobeCommandPlan(
    string ExecutablePath,
    ImmutableArray<string> Arguments,
    ProcessLaunchPolicy LaunchPolicy,
    TimeSpan Timeout,
    int MaxStandardOutputBytes,
    int MaxStandardErrorBytes)
{
    /// <summary>转换为平台无关的执行合同；Windows 适配器不依赖 Media 程序集。</summary>
    public ExternalProcessPlan ToExternalProcessPlan() => new(
        ExecutablePath,
        Arguments,
        LaunchPolicy,
        Timeout,
        MaxStandardOutputBytes,
        MaxStandardErrorBytes);
}

public sealed class FfprobeCommandValidationException : Exception
{
    public FfprobeCommandValidationException(MediaProbeFailureCode code, string message)
        : base(message)
    {
        Code = code;
    }

    public MediaProbeFailureCode Code { get; }
}

/// <summary>只构造固定 FFprobe 命令，不启动进程。</summary>
public static class FfprobeCommandBuilder
{
    private const string ExpectedExecutableFileName = "ffprobe.exe";

    public static FfprobeCommandPlan Create(
        string executablePath,
        string mediaPath,
        FfprobeLimits? limits = null)
    {
        var configuredLimits = limits ?? FfprobeLimits.Default;
        configuredLimits.Validate();
        ValidateExecutablePath(executablePath);

        var pathError = MediaPathPolicy.Validate(mediaPath, requireExistingFile: false, out var validatedPath);
        if (pathError is not null)
        {
            throw new FfprobeCommandValidationException(pathError.Code, pathError.Message);
        }

        return new FfprobeCommandPlan(
            ExecutablePath: Path.GetFullPath(executablePath),
            Arguments: ImmutableArray.Create(
                "-v", "error",
                "-hide_banner",
                "-print_format", "json=compact=1",
                "-show_format",
                "-show_streams",
                "-show_chapters",
                validatedPath.CanonicalPath),
            LaunchPolicy: ProcessLaunchPolicy.HiddenNoShellProcessTree,
            Timeout: configuredLimits.Timeout,
            MaxStandardOutputBytes: configuredLimits.MaxStandardOutputBytes,
            MaxStandardErrorBytes: configuredLimits.MaxStandardErrorBytes);
    }

    private static void ValidateExecutablePath(string executablePath)
    {
        if (string.IsNullOrWhiteSpace(executablePath) || executablePath.Any(char.IsControl))
        {
            throw new FfprobeCommandValidationException(
                MediaProbeFailureCode.InvalidExecutable,
                "媒体探测程序路径无效。");
        }

        try
        {
            if (!Path.IsPathFullyQualified(executablePath))
            {
                throw new FfprobeCommandValidationException(
                    MediaProbeFailureCode.InvalidExecutable,
                    "媒体探测程序路径必须是绝对路径。");
            }

            var fullPath = Path.GetFullPath(executablePath);
            var fileName = Path.GetFileName(fullPath);
            if (!string.Equals(fileName, ExpectedExecutableFileName, StringComparison.OrdinalIgnoreCase))
            {
                throw new FfprobeCommandValidationException(
                    MediaProbeFailureCode.InvalidExecutable,
                    "媒体探测程序不是受支持的 FFprobe 资源。");
            }
        }
        catch (Exception exception) when (exception is ArgumentException or NotSupportedException or PathTooLongException)
        {
            throw new FfprobeCommandValidationException(
                MediaProbeFailureCode.InvalidExecutable,
                "媒体探测程序路径无效。");
        }
    }
}
