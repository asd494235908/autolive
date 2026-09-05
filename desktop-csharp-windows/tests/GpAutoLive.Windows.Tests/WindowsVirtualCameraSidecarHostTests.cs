using System.Buffers.Binary;
using System.Collections.Immutable;
using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsVirtualCameraSidecarHostTests
{
    [TestMethod]
    public async Task Forged_plan_without_memory_token_is_rejected()
    {
        await using var host = new WindowsVirtualCameraSidecarHost();
        var plan = new WindowsVirtualCameraSidecarLaunchPlan(
            "C:\\invalid\\akvirtualcamera-sidecar-x64.exe",
            "C:\\invalid",
            ImmutableArray.Create("--session-token-stdin"),
            VirtualCameraConfig.Default,
            TimeSpan.FromSeconds(5));

        var result = await host.StartAsync(plan);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarHostErrorCode.InvalidPlan, result.Error!.Code);
        Assert.AreEqual(WindowsVirtualCameraSidecarHostState.Ready, result.Snapshot.State);
    }

    [TestMethod]
    public async Task Connect_before_start_does_not_expose_pipe_name()
    {
        await using var host = new WindowsVirtualCameraSidecarHost();
        await using var client = new WindowsVirtualCameraSidecarClient();

        var result = await host.ConnectClientAsync(client, TimeSpan.FromSeconds(1));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarClientErrorCode.InvalidPipeName, result.Error!.Code);
    }

    [TestMethod]
    public async Task Valid_plan_with_non_executable_pe_fails_before_running()
    {
        using var fixture = SidecarFixture.Create();
        var request = new WindowsVirtualCameraSidecarLaunchRequest(
            fixture.Path,
            WindowsVirtualCameraSidecarLaunchPlanBuilder.CreateSessionToken(),
            VirtualCameraConfig.Default,
            TimeSpan.FromSeconds(5));
        Assert.IsTrue(
            WindowsVirtualCameraSidecarLaunchPlanBuilder.TryCreate(request, out var plan, out var planError),
            planError);

        await using var host = new WindowsVirtualCameraSidecarHost();
        var result = await host.StartAsync(plan);

        Assert.IsFalse(result.IsSuccess);
        Assert.IsTrue(
            result.Error!.Code is WindowsVirtualCameraSidecarHostErrorCode.StartFailed
                or WindowsVirtualCameraSidecarHostErrorCode.JobObjectUnavailable);
        Assert.IsFalse(result.Snapshot.State is WindowsVirtualCameraSidecarHostState.Running or WindowsVirtualCameraSidecarHostState.Stopping);
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

    private sealed class TemporaryDirectory : IDisposable
    {
        public TemporaryDirectory()
        {
            Path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "gpautolive-vc-host-" + Guid.NewGuid().ToString("N"));
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
