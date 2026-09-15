using System.IO;
using System.Windows.Controls;
using GpAutoLive.App.Features.Auth;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

public sealed partial class VideoPlaybackAudioFallbackTests
{
    [TestMethod]
    public void Explicit_seek_keeps_paused_audio_closed_and_resumes_at_new_position()
    {
        var fixture = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_SEEK_MEDIA");
        if (string.IsNullOrWhiteSpace(fixture)) return;
        Assert.IsTrue(File.Exists(fixture));
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                window.Show();
                PumpDispatcherUntilLoaded();
                GetPrivateField<LoginViewModel>(window, "_login").ApplyActivated("fixture-account");
                var pool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                var state = GetPrivateField<ShellState>(window, "_state");
                state.AudioProcessing = false;
                state.VideoProcessing = false;
                var committed = pool.ReplaceAll([CreateSyntheticVideoSource(fixture) with { DurationMs = 30_000 }]);
                state.ApplyMediaSnapshot(committed.Snapshot);
                InvokePrivate(window, "UpdateMediaProjection");
                RunWindowTask(window, "TogglePlaybackAsync");
                var audio = GetPrivateField<WindowsAudioPlaybackController>(window, "_audioPlaybackController");
                Assert.AreEqual(WindowsAudioPlaybackState.Playing, audio.Snapshot.State);

                RunWindowTask(window, "TogglePlaybackAsync");
                Assert.AreEqual(PlaybackState.Paused, pool.Snapshot.PlaybackState);
                GetPrivateField<Slider>(window, "PlaybackSlider").Value = 0.4;
                RunWindowTask(window, "SeekVideoFromSliderAsync");
                Assert.AreEqual(PlaybackState.Paused, pool.Snapshot.PlaybackState);
                Assert.IsNull(audio.Snapshot.Output, "暂停中跳转不得短暂开启设备消费PCM。");
                RunWindowTask(window, "TogglePlaybackAsync");
                Assert.AreEqual(WindowsAudioPlaybackState.Playing, audio.Snapshot.State);
                Assert.IsTrue(audio.Snapshot.AudibleClock?.PlaybackTimeMs is >= 11_800 and < 14_000,
                    "恢复必须从新的12秒位置重建声音。");

                GetPrivateField<Slider>(window, "PlaybackSlider").Value = 0.6;
                RunWindowTask(window, "SeekVideoFromSliderAsync");
                var videoPosition = (Task<ulong?>)InvokePrivate(window, "ReadCurrentVideoPositionAsync", pool.CurrentIdentity, (ulong?)30_000)!;
                PumpUntilCompleted(videoPosition);
                var audioPosition = audio.Snapshot.AudibleClock?.PlaybackTimeMs;
                Assert.IsNotNull(videoPosition.Result);
                Assert.IsNotNull(audioPosition);
                Assert.IsTrue(Math.Abs((double)videoPosition.Result.Value - audioPosition.Value) < 300,
                    $"声音重建后应校正视频：video={videoPosition.Result}, audio={audioPosition}。");
            }
            finally
            {
                StopPlaybackForTest(window);
                window.Close();
            }
        });
    }

    private static void RunWindowTask(MainWindow window, string method)
    {
        var task = (Task)InvokePrivate(window, method)!;
        PumpUntilCompleted(task);
        task.GetAwaiter().GetResult();
    }
}
