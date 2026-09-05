using System.Collections.Immutable;
using System.Security.Cryptography;
using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class MpvLaunchPlanTests
{
    [TestMethod]
    public async Task Gpu83PlanUsesVerifiedMpvAndHostWindowArguments()
    {
        using var fixture = RuntimeFixture.Create();
        var runtime = await fixture.VerifyAsync();
        Assert.IsNotNull(runtime);
        Assert.IsTrue(MpvIpcPipeEndpoint.TryCreate(
            $@"\\.\pipe\autolive-launch-{Guid.NewGuid():N}",
            out var endpoint,
            out var endpointError));
        Assert.IsNull(endpointError);
        Assert.IsNotNull(endpoint);

        var created = MpvLaunchPlan.TryCreate(
            runtime,
            fixture.MediaPath,
            hostWindowId: 1234,
            ipcEndpoint: endpoint,
            mode: MpvLaunchMode.Gpu83,
            sourceStartMs: 1_250,
            durationMs: 10_000,
            out var plan,
            out var error);

        Assert.IsTrue(created);
        Assert.IsNull(error);
        Assert.IsNotNull(plan);
        Assert.AreEqual(fixture.MpvPath, plan.ExecutablePath);
        CollectionAssert.Contains(plan.Arguments, "--vo=gpu-next");
        CollectionAssert.Contains(plan.Arguments, "--hwdec=auto-safe");
        CollectionAssert.Contains(plan.Arguments, "--wid=1234");
        CollectionAssert.Contains(plan.Arguments, "--start=1.250");
        CollectionAssert.Contains(plan.Arguments, $"--input-ipc-server={endpoint!.PipePath}");
        CollectionAssert.Contains(plan.Arguments, $"--glsl-shaders={fixture.ShaderPath}");
        CollectionAssert.DoesNotContain(plan.Arguments, $"--vf={MpvLaunchPlan.Cpu4FilterChain}");
        Assert.AreEqual(fixture.MediaPath, plan.MediaPath.CanonicalPath);
    }

    [TestMethod]
    public async Task Gpu83PlanFails_closed_when_verified_runtime_has_no_shader()
    {
        using var fixture = RuntimeFixture.Create(includeShader: false);
        var runtime = await fixture.VerifyAsync();
        Assert.IsNotNull(runtime);
        Assert.IsTrue(MpvIpcPipeEndpoint.TryCreate(
            $@"\\.\pipe\autolive-launch-{Guid.NewGuid():N}",
            out var endpoint,
            out var endpointError));
        Assert.IsNull(endpointError);

        var created = MpvLaunchPlan.TryCreate(
            runtime,
            fixture.MediaPath,
            hostWindowId: 1234,
            ipcEndpoint: endpoint,
            mode: MpvLaunchMode.Gpu83,
            sourceStartMs: 0,
            durationMs: 10_000,
            out var plan,
            out var error);

        Assert.IsFalse(created);
        Assert.AreEqual(MpvLaunchFailureCode.MissingGpu83Shader, error?.Code);
        Assert.IsNull(plan);
    }

    [TestMethod]
    public async Task Cpu4PlanIsBoundedAndRejectsInvalidHostWindowOrStartPosition()
    {
        using var fixture = RuntimeFixture.Create();
        var runtime = await fixture.VerifyAsync();
        Assert.IsNotNull(runtime);
        Assert.IsTrue(MpvIpcPipeEndpoint.TryCreate(
            $@"\\.\pipe\autolive-launch-{Guid.NewGuid():N}",
            out var endpoint,
            out var endpointError));
        Assert.IsNull(endpointError);
        Assert.IsNotNull(endpoint);

        var created = MpvLaunchPlan.TryCreate(
            runtime,
            fixture.MediaPath,
            hostWindowId: 42,
            ipcEndpoint: endpoint,
            mode: MpvLaunchMode.Cpu4,
            sourceStartMs: 1,
            durationMs: 1,
            out var plan,
            out var error);

        Assert.IsFalse(created);
        Assert.AreEqual(MpvLaunchFailureCode.InvalidStartPosition, error?.Code);
        Assert.IsNull(plan);

        created = MpvLaunchPlan.TryCreate(
            runtime,
            fixture.MediaPath,
            hostWindowId: 0,
            ipcEndpoint: endpoint,
            mode: MpvLaunchMode.Cpu4,
            sourceStartMs: 0,
            durationMs: 10_000,
            out plan,
            out error);

        Assert.IsFalse(created);
        Assert.AreEqual(MpvLaunchFailureCode.InvalidHostWindow, error?.Code);
        Assert.IsNull(plan);

        created = MpvLaunchPlan.TryCreate(
            runtime,
            fixture.MediaPath,
            hostWindowId: 42,
            ipcEndpoint: endpoint,
            mode: MpvLaunchMode.Cpu4,
            sourceStartMs: 2_500,
            durationMs: 10_000,
            out plan,
            out error);

        Assert.IsTrue(created);
        Assert.IsNull(error);
        Assert.IsNotNull(plan);
        CollectionAssert.Contains(plan.Arguments, "--hwdec=no");
        // CPU4 is installed after the IPC handshake so the session has one
        // mode-specific command path instead of a static launch-time filter.
        CollectionAssert.DoesNotContain(plan.Arguments, $"--vf={MpvLaunchPlan.Cpu4FilterChain}");
        Assert.IsTrue(plan.Arguments.Length <= MpvLaunchPlan.MaxArgumentCount);
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

    private sealed class RuntimeFixture : IDisposable
    {
        private RuntimeFixture(
            string installationDirectory,
            string version,
            string mpvPath,
            string mediaPath,
            string? shaderPath)
        {
            InstallationDirectory = installationDirectory;
            Version = version;
            MpvPath = mpvPath;
            MediaPath = mediaPath;
            ShaderPath = shaderPath;
        }

        public string InstallationDirectory { get; }
        public string Version { get; }
        public string MpvPath { get; }
        public string MediaPath { get; }
        public string? ShaderPath { get; }

        public static RuntimeFixture Create(bool includeShader = true)
        {
            var installationDirectory = Path.Combine(
                Path.GetTempPath(),
                "gpautolive-mpv-launch-tests",
                Guid.NewGuid().ToString("N"));
            var version = "1.0.0";
            var binDirectory = Path.Combine(installationDirectory, "runtime", "media", version, "bin");
            Directory.CreateDirectory(binDirectory);
            var mpvPath = Path.Combine(binDirectory, "mpv.exe");
            var shaderPath = includeShader ? Path.Combine(binDirectory, "gpu83.hook") : null;
            var mediaPath = Path.Combine(installationDirectory, "sample.mp4");
            File.WriteAllBytes(mpvPath, [0x4D, 0x50, 0x56]);
            if (shaderPath is not null)
            {
                File.WriteAllBytes(shaderPath, [0x47, 0x50, 0x41, 0x55, 0x54, 0x4F, 0x4C, 0x49, 0x56, 0x45]);
            }
            File.WriteAllBytes(mediaPath, [0x00, 0x01]);

            var bytes = File.ReadAllBytes(mpvPath);
            var resources = new List<RuntimeMediaResourceDto>
            {
                new()
                {
                    Name = "mpv.exe",
                    RelativePath = "bin/mpv.exe",
                    SizeBytes = bytes.Length,
                    Sha256 = Convert.ToHexString(SHA256.HashData(bytes)),
                },
            };
            if (shaderPath is not null)
            {
                var shaderBytes = File.ReadAllBytes(shaderPath);
                resources.Add(new RuntimeMediaResourceDto
                {
                    Name = "gpu83.hook",
                    RelativePath = "bin/gpu83.hook",
                    SizeBytes = shaderBytes.Length,
                    Sha256 = Convert.ToHexString(SHA256.HashData(shaderBytes)),
                });
            }

            var manifestPath = Path.Combine(installationDirectory, "runtime", "media", version, "manifest.json");
            File.WriteAllText(manifestPath, CreateManifestJson(version, resources.ToArray()));
            return new RuntimeFixture(installationDirectory, version, mpvPath, mediaPath, shaderPath);
        }

        public async Task<VerifiedMediaRuntime?> VerifyAsync()
        {
            var result = await RuntimeMediaManifestBoundary.LoadAndVerifyAsync(
                InstallationDirectory,
                Version);
            Assert.IsTrue(result.IsSuccess, result.Error?.Message);
            return result.Runtime;
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
