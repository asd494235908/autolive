using System.Reflection;
using System.Windows.Controls;
using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class VirtualCameraSourceResolutionTests
{
    [TestMethod]
    [DataRow(1920, 1080, 1920)]
    [DataRow(1080, 1920, 1080)]
    [DataRow(3840, 2160, 3840)]
    [DataRow(853, 481, 854)]
    public void Output_uses_source_dimensions_and_only_aligns_yuy2_width(int width, int height, int expectedWidth)
    {
        Assert.IsTrue(MainWindow.TryCreateVirtualCameraSourceConfig(
            Source((uint)width, (uint)height), out var config, out var error), error);
        Assert.IsNotNull(config);
        Assert.AreEqual((uint)expectedWidth, config.Width);
        Assert.AreEqual((uint)height, config.Height);
        Assert.AreEqual((uint)30, config.Fps);
    }

    [TestMethod]
    public void Missing_audio_and_oversized_sources_are_rejected_without_720p_fallback()
    {
        foreach (var source in new[]
        {
            null,
            Source(null, 1080),
            Source(1920, 0),
            Source(uint.MaxValue, 1080),
            Source(4096, 4096),
            Source(1920, 1080) with { MediaKind = MediaKind.Audio },
        })
        {
            Assert.IsFalse(MainWindow.TryCreateVirtualCameraSourceConfig(source, out var config, out var error));
            Assert.IsNull(config);
            Assert.IsFalse(string.IsNullOrWhiteSpace(error));
        }
    }

    [TestMethod]
    public void Workbench_displays_current_source_resolution_before_device_installation()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                var pool = (MediaPoolService)typeof(MainWindow).GetField(
                    "_mediaPool", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(window)!;
                var output = (TextBlock)window.FindName("VirtualCameraResolutionText");
                foreach (var (width, height) in new[] { (1920u, 1080u), (1080u, 1920u) })
                {
                    Assert.IsTrue(pool.ReplaceAll([Source(width, height)]).IsSuccess);
                    typeof(MainWindow).GetMethod("UpdateVirtualCameraProjection",
                        BindingFlags.Instance | BindingFlags.NonPublic)!.Invoke(window, null);
                    Assert.AreEqual($"{width}×{height} · 30fps", output.Text);
                }
            }
            finally { window.Close(); }
        });
    }

    private static SourceMediaDto Source(uint? width, uint? height) => new(
        @"C:\fixtures\video.mp4", @"C:\fixtures\video.mp4", MediaKind.Video,
        MediaCompatibilityMode.Direct, "video.mp4", 1, 30_000, null, null,
        width, height, 30, null, null, "h264", null, null, "not_computed");
}
