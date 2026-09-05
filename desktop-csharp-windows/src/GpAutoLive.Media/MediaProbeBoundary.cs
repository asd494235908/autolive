using System.Text;
using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Core.Processes;

namespace GpAutoLive.Media;

/// <summary>媒体导入/探测错误的稳定分类；错误正文不包含路径、命令或外部输出。</summary>
public enum MediaProbeFailureCode
{
    EmptyPath,
    PathNotFullyQualified,
    PathTooLong,
    PathContainsControlCharacter,
    UnsupportedExtension,
    FileNotFound,
    PathIsDirectory,
    InvalidExecutable,
    ProcessStartFailed,
    ProcessTimedOut,
    OperationCancelled,
    StandardOutputLimitExceeded,
    StandardErrorLimitExceeded,
    ProcessExitedWithError,
    MalformedProbeOutput,
    UnsupportedMediaStream
}

/// <summary>可安全展示给用户的失败结果。不会保存原始路径或完整命令。</summary>
public sealed record MediaProbeError(
    MediaProbeFailureCode Code,
    string Message,
    bool Retryable);

/// <summary>已通过路径语法和扩展名校验的媒体路径。</summary>
public readonly record struct ValidatedMediaPath(string CanonicalPath, MediaKind Kind);

/// <summary>FFprobe 输出上限及时间预算。</summary>
public sealed record FfprobeLimits(
    TimeSpan Timeout,
    int MaxStandardOutputBytes,
    int MaxStandardErrorBytes)
{
    public static FfprobeLimits Default { get; } = new(
        Timeout: TimeSpan.FromSeconds(5),
        MaxStandardOutputBytes: 512 * 1024,
        MaxStandardErrorBytes: 64 * 1024);

    public void Validate()
    {
        if (Timeout < TimeSpan.FromSeconds(1) || Timeout > TimeSpan.FromMinutes(1))
        {
            throw new ArgumentOutOfRangeException(nameof(Timeout), "探测超时必须在 1 秒到 60 秒内。");
        }

        if (MaxStandardOutputBytes is < 1 or > 16 * 1024 * 1024)
        {
            throw new ArgumentOutOfRangeException(nameof(MaxStandardOutputBytes), "标准输出上限必须在 1 字节到 16 MiB 内。");
        }

        if (MaxStandardErrorBytes is < 1 or > 4 * 1024 * 1024)
        {
            throw new ArgumentOutOfRangeException(nameof(MaxStandardErrorBytes), "标准错误上限必须在 1 字节到 4 MiB 内。");
        }
    }
}

/// <summary>探测前的路径策略，不访问 shell，也不执行文件。</summary>
public static class MediaPathPolicy
{
    // Windows extended paths are accepted by the OS up to 32767 UTF-16 chars;
    // leave a conservative margin for the application and child-process boundary.
    public const int MaxPathCharacters = 32_000;

    public static MediaProbeError? Validate(
        string? path,
        bool requireExistingFile,
        out ValidatedMediaPath validatedPath)
    {
        validatedPath = default;

        if (string.IsNullOrWhiteSpace(path))
        {
            return Error(MediaProbeFailureCode.EmptyPath, "未提供媒体文件。", retryable: false);
        }

        if (path.Length > MaxPathCharacters)
        {
            return Error(MediaProbeFailureCode.PathTooLong, "媒体文件路径过长。", retryable: false);
        }

        if (path.Any(char.IsControl))
        {
            return Error(MediaProbeFailureCode.PathContainsControlCharacter, "媒体文件路径包含无效字符。", retryable: false);
        }

        if (!Path.IsPathFullyQualified(path))
        {
            return Error(MediaProbeFailureCode.PathNotFullyQualified, "媒体文件路径必须是绝对路径。", retryable: false);
        }

        string canonicalPath;
        try
        {
            canonicalPath = Path.TrimEndingDirectorySeparator(Path.GetFullPath(path));
        }
        catch (Exception exception) when (exception is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return Error(MediaProbeFailureCode.PathTooLong, "媒体文件路径无效。", retryable: false);
        }

        if (canonicalPath.Length > MaxPathCharacters)
        {
            return Error(MediaProbeFailureCode.PathTooLong, "媒体文件路径过长。", retryable: false);
        }

        if (!MediaFormatCatalog.TryGetKind(canonicalPath, out var kind))
        {
            return Error(MediaProbeFailureCode.UnsupportedExtension, "媒体文件格式不在支持范围内。", retryable: false);
        }

        if (requireExistingFile)
        {
            try
            {
                if (Directory.Exists(canonicalPath))
                {
                    return Error(MediaProbeFailureCode.PathIsDirectory, "媒体路径不是文件。", retryable: false);
                }

                if (!File.Exists(canonicalPath))
                {
                    return Error(MediaProbeFailureCode.FileNotFound, "媒体文件不存在或不可访问。", retryable: true);
                }
            }
            catch (IOException)
            {
                return Error(MediaProbeFailureCode.FileNotFound, "媒体文件不存在或不可访问。", retryable: true);
            }
            catch (UnauthorizedAccessException)
            {
                return Error(MediaProbeFailureCode.FileNotFound, "媒体文件不存在或不可访问。", retryable: true);
            }
        }

        validatedPath = new ValidatedMediaPath(canonicalPath, kind);
        return null;
    }

    private static MediaProbeError Error(MediaProbeFailureCode code, string message, bool retryable) =>
        new(code, message, retryable);
}

