using GpAutoLive.App.Features.Playback;
using GpAutoLive.Contracts;
using System.Windows;
using System.Windows.Controls;
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
            PrepareWpfHost();
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
    public void Final_effect_window_keeps_footer_and_hwnd_contract_across_fullscreen_round_trip()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        WpfTestApplicationHost.Run(() =>
        {
            PrepareWpfHost();
            var controller = new FinalEffectWindowController();
            var window = new FinalEffectWindow(controller);
            try
            {
                window.Show();
                PumpWpfLayout();
                controller.Open(FinalEffectSnapshot.Create(
                    PlaybackState.Playing,
                    FinalEffectSurfaceKind.VideoHwndReserved,
                    TimeSpan.FromSeconds(3),
                    TimeSpan.FromSeconds(10),
                    7,
                    11,
                    2));

                AssertFooterAndProjection(window);
                AssertHwndContractWhenWindowIsAvailable(window);

                window.ToggleFullscreen();
                Assert.IsTrue(window.IsFullscreen);
                AssertFooterAndProjection(window);
                AssertHwndContractWhenWindowIsAvailable(window);

                window.ToggleFullscreen();
                Assert.IsFalse(window.IsFullscreen);
                AssertFooterAndProjection(window);
                AssertHwndContractWhenWindowIsAvailable(window);
            }
            finally
            {
                window.Close();
            }
        });
    }

    [TestMethod]
    public void Final_effect_window_buttons_forward_to_controller_command_boundary()
    {
        WpfTestApplicationHost.Run(() =>
        {
            PrepareWpfHost();
            var controller = new FinalEffectWindowController();
            var window = new FinalEffectWindow(controller);
            var commands = new List<FinalEffectPlaybackCommand>();
            controller.CommandRequested += (_, args) => commands.Add(args.Command);
            try
            {
                window.Show();
                PumpWpfLayout();
                controller.Open(FinalEffectSnapshot.Empty with
                {
                    SurfaceKind = FinalEffectSurfaceKind.VideoHwndReserved,
                    PlaybackState = PlaybackState.Playing,
                    Duration = TimeSpan.FromSeconds(10),
                });

                var playPauseButton = (Button?)window.FindName("PlayPauseButton");
                var stopButton = (Button?)window.FindName("StopButton");
                var closeButton = (Button?)window.FindName("CloseButton");
                Assert.IsNotNull(playPauseButton);
                Assert.IsNotNull(stopButton);
                Assert.IsNotNull(closeButton);

                playPauseButton!.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
                stopButton!.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
                closeButton!.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));

                CollectionAssert.AreEqual(
                    new[]
                    {
                        FinalEffectPlaybackCommand.TogglePlayPause,
                        FinalEffectPlaybackCommand.Stop,
                    },
                    commands);
                Assert.IsFalse(controller.IsOpen);
            }
            finally
            {
                window.Close();
            }
        });
    }

    private static void AssertFooterAndProjection(FinalEffectWindow window)
    {
        var root = (FrameworkElement)window.Content;
        root.Measure(new Size(window.Width, window.Height));
        root.Arrange(new Rect(0, 0, window.Width, window.Height));
        window.UpdateLayout();

        var footer = (Border?)window.FindName("Footer");
        var playPauseButton = (Button?)window.FindName("PlayPauseButton");
        var stopButton = (Button?)window.FindName("StopButton");
        var playbackStateText = (TextBlock?)window.FindName("PlaybackStateText");
        var playbackProgress = (ProgressBar?)window.FindName("PlaybackProgress");
        var identityText = (TextBlock?)window.FindName("IdentityText");

        Assert.IsNotNull(footer);
        Assert.AreEqual(Visibility.Visible, footer!.Visibility);
        Assert.IsTrue(footer.IsHitTestVisible);
        Assert.IsTrue(footer.ActualHeight > 0);
        Assert.IsNotNull(playPauseButton);
        Assert.IsNotNull(stopButton);
        Assert.AreEqual(Visibility.Visible, playPauseButton!.Visibility);
        Assert.AreEqual(Visibility.Visible, stopButton!.Visibility);
        Assert.IsTrue(playPauseButton.ActualWidth > 0);
        Assert.IsTrue(stopButton.ActualWidth > 0);
        Assert.IsTrue(playPauseButton.IsEnabled);
        Assert.IsTrue(stopButton.IsEnabled);
        Assert.AreEqual("播放中", playbackStateText!.Text);
        Assert.AreEqual(0.3d, playbackProgress!.Value, 0.001d);
        StringAssert.Contains(identityText!.Text, "会话 7 · 源 11 · 循环 2");
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

    private static void PrepareWpfHost() =>
        Application.Current!.ShutdownMode = ShutdownMode.OnExplicitShutdown;

    private static void PumpWpfLayout()
    {
        var frame = new DispatcherFrame();
        _ = Dispatcher.CurrentDispatcher.BeginInvoke(
            DispatcherPriority.ApplicationIdle,
            new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }
}
