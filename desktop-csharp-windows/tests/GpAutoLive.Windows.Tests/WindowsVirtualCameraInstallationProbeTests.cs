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

    [TestMethod]
    public void Formal_release_component_set_requires_marker_and_all_fixed_architecture_files()
    {
        var root = Path.Combine(Path.GetTempPath(), "gpautolive-vc-install-components", Guid.NewGuid().ToString("N"));
        try
        {
            Directory.CreateDirectory(Path.Combine(root, "x64"));
            Directory.CreateDirectory(Path.Combine(root, "x86"));
            Directory.CreateDirectory(Path.Combine(root, "bin"));

            var required = new[]
            {
                Path.Combine(root, "release-ready.json"),
                Path.Combine(root, "x64", "AkVirtualCamera.dll"),
                Path.Combine(root, "x64", "AkVCamAssistant.exe"),
                Path.Combine(root, "x64", "AkVCamManager.exe"),
                Path.Combine(root, "x86", "AkVirtualCamera.dll"),
                Path.Combine(root, "bin", "akvirtualcamera-sidecar-x64.exe"),
                Path.Combine(root, "bin", "vcam_capi.dll"),
            };
            foreach (var path in required.Skip(1))
            {
                File.WriteAllBytes(path, [0x01]);
            }

            var checker = typeof(WindowsVirtualCameraInstallationProbe)
                .GetMethod("HasFixedComponents", System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.NonPublic)!;
            Assert.IsFalse((bool)checker.Invoke(null, [root])!);

            File.WriteAllText(required[0], "{}");

            Assert.IsTrue((bool)checker.Invoke(null, [root])!);
        }
        finally
        {
            if (Directory.Exists(root))
            {
                Directory.Delete(root, recursive: true);
            }
        }
    }
}
