namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsAuthenticodeProbeTests
{
    [TestMethod]
    public void Missing_or_relative_paths_fail_closed()
    {
        var missing = WindowsAuthenticodeProbe.Probe(
            Path.Combine(Path.GetTempPath(), $"gpautolive-missing-{Guid.NewGuid():N}.exe"));
        var relative = WindowsAuthenticodeProbe.Probe("GpAutoLive.exe");

        Assert.AreEqual(WindowsAuthenticodeProbeCode.FileMissing, missing.Code);
        Assert.AreEqual(WindowsAuthenticodeProbeCode.InvalidPath, relative.Code);
    }

    [TestMethod]
    public void Directory_path_is_not_treated_as_a_signed_file()
    {
        var result = WindowsAuthenticodeProbe.Probe(Path.GetTempPath());

        Assert.AreEqual(WindowsAuthenticodeProbeCode.InvalidPath, result.Code);
    }

    [TestMethod]
    public void Current_test_assembly_returns_a_bounded_signature_classification()
    {
        var assemblyPath = typeof(WindowsAuthenticodeProbe).Assembly.Location;
        var result = WindowsAuthenticodeProbe.Probe(assemblyPath);

        Assert.AreNotEqual(WindowsAuthenticodeProbeCode.FileMissing, result.Code);
        Assert.AreNotEqual(WindowsAuthenticodeProbeCode.InvalidPath, result.Code);
    }
}
