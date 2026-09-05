using System.Collections.Immutable;
using System.Globalization;
using GpAutoLive.Contracts;
using GpAutoLive.Core.Processes;

namespace GpAutoLive.Windows;

/// <summary>抖音 M1 受管 Python 探针的启动输入；不包含 Cookie、Token 或登录态。</summary>
public sealed record WindowsDouyinProbeLaunchRequest(
    string UpstreamRoot,
    string ProbeScriptPath,
    string CondaExecutablePath,
    string CondaEnvironment,
    string QrOutputPath,
    DouyinLiveConfig Config,
    TimeSpan Timeout);

/// <summary>为 Conda 探针构造的安全外部进程计划。</summary>
public static class WindowsDouyinProbeLaunchPlanBuilder
{
    /// <summary>探针最短监听超时，与参考实现一致。</summary>
    public static readonly TimeSpan MinTimeout = TimeSpan.FromSeconds(30);
    /// <summary>探针最长监听超时，与参考实现一致。</summary>
    public static readonly TimeSpan MaxTimeout = TimeSpan.FromSeconds(900);
    /// <summary>探针标准输出最大保留量；事件只应是短 JSON 行。</summary>
    public const int MaxStandardOutputBytes = 256 * 1024;
    /// <summary>探针标准错误最大保留量。</summary>
    public const int MaxStandardErrorBytes = 64 * 1024;

    /// <summary>校验路径、环境、M1 配置并构造参数数组；不会启动进程。</summary>
    public static bool TryCreate(
        WindowsDouyinProbeLaunchRequest? request,
        out ExternalProcessPlan? plan,
        out string? error)
    {
        plan = null;
        error = null;
        if (!OperatingSystem.IsWindows())
        {
            error = "抖音 M1 探针仅支持 Windows。";
            return false;
        }

        if (request is null || !TryValidateDuration(request.Timeout, out error))
        {
            error ??= "抖音探针启动参数不能为空。";
            return false;
        }

        if (!TryGetRegularDirectory(request.UpstreamRoot, out var upstreamRoot)
            || !HasRequiredUpstreamFiles(upstreamRoot))
        {
            error = "Douyin_Spider 路径不存在、不可访问或缺少必要文件。";
            return false;
        }

        if (!TryGetRegularFile(request.ProbeScriptPath, out var scriptPath)
            || !TryGetRegularFile(request.CondaExecutablePath, out var condaPath))
        {
            error = "Conda、探针脚本必须是已存在的普通文件。";
            return false;
        }

        if (!TryValidateEnvironment(request.CondaEnvironment, out var environment))
        {
            error = "Conda 环境名只允许 1～64 个 ASCII 字母、数字、点、短横线或下划线。";
            return false;
        }

        if (!TryValidateOutputPath(request.QrOutputPath, out var qrOutputPath))
        {
            error = "二维码输出路径必须是已有目录下的绝对 PNG 文件路径。";
            return false;
        }

        if (!DouyinLiveRules.TryNormalize(request.Config, out var config, out var configError))
        {
            error = configError?.Message ?? "抖音 M1 配置无效。";
            return false;
        }

        var arguments = ImmutableArray.CreateBuilder<string>(12 + config!.Replies.Length * 2);
        arguments.Add("run");
        arguments.Add("--no-capture-output");
        arguments.Add("-n");
        arguments.Add(environment);
        arguments.Add("python");
        arguments.Add(scriptPath);
        arguments.Add("--upstream-root");
        arguments.Add(upstreamRoot);
        arguments.Add("--room-id");
        arguments.Add(config.RoomId);
        arguments.Add("--timeout");
        arguments.Add(((long)request.Timeout.TotalSeconds).ToString(CultureInfo.InvariantCulture));
        arguments.Add("--qr-output");
        arguments.Add(qrOutputPath);
        foreach (var reply in config.Replies)
        {
            arguments.Add("--reply");
            arguments.Add(reply);
        }

        plan = new ExternalProcessPlan(
            condaPath,
            arguments.ToImmutable(),
            ProcessLaunchPolicy.HiddenNoShellProcessTree,
            request.Timeout,
            MaxStandardOutputBytes,
            MaxStandardErrorBytes);
        return true;
    }

    private static bool TryValidateDuration(TimeSpan value, out string? error)
    {
        error = null;
        if (value < MinTimeout || value > MaxTimeout || value != TimeSpan.FromSeconds(Math.Truncate(value.TotalSeconds)))
        {
            error = "监听总超时必须为 30～900 秒的整数。";
            return false;
        }

        return true;
    }

    private static bool TryValidateEnvironment(string? value, out string environment)
    {
        environment = value?.Trim() ?? string.Empty;
        return environment.Length is >= 1 and <= 64
            && environment.All(static character =>
                character is >= 'a' and <= 'z'
                    or >= 'A' and <= 'Z'
                    or >= '0' and <= '9'
                    or '.' or '-' or '_');
    }

    private static bool TryValidateOutputPath(string? value, out string path)
    {
        path = string.Empty;
        if (string.IsNullOrWhiteSpace(value) || value.Any(char.IsControl))
        {
            return false;
        }

        try
        {
            path = Path.GetFullPath(value.Trim());
            var directory = Path.GetDirectoryName(path);
            return Path.IsPathFullyQualified(path)
                && string.Equals(Path.GetExtension(path), ".png", StringComparison.OrdinalIgnoreCase)
                && directory is not null
                && Directory.Exists(directory)
                && !HasReparsePoint(directory);
        }
        catch (ArgumentException)
        {
            return false;
        }
        catch (NotSupportedException)
        {
            return false;
        }
    }

    private static bool TryGetRegularFile(string? value, out string path)
    {
        path = string.Empty;
        if (string.IsNullOrWhiteSpace(value) || value.Any(char.IsControl))
        {
            return false;
        }

        try
        {
            path = Path.GetFullPath(value.Trim());
            return Path.IsPathFullyQualified(path)
                && File.Exists(path)
                && !HasReparsePoint(path);
        }
        catch (ArgumentException)
        {
            return false;
        }
        catch (NotSupportedException)
        {
            return false;
        }
    }

    private static bool TryGetRegularDirectory(string? value, out string path)
    {
        path = string.Empty;
        if (string.IsNullOrWhiteSpace(value) || value.Any(char.IsControl))
        {
            return false;
        }

        try
        {
            path = Path.GetFullPath(value.Trim());
            return Path.IsPathFullyQualified(path)
                && Directory.Exists(path)
                && !HasReparsePoint(path);
        }
        catch (ArgumentException)
        {
            return false;
        }
        catch (NotSupportedException)
        {
            return false;
        }
    }

    private static bool HasRequiredUpstreamFiles(string root) =>
        IsRegularPath(Path.Combine(root, "builder", "auth.py"))
        && IsRegularPath(Path.Combine(root, "dy_live", "server.py"))
        && IsRegularPath(Path.Combine(root, "static", "Live_pb2.py"));

    private static bool IsRegularPath(string path) =>
        File.Exists(path) && !HasReparsePoint(path);

    private static bool HasReparsePoint(string path)
    {
        try
        {
            return (File.GetAttributes(path) & FileAttributes.ReparsePoint) != 0;
        }
        catch (FileNotFoundException)
        {
            return true;
        }
        catch (DirectoryNotFoundException)
        {
            return true;
        }
        catch (UnauthorizedAccessException)
        {
            return true;
        }
    }
}
