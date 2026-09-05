namespace GpAutoLive.Windows;

/// <summary>已部署 AkVirtualCamera sidecar 的本机探测结果。</summary>
public enum WindowsVirtualCameraSidecarProbeCode
{
    Available,
    NotWindows,
    InvalidInstallRoot,
    NotFound,
    InvalidSidecar
}

/// <summary>sidecar 探测结果；路径只供后续受管启动计划使用，不进入 UI 快照。</summary>
public sealed record WindowsVirtualCameraSidecarProbeResult(
    WindowsVirtualCameraSidecarProbeCode Code,
    string? ExecutablePath = null,
    WindowsAuthenticodeProbeCode? SignatureCode = null)
{
    /// <summary>sidecar 是否已通过文件、目录和 x64 PE 校验。</summary>
    public bool IsAvailable => Code == WindowsVirtualCameraSidecarProbeCode.Available;

    /// <summary>sidecar 是否同时通过 Authenticode 签名门禁。</summary>
    public bool IsTrusted => IsAvailable && SignatureCode == WindowsAuthenticodeProbeCode.Valid;
}

/// <summary>
/// 解析虚拟摄像头资源包中的固定 sidecar 路径。只做只读探测，不注册设备、不启动进程。
/// </summary>
public static class WindowsVirtualCameraSidecarLocator
{
    /// <summary>可选的本机资源包根目录环境变量，仅用于开发/安装验证。</summary>
    public const string InstallRootEnvironmentVariable = "AUTOLIVE_AKVIRTUALCAMERA_ROOT";

    /// <summary>资源包内固定的 sidecar 相对路径。</summary>
    public static readonly string RelativeExecutablePath = Path.Combine(
        "virtual-camera",
        "bin",
        WindowsVirtualCameraSidecarLaunchPlanBuilder.SidecarFileName);

    /// <summary>
    /// 从指定安装根目录探测 sidecar；根目录为空时使用当前应用目录。
    /// </summary>
    public static WindowsVirtualCameraSidecarProbeResult Probe(string? installationRoot = null)
    {
        if (!OperatingSystem.IsWindows())
        {
            return new(WindowsVirtualCameraSidecarProbeCode.NotWindows);
        }

        if (!TryResolveInstallRoot(installationRoot, out var root))
        {
            return new(WindowsVirtualCameraSidecarProbeCode.InvalidInstallRoot);
        }

        var executablePath = Path.Combine(root, RelativeExecutablePath);
        if (!File.Exists(executablePath))
        {
            return new(WindowsVirtualCameraSidecarProbeCode.NotFound);
        }

        return WindowsVirtualCameraSidecarLaunchPlanBuilder.TryResolveValidatedSidecar(
                executablePath,
                out var validatedPath)
            ? CreateAvailableResult(validatedPath)
            : new(WindowsVirtualCameraSidecarProbeCode.InvalidSidecar);
    }

    /// <summary>按开发环境变量或应用目录探测 sidecar。</summary>
    public static WindowsVirtualCameraSidecarProbeResult ProbeFromEnvironment(
        Func<string, string?>? readEnvironment = null)
    {
        readEnvironment ??= Environment.GetEnvironmentVariable;
        return Probe(readEnvironment(InstallRootEnvironmentVariable));
    }

    private static bool TryResolveInstallRoot(string? value, out string root)
    {
        root = string.Empty;
        var candidate = string.IsNullOrWhiteSpace(value) ? AppContext.BaseDirectory : value.Trim();
        if (candidate.Any(char.IsControl))
        {
            return false;
        }

        try
        {
            root = Path.GetFullPath(candidate);
            if (!Path.IsPathFullyQualified(root)
                || root.StartsWith(@"\\", StringComparison.Ordinal)
                || root.StartsWith(@"\\?\", StringComparison.Ordinal)
                || root.StartsWith(@"\\.\", StringComparison.Ordinal)
                || !Directory.Exists(root))
            {
                return false;
            }

            return (File.GetAttributes(root) & FileAttributes.ReparsePoint) == 0;
        }
        catch (ArgumentException)
        {
            return false;
        }
        catch (NotSupportedException)
        {
            return false;
        }
        catch (IOException)
        {
            return false;
        }
        catch (UnauthorizedAccessException)
        {
            return false;
        }
    }

    private static WindowsVirtualCameraSidecarProbeResult CreateAvailableResult(string executablePath)
    {
        var signature = WindowsAuthenticodeProbe.Probe(executablePath);
        return new(
            WindowsVirtualCameraSidecarProbeCode.Available,
            executablePath,
            signature.Code);
    }
}
