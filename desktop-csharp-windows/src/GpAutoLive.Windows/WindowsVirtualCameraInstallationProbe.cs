using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Security;
using Microsoft.Win32;

namespace GpAutoLive.Windows;

/// <summary>Windows 虚拟摄像头安装探测的稳定结果码。</summary>
public enum WindowsVirtualCameraInstallationProbeCode
{
    Available,
    NotWindows,
    InvalidInstallRoot,
    RegistryOwnerMissing,
    RegistryOwnerMismatch,
    ComponentMissing,
    DeviceMissing,
    ProbeFailed,
}

/// <summary>
/// 虚拟摄像头安装探测的脱敏结果；不返回注册表路径、设备实例 ID 或组件路径。
/// </summary>
public sealed record WindowsVirtualCameraInstallationProbeResult(
    WindowsVirtualCameraInstallationProbeCode Code,
    bool HasX64RegistryOwner,
    bool HasX86RegistryOwner,
    bool HasFixedComponents,
    bool HasPresentDevice)
{
    /// <summary>是否满足进入 Core Installed 状态的本机安装门禁。</summary>
    public bool IsAvailable => Code == WindowsVirtualCameraInstallationProbeCode.Available;
}

/// <summary>
/// 只读验证 AkVirtualCamera DirectShow 安装状态。它不调用安装器、regsvr32、Manager 或 sidecar。
/// </summary>
public static class WindowsVirtualCameraInstallationProbe
{
    private const string RegistrySubKey = @"SOFTWARE\Webcamoid\VirtualCamera";
    private const string InstallPathValue = "installPath";
    private const string ExpectedDeviceName = "GpAutoLive Camera";
    private const string AlternateDeviceName = "GpAutoLiveCamera";
    private const int MaxDeviceEntries = 4096;

    /// <summary>
    /// 探测固定双注册表视图、发布/开发组件和 DirectShow 视频输入设备类别。
    /// 显式根目录只用于开发/安装验证；为空时使用注册表所有者提供的根目录。
    /// </summary>
    public static WindowsVirtualCameraInstallationProbeResult Probe(string? installationRoot = null)
    {
        if (!OperatingSystem.IsWindows())
        {
            return Result(WindowsVirtualCameraInstallationProbeCode.NotWindows, false, false, false, false);
        }

        if (installationRoot is null)
        {
            var packageRoot = Environment.GetEnvironmentVariable(WindowsVirtualCameraSidecarLocator.InstallRootEnvironmentVariable);
            if (!string.IsNullOrWhiteSpace(packageRoot))
                installationRoot = Path.Combine(packageRoot, "akvirtualcamera");
        }

        if (!TryNormalizeInstallRoot(installationRoot, out var explicitRoot, out var explicitRootInvalid))
        {
            return Result(
                explicitRootInvalid
                    ? WindowsVirtualCameraInstallationProbeCode.InvalidInstallRoot
                    : WindowsVirtualCameraInstallationProbeCode.ProbeFailed,
                false,
                false,
                false,
                false);
        }

        var x64RawRoot = TryReadInstallRoot(RegistryView.Registry64);
        var x86RawRoot = TryReadInstallRoot(RegistryView.Registry32);
        var x64RootValid = TryNormalizeInstallRoot(x64RawRoot, out var x64Root, out _);
        var x86RootValid = TryNormalizeInstallRoot(x86RawRoot, out var x86Root, out _);
        if (!x64RootValid || !x86RootValid)
        {
            return Result(
                WindowsVirtualCameraInstallationProbeCode.InvalidInstallRoot,
                !string.IsNullOrWhiteSpace(x64RawRoot),
                !string.IsNullOrWhiteSpace(x86RawRoot),
                false,
                false);
        }

        var hasX64Owner = x64Root is not null;
        var hasX86Owner = x86Root is not null;
        var registryCode = !hasX64Owner || !hasX86Owner
            ? WindowsVirtualCameraInstallationProbeCode.RegistryOwnerMissing
            : (!string.Equals(x64Root, x86Root, StringComparison.OrdinalIgnoreCase)
                || (explicitRoot is not null && !string.Equals(explicitRoot, x64Root, StringComparison.OrdinalIgnoreCase)))
                ? WindowsVirtualCameraInstallationProbeCode.RegistryOwnerMismatch
                : (WindowsVirtualCameraInstallationProbeCode?)null;

        var root = explicitRoot ?? x64Root ?? x86Root;
        var hasComponents = root is not null && HasFixedComponents(root);
        var hasDevice = TryFindPresentDevice(out var deviceProbeFailed);
        if (registryCode is not null)
        {
            return Result(registryCode.Value, hasX64Owner, hasX86Owner, hasComponents, hasDevice);
        }

        if (!hasComponents)
        {
            return Result(WindowsVirtualCameraInstallationProbeCode.ComponentMissing, true, true, false, hasDevice);
        }

        if (deviceProbeFailed)
        {
            return Result(WindowsVirtualCameraInstallationProbeCode.ProbeFailed, true, true, true, false);
        }

        return hasDevice
            ? Result(WindowsVirtualCameraInstallationProbeCode.Available, true, true, true, true)
            : Result(WindowsVirtualCameraInstallationProbeCode.DeviceMissing, true, true, true, false);
    }

