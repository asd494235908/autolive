using System.Security.Cryptography;
using System.Text.Json;

namespace GpAutoLive.Windows;

/// <summary>显式本机开发包完整性策略，不授予正式发布状态。</summary>
internal static class WindowsVirtualCameraDevelopmentTrust
{
    internal const string EnvironmentVariable = "AUTOLIVE_AKVIRTUALCAMERA_DEVELOPMENT";
    internal static readonly string[] ComponentPaths =
    [
        "bin/akvirtualcamera-sidecar-x64.exe", "bin/vcam_capi.dll",
        "x64/AkVirtualCamera.dll", "x64/AkVCamAssistant.exe", "x64/AkVCamManager.exe",
        "x86/AkVirtualCamera.dll",
    ];

    internal static bool IsDevelopmentAllowed(string? profile, string? optIn, string? root) =>
        (profile is null or "cloud-test-v1") && optIn == "1"
        && !string.IsNullOrWhiteSpace(root) && !root.Any(char.IsControl)
        && Path.IsPathFullyQualified(root) && !root.StartsWith(@"\\", StringComparison.Ordinal);

    internal static bool TryGetRoot(string executablePath, out string root, Func<string, string?>? readEnvironment = null)
    {
        root = string.Empty;
        readEnvironment ??= Environment.GetEnvironmentVariable;
        try
        {
            var profilePath = Path.Combine(AppContext.BaseDirectory, "GpAutoLive.control-plane-profile");
            string? profile = null;
            if (File.Exists(profilePath))
            {
                if (!IsOrdinaryPath(profilePath) || new FileInfo(profilePath).Length is <= 0 or > 64) return false;
                profile = File.ReadAllText(profilePath).Trim();
            }
            var packageRoot = readEnvironment(WindowsVirtualCameraSidecarLocator.InstallRootEnvironmentVariable);
            if (!IsDevelopmentAllowed(profile, readEnvironment(EnvironmentVariable), packageRoot)) return false;
            root = Path.Combine(Path.GetFullPath(packageRoot!), "akvirtualcamera");
            return IsOrdinaryPath(root)
                && string.Equals(Path.GetFullPath(executablePath), Path.Combine(root, "bin", WindowsVirtualCameraSidecarLaunchPlanBuilder.SidecarFileName), StringComparison.OrdinalIgnoreCase);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException or System.Security.SecurityException)
        {
            return false;
        }
    }

    internal static bool ValidateManifest(string root, Func<string, WindowsAuthenticodeProbeCode>? signatureProbe = null)
    {
        signatureProbe ??= path => WindowsAuthenticodeProbe.Probe(path).Code;
        try
        {
            var manifestPath = Path.Combine(root, "development-manifest.json");
            if (!IsOrdinaryPath(manifestPath) || new FileInfo(manifestPath).Length is <= 0 or > 16_384) return false;
            using var document = JsonDocument.Parse(File.ReadAllText(manifestPath), new JsonDocumentOptions { MaxDepth = 4 });
            var json = document.RootElement;
            if (json.GetProperty("schemaVersion").GetInt32() != 1) return false;
            var files = json.GetProperty("files");
            if (files.ValueKind != JsonValueKind.Object || files.EnumerateObject().Count() != ComponentPaths.Length) return false;
            foreach (var relative in ComponentPaths)
            {
                var path = Path.Combine(root, relative);
                if (!IsOrdinaryPath(path) || new FileInfo(path).Length is <= 0 or > WindowsVirtualCameraSidecarLaunchPlanBuilder.MaxSidecarBytes) return false;
                var expected = files.GetProperty(relative).GetString();
                if (expected is null || expected.Length != 64 || !expected.All(Uri.IsHexDigit)) return false;
                using var stream = File.OpenRead(path);
                if (!string.Equals(expected, Convert.ToHexString(SHA256.HashData(stream)), StringComparison.OrdinalIgnoreCase)) return false;
                if (signatureProbe(path) is not (WindowsAuthenticodeProbeCode.Valid or WindowsAuthenticodeProbeCode.Unsigned)) return false;
            }
            return true;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or ArgumentException or InvalidOperationException or FormatException or JsonException or KeyNotFoundException or System.Security.SecurityException)
        {
            return false;
        }
    }

    private static bool IsOrdinaryPath(string path)
    {
        for (string? current = Path.GetFullPath(path); current is not null; current = Path.GetDirectoryName(current))
            if ((File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0) return false;
        return true;
    }

    internal static IDisposable? LockAndValidate(string executablePath)
    {
        if (!TryGetRoot(executablePath, out var root)) return null;
        var locks = new ComponentLocks();
        try
        {
            foreach (var relative in ComponentPaths.Prepend("development-manifest.json"))
                locks.Streams.Add(new FileStream(Path.Combine(root, relative), FileMode.Open, FileAccess.Read, FileShare.Read));
            if (ValidateManifest(root)) return locks;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or ArgumentException or System.Security.SecurityException) { }
        locks.Dispose();
        return null;
    }

    private sealed class ComponentLocks : IDisposable
    {
        internal List<FileStream> Streams { get; } = [];
        public void Dispose() { foreach (var stream in Streams) stream.Dispose(); }
    }
}
