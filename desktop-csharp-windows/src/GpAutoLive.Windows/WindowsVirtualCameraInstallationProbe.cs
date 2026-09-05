using System.Runtime.InteropServices;
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
    private const uint DigcfPresent = 0x00000002;
    private const uint SpdrpDevicedesc = 0x00000000;
    private const uint SpdrpFriendlyname = 0x0000000C;
    private static readonly nint InvalidDeviceInfoSet = new(-1);

    /// <summary>
    /// 探测固定双注册表视图、安装根组件和当前存在的目标 PnP 设备。
    /// 显式根目录只用于开发/安装验证；为空时使用注册表所有者提供的根目录。
    /// </summary>
    public static WindowsVirtualCameraInstallationProbeResult Probe(string? installationRoot = null)
    {
        if (!OperatingSystem.IsWindows())
        {
            return Result(WindowsVirtualCameraInstallationProbeCode.NotWindows, false, false, false, false);
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
            : !string.Equals(x64Root, x86Root, StringComparison.OrdinalIgnoreCase)
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

        if (value.Any(char.IsControl))
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
        var expected = new[]
        {
            Path.Combine(root, "x64", "AkVirtualCamera.dll"),
            Path.Combine(root, "x86", "AkVirtualCamera.dll"),
            Path.Combine(root, "x64", "AkVCamManager.exe"),
        };

        foreach (var path in expected)
        {
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
        nint deviceInfoSet;
        try
        {
            deviceInfoSet = SetupDiGetClassDevsW(IntPtr.Zero, null, IntPtr.Zero, DigcfPresent);
        }
        catch (DllNotFoundException)
        {
            probeFailed = true;
            return false;
        }
        catch (EntryPointNotFoundException)
        {
            probeFailed = true;
            return false;
        }

        if (deviceInfoSet == InvalidDeviceInfoSet)
        {
            probeFailed = true;
            return false;
        }

        try
        {
            var data = new SpDevinfoData
            {
                CbSize = Marshal.SizeOf<SpDevinfoData>(),
            };
            for (uint index = 0; index < MaxDeviceEntries; index++)
            {
                if (!SetupDiEnumDeviceInfo(deviceInfoSet, index, ref data))
                {
                    var error = Marshal.GetLastWin32Error();
                    if (error == 259) // ERROR_NO_MORE_ITEMS
                    {
                        return false;
                    }

                    probeFailed = true;
                    return false;
                }

                if (IsExpectedDevice(deviceInfoSet, ref data, SpdrpFriendlyname)
                    || IsExpectedDevice(deviceInfoSet, ref data, SpdrpDevicedesc))
                {
                    return true;
                }

                data.CbSize = Marshal.SizeOf<SpDevinfoData>();
            }

            probeFailed = true;
            return false;
        }
        finally
        {
            try
            {
                SetupDiDestroyDeviceInfoList(deviceInfoSet);
            }
            catch (DllNotFoundException)
            {
            }
            catch (EntryPointNotFoundException)
            {
            }
        }
    }

    private static bool IsExpectedDevice(nint deviceInfoSet, ref SpDevinfoData data, uint property)
    {
        var name = new System.Text.StringBuilder(512);
        if (!SetupDiGetDeviceRegistryPropertyW(
                deviceInfoSet,
                ref data,
                property,
                out _,
                name,
                name.Capacity,
                out _))
        {
            return false;
        }

        var value = name.ToString().Trim();
        return string.Equals(value, ExpectedDeviceName, StringComparison.OrdinalIgnoreCase)
            || string.Equals(value, AlternateDeviceName, StringComparison.OrdinalIgnoreCase);
    }

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern nint SetupDiGetClassDevsW(
        nint classGuid,
        string? enumerator,
        nint hwndParent,
        uint flags);

    [DllImport("setupapi.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiEnumDeviceInfo(
        nint deviceInfoSet,
        uint memberIndex,
        ref SpDevinfoData deviceInfoData);

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiGetDeviceRegistryPropertyW(
        nint deviceInfoSet,
        ref SpDevinfoData deviceInfoData,
        uint property,
        out uint propertyRegDataType,
        System.Text.StringBuilder propertyBuffer,
        int propertyBufferSize,
        out int requiredSize);

    [DllImport("setupapi.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiDestroyDeviceInfoList(nint deviceInfoSet);

    [StructLayout(LayoutKind.Sequential)]
    private struct SpDevinfoData
    {
        public int CbSize;
        public Guid ClassGuid;
        public uint DevInst;
        public nint Reserved;
    }
}
