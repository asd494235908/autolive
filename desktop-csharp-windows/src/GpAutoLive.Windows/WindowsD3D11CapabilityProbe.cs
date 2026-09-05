using System.Runtime.InteropServices;

namespace GpAutoLive.Windows;

/// <summary>D3D11 硬件前置探测结果。</summary>
public enum WindowsD3D11CapabilityCode
{
    Ready,
    NotWindows,
    RuntimeUnavailable,
    DeviceCreationFailed,
    FeatureLevelInsufficient
}

/// <summary>脱敏的 D3D11 前置事实；不代表 WGC 或虚拟摄像头已启动。</summary>
public sealed record WindowsD3D11CapabilityResult(
    WindowsD3D11CapabilityCode Code,
    uint? FeatureLevel = null)
{
    /// <summary>系统硬件 D3D11 前置是否通过。</summary>
    public bool IsReady => Code == WindowsD3D11CapabilityCode.Ready;
}

/// <summary>
/// Windows D3D11 硬件前置探测。只创建并立即释放一个硬件设备，不加载应用 DLL、WGC 或 WARP。
/// </summary>
public static class WindowsD3D11CapabilityProbe
{
    /// <summary>D3D11.0 的最低 Feature Level。</summary>
    public const uint MinimumFeatureLevel = 0xb000;

    private const uint BgraSupport = 0x20;
    private const uint D3D11SdkVersion = 7;
    private const uint HardwareDriverType = 1;

    /// <summary>执行一次短时、只读硬件前置探测。</summary>
    public static WindowsD3D11CapabilityResult Probe()
    {
        if (!OperatingSystem.IsWindows())
        {
            return new(WindowsD3D11CapabilityCode.NotWindows);
        }

        IntPtr device = IntPtr.Zero;
        IntPtr context = IntPtr.Zero;
        try
        {
            var hresult = D3D11CreateDevice(
                IntPtr.Zero,
                HardwareDriverType,
                IntPtr.Zero,
                BgraSupport,
                IntPtr.Zero,
                0,
                D3D11SdkVersion,
                out device,
                out var featureLevel,
                out context);
            if (hresult < 0 || device == IntPtr.Zero || context == IntPtr.Zero)
            {
                return new(WindowsD3D11CapabilityCode.DeviceCreationFailed);
            }

            return featureLevel < MinimumFeatureLevel
                ? new(WindowsD3D11CapabilityCode.FeatureLevelInsufficient, featureLevel)
                : new(WindowsD3D11CapabilityCode.Ready, featureLevel);
        }
        catch (DllNotFoundException)
        {
            return new(WindowsD3D11CapabilityCode.RuntimeUnavailable);
        }
        catch (EntryPointNotFoundException)
        {
            return new(WindowsD3D11CapabilityCode.RuntimeUnavailable);
        }
        catch (BadImageFormatException)
        {
            return new(WindowsD3D11CapabilityCode.RuntimeUnavailable);
        }
        catch (SEHException)
        {
            return new(WindowsD3D11CapabilityCode.DeviceCreationFailed);
        }
        finally
        {
            ReleaseComObject(context);
            ReleaseComObject(device);
        }
    }

    private static void ReleaseComObject(IntPtr value)
    {
        if (value == IntPtr.Zero)
        {
            return;
        }

        try
        {
            Marshal.Release(value);
        }
        catch (SEHException)
        {
            // 设备创建失败时可能返回不完整句柄；释放失败不能破坏 UI 生命周期。
        }
    }

    [DllImport("d3d11.dll", ExactSpelling = true)]
    private static extern int D3D11CreateDevice(
        IntPtr pAdapter,
        uint driverType,
        IntPtr software,
        uint flags,
        IntPtr pFeatureLevels,
        uint featureLevels,
        uint sdkVersion,
        out IntPtr ppDevice,
        out uint pFeatureLevel,
        out IntPtr ppImmediateContext);
}
