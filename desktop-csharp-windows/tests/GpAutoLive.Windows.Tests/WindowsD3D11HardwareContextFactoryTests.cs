using GpAutoLive.Windows;
using Vortice.Direct3D11;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsD3D11HardwareContextFactoryTests
{
    [TestMethod]
    public void Required_device_creation_flags_include_video_support_for_wgc_video_processor()
    {
        var flags = WindowsD3D11HardwareContextFactory.RequiredDeviceCreationFlags;

        Assert.IsTrue(flags.HasFlag(DeviceCreationFlags.BgraSupport));
        Assert.IsTrue(flags.HasFlag(DeviceCreationFlags.VideoSupport));
    }
}
