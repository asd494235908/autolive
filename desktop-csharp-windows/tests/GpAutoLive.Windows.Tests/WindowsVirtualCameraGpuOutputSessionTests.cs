using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsVirtualCameraGpuOutputSessionTests
{
    [TestMethod]
    public async Task Failed_capture_start_returns_to_installed_and_can_be_retried()
    {
        var manager = new VirtualCameraOutputManager();
        Assert.IsTrue(manager.MarkInstalled().IsSuccess);
        using var binding = new WindowsVirtualCameraSurfaceBinding();
        Assert.IsTrue(binding.Bind(1).IsSuccess);
        await using var session = new WindowsVirtualCameraGpuOutputSession(manager, binding);

        var result = await session.StartAsync(TimeSpan.FromMilliseconds(100));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(VirtualCameraState.Installed, result.Status.State);
        Assert.AreEqual(VirtualCameraState.Installed, manager.Snapshot.State);
    }
}
