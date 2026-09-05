using Windows.Graphics.Capture;

namespace GpAutoLive.Windows;

/// <summary>Windows Graphics Capture 前置事实分类。</summary>
public enum WindowsGraphicsCaptureCapabilityCode
{
    Ready,
    NotWindows,
    UnsupportedVersion,
    RuntimeUnavailable,
}

/// <summary>脱敏的 WGC 前置结果；不代表某个 HWND 已建立捕获会话。</summary>
public sealed record WindowsGraphicsCaptureCapabilityResult(
    WindowsGraphicsCaptureCapabilityCode Code)
{
    public bool IsReady => Code == WindowsGraphicsCaptureCapabilityCode.Ready;
}

/// <summary>查询系统 Windows Graphics Capture 能力，不创建捕获会话或像素资源。</summary>
public static class WindowsGraphicsCaptureCapabilityProbe
{
    public static WindowsGraphicsCaptureCapabilityResult Probe()
    {
        if (!OperatingSystem.IsWindows())
        {
            return new(WindowsGraphicsCaptureCapabilityCode.NotWindows);
        }

        if (!OperatingSystem.IsWindowsVersionAtLeast(10, 0, 17763))
        {
            return new(WindowsGraphicsCaptureCapabilityCode.UnsupportedVersion);
        }

        try
        {
            return GraphicsCaptureSession.IsSupported()
                ? new(WindowsGraphicsCaptureCapabilityCode.Ready)
                : new(WindowsGraphicsCaptureCapabilityCode.RuntimeUnavailable);
        }
        catch (Exception)
        {
            return new(WindowsGraphicsCaptureCapabilityCode.RuntimeUnavailable);
        }
    }
}
