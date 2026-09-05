namespace GpAutoLive.Installer.Tests;

[TestClass]
public sealed class InstallerOperationTests
{
    [TestMethod]
    public void InstallPreview_UsesFixedScriptAndWhatIf()
    {
        using var fixture = new DirectoryFixture();
        var package = fixture.CreateDirectory("package");
        var install = Path.Combine(fixture.Root, "install");

        var invocation = InstallerCommandFactory.Create(new(
            InstallerOperation.InstallOrUpgrade,
            package,
            install,
            "v84-test",
            RequireSigned: true,
            WhatIf: true));

        Assert.AreEqual("install-csharp-windows-package.ps1", invocation.ScriptName);
        CollectionAssert.Contains(invocation.Arguments.ToList(), "-RequireSigned");
        CollectionAssert.Contains(invocation.Arguments.ToList(), "-WhatIf");
        Assert.IsFalse(invocation.IsMutation);
    }

    [TestMethod]
    public void RollbackWithoutVersion_UsesPreviousVersion()
    {
        using var fixture = new DirectoryFixture();
        var install = fixture.CreateDirectory("install");

        var invocation = InstallerCommandFactory.Create(new(
            InstallerOperation.Rollback,
            null,
            install,
            null,
            RequireSigned: true,
            WhatIf: false));

        Assert.AreEqual("rollback-csharp-windows-package.ps1", invocation.ScriptName);
        CollectionAssert.DoesNotContain(invocation.Arguments.ToList(), "-Version");
        Assert.IsTrue(invocation.IsMutation);
    }

    [TestMethod]
    public void Uninstall_RejectsInvalidVersion()
    {
        using var fixture = new DirectoryFixture();
        var install = fixture.CreateDirectory("install");

        Assert.ThrowsExactly<ArgumentException>(() => InstallerCommandFactory.Create(new(
            InstallerOperation.UninstallVersion,
            null,
            install,
            "../escape",
            RequireSigned: false,
            WhatIf: true)));
    }

    [TestMethod]
    public void Install_RejectsOverlappingPackageAndInstallRoots()
    {
        using var fixture = new DirectoryFixture();
        var install = fixture.CreateDirectory("install");
        var package = fixture.CreateDirectory(Path.Combine("install", "package"));

        Assert.ThrowsExactly<ArgumentException>(() => InstallerCommandFactory.Create(new(
            InstallerOperation.InstallOrUpgrade,
            package,
            install,
            "v84",
            RequireSigned: false,
            WhatIf: true)));
    }

    [TestMethod]
    public void ExistingInstallRoot_RejectsUnmanagedEntries()
    {
        using var fixture = new DirectoryFixture();
        var install = fixture.CreateDirectory("install");
        File.WriteAllText(Path.Combine(install, "unmanaged.txt"), "keep");

        Assert.ThrowsExactly<ArgumentException>(() => InstallerCommandFactory.Create(new(
            InstallerOperation.CheckInstallState,
            null,
            install,
            null,
            RequireSigned: false,
            WhatIf: false)));
    }

    private sealed class DirectoryFixture : IDisposable
    {
        public DirectoryFixture()
        {
            Root = Path.Combine(Path.GetTempPath(), "gpautolive-installer-tests", Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(Root);
        }

        public string Root { get; }

        public string CreateDirectory(string relativePath)
        {
            var path = Path.Combine(Root, relativePath);
            Directory.CreateDirectory(path);
            return path;
        }

        public void Dispose() => Directory.Delete(Root, recursive: true);
    }
}
