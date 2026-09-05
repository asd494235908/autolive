using System.Runtime.InteropServices;
using Vortice.Direct3D;
using Vortice.Direct3D11;
using Vortice.DXGI;
using GpAutoLive.Contracts;

namespace GpAutoLive.Windows;

/// <summary>D3D11 硬件设备创建的稳定错误码。</summary>
public enum WindowsD3D11HardwareContextCode
{
    Ready,
    NotWindows,
    UnsupportedFeatureLevel,
    DeviceUnavailable,
}

/// <summary>D3D11 硬件上下文创建结果。</summary>
public sealed record WindowsD3D11HardwareContextResult(
    bool IsSuccess,
    WindowsD3D11HardwareContextCode Code,
    FeatureLevel? FeatureLevel,
    GpuCaptureFacts? Facts,
    WindowsGraphicsCaptureD3D11Context? Context);

/// <summary>
/// 使用硬件适配器创建带 BGRA 与 VIDEO 支持的 D3D11 设备；禁止回退到 WARP/Reference。
/// </summary>
public static class WindowsD3D11HardwareContextFactory
{
    /// <summary>WGC 视频处理器所需的 D3D11 设备创建标志。</summary>
    internal const DeviceCreationFlags RequiredDeviceCreationFlags =
        DeviceCreationFlags.BgraSupport | DeviceCreationFlags.VideoSupport;

    public static WindowsD3D11HardwareContextResult TryCreate()
    {
        if (!OperatingSystem.IsWindows())
        {
            return new(false, WindowsD3D11HardwareContextCode.NotWindows, null, null, null);
        }

        ID3D11Device? device = null;
        ID3D11DeviceContext? context = null;
        try
        {
            var result = D3D11.D3D11CreateDevice(
                null,
                DriverType.Hardware,
                RequiredDeviceCreationFlags,
                new[] { FeatureLevel.Level_11_0 },
                out device,
                out var featureLevel,
                out context);
            if (result.Failure || device is null || context is null)
            {
                device?.Dispose();
                context?.Dispose();
                return new(false, WindowsD3D11HardwareContextCode.DeviceUnavailable, null, null, null);
            }

            if (featureLevel < FeatureLevel.Level_11_0)
            {
                context.Dispose();
                device.Dispose();
                return new(false, WindowsD3D11HardwareContextCode.UnsupportedFeatureLevel, featureLevel, null, null);
            }

            if (!TryBuildFacts(device, out var facts))
            {
                context.Dispose();
                device.Dispose();
                return new(false, WindowsD3D11HardwareContextCode.DeviceUnavailable, featureLevel, null, null);
            }

            return new(
                true,
                WindowsD3D11HardwareContextCode.Ready,
                featureLevel,
                facts,
                new WindowsGraphicsCaptureD3D11Context(device, context));
        }
        catch (SharpGen.Runtime.SharpGenException)
        {
            context?.Dispose();
            device?.Dispose();
            return new(false, WindowsD3D11HardwareContextCode.DeviceUnavailable, null, null, null);
        }
        catch (COMException)
        {
            context?.Dispose();
            device?.Dispose();
            return new(false, WindowsD3D11HardwareContextCode.DeviceUnavailable, null, null, null);
        }
    }

    /// <summary>从实际 D3D11 设备读取 adapter LUID、厂商和 feature level，拒绝猜测值。</summary>
    public static bool TryBuildFacts(ID3D11Device device, out GpuCaptureFacts? facts)
    {
        ArgumentNullException.ThrowIfNull(device);
        facts = null;
        try
        {
            using var dxgiDevice = device.QueryInterface<IDXGIDevice>();
            using var adapter = dxgiDevice.GetAdapter();
            var description = adapter.Description;
            facts = new GpuCaptureFacts(
                VirtualCameraRules.CaptureApi,
                description.Luid.ToString(),
                description.Description,
                description.VendorId,
                description.DeviceId,
                $"0x{(uint)device.FeatureLevel:X}",
                false,
                true,
                true,
                VirtualCameraRules.Transport,
                false,
                VirtualCameraRules.Width,
                VirtualCameraRules.Height,
                VirtualCameraRules.Fps);
            return true;
        }
        catch (SharpGen.Runtime.SharpGenException)
        {
            return false;
        }
        catch (COMException)
        {
            return false;
        }
    }
}
