using System.IO;
using System.Reflection;
using GpAutoLive.App.Features.Auth;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;
using GpAutoLive.Media;

namespace GpAutoLive.App.Tests;

public sealed partial class VideoPlaybackAudioFallbackTests
{
    [TestMethod]
    public void Explicit_video_clock_recovers_from_drift_without_restarting_audio_or_video()
    {
        var fixture = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_SYNC_MEDIA");
        if (string.IsNullOrWhiteSpace(fixture))
            Assert.Inconclusive("Set AUTOLIVE_TEST_SYNC_MEDIA to run the actual mpv/PortAudio synchronization fixture.");
        Assert.IsTrue(File.Exists(fixture));
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                window.Show();
                PumpDispatcherUntilLoaded();
                GetPrivateField<LoginViewModel>(window, "_login").ApplyActivated("sync-fixture");
                var state = GetPrivateField<ShellState>(window, "_state");
                state.AudioProcessing = false;
                state.VideoProcessing = false;
                Func<Task<MediaImportRequest?>> request = () => Task.FromResult<MediaImportRequest?>(
                    new(MediaImportOperation.ReplaceAll, [fixture]));
                var import = (Task)InvokePrivate(window, "RunImportAsync", request)!;
                PumpUntilCompleted(import);
                import.GetAwaiter().GetResult();
                typeof(MainWindow).GetField("_interludeConfig", BindingFlags.Instance | BindingFlags.NonPublic)!
                    .SetValue(window, InterludeAudioConfig.Default with { Enabled = false });
                RunWindowTask(window, "TogglePlaybackAsync");
                var pool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                var audio = GetPrivateField<WindowsAudioPlaybackController>(window, "_audioPlaybackController");
                var video = GetPrivateField<WindowsMpvPlaybackController>(window, "_mpvController");
                Assert.IsTrue(PumpUntil(() => audio.Snapshot.AudibleClock?.PlaybackTimeMs > 200,
                    TimeSpan.FromSeconds(5)), "No actual audible PCM clock.");
                var audioPid = audio.Snapshot.Decoder?.ProcessId;
                var videoPid = video.Snapshot.Runtime.Host.ProcessId;
                var seekElapsed = System.Diagnostics.Stopwatch.StartNew();
                var seek = video.SeekAsync(pool.CurrentIdentity, audio.Snapshot.AudibleClock!.PlaybackTimeMs!.Value + 900);
                PumpUntilCompleted(seek);
                Assert.IsTrue(seek.Result.IsSuccess);
                Console.WriteLine($"injectedSeekCompletionMs={seekElapsed.Elapsed.TotalMilliseconds:F1}");
                double drift = double.MaxValue;
                double signedDrift = double.MaxValue;
                Assert.IsTrue(PumpUntil(() =>
                {
                    var read = (Task<ulong?>)InvokePrivate(window, "ReadCurrentVideoPositionAsync",
                        pool.CurrentIdentity, (ulong?)null)!;
                    PumpUntilCompleted(read);
                    var audible = audio.Snapshot.AudibleClock?.PlaybackTimeMs;
                    if (read.Result is not ulong position || audible is null) return false;
                    signedDrift = (double)position - audible.Value;
                    drift = Math.Abs(signedDrift);
                    return drift < 180;
                }, TimeSpan.FromSeconds(3)), $"Video did not follow the audible clock; drift={drift:F0}ms; signedVideoMinusAudio={signedDrift:F0}ms.");
                Console.WriteLine($"recoveredSignedVideoMinusAudioMs={signedDrift:F1}");
                Assert.AreEqual(audioPid, audio.Snapshot.Decoder?.ProcessId);
                Assert.AreEqual(videoPid, video.Snapshot.Runtime.Host.ProcessId);
            }
            finally
            {
                StopPlaybackForTest(window);
                window.Close();
            }
        });
    }
}