    private static WindowsVirtualCameraInstallationProbeResult Result(
        WindowsVirtualCameraInstallationProbeCode code,
        bool hasX64Owner,
        bool hasX86Owner,
        bool hasComponents,
        bool hasDevice) => new(code, hasX64Owner, hasX86Owner, hasComponents, hasDevice);

    private static string? TryReadInstallRoot(RegistryView view)
    {
        try
        {
            using var baseKey = RegistryKey.OpenBaseKey(RegistryHive.LocalMachine, view);
            using var key = baseKey.OpenSubKey(RegistrySubKey, writable: false);
            var value = key?.GetValue(InstallPathValue, null, RegistryValueOptions.DoNotExpandEnvironmentNames);
            return value is string text && !string.IsNullOrWhiteSpace(text)
                ? NormalizeRegistryPath(text)
                : null;
        }
        catch (SecurityException)
        {
            return null;
        }
        catch (UnauthorizedAccessException)
        {
            return null;
        }
        catch (IOException)
        {
            return null;
        }
    }

    private static string NormalizeRegistryPath(string path) => path.Trim().TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);

    private static bool TryNormalizeInstallRoot(
        string? value,
        out string? root,
        out bool invalid)
    {
        root = null;
        invalid = false;
        if (string.IsNullOrWhiteSpace(value))
        {
            return true;
        }

        if (value.Any(char.IsControl) || !Path.IsPathFullyQualified(value))
        {
            invalid = true;
            return false;
        }

        try
        {
            var candidate = Path.GetFullPath(value.Trim());
            if (!Path.IsPathFullyQualified(candidate)
                || candidate.StartsWith(@"\\", StringComparison.Ordinal)
                || candidate.StartsWith(@"\\?\", StringComparison.Ordinal)
                || candidate.StartsWith(@"\\.\", StringComparison.Ordinal)
                || !Directory.Exists(candidate)
                || (File.GetAttributes(candidate) & FileAttributes.ReparsePoint) != 0)
            {
                invalid = true;
                return false;
            }

            root = candidate.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
            return true;
        }
        catch (ArgumentException)
        {
            invalid = true;
            return false;
        }
        catch (NotSupportedException)
        {
            invalid = true;
            return false;
        }
        catch (IOException)
        {
            invalid = true;
            return false;
        }
        catch (UnauthorizedAccessException)
        {
            invalid = true;
            return false;
        }
    }

    private static bool HasFixedComponents(string root)
    {
        var development = WindowsVirtualCameraDevelopmentTrust.TryGetRoot(
            Path.Combine(root, "bin", WindowsVirtualCameraSidecarLaunchPlanBuilder.SidecarFileName), out var developmentRoot)
            && string.Equals(root, developmentRoot, StringComparison.OrdinalIgnoreCase)
            && WindowsVirtualCameraDevelopmentTrust.ValidateManifest(root);
        var expected = new[]
        {
            Path.Combine(root, "release-ready.json"),
            Path.Combine(root, "x64", "AkVirtualCamera.dll"),
            Path.Combine(root, "x64", "AkVCamAssistant.exe"),
            Path.Combine(root, "x64", "AkVCamManager.exe"),
            Path.Combine(root, "x86", "AkVirtualCamera.dll"),
            Path.Combine(root, "bin", "akvirtualcamera-sidecar-x64.exe"),
            Path.Combine(root, "bin", "vcam_capi.dll"),
        };

        foreach (var path in expected)
        {
            if (development && Path.GetFileName(path) == "release-ready.json") continue;
            try
            {
                if (!File.Exists(path)
                    || (File.GetAttributes(path) & FileAttributes.ReparsePoint) != 0)
                {
                    return false;
                }
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

        return true;
    }

    private static bool TryFindPresentDevice(out bool probeFailed)
    {
        probeFailed = false;
        object? systemEnumerator = null;
        IEnumMoniker? devices = null;
        try
        {
            var type = Type.GetTypeFromCLSID(new Guid("62BE5D10-60EB-11D0-BD3B-00A0C911CE86"), throwOnError: true);
            systemEnumerator = Activator.CreateInstance(type!);
            var category = new Guid("860BB310-5D01-11D0-BD3B-00A0C911CE86");
            var result = ((ICreateDevEnum)systemEnumerator!).CreateClassEnumerator(ref category, out devices, 0);
            if (result == 1) return false; // S_FALSE 表示空类别。
            if (result != 0 || devices is null) { probeFailed = true; return false; }
            var monikers = new IMoniker[1];
            for (var index = 0; index < MaxDeviceEntries; index++)
            {
                var next = devices.Next(1, monikers, IntPtr.Zero);
                if (next == 1) return false;
                if (next != 0) { probeFailed = true; return false; }
                object? storage = null;
                try
                {
                    var bagId = new Guid("55272A00-42CB-11CE-8135-00AA004BB851");
                    monikers[0].BindToStorage(null!, null!, ref bagId, out storage);
                    if (((IPropertyBag)storage).Read("FriendlyName", out var value, IntPtr.Zero) == 0
                        && value is string name
                        && (string.Equals(name, ExpectedDeviceName, StringComparison.OrdinalIgnoreCase)
                            || string.Equals(name, AlternateDeviceName, StringComparison.OrdinalIgnoreCase))) return true;
                }
                finally
                {
                    if (storage is not null) Marshal.ReleaseComObject(storage);
                    Marshal.ReleaseComObject(monikers[0]);
                }
            }
            probeFailed = true;
            return false;
        }
        catch (Exception ex) when (ex is COMException or InvalidCastException or TypeLoadException or UnauthorizedAccessException)
        {
            probeFailed = true;
            return false;
        }
        finally
        {
            if (devices is not null) Marshal.ReleaseComObject(devices);
            if (systemEnumerator is not null) Marshal.ReleaseComObject(systemEnumerator);
        }
    }

    [ComImport, Guid("29840822-5B84-11D0-BD3B-00A0C911CE86"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface ICreateDevEnum
    {
        [PreserveSig] int CreateClassEnumerator(ref Guid category, out IEnumMoniker? enumerator, int flags);
    }

    [ComImport, Guid("55272A00-42CB-11CE-8135-00AA004BB851"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IPropertyBag
    {
        [PreserveSig] int Read([MarshalAs(UnmanagedType.LPWStr)] string name, [MarshalAs(UnmanagedType.Struct)] out object value, IntPtr errorLog);
        [PreserveSig] int Write([MarshalAs(UnmanagedType.LPWStr)] string name, [MarshalAs(UnmanagedType.Struct)] ref object value);
    }
}
