using System.Collections.Immutable;
using System.Security.Cryptography;
using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsMpvProcessHostTests
{
    [TestMethod]
    public async Task NullPlanIsRejectedWithoutStartingAProcess()
    {
        await using var host = new WindowsMpvProcessHost();

        var result = await host.StartAsync(null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsMpvHostFailureCode.InvalidPlan, result.Error?.Code);
        Assert.AreEqual(WindowsMpvHostState.Ready, result.Snapshot.State);
        Assert.IsNull(result.Snapshot.ProcessId);
    }

    [TestMethod]
    public async Task VerifiedLaunchPlanUsesBoundedHostLifecycle()
    {
        using var fixture = HostFixture.Create();
        var runtimeResult = await RuntimeMediaManifestBoundary.LoadAndVerifyAsync(
            fixture.InstallationDirectory,
            fixture.Version);
        Assert.IsTrue(runtimeResult.IsSuccess, runtimeResult.Error?.Message);
        Assert.IsTrue(MpvIpcPipeEndpoint.TryCreate(
            $@"\\.\pipe\autolive-host-{Guid.NewGuid():N}",
            out var endpoint,
            out var endpointError));
        Assert.IsNull(endpointError);
        Assert.IsTrue(MpvLaunchPlan.TryCreate(
            runtimeResult.Runtime,
            fixture.MediaPath,
            hostWindowId: 1,
            ipcEndpoint: endpoint,
            mode: MpvLaunchMode.Gpu83,
            sourceStartMs: 0,
            durationMs: null,
            out var plan,
            out var planError), planError?.Message);

        await using var host = new WindowsMpvProcessHost();
        var started = await host.StartAsync(plan);
        if (started.IsSuccess)
        {
            Assert.AreEqual(WindowsMpvHostState.Running, started.Snapshot.State);
            Assert.IsNotNull(started.Snapshot.ProcessId);

            var stopped = await host.StopAsync();
            Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
            Assert.AreEqual(WindowsMpvHostState.Stopped, stopped.Snapshot.State);
        }
        else
        {
            // The copied cmd.exe is only a local process fixture, not mpv; it may reject
            // mpv flags on a particular Windows image, but must fail closed and not leak.
            Assert.IsTrue(
                started.Error?.Code is WindowsMpvHostFailureCode.ProcessExited
                    or WindowsMpvHostFailureCode.StartFailed);
            Assert.IsNull(started.Snapshot.ProcessId);
        }
    }

    private static string CreateManifestJson(string version, params RuntimeMediaResourceDto[] resources) =>
        JsonSerializer.Serialize(new RuntimeMediaManifestDto
        {
            SchemaVersion = RuntimeMediaManifestBoundary.CurrentSchemaVersion,
            RuntimeVersion = version,
            Platform = "windows",
            Architecture = "x64",
            Resources = resources.ToImmutableArray(),
        }, ContractJson.CreateOptions());

    private sealed class HostFixture : IDisposable
    {
        private HostFixture(string installationDirectory, string version, string mediaPath)
        {
            InstallationDirectory = installationDirectory;
            Version = version;
            MediaPath = mediaPath;
        }

        public string InstallationDirectory { get; }
        public string Version { get; }
        public string MediaPath { get; }

        public static HostFixture Create()
        {
            var root = Path.Combine(
                Path.GetTempPath(),
                "gpautolive-windows-mpv-host-tests",
                Guid.NewGuid().ToString("N"));
            var version = "1.0.0";
            var versionDirectory = Path.Combine(root, "runtime", "media", version);
            var binDirectory = Path.Combine(versionDirectory, "bin");
            Directory.CreateDirectory(binDirectory);

            var mpvPath = Path.Combine(binDirectory, "mpv.exe");
            File.Copy(Path.Combine(Environment.SystemDirectory, "cmd.exe"), mpvPath);
            var shaderPath = Path.Combine(binDirectory, "gpu83.hook");
            File.WriteAllBytes(shaderPath, [0x47, 0x50, 0x41, 0x55, 0x54, 0x4F, 0x4C, 0x49, 0x56, 0x45]);
            var mediaPath = Path.Combine(root, "sample.mp4");
            File.WriteAllBytes(mediaPath, [0, 1, 2]);
            var bytes = File.ReadAllBytes(mpvPath);
            var shaderBytes = File.ReadAllBytes(shaderPath);
            File.WriteAllText(
                Path.Combine(versionDirectory, "manifest.json"),
                CreateManifestJson(
                    version,
                    new RuntimeMediaResourceDto
                    {
                        Name = "mpv.exe",
                        RelativePath = "bin/mpv.exe",
                        SizeBytes = bytes.Length,
                        Sha256 = Convert.ToHexString(SHA256.HashData(bytes)),
                    },
                    new RuntimeMediaResourceDto
                    {
                        Name = "gpu83.hook",
                        RelativePath = "bin/gpu83.hook",
                        SizeBytes = shaderBytes.Length,
                        Sha256 = Convert.ToHexString(SHA256.HashData(shaderBytes)),
                    }));
            return new HostFixture(root, version, mediaPath);
        }

        public void Dispose()
        {
            if (Directory.Exists(InstallationDirectory))
            {
                Directory.Delete(InstallationDirectory, recursive: true);
            }
        }
    }
}
