using System.IO;
using System.Reflection;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
[DoNotParallelize]
public sealed class RtmpSourceTransitionTests
{
    [TestMethod]
    public void Paused_source_change_keeps_the_started_config_for_resume()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                var pool = PreparePool(window, paused: true);
                var config = Config();
                Set(window, "_pausedRtmpConfig", config);
                Change(window, () =>
                {
                    Assert.IsTrue(pool.Next(pool.CurrentIdentity).IsSuccess);
                    return Task.CompletedTask;
                }).GetAwaiter().GetResult();

                Assert.AreEqual(1, pool.Snapshot.SourceMediaIndex);
                Assert.AreEqual(PlaybackState.Paused, pool.Snapshot.PlaybackState);
                Assert.AreSame(config, Get<RtmpOutputConfig>(window, "_pausedRtmpConfig"));
                AssertNoPublication(window);
            }
            finally { window.Close(); }
        });
    }

    [TestMethod]
    public void No_source_change_does_not_start_publication_or_lose_paused_resume_config()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                var pool = PreparePool(window, paused: true);
                var identity = pool.CurrentIdentity;
                var config = Config();
                Set(window, "_pausedRtmpConfig", config);
                Get<ShellState>(window, "_state").SetStatus("unchanged-source");

                Change(window, () => Task.CompletedTask).GetAwaiter().GetResult();

                Assert.AreEqual(identity, pool.CurrentIdentity);
                Assert.AreSame(config, Get<RtmpOutputConfig>(window, "_pausedRtmpConfig"));
                Assert.AreEqual("unchanged-source", Get<ShellState>(window, "_state").StatusMessage);
                AssertNoPublication(window);
            }
            finally { window.Close(); }
        });
    }

    [TestMethod]
    public void Failed_source_load_in_ready_state_does_not_publish_the_new_identity()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                var pool = PreparePool(window, paused: false);
                Set(window, "_lastRtmpConfig", Config());
                Change(window, () =>
                {
                    Assert.IsTrue(pool.ReplaceAll([Media("failed-new-source.mp4")]).IsSuccess);
                    Get<ShellState>(window, "_state").SetStatus("source-load-failed");
                    return Task.CompletedTask;
                }).GetAwaiter().GetResult();

                Assert.AreEqual(PlaybackState.Ready, pool.Snapshot.PlaybackState);
                Assert.AreEqual("source-load-failed", Get<ShellState>(window, "_state").StatusMessage);
                AssertNoPublication(window);
            }
            finally { window.Close(); }
        });
    }

    [TestMethod]
    public void Source_change_exception_is_propagated_without_starting_publication()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                PreparePool(window, paused: false);
                Set(window, "_lastRtmpConfig", Config());
                var expected = new InvalidOperationException("source-change-failed");
                var actual = Assert.ThrowsExactly<InvalidOperationException>(() =>
                    Change(window, () => Task.FromException(expected)).GetAwaiter().GetResult());
                Assert.AreSame(expected, actual);
                AssertNoPublication(window);
            }
            finally { window.Close(); }
        });
    }

    private static MediaPoolService PreparePool(MainWindow window, bool paused)
    {
        var pool = Get<MediaPoolService>(window, "_mediaPool");
        Assert.IsTrue(pool.ReplaceAll([Media("first.mp4"), Media("second.mp4")]).IsSuccess);
        Assert.IsTrue(pool.StartPlayback().IsSuccess);
        if (paused) Assert.IsTrue(pool.PausePlayback().IsSuccess);
        return pool;
    }

    private static void AssertNoPublication(MainWindow window)
    {
        var snapshot = Get<WindowsRtmpOutputManager>(window, "_rtmpOutputManager").Snapshot;
        Assert.AreEqual(RtmpOutputState.Idle, snapshot.State);
        Assert.IsNull(snapshot.ProcessId);
        Assert.IsFalse(Get<WindowsRtmpAudioSession>(window, "_rtmpAudioSession").Snapshot.IsRunning);
    }

    private static Task Change(MainWindow window, Func<Task> action) =>
        (Task)typeof(MainWindow).GetMethod("ChangeMediaSourceWithRtmpAsync",
            BindingFlags.Instance | BindingFlags.NonPublic)!.Invoke(window, [action])!;

    private static T Get<T>(MainWindow window, string field) =>
        (T)typeof(MainWindow).GetField(field, BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(window)!;

    private static void Set(MainWindow window, string field, object value) =>
        typeof(MainWindow).GetField(field, BindingFlags.Instance | BindingFlags.NonPublic)!.SetValue(window, value);

    private static RtmpOutputConfig Config() => RtmpOutputConfig.Default with
    {
        TargetUrl = "rtmp://127.0.0.1/live/transition-fixture",
        AudioEnabled = false,
    };

    private static SourceMediaDto Media(string name)
    {
        var path = Path.Combine(Path.GetTempPath(), "gpautolive-rtmp-transition", name);
        return new(path, path, MediaKind.Video, MediaCompatibilityMode.Direct, name,
            1, 10_000, null, null, 320, 180, 30, null, null, "h264", null, null, "disabled");
    }
}
