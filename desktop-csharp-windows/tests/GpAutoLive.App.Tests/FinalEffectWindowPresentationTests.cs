using System.IO;
using System.Reflection;
using GpAutoLive.App.Features.Playback;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using System.Windows;
using System.Windows.Threading;

namespace GpAutoLive.App.Tests;

[TestClass]
[DoNotParallelize]
public sealed class FinalEffectWindowPresentationTests
{
    [TestMethod]
    public void Final_effect_window_is_not_presented_as_a_second_taskbar_application()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new FinalEffectWindow(new FinalEffectWindowController());
            try
            {
                Assert.IsFalse(window.ShowInTaskbar);
            }
            finally
            {
                window.Close();
            }
        });
    }

    [TestMethod]
    public void Final_effect_window_keeps_video_only_surface_and_hwnd_contract_across_fullscreen_round_trip()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        WpfTestApplicationHost.Run(() =>
        {
            var controller = new FinalEffectWindowController();
            var window = new FinalEffectWindow(controller);
            try
            {
                window.Show();
                PumpWpfLayout();
                controller.Open(FinalEffectSnapshot.Create(
                    FinalEffectSurfaceKind.VideoHwndReserved,
                    videoWidth: 1920,
                    videoHeight: 1080));

                AssertVideoOnlySurface(window);
                AssertInitialWindowSize(window);
                AssertHwndContractWhenWindowIsAvailable(window);

                window.ToggleFullscreen();
                Assert.IsTrue(window.IsFullscreen);
                AssertVideoOnlySurface(window);
                AssertHwndContractWhenWindowIsAvailable(window);

                window.ToggleFullscreen();
                Assert.IsFalse(window.IsFullscreen);
                AssertVideoOnlySurface(window);
                AssertHwndContractWhenWindowIsAvailable(window);
            }
            finally
            {
                window.Close();
            }
        });
    }

    [TestMethod]
    public void Main_window_projects_current_video_dimensions_to_final_effect_window()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                var path = Path.Combine(Path.GetTempPath(), "GpAutoLive.final-effect-sizing.mp4");
                var source = new SourceMediaDto(
                    path,
                    path,
                    MediaKind.Video,
                    MediaCompatibilityMode.Direct,
                    "final-effect-sizing.mp4",
                    1,
                    1_000,
                    null,
                    null,
                    1920,
                    1080,
                    30,
                    null,
                    null,
                    "h264",
                    null,
                    null,
                    "disabled");
                var result = mediaPool.ReplaceAll([source]);
                Assert.IsTrue(result.IsSuccess, result.Error?.Message);

                var createSnapshot = typeof(MainWindow).GetMethod(
                    "CreateFinalEffectSnapshot",
                    BindingFlags.Instance | BindingFlags.NonPublic);
                var snapshot = createSnapshot?.Invoke(window, null) as FinalEffectSnapshot;

                Assert.IsNotNull(snapshot);
                Assert.AreEqual(FinalEffectSurfaceKind.VideoHwndReserved, snapshot.SurfaceKind);
                Assert.AreEqual(1920u, snapshot.VideoWidth);
                Assert.AreEqual(1080u, snapshot.VideoHeight);
            }
            finally
            {
                window?.Close();
            }
        });
    }

    private static void AssertVideoOnlySurface(FinalEffectWindow window)
    {
        var root = (FrameworkElement)window.Content;
        root.Measure(new Size(window.Width, window.Height));
        root.Arrange(new Rect(0, 0, window.Width, window.Height));
        window.UpdateLayout();

        Assert.IsNotNull(window.FindName("VideoSurface"));
        Assert.IsNull(window.FindName("Footer"));
        Assert.IsNull(window.FindName("PlayPauseButton"));
        Assert.IsNull(window.FindName("StopButton"));
        Assert.IsNull(window.FindName("PlaybackStateText"));
        Assert.IsNull(window.FindName("PlaybackProgress"));
        Assert.IsNull(window.FindName("IdentityText"));
        Assert.IsNull(window.FindName("CloseButton"));
        Assert.IsNull(window.FindName("SurfaceModeText"));
        Assert.IsNull(window.FindName("EmptySurface"));
    }

    private static void AssertInitialWindowSize(FinalEffectWindow window)
    {
        Assert.AreEqual(320d, window.MinWidth);
        Assert.AreEqual(180d, window.MinHeight);
        Assert.AreEqual(16d / 9d, window.VideoAspectRatio, 0.001d);
        Assert.IsTrue(window.IsVideoAspectRatioLocked);
        Assert.IsTrue(window.ActualWidth > 0);
        Assert.IsTrue(window.ActualHeight > 0);
    }

    private static void AssertHwndContractWhenWindowIsAvailable(FinalEffectWindow window)
    {
        var hasCaptureWindow = window.TryGetCaptureWindowHandle(out var captureWindow);
        var hasVideoSurface = window.TryGetVideoSurfaceHandle(out var videoSurface);

        if (hasCaptureWindow && hasVideoSurface)
        {
            Assert.AreNotEqual(captureWindow, videoSurface);
        }
    }

    private static void PumpWpfLayout()
    {
        var frame = new DispatcherFrame();
        _ = Dispatcher.CurrentDispatcher.BeginInvoke(
            DispatcherPriority.ApplicationIdle,
            new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }

    private static T GetPrivateField<T>(object instance, string fieldName) =>
        instance.GetType()
            .GetField(fieldName, BindingFlags.Instance | BindingFlags.NonPublic)
            ?.GetValue(instance) is T value
            ? value
            : throw new MissingFieldException(instance.GetType().FullName, fieldName);
}
