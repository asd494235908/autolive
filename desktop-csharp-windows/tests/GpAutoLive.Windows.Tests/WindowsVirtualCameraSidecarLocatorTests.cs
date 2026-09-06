using System.Buffers.Binary;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsVirtualCameraSidecarLocatorTests
{
    [TestMethod]
    public void Missing_fixed_sidecar_is_not_found()
    {
        using var fixture = InstallFixture.Create();

        var result = WindowsVirtualCameraSidecarLocator.Probe(fixture.Root);

        Assert.AreEqual(WindowsVirtualCameraSidecarProbeCode.NotFound, result.Code);
        Assert.IsFalse(result.IsAvailable);
        Assert.IsFalse(result.IsTrusted);
        Assert.IsNull(result.SignatureCode);
        Assert.IsNull(result.ExecutablePath);
    }

    [TestMethod]
    public void Valid_x64_sidecar_is_available()
    {
        using var fixture = InstallFixture.Create();
        Directory.CreateDirectory(fixture.BinDirectory);
        File.WriteAllBytes(fixture.ExecutablePath, CreateMinimalPe(machine: 0x8664, optionalHeaderMagic: 0x20b));

        var result = WindowsVirtualCameraSidecarLocator.ProbeFromEnvironment(
            name => name == WindowsVirtualCameraSidecarLocator.InstallRootEnvironmentVariable
                ? fixture.Root
                : null);

        Assert.AreEqual(WindowsVirtualCameraSidecarProbeCode.Available, result.Code);
        Assert.IsTrue(result.IsAvailable);
        Assert.IsFalse(result.IsTrusted);
        Assert.IsNotNull(result.SignatureCode);
        Assert.AreEqual(Path.GetFullPath(fixture.ExecutablePath), result.ExecutablePath);
    }

    [TestMethod]
    public void Legacy_virtual_camera_path_remains_compatible()
    {
        using var fixture = InstallFixture.Create();
        Directory.CreateDirectory(fixture.LegacyBinDirectory);
        File.WriteAllBytes(fixture.LegacyExecutablePath, CreateMinimalPe(machine: 0x8664, optionalHeaderMagic: 0x20b));

        var result = WindowsVirtualCameraSidecarLocator.Probe(fixture.Root);

        Assert.AreEqual(WindowsVirtualCameraSidecarProbeCode.Available, result.Code);
        Assert.AreEqual(Path.GetFullPath(fixture.LegacyExecutablePath), result.ExecutablePath);
    }

    [TestMethod]
    public void Invalid_formal_path_does_not_fallback_to_legacy_path()
    {
        using var fixture = InstallFixture.Create();
        Directory.CreateDirectory(fixture.BinDirectory);
        Directory.CreateDirectory(fixture.LegacyBinDirectory);
        File.WriteAllBytes(fixture.ExecutablePath, CreateMinimalPe(machine: 0x14c, optionalHeaderMagic: 0x10b));
        File.WriteAllBytes(fixture.LegacyExecutablePath, CreateMinimalPe(machine: 0x8664, optionalHeaderMagic: 0x20b));

        var result = WindowsVirtualCameraSidecarLocator.Probe(fixture.Root);

        Assert.AreEqual(WindowsVirtualCameraSidecarProbeCode.InvalidSidecar, result.Code);
        Assert.IsNull(result.ExecutablePath);
    }

    [TestMethod]
    public void Wrong_architecture_is_invalid_sidecar()
    {
        using var fixture = InstallFixture.Create();
        Directory.CreateDirectory(fixture.BinDirectory);
        File.WriteAllBytes(fixture.ExecutablePath, CreateMinimalPe(machine: 0x14c, optionalHeaderMagic: 0x10b));

        var result = WindowsVirtualCameraSidecarLocator.Probe(fixture.Root);

        Assert.AreEqual(WindowsVirtualCameraSidecarProbeCode.InvalidSidecar, result.Code);
        Assert.IsFalse(result.IsAvailable);
    }

    [TestMethod]
    public void Control_character_install_root_is_rejected()
    {
        var result = WindowsVirtualCameraSidecarLocator.Probe("C:\\invalid\0root");

        Assert.AreEqual(WindowsVirtualCameraSidecarProbeCode.InvalidInstallRoot, result.Code);
    }

    [TestMethod]
    public void Unc_install_root_is_rejected_without_directory_probe()
    {
        var result = WindowsVirtualCameraSidecarLocator.Probe(@"\\server\share");

        Assert.AreEqual(WindowsVirtualCameraSidecarProbeCode.InvalidInstallRoot, result.Code);
    }

    private static byte[] CreateMinimalPe(ushort machine, ushort optionalHeaderMagic)
    {
        var bytes = new byte[512];
        bytes[0] = (byte)'M';
        bytes[1] = (byte)'Z';
        BinaryPrimitives.WriteInt32LittleEndian(bytes.AsSpan(0x3c), 0x80);
        bytes[0x80] = (byte)'P';
        bytes[0x81] = (byte)'E';
        BinaryPrimitives.WriteUInt16LittleEndian(bytes.AsSpan(0x84), machine);
        BinaryPrimitives.WriteUInt16LittleEndian(bytes.AsSpan(0x94), 240);
        BinaryPrimitives.WriteUInt16LittleEndian(bytes.AsSpan(0x98), optionalHeaderMagic);
        return bytes;
    }

    private sealed class InstallFixture : IDisposable
    {
        private InstallFixture(string root)
        {
            Root = root;
            BinDirectory = Path.Combine(root, "akvirtualcamera", "bin");
            ExecutablePath = Path.Combine(BinDirectory, WindowsVirtualCameraSidecarLaunchPlanBuilder.SidecarFileName);
            LegacyBinDirectory = Path.Combine(root, "virtual-camera", "bin");
            LegacyExecutablePath = Path.Combine(LegacyBinDirectory, WindowsVirtualCameraSidecarLaunchPlanBuilder.SidecarFileName);
        }

        public string Root { get; }
        public string BinDirectory { get; }
        public string ExecutablePath { get; }
        public string LegacyBinDirectory { get; }
        public string LegacyExecutablePath { get; }

        public static InstallFixture Create()
        {
            var root = Path.Combine(Path.GetTempPath(), "gpautolive-vc-locator-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(root);
            return new InstallFixture(root);
        }

        public void Dispose()
        {
            if (Directory.Exists(Root))
            {
                Directory.Delete(Root, recursive: true);
            }
        }
    }
}
