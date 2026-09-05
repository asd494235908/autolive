using System.Collections.Immutable;
using System.Security.Cryptography;
using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Core.Processes;
using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class RuntimeMediaManifestTests
{
    [TestMethod]
    public void ParserRejectsUnknownManifestFieldsWithoutEchoingThem()
    {
        const string json = """
            {
              "schema_version": 1,
              "runtime_version": "1.0.0",
              "platform": "windows",
              "architecture": "x64",
              "resources": [],
              "unexpected": "do-not-accept"
            }
            """;

        var result = RuntimeMediaManifestBoundary.Parse(json);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(RuntimeResourceFailureCode.UnknownField, result.Error?.Code);
        Assert.IsFalse(result.Error?.Message.Contains("unexpected", StringComparison.Ordinal) ?? true);
        Assert.IsNull(result.Manifest);
    }

    [TestMethod]
    public void ParserRejectsPathTraversalAndUnknownResourceNames()
    {
        var traversal = CreateManifestJson(
            "1.0.0",
            new ResourceSpec("ffprobe.exe", "bin/../ffprobe.exe", 1, new string('0', 64)));

        var traversalResult = RuntimeMediaManifestBoundary.Parse(traversal);

        Assert.IsFalse(traversalResult.IsSuccess);
        Assert.AreEqual(RuntimeResourceFailureCode.InvalidResourcePath, traversalResult.Error?.Code);

        var unknown = CreateManifestJson(
            "1.0.0",
            new ResourceSpec("evil.exe", "bin/evil.exe", 1, new string('0', 64)));

        var unknownResult = RuntimeMediaManifestBoundary.Parse(unknown);

        Assert.IsFalse(unknownResult.IsSuccess);
        Assert.AreEqual(RuntimeResourceFailureCode.UnknownResourceName, unknownResult.Error?.Code);
    }

    [TestMethod]
    public void ParserAcceptsTheExternalD3dCompilerRuntimeResource()
    {
        var manifest = CreateManifestJson(
            "1.0.0",
            new ResourceSpec("d3dcompiler_43.dll", "bin/d3dcompiler_43.dll", 1, new string('0', 64)));

        var result = RuntimeMediaManifestBoundary.Parse(manifest);

        Assert.IsTrue(result.IsSuccess, result.Error?.Message);
        Assert.AreEqual("d3dcompiler_43.dll", result.Manifest?.Resources[0].Name);
    }

    [TestMethod]
    public async Task VerificationRejectsMissingResourceAndReturnsNoPartialRuntime()
    {
        using var fixture = RuntimeFixture.Create();
        var manifest = CreateManifestJson(
            fixture.Version,
            new ResourceSpec("ffprobe.exe", "bin/ffprobe.exe", 3, new string('0', 64)));

        var result = await RuntimeMediaManifestBoundary.VerifyAsync(
            fixture.InstallationDirectory,
            RuntimeMediaManifestBoundary.Parse(manifest).Manifest);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(RuntimeResourceFailureCode.ResourceMissing, result.Error?.Code);
        Assert.IsNull(result.Runtime);
        Assert.IsFalse(result.Error?.Message.Contains(fixture.InstallationDirectory, StringComparison.OrdinalIgnoreCase) ?? true);
    }

    [TestMethod]
    public async Task VerificationRejectsSha256Mismatch()
    {
        using var fixture = RuntimeFixture.Create();
        File.WriteAllBytes(fixture.FfprobePath, [1, 2, 3]);
        var manifest = CreateManifestJson(
            fixture.Version,
            new ResourceSpec("ffprobe.exe", "bin/ffprobe.exe", 3, new string('0', 64)));
        var parsed = RuntimeMediaManifestBoundary.Parse(manifest);

        var result = await RuntimeMediaManifestBoundary.VerifyAsync(
            fixture.InstallationDirectory,
            parsed.Manifest);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(RuntimeResourceFailureCode.ResourceHashMismatch, result.Error?.Code);
        Assert.IsNull(result.Runtime);
    }

    [TestMethod]
    public async Task SuccessfulVerificationCreatesFfprobeBoundaryFromVerifiedPathOnly()
    {
        using var fixture = RuntimeFixture.Create();
        var bytes = new byte[] { 0x46, 0x50, 0x52, 0x4F, 0x42, 0x45 };
        File.WriteAllBytes(fixture.FfprobePath, bytes);
        var hash = Convert.ToHexString(SHA256.HashData(bytes));
        var manifest = CreateManifestJson(
            fixture.Version,
            new ResourceSpec("ffprobe.exe", "bin/ffprobe.exe", bytes.Length, hash));
        var parsed = RuntimeMediaManifestBoundary.Parse(manifest);

        var result = await RuntimeMediaManifestBoundary.VerifyAsync(
            fixture.InstallationDirectory,
            parsed.Manifest);

        Assert.IsTrue(result.IsSuccess);
        Assert.IsNotNull(result.Runtime);
        Assert.AreEqual(1, result.Runtime.Resources.Length);
        Assert.AreEqual(fixture.FfprobePath, result.Runtime.Resources[0].AbsolutePath);

        var runner = new CountingRunner();
        var created = result.Runtime.TryCreateFfprobeProbe(runner, out var probe, out var error);

        Assert.IsTrue(created);
        Assert.IsNotNull(probe);
        Assert.IsNull(error);
        Assert.AreEqual(0, runner.CallCount, "创建探测器不能启动 FFprobe。");
    }

    [TestMethod]
    public async Task LoadAndVerifyUsesVersionDirectoryAndRejectsVersionMismatch()
    {
        using var fixture = RuntimeFixture.Create();
        var bytes = new byte[] { 7, 8, 9 };
        File.WriteAllBytes(fixture.FfprobePath, bytes);
        var hash = Convert.ToHexString(SHA256.HashData(bytes));
        File.WriteAllText(
            fixture.ManifestPath,
            CreateManifestJson(
                fixture.Version,
                new ResourceSpec("ffprobe.exe", "bin/ffprobe.exe", bytes.Length, hash)));

        var result = await RuntimeMediaManifestBoundary.LoadAndVerifyAsync(
            fixture.InstallationDirectory,
            fixture.Version);

        Assert.IsTrue(result.IsSuccess);
        Assert.AreEqual(fixture.Version, result.Runtime?.RuntimeVersion);

        var mismatch = await RuntimeMediaManifestBoundary.LoadAndVerifyAsync(
            fixture.InstallationDirectory,
            "2.0.0");
        Assert.IsFalse(mismatch.IsSuccess);
        Assert.AreEqual(RuntimeResourceFailureCode.ManifestMissing, mismatch.Error?.Code);
    }

    private static string CreateManifestJson(string version, params ResourceSpec[] resources)
    {
        var dto = new RuntimeMediaManifestDto
        {
            SchemaVersion = RuntimeMediaManifestBoundary.CurrentSchemaVersion,
            RuntimeVersion = version,
            Platform = "windows",
            Architecture = "x64",
            Resources = resources
                .Select(resource => new RuntimeMediaResourceDto
                {
                    Name = resource.Name,
                    RelativePath = resource.RelativePath,
                    SizeBytes = resource.SizeBytes,
                    Sha256 = resource.Sha256
                })
                .ToImmutableArray()
        };

        return JsonSerializer.Serialize(dto, ContractJson.CreateOptions());
    }

    private sealed record ResourceSpec(
        string Name,
        string RelativePath,
        long SizeBytes,
        string Sha256);

    private sealed class CountingRunner : IExternalProcessRunner
    {
        public int CallCount { get; private set; }

        public Task<ExternalProcessResult> RunAsync(
            ExternalProcessPlan plan,
            CancellationToken cancellationToken)
        {
            CallCount++;
            return Task.FromResult(new ExternalProcessResult(
                ExternalProcessRunStatus.StartFailed,
                null,
                string.Empty,
                string.Empty));
        }
    }

    private sealed class RuntimeFixture : IDisposable
    {
        private RuntimeFixture(
            string installationDirectory,
            string version,
            string ffprobePath,
            string manifestPath)
        {
            InstallationDirectory = installationDirectory;
            Version = version;
            FfprobePath = ffprobePath;
            ManifestPath = manifestPath;
        }

        public string InstallationDirectory { get; }
        public string Version { get; }
        public string FfprobePath { get; }
        public string ManifestPath { get; }

        public static RuntimeFixture Create()
        {
            var installationDirectory = Path.Combine(
                Path.GetTempPath(),
                "gpautolive-runtime-tests",
                Guid.NewGuid().ToString("N"));
            var version = "1.0.0";
            var versionDirectory = Path.Combine(installationDirectory, "runtime", "media", version);
            var binDirectory = Path.Combine(versionDirectory, "bin");
            Directory.CreateDirectory(binDirectory);
            return new RuntimeFixture(
                installationDirectory,
                version,
                Path.Combine(binDirectory, "ffprobe.exe"),
                Path.Combine(versionDirectory, "manifest.json"));
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
