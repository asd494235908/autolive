using System.IO;
using System.Linq;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class VirtualCameraOutputContextWiringTests
{
    [TestMethod]
    public void Main_window_synchronizes_virtual_camera_context_and_valid_frame()
    {
        var sourceDirectory = FindWorkspaceDirectory("src");
        var source = string.Join(
            Environment.NewLine,
            Directory.EnumerateFiles(
                    sourceDirectory,
                    "*.cs",
                    SearchOption.AllDirectories)
                .Where(path => path.Contains(
                    $"{Path.DirectorySeparatorChar}GpAutoLive.App{Path.DirectorySeparatorChar}MainWindow",
                    StringComparison.Ordinal)
                    || path.EndsWith(
                        $"{Path.DirectorySeparatorChar}GpAutoLive.Windows{Path.DirectorySeparatorChar}WindowsVirtualCameraGpuOutputSession.cs",
                        StringComparison.Ordinal))
                .OrderBy(path => path, StringComparer.Ordinal)
                .Select(File.ReadAllText));

        StringAssert.Contains(
            source,
            "_virtualCameraOutput.SetOutputContext",
            "播放状态变更必须同步到虚拟摄像头输出上下文，否则 sidecar 只能持续收到黑帧。");
        StringAssert.Contains(
            source,
            "HasValidFrame = true",
            "WGC 首次成功 GPU 回读后必须把有效帧事实同步给输出策略。");
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
