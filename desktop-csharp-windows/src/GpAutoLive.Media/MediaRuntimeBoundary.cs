namespace GpAutoLive.Media;

/// <summary>
/// Media process boundary reserved for the existing mpv/FFmpeg/PortAudio
/// runtime. Phase C1 keeps it side-effect free; resources are not started by
/// the shell and remain external to the main executable.
/// </summary>
public static class MediaRuntimeBoundary
{
    private const string RuntimeRootEnvironmentVariable = "AUTOLIVE_MEDIA_RUNTIME_ROOT";

    public const string DefaultRuntimeVersion = "1.0.0";
    public const string Status = "代码已接入·待资源包验收";

    public static string ResourceSummary =>
        "外置媒体运行资源 · Windows 10/11 x64";

    /// <summary>
    /// 从应用安装目录的固定版本目录按需校验媒体运行资源；不读取用户数据目录。
    /// </summary>
    public static Task<RuntimeMediaVerificationResult> VerifyInstalledAsync(
        string? runtimeVersion = null,
        CancellationToken cancellationToken = default) =>
        RuntimeMediaManifestBoundary.LoadAndVerifyAsync(
            ResolveInstallationDirectory(),
            runtimeVersion ?? DefaultRuntimeVersion,
            cancellationToken);

    private static string ResolveInstallationDirectory()
    {
        var configuredRoot = Environment.GetEnvironmentVariable(RuntimeRootEnvironmentVariable)?.Trim();
        if (string.IsNullOrWhiteSpace(configuredRoot)
            || configuredRoot.Any(char.IsControl)
            || !Path.IsPathFullyQualified(configuredRoot))
        {
            return AppContext.BaseDirectory;
        }

        try
        {
            return Path.GetFullPath(configuredRoot);
        }
        catch (Exception exception) when (exception is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return AppContext.BaseDirectory;
        }
    }
}
