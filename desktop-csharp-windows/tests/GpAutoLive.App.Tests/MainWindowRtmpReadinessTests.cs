using GpAutoLive.Contracts;
using GpAutoLive.Windows;
using System.Diagnostics;
using System.Threading;
using System.Windows.Threading;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class MainWindowRtmpReadinessTests
{
    [TestMethod]
    public void Selected_rtmp_tracks_are_not_ready_while_manager_is_starting()
    {
        var config = RtmpOutputConfig.Default with
        {
            TargetUrl = "rtmp://127.0.0.1/live/test",
            VideoEnabled = true,
            AudioEnabled = true,
        };
        var audio = new WindowsRtmpAudioSessionSnapshot(
            IsRunning: true,
            ProducedFrames: 1,
            ForwardedFrames: 1,
            ErrorCode: null,
            Error: null);

        var ready = MainWindow.AreSelectedRtmpTracksReady(
            config,
            RtmpOutputState.Starting,
            audio);

        Assert.IsFalse(ready);
    }

    [TestMethod]
    public void Selected_rtmp_tracks_require_publishing_and_running_audio()
    {
        var config = RtmpOutputConfig.Default with
        {
            TargetUrl = "rtmp://127.0.0.1/live/test",
            VideoEnabled = true,
            AudioEnabled = true,
        };

        var ready = MainWindow.AreSelectedRtmpTracksReady(
            config,
            RtmpOutputState.Publishing,
            new WindowsRtmpAudioSessionSnapshot(true, 1, 1, null, null));
        var missingAudio = MainWindow.AreSelectedRtmpTracksReady(
            config,
            RtmpOutputState.Publishing,
            new WindowsRtmpAudioSessionSnapshot(false, 1, 0, null, null));

        Assert.IsTrue(ready);
        Assert.IsFalse(missingAudio);
    }

    [TestMethod]
    public void Stop_is_available_while_reconnect_coordinator_is_in_backoff()
    {
        var canStop = MainWindow.CanStopRtmpSession(
            RtmpOutputState.Failed,
            RtmpOutputState.Reconnecting,
            new WindowsRtmpAudioSessionSnapshot(
                IsRunning: false,
                ProducedFrames: 0,
                ForwardedFrames: 0,
                ErrorCode: "process_exited",
                Error: "RTMP 进程已退出"));

        Assert.IsTrue(canStop);
    }

    [TestMethod]
    public void Reconnect_prefers_the_last_started_config_over_changed_editor_values()
    {
        var started = RtmpOutputConfig.Default with
        {
            TargetUrl = "rtmp://127.0.0.1/live/started",
            VideoEnabled = true,
            AudioEnabled = false,
        };
        var edited = started with
        {
            TargetUrl = "rtmp://127.0.0.1/live/edited",
            VideoEnabled = false,
            AudioEnabled = true,
        };

        var selected = MainWindow.SelectRtmpReconnectConfig(started, edited);

        Assert.AreEqual(started.TargetUrl, selected.TargetUrl);
        Assert.AreEqual(started.VideoEnabled, selected.VideoEnabled);
        Assert.AreEqual(started.AudioEnabled, selected.AudioEnabled);
    }

    [TestMethod]
    public void Stop_is_available_when_failed_manager_still_has_resources_to_reclaim()
    {
        var canStop = MainWindow.CanStopRtmpSession(
            RtmpOutputState.Failed,
            RtmpOutputState.Idle,
            new WindowsRtmpAudioSessionSnapshot(false, 0, 0, null, null),
            managerNeedsCleanup: true);

        Assert.IsTrue(canStop);
    }

    [TestMethod]
    public void Rtmp_reconnect_attempt_from_background_returns_to_window_dispatcher()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var dispatcher = Dispatcher.CurrentDispatcher;
            var callback = Task.Run(
                () => MainWindow.RunOnDispatcherAsync(
                    dispatcher,
                    () => Task.FromResult(dispatcher.CheckAccess()),
                    CancellationToken.None));

            var deadline = Stopwatch.GetTimestamp() + 2 * Stopwatch.Frequency;
            while (!callback.IsCompleted && Stopwatch.GetTimestamp() < deadline)
            {
                dispatcher.Invoke(
                    DispatcherPriority.Background,
                    new Action(static () => { }));
                Thread.Sleep(10);
            }

            Assert.IsTrue(callback.IsCompleted);
            Assert.IsTrue(callback.GetAwaiter().GetResult());
        });
    }
}
