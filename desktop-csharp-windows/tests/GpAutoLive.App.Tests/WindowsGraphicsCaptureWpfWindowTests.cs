using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using GpAutoLive.App.Features.Playback;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
[DoNotParallelize]
public sealed class WindowsGraphicsCaptureWpfWindowTests
{
    [TestMethod]
    public void WpfHost_bounds_a_blocked_dispatcher_action()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        using var started = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        using var completed = new ManualResetEventSlim();
        try
        {
            Assert.ThrowsExactly<AssertFailedException>(() =>
                WpfTestApplicationHost.Run(
                    () =>
                    {
                        started.Set();
                        try
                        {
                            release.Wait(TimeSpan.FromSeconds(2));
                        }
                        finally
                        {
                            completed.Set();
                        }
                    },
                    TimeSpan.FromMilliseconds(100)));

            Assert.IsTrue(started.Wait(TimeSpan.FromSeconds(1)));
            Assert.IsFalse(completed.IsSet);
        }
        finally
        {
            release.Set();
            Assert.IsTrue(completed.Wait(TimeSpan.FromSeconds(2)));
        }
    }

    [TestMethod]
    public void VisibleWpfWindow_StartsRealWgcFramePool()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        WpfTestApplicationHost.Run(() =>
        {
            var window = new Window
            {
                Title = "GpAutoLive WPF WGC integration",
                Width = 640,
                Height = 360,
                Content = new Border
                {
                    Background = Brushes.DarkSlateBlue,
                    Child = new TextBlock
                    {
                        Text = "WGC",
                        Foreground = Brushes.White,
                        FontSize = 32,
                        HorizontalAlignment = HorizontalAlignment.Center,
                        VerticalAlignment = VerticalAlignment.Center,
                    },
                },
            };
            window.Show();
            try
            {
                window.UpdateLayout();
                using var binding = new WindowsVirtualCameraSurfaceBinding();
                var handle = new System.Windows.Interop.WindowInteropHelper(window).Handle;
                Assert.AreNotEqual(IntPtr.Zero, handle);
                var bindResult = binding.Bind(unchecked((uint)handle.ToInt64()));
                Assert.IsTrue(bindResult.IsSuccess);

                var session = new WindowsGraphicsCaptureWindowSession();
                try
                {
                    var result = session.StartAsync(
                            binding,
                            TimeSpan.FromSeconds(10))
                        .GetAwaiter()
                        .GetResult();

                    Assert.IsTrue(result.IsSuccess, $"WPF WGC 启动失败：{result.Code}");
                    Assert.AreEqual(WindowsGraphicsCaptureWindowSessionCode.Running, result.Code);
                    Assert.IsTrue(session.Snapshot.Width > 0);
                    Assert.IsTrue(session.Snapshot.Height > 0);
                    session.StopAsync().GetAwaiter().GetResult();
                    Assert.AreEqual(WindowsGraphicsCaptureWindowSessionCode.Stopped, session.Snapshot.Code);
                }
                finally
                {
                    session.DisposeAsync().AsTask().GetAwaiter().GetResult();
                }
            }
            finally
            {
                window.Close();
            }
        });
    }

    [TestMethod]
    public void VisibleFinalEffectWindow_StartsRealWgcFramePool()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        WpfTestApplicationHost.Run(() =>
        {
            var window = new FinalEffectWindow(new FinalEffectWindowController());
            window.Show();
            try
            {
                window.UpdateLayout();
                using var binding = new WindowsVirtualCameraSurfaceBinding();
                Assert.IsTrue(window.TryGetCaptureWindowHandle(out var handle));
                var bindResult = binding.Bind(handle);
                Assert.IsTrue(bindResult.IsSuccess);

                var session = new WindowsGraphicsCaptureWindowSession();
                try
                {
                    var result = session.StartAsync(
                            binding,
                            TimeSpan.FromSeconds(10))
                        .GetAwaiter()
                        .GetResult();

                    Assert.IsTrue(result.IsSuccess, $"最终效果窗口 WGC 启动失败：{result.Code}");
                    Assert.AreEqual(WindowsGraphicsCaptureWindowSessionCode.Running, result.Code);
                    Assert.IsTrue(session.Snapshot.Width > 0);
                    Assert.IsTrue(session.Snapshot.Height > 0);
                    session.StopAsync().GetAwaiter().GetResult();
                    Assert.AreEqual(WindowsGraphicsCaptureWindowSessionCode.Stopped, session.Snapshot.Code);
                }
                finally
                {
                    session.DisposeAsync().AsTask().GetAwaiter().GetResult();
                }
            }
            finally
            {
                window.Close();
            }
        });
    }

    [TestMethod]
    public void HiddenFinalEffectWindow_DoesNotExposeCaptureHandle()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        WpfTestApplicationHost.Run(() =>
        {
            var window = new FinalEffectWindow(new FinalEffectWindowController());
            window.Show();
            window.UpdateLayout();
            window.Close();

            Assert.IsFalse(window.TryGetCaptureWindowHandle(out _));
        });
    }

}