/// <summary>受管 FFprobe 探测入口；进程生命周期由注入的 Core 执行器负责。</summary>
public sealed class FfprobeMediaProbe
{
    private readonly IExternalProcessRunner _processRunner;
    private readonly FfprobeLimits _limits;
    private readonly string _executablePath;

    public FfprobeMediaProbe(
        string executablePath,
        IExternalProcessRunner processRunner,
        FfprobeLimits? limits = null)
    {
        if (string.IsNullOrWhiteSpace(executablePath))
        {
            throw new ArgumentException("必须提供受管 FFprobe 路径。", nameof(executablePath));
        }

        _executablePath = executablePath;
        _processRunner = processRunner ?? throw new ArgumentNullException(nameof(processRunner));
        _limits = limits ?? FfprobeLimits.Default;
        _limits.Validate();
    }

    /// <summary>
    /// 对单个源执行有界探测。此类不启动任意外部命令，也不把路径或错误输出写入持久化。
    /// </summary>
    public async Task<FfprobeProbeResult> ProbeAsync(
        string? path,
        CancellationToken cancellationToken = default)
    {
        var pathError = MediaPathPolicy.Validate(path, requireExistingFile: true, out var validatedPath);
        if (pathError is not null)
        {
            return FfprobeProbeResult.Failed(pathError);
        }

        FfprobeCommandPlan plan;
        try
        {
            plan = FfprobeCommandBuilder.Create(
                executablePath: _executablePath,
                mediaPath: validatedPath.CanonicalPath,
                limits: _limits);
        }
        catch (FfprobeCommandValidationException exception)
        {
            return FfprobeProbeResult.Failed(new MediaProbeError(
                exception.Code,
                exception.Message,
                Retryable: false));
        }

        ExternalProcessResult result;
        try
        {
            result = await _processRunner.RunAsync(
                plan.ToExternalProcessPlan(),
                cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return FfprobeProbeResult.Failed(new MediaProbeError(
                MediaProbeFailureCode.OperationCancelled,
                "媒体探测已取消。",
                Retryable: true));
        }

        var processError = MapProcessError(result);
        if (processError is not null)
        {
            return FfprobeProbeResult.Failed(processError);
        }

        if (!IsWithinLimit(result.StandardOutput, _limits.MaxStandardOutputBytes))
        {
            return FfprobeProbeResult.Failed(new MediaProbeError(
                MediaProbeFailureCode.StandardOutputLimitExceeded,
                "媒体探测输出超过限制。",
                Retryable: false));
        }

        if (!IsWithinLimit(result.StandardError, _limits.MaxStandardErrorBytes))
        {
            return FfprobeProbeResult.Failed(new MediaProbeError(
                MediaProbeFailureCode.StandardErrorLimitExceeded,
                "媒体探测错误输出超过限制。",
                Retryable: false));
        }

        try
        {
            using var document = JsonDocument.Parse(result.StandardOutput);
            if (document.RootElement.ValueKind != JsonValueKind.Object)
            {
                return FfprobeProbeResult.Failed(new MediaProbeError(
                    MediaProbeFailureCode.MalformedProbeOutput,
                    "媒体探测返回的数据格式无效。",
                    Retryable: false));
            }
        }
        catch (JsonException)
        {
            return FfprobeProbeResult.Failed(new MediaProbeError(
                MediaProbeFailureCode.MalformedProbeOutput,
                "媒体探测返回的数据格式无效。",
                Retryable: false));
        }

        return FfprobeProbeResult.Succeeded(validatedPath, result.StandardOutput);
    }

    private static MediaProbeError? MapProcessError(ExternalProcessResult result) => result.Status switch
    {
        ExternalProcessRunStatus.StartFailed => new(MediaProbeFailureCode.ProcessStartFailed, "媒体探测程序无法启动。", true),
        ExternalProcessRunStatus.TimedOut => new(MediaProbeFailureCode.ProcessTimedOut, "媒体探测超时。", true),
        ExternalProcessRunStatus.Cancelled => new(MediaProbeFailureCode.OperationCancelled, "媒体探测已取消。", true),
        ExternalProcessRunStatus.StandardOutputLimitExceeded => new(MediaProbeFailureCode.StandardOutputLimitExceeded, "媒体探测输出超过限制。", false),
        ExternalProcessRunStatus.StandardErrorLimitExceeded => new(MediaProbeFailureCode.StandardErrorLimitExceeded, "媒体探测错误输出超过限制。", false),
        ExternalProcessRunStatus.OutputReadFailed => new(MediaProbeFailureCode.ProcessExitedWithError, "媒体探测输出读取失败。", true),
        ExternalProcessRunStatus.Completed when result.ExitCode is not 0 => new(MediaProbeFailureCode.ProcessExitedWithError, "媒体探测程序返回失败。", true),
        _ => null
    };

    private static bool IsWithinLimit(string text, int maxBytes) =>
        Encoding.UTF8.GetByteCount(text) <= maxBytes;

}

/// <summary>FFprobe 的执行结果；原始 JSON 只在内存中交给后续解析层。</summary>
public sealed record FfprobeProbeResult(
    ValidatedMediaPath? MediaPath,
    string? ProbeJson,
    MediaProbeError? Error)
{
    public bool IsSuccess => Error is null;

    public static FfprobeProbeResult Succeeded(ValidatedMediaPath path, string probeJson) =>
        new(path, probeJson, null);

    public static FfprobeProbeResult Failed(MediaProbeError error) =>
        new(null, null, error);
}
