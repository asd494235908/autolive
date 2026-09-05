using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsVirtualCameraInstallationProbeTests
{
    [TestMethod]
    public void Rejects_relative_or_missing_install_root_without_touching_devices()
    {
        var relative = WindowsVirtualCameraInstallationProbe.Probe("relative-install-root");
        Assert.AreEqual(
            WindowsVirtualCameraInstallationProbeCode.InvalidInstallRoot,
            relative.Code);

        var missing = WindowsVirtualCameraInstallationProbe.Probe(
            Path.Combine(Path.GetTempPath(), "gpautolive-missing-install-root", Guid.NewGuid().ToString("N")));
        Assert.AreEqual(
            WindowsVirtualCameraInstallationProbeCode.InvalidInstallRoot,
            missing.Code);
        Assert.IsFalse(missing.IsAvailable);
    }

    [TestMethod]
    public void Probe_is_fail_closed_and_never_returns_available_without_all_facts()
    {
        var result = WindowsVirtualCameraInstallationProbe.Probe();

        if (result.IsAvailable)
        {
            Assert.IsTrue(result.HasX64RegistryOwner);
            Assert.IsTrue(result.HasX86RegistryOwner);
            Assert.IsTrue(result.HasFixedComponents);
            Assert.IsTrue(result.HasPresentDevice);
        }
        else
        {
            Assert.AreNotEqual(WindowsVirtualCameraInstallationProbeCode.Available, result.Code);
        }
    }
}
