using System.Reflection;
using System.Windows.Threading;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
[DoNotParallelize]
public sealed class AudioDeviceSelectionTests
{
    [TestMethod]
    public void Refresh_waits_for_playback_command_and_cancels_without_entering_native_probe()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            var gate = (SemaphoreSlim)typeof(MainWindow)
                .GetField("_playbackCommandSerial", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(window)!;
            gate.Wait();
            try
            {
                var refresh = (Task)typeof(MainWindow)
                    .GetMethod("RefreshAudioDevicesAsync", BindingFlags.Instance | BindingFlags.NonPublic)!.Invoke(window, null)!;
                Assert.IsFalse(refresh.IsCompleted, "刷新必须等待当前播放命令退出。");
                var cancellation = (CancellationTokenSource)typeof(MainWindow)
                    .GetField("_windowCancellation", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(window)!;
                cancellation.Cancel();
                PumpUntil(refresh);
                Assert.IsFalse((bool)typeof(MainWindow)
                    .GetField("_audioDevicesEnumerated", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(window)!);
            }
            finally
            {
                gate.Release();
                window.Close();
                if (window.ShutdownCompletion is Task shutdown) PumpUntil(shutdown);
            }
        });
    }

    private static void PumpUntil(Task task)
    {
        var frame = new DispatcherFrame();
        var dispatcher = Dispatcher.CurrentDispatcher;
        _ = task.ContinueWith(_ => dispatcher.BeginInvoke(() => frame.Continue = false), TaskScheduler.Default);
        var timer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(10) };
        timer.Tick += (_, _) => frame.Continue = false;
        timer.Start();
        Dispatcher.PushFrame(frame);
        timer.Stop();
        Assert.IsTrue(task.IsCompleted);
        task.GetAwaiter().GetResult();
    }

    [TestMethod]
    public void Refresh_preserves_unique_identity_with_new_index_and_rejects_missing_or_ambiguous_devices()
    {
        var previous = Device(3, "USB", "WASAPI");
        var current = Device(9, "USB", "WASAPI");
        Assert.AreSame(current, MainWindow.MatchAudioDevice([current, Device(3, "Other", "MME")], previous, 3, false));
        Assert.IsNull(MainWindow.MatchAudioDevice([Device(3, "Other", "MME")], previous, 3, false));
        Assert.IsNull(MainWindow.MatchAudioDevice([current, Device(10, "USB", "WASAPI")], previous, 9, false));
        Assert.IsNull(MainWindow.MatchAudioDevice([Device(3, "USB", "MME")], previous, 3, false));
    }

    [TestMethod]
    public void Only_initial_enumeration_uses_actual_default_and_never_first_device()
    {
        var device = Device(9, "USB", "WASAPI");
        Assert.AreSame(device, MainWindow.MatchAudioDevice([device], null, 9, true));
        Assert.IsNull(MainWindow.MatchAudioDevice([device], null, null, true));
        Assert.IsNull(MainWindow.MatchAudioDevice([device], null, 4, true));
        Assert.IsNull(MainWindow.MatchAudioDevice([device], null, 9, false));
        Assert.IsNull(MainWindow.MatchAudioDevice([], null, 9, true));
    }

    [TestMethod]
    public void Refresh_rejects_active_or_paused_resources_and_allows_failed_owner_cleanup()
    {
        foreach (var state in new[] { WindowsAudioPlaybackState.Starting, WindowsAudioPlaybackState.Playing,
                     WindowsAudioPlaybackState.Paused, WindowsAudioPlaybackState.Stopping })
            Assert.IsFalse(MainWindow.CanRefreshAudioDevices(state, WindowsMicrophoneInterludeState.Idle));
        foreach (var state in new[] { WindowsMicrophoneInterludeState.Starting,
                     WindowsMicrophoneInterludeState.Listening, WindowsMicrophoneInterludeState.Stopping })
            Assert.IsFalse(MainWindow.CanRefreshAudioDevices(WindowsAudioPlaybackState.Idle, state));
        Assert.IsTrue(MainWindow.CanRefreshAudioDevices(WindowsAudioPlaybackState.Failed, WindowsMicrophoneInterludeState.Failed));
    }

    private static WindowsPortAudioDevice Device(int index, string name, string host) => new(index, name, host, 1, 2, 48_000);

}
