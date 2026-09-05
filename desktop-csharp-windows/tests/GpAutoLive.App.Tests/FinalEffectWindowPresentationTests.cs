using GpAutoLive.App.Features.Playback;
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
                controller.Open(FinalEffectSnapshot.Create(FinalEffectSurfaceKind.VideoHwndReserved));

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
        Assert.AreEqual(1280d, window.Width);
        Assert.AreEqual(720d, window.Height);
        Assert.AreEqual(320d, window.MinWidth);
        Assert.AreEqual(180d, window.MinHeight);
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
}
