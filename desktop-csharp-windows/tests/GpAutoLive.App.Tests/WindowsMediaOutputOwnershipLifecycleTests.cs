using System.IO;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class WindowsMediaOutputOwnershipLifecycleTests
{
    [TestMethod]
    public void App_acquires_media_output_before_single_instance_and_releases_in_reverse_order()
    {
        var source = File.ReadAllText(
            Path.Combine(FindWorkspaceDirectory("src"), "GpAutoLive.App", "App.xaml.cs"));

        var singleInstanceAcquire = source.IndexOf(
            "var singleInstance = WindowsSingleInstanceLease.TryAcquire();",
            StringComparison.Ordinal);
        var mediaOutputAcquire = source.IndexOf(
            "var ownership = WindowsMediaOutputOwnershipLease.TryAcquire();",
            StringComparison.Ordinal);
        Assert.IsTrue(singleInstanceAcquire >= 0);
        Assert.IsTrue(mediaOutputAcquire >= 0);
        Assert.IsTrue(
            mediaOutputAcquire < singleInstanceAcquire,
            "启动时应先取得 C# 媒体输出锁，再暴露 C# 单实例锁。");

        var mediaOutputRelease = source.IndexOf(
            "_mediaOutputOwnership?.Dispose();",
            StringComparison.Ordinal);
        var singleInstanceRelease = source.IndexOf(
            "_singleInstance.Dispose();",
            StringComparison.Ordinal);
        Assert.IsTrue(mediaOutputRelease >= 0);
        Assert.IsTrue(singleInstanceRelease >= 0);
        Assert.IsTrue(
            mediaOutputRelease < singleInstanceRelease,
            "退出时应先释放 C# 媒体输出锁，再释放 C# 单实例锁。");
    }

    [TestMethod]
    public void App_does_not_probe_or_reference_rust_process_at_runtime()
    {
        var source = File.ReadAllText(
            Path.Combine(FindWorkspaceDirectory("src"), "GpAutoLive.App", "App.xaml.cs"));

        Assert.IsFalse(source.Contains("autolive-desktop-core", StringComparison.Ordinal));
        Assert.IsFalse(source.Contains("ReferenceClient", StringComparison.Ordinal));
    }

    private static string FindWorkspaceDirectory(params string[] segments)
    {
        for (var directory = new DirectoryInfo(AppContext.BaseDirectory);
             directory is not null;
             directory = directory.Parent)
        {
            var candidate = Path.Combine([directory.FullName, .. segments]);
            if (Directory.Exists(candidate))
            {
                return candidate;
            }
        }

        Assert.Fail($"未找到工作区目录：{Path.Combine(segments)}");
        return string.Empty;
    }
}
