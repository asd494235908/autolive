using System.Diagnostics;

namespace GpAutoLive.Installer.Tests;

[TestClass]
public sealed class InstallerProcessRunnerTests
{
    [TestMethod]
    public async Task RunAsync_CancellationStopsChildProcess()
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "cmd.exe"),
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true
        };
        startInfo.ArgumentList.Add("/d");
        startInfo.ArgumentList.Add("/c");
        startInfo.ArgumentList.Add("ping -n 30 127.0.0.1 > nul");
        using var cancellation = new CancellationTokenSource(TimeSpan.FromMilliseconds(200));

        await Assert.ThrowsAsync<OperationCanceledException>(
            () => InstallerProcessRunner.RunAsync(startInfo, cancellation.Token));
    }

    [TestMethod]
    public void ResolveScriptPath_RejectsUnknownScript()
    {
        using var fixture = new DirectoryFixture();
        var runner = new PowerShellInstallerRunner(fixture.Root, "pwsh.exe");

        Assert.ThrowsExactly<InvalidOperationException>(() => runner.ResolveScriptPath("other.ps1"));
    }

    private sealed class DirectoryFixture : IDisposable
    {
        public DirectoryFixture()
        {
            Root = Path.Combine(Path.GetTempPath(), "gpautolive-installer-runner-tests", Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(Root);
        }

        public string Root { get; }

        public void Dispose() => Directory.Delete(Root, recursive: true);
    }
}
