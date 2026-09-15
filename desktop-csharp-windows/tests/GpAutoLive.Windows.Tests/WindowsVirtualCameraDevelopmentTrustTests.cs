using GpAutoLive.Windows;
using System.Security.Cryptography;
using System.Text.Json;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsVirtualCameraDevelopmentTrustTests
{
    [TestMethod]
    public void Locally_built_unsigned_package_passes_real_integrity_and_authenticode_probe()
    {
        var package = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_VCAM_PACKAGE");
        if (string.IsNullOrWhiteSpace(package))
            Assert.Inconclusive("需要显式 AUTOLIVE_TEST_VCAM_PACKAGE 本地 native 构建产物。");
        var result = WindowsVirtualCameraSidecarLocator.ProbeFromEnvironment(name => name switch
        {
            WindowsVirtualCameraSidecarLocator.InstallRootEnvironmentVariable => package,
            WindowsVirtualCameraDevelopmentTrust.EnvironmentVariable => "1",
            _ => null,
        });
        Assert.IsTrue(result.IsAvailable, result.Code.ToString());
        Assert.AreEqual(WindowsAuthenticodeProbeCode.Unsigned, result.SignatureCode);
        Assert.IsTrue(result.IsDevelopmentTrusted, result.DiagnosticCode);
        Assert.IsTrue(result.IsTrusted);
    }

    [TestMethod]
    public void Development_requires_explicit_opt_in_absolute_root_and_nonproduction_profile()
    {
        Assert.IsFalse(WindowsVirtualCameraDevelopmentTrust.IsDevelopmentAllowed("production-v1", "1", @"C:\camera"));
        Assert.IsFalse(WindowsVirtualCameraDevelopmentTrust.IsDevelopmentAllowed("unknown", "1", @"C:\camera"));
        Assert.IsFalse(WindowsVirtualCameraDevelopmentTrust.IsDevelopmentAllowed(null, null, @"C:\camera"));
        Assert.IsFalse(WindowsVirtualCameraDevelopmentTrust.IsDevelopmentAllowed(null, "1", "camera"));
        Assert.IsTrue(WindowsVirtualCameraDevelopmentTrust.IsDevelopmentAllowed(null, "1", @"C:\camera"));
        Assert.IsTrue(WindowsVirtualCameraDevelopmentTrust.IsDevelopmentAllowed("cloud-test-v1", "1", @"C:\camera"));
    }

    [TestMethod]
    public void Complete_manifest_accepts_unsigned_but_rejects_bad_signature_and_tampering()
    {
        var root = Path.Combine(Path.GetTempPath(), "gpautolive-camera-trust-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try
        {
            var files = new Dictionary<string, string>();
            foreach (var relative in WindowsVirtualCameraDevelopmentTrust.ComponentPaths)
            {
                var path = Path.Combine(root, relative);
                Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                File.WriteAllBytes(path, [1, 2, 3]);
                files.Add(relative, Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(path))));
            }
            File.WriteAllText(Path.Combine(root, "development-manifest.json"), JsonSerializer.Serialize(new { schemaVersion = 1, files }));
            Assert.IsTrue(WindowsVirtualCameraDevelopmentTrust.ValidateManifest(root, _ => WindowsAuthenticodeProbeCode.Unsigned));
            Assert.IsFalse(WindowsVirtualCameraDevelopmentTrust.ValidateManifest(root, _ => WindowsAuthenticodeProbeCode.InvalidSignature));
            Assert.IsFalse(WindowsVirtualCameraDevelopmentTrust.ValidateManifest(root, _ => WindowsAuthenticodeProbeCode.ProbeFailed));
            var manifestPath = Path.Combine(root, "development-manifest.json");
            var original = File.ReadAllText(manifestPath);
            File.WriteAllText(manifestPath, "{\"schemaVersion\":999999999999,\"files\":{}}");
            Assert.IsFalse(WindowsVirtualCameraDevelopmentTrust.ValidateManifest(root, _ => WindowsAuthenticodeProbeCode.Unsigned));
            files.Add("bin/unlisted.dll", new string('0', 64));
            File.WriteAllText(manifestPath, JsonSerializer.Serialize(new { schemaVersion = 1, files }));
            Assert.IsFalse(WindowsVirtualCameraDevelopmentTrust.ValidateManifest(root, _ => WindowsAuthenticodeProbeCode.Unsigned));
            files.Remove("bin/unlisted.dll");
            files.Remove(WindowsVirtualCameraDevelopmentTrust.ComponentPaths[0]);
            File.WriteAllText(manifestPath, JsonSerializer.Serialize(new { schemaVersion = 1, files }));
            Assert.IsFalse(WindowsVirtualCameraDevelopmentTrust.ValidateManifest(root, _ => WindowsAuthenticodeProbeCode.Unsigned));
            File.WriteAllText(manifestPath, original);
            File.AppendAllText(Path.Combine(root, WindowsVirtualCameraDevelopmentTrust.ComponentPaths[0]), "changed");
            Assert.IsFalse(WindowsVirtualCameraDevelopmentTrust.ValidateManifest(root, _ => WindowsAuthenticodeProbeCode.Unsigned));
        }
        finally { Directory.Delete(root, true); }
    }
}
