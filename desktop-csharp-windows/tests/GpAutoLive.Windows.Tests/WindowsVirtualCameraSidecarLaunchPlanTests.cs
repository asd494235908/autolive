using GpAutoLive.Contracts;
using GpAutoLive.Windows;
using System.Buffers.Binary;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsVirtualCameraSidecarLaunchPlanTests
{
    [TestMethod]
    public void Valid_sidecar_plan_uses_stdin_token_and_fixed_pipe_contract()
    {
        using var fixture = SidecarFixture.Create();
        var token = Enumerable.Repeat((byte)0x2a, 16).ToArray();
        var request = new WindowsVirtualCameraSidecarLaunchRequest(
            fixture.Path,
            token,
            VirtualCameraConfig.Default,
            WindowsVirtualCameraSidecarLaunchPlanBuilder.DefaultStartupTimeout);

        Assert.IsTrue(
            WindowsVirtualCameraSidecarLaunchPlanBuilder.TryCreate(request, out var plan, out var error),
            error);
        Assert.IsNotNull(plan);
        Assert.AreEqual(WindowsVirtualCameraSidecarLaunchPlanBuilder.SidecarFileName, Path.GetFileName(plan!.ExecutablePath));
        Assert.AreEqual(Path.GetDirectoryName(plan.ExecutablePath), plan.WorkingDirectory);
        CollectionAssert.AreEqual(
            new[] { "--session-token-stdin" },
            plan.Arguments.ToArray());
        Assert.IsFalse(plan.Arguments.Any(argument => argument.Contains("2a2a", StringComparison.OrdinalIgnoreCase)));
    }

    [TestMethod]
    public void Wrong_file_name_is_rejected()
    {
        using var root = new TemporaryDirectory();
        var wrongPath = Path.Combine(root.Path, "sidecar.exe");
        File.WriteAllBytes(wrongPath, new byte[] { 1 });
        var request = new WindowsVirtualCameraSidecarLaunchRequest(
            wrongPath,
            WindowsVirtualCameraSidecarLaunchPlanBuilder.CreateSessionToken(),
            VirtualCameraConfig.Default,
            TimeSpan.FromSeconds(5));

        Assert.IsFalse(WindowsVirtualCameraSidecarLaunchPlanBuilder.TryCreate(request, out _, out _));
    }

    [TestMethod]
    public void Wrong_pe_architecture_is_rejected()
    {
        using var root = new TemporaryDirectory();
        var path = System.IO.Path.Combine(root.Path, WindowsVirtualCameraSidecarLaunchPlanBuilder.SidecarFileName);
        var x86Pe = CreateMinimalX64Pe();
        BinaryPrimitives.WriteUInt16LittleEndian(x86Pe.AsSpan(0x84), 0x014c);
        BinaryPrimitives.WriteUInt16LittleEndian(x86Pe.AsSpan(0x98), 0x10b);
        File.WriteAllBytes(path, x86Pe);
        var request = new WindowsVirtualCameraSidecarLaunchRequest(
            path,
            WindowsVirtualCameraSidecarLaunchPlanBuilder.CreateSessionToken(),
            VirtualCameraConfig.Default,
            TimeSpan.FromSeconds(5));

        Assert.IsFalse(WindowsVirtualCameraSidecarLaunchPlanBuilder.TryCreate(request, out _, out _));
    }

    [TestMethod]
    public void Invalid_token_configuration_and_timeout_fail_closed()
    {
        using var fixture = SidecarFixture.Create();
        var invalidRequests = new[]
        {
            new WindowsVirtualCameraSidecarLaunchRequest(fixture.Path, new byte[16], VirtualCameraConfig.Default, TimeSpan.FromSeconds(5)),
            new WindowsVirtualCameraSidecarLaunchRequest(fixture.Path, WindowsVirtualCameraSidecarLaunchPlanBuilder.CreateSessionToken(), VirtualCameraConfig.Default with { ZeroCopy = true }, TimeSpan.FromSeconds(5)),
            new WindowsVirtualCameraSidecarLaunchRequest(fixture.Path, WindowsVirtualCameraSidecarLaunchPlanBuilder.CreateSessionToken(), VirtualCameraConfig.Default, TimeSpan.FromMilliseconds(50)),
        };

        foreach (var request in invalidRequests)
        {
            Assert.IsFalse(WindowsVirtualCameraSidecarLaunchPlanBuilder.TryCreate(request, out _, out _));
        }
    }

    private static byte[] CreateMinimalX64Pe()
    {
        var bytes = new byte[512];
        bytes[0] = (byte)'M';
        bytes[1] = (byte)'Z';
        BinaryPrimitives.WriteInt32LittleEndian(bytes.AsSpan(0x3c), 0x80);
        bytes[0x80] = (byte)'P';
        bytes[0x81] = (byte)'E';
        BinaryPrimitives.WriteUInt16LittleEndian(bytes.AsSpan(0x84), 0x8664);
        BinaryPrimitives.WriteUInt16LittleEndian(bytes.AsSpan(0x94), 240);
        BinaryPrimitives.WriteUInt16LittleEndian(bytes.AsSpan(0x98), 0x20b);
        return bytes;
    }

    private sealed class SidecarFixture : IDisposable
    {
        private readonly TemporaryDirectory _directory;

        private SidecarFixture(TemporaryDirectory directory, string path)
        {
            _directory = directory;
            Path = path;
        }

        public string Path { get; }

        public static SidecarFixture Create()
        {
            var directory = new TemporaryDirectory();
            var path = System.IO.Path.Combine(directory.Path, WindowsVirtualCameraSidecarLaunchPlanBuilder.SidecarFileName);
            File.WriteAllBytes(path, CreateMinimalX64Pe());
            return new SidecarFixture(directory, path);
        }

        public void Dispose() => _directory.Dispose();
    }

    private sealed class TemporaryDirectory : IDisposable
    {
        public TemporaryDirectory()
        {
            Path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "gpautolive-vc-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(Path);
        }

        public string Path { get; }

        public void Dispose()
        {
            if (Directory.Exists(Path))
            {
                Directory.Delete(Path, recursive: true);
            }
        }
    }
}
