using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Windows.Threading;
using GpAutoLive.App.Features.Auth;
using GpAutoLive.App.Features.Playback;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

public sealed partial class VideoTransitionPixelFixtureTests
{
    [TestMethod]
    public void Explicit_user_files_manual_previous_paused_switch_and_resume() => RunUserFiles(automatic: false);

    [TestMethod]
    public void Explicit_user_files_full_natural_eof_and_wrap() => RunUserFiles(automatic: true);

    private void RunUserFiles(bool automatic)
    {
        if (Environment.GetEnvironmentVariable("AUTOLIVE_TEST_VIDEO_TRANSITIONS") != "1")
            Assert.Inconclusive("Explicit user-media fixture is disabled; no real-media verdict.");
        var paths = new[] { "AUTOLIVE_TEST_TRANSITION_FIRST", "AUTOLIVE_TEST_TRANSITION_SECOND" }
            .Select(name => Environment.GetEnvironmentVariable(name)).ToArray();
        foreach (var path in paths) Assert.IsTrue(File.Exists(path), "Set both user fixture paths to existing files.");
        Assert.AreNotEqual(Path.GetFullPath(paths[0]!), Path.GetFullPath(paths[1]!));
        var files = paths.Select(path => new FileInfo(path!)).ToArray();
        var original = files.Select(file => (file.Length, file.LastWriteTimeUtc)).ToArray();
        try
        {
            WpfTestApplicationHost.Run(() => RunUserFilesWindow(paths[0]!, paths[1]!, automatic), TimeSpan.FromMinutes(6));
        }
        finally
        {
            for (var index = 0; index < files.Length; index++)
            {
                files[index].Refresh();
                Assert.IsTrue(files[index].Exists, "User source must not be removed.");
                Assert.AreEqual(original[index], (files[index].Length, files[index].LastWriteTimeUtc), "User source must remain unchanged.");
            }
        }
    }

    private void RunUserFilesWindow(string first, string second, bool automatic)
    {
        MainWindow? window = null;
        WindowsGraphicsCaptureWindowSession? capture = null;
        Task<(MediaPlaybackIdentity Identity, ulong? VideoMs, WindowsAudibleAudioClockSnapshot? Audio)>? clockRead = null;
        using var binding = new WindowsVirtualCameraSurfaceBinding();
        using var sampler = new VideoTransitionFrameSampler();
        List<double>[] driftsBySource = [[], []];
        var elapsed = Stopwatch.StartNew();
        var videoProcessing = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_TRANSITION_VIDEO_PROCESSING") == "1";
        var audioProcessing = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_TRANSITION_AUDIO_PROCESSING") == "1";
        try
        {
            window = new MainWindow();
            window.Show();
            var state = Field<ShellState>(window, "_state");
            state.VideoProcessing = videoProcessing;
            state.AudioProcessing = audioProcessing;
            Field<LoginViewModel>(window, "_login").ApplyActivated("user-transition-fixture");
            Func<Task<MediaImportRequest?>> request = () => Task.FromResult<MediaImportRequest?>(
                new(MediaImportOperation.ReplaceAll, [first, second]));
            Await(Invoke(window, "RunImportAsync", request));
            window.GetType().GetField("_interludeConfig", BindingFlags.Instance | BindingFlags.NonPublic)!
                .SetValue(window, InterludeAudioConfig.Default with { Enabled = false });
            var pool = Field<MediaPoolService>(window, "_mediaPool");
            Assert.AreEqual(2, pool.Snapshot.SourceMediaPool.Length);
            TestContext.WriteLine($"userFiles: firstDurationMs={pool.Snapshot.SourceMediaPool[0].DurationMs}; secondDurationMs={pool.Snapshot.SourceMediaPool[1].DurationMs}; videoProcessing={videoProcessing}; audioProcessing={audioProcessing}; automatic={automatic}");
            Await(Invoke(window, "TogglePlaybackCoreAsync"));
            Assert.AreEqual(PlaybackState.Playing, pool.Snapshot.PlaybackState);
            var mpv = Field<WindowsMpvPlaybackController>(window, "_mpvController");
            var originalPid = mpv.Snapshot.Runtime.Host.ProcessId;
            Assert.IsNotNull(originalPid);
            if (videoProcessing) Assert.AreEqual(MpvVideoProcessingMode.Gpu83, mpv.Snapshot.ActiveVideoProcessingMode);
            var audio = Field<WindowsAudioPlaybackController>(window, "_audioPlaybackController");
            var final = Field<FinalEffectWindow>(window, "_finalEffectWindow");
            Assert.IsTrue(final.TryGetCaptureWindowHandle(out var hwnd));
            Assert.IsTrue(binding.Bind(hwnd).IsSuccess);
            capture = new WindowsGraphicsCaptureWindowSession();
            var started = capture.StartAsync(binding, TimeSpan.FromSeconds(10), frameConsumer: sampler.Accept);
            Await(started);
            Assert.IsTrue(started.Result.IsSuccess, started.Result.Code.ToString());
            Until(() => sampler.Snapshot.Samples > 0, "initial user source WGC frame");
            sampler.Arm();

            var nextSample = TimeSpan.Zero;
            var readStarted = TimeSpan.Zero;
            async Task<(MediaPlaybackIdentity Identity, ulong? VideoMs, WindowsAudibleAudioClockSnapshot? Audio)> ReadClockPairAsync()
            {
                var identity = pool.CurrentIdentity;
                var source = pool.Snapshot.SourceMediaPool[pool.Snapshot.SourceMediaIndex];
                var read = (Task<ulong?>)Invoke(window, "ReadCurrentVideoPositionAsync", identity, source.DurationMs!.Value);
                var videoMs = await read;
                return (identity, videoMs, audio.Snapshot.AudibleClock);
            }

            void SampleClocks()
            {
                if (clockRead is not null)
                {
                    Assert.IsTrue(clockRead.IsCompleted || elapsed.Elapsed - readStarted < TimeSpan.FromSeconds(5),
                        "Fixture clock read remained pending for five seconds; no nested Dispatcher wait is used.");
                    if (!clockRead.IsCompleted) return;
                    var pair = clockRead.GetAwaiter().GetResult();
                    clockRead = null;
                    if (pair.Identity != pool.CurrentIdentity || final.HasHeldVideoFrame
                        || pair.VideoMs is not ulong videoMs || pair.Audio?.PlaybackTimeMs is not ulong audioMs) return;
                    var signedVideoMinusAudio = (double)videoMs - audioMs;
                    var drift = Math.Abs(signedVideoMinusAudio);
                    driftsBySource[pool.Snapshot.SourceMediaIndex].Add(drift);
                    var pcm = audio.Snapshot;
                    TestContext.WriteLine($"clock: wallSeconds={elapsed.Elapsed.TotalSeconds:F2}; source={pool.Snapshot.SourceMediaIndex}; loop={pool.Snapshot.LoopIndex}; videoMs={videoMs}; audioMs={audioMs}; rate={pair.Audio.PlaybackRate:F4}; signedVideoMinusAudio={signedVideoMinusAudio:F1}; driftMs={drift:F1}; pid={mpv.Snapshot.Runtime.Host.ProcessId}");
                    TestContext.WriteLine($"pcm: underrunFrames={pcm.Output?.UnderrunFrames}; xruns={pcm.Output?.XrunCount}; latencyUs={pcm.Output?.OutputLatencyMicroseconds}; decoderDroppedFrames={pcm.Decoder?.DroppedFrames}");
                }
                if (elapsed.Elapsed < nextSample) return;
                nextSample = elapsed.Elapsed + TimeSpan.FromSeconds(1);
                if (pool.Snapshot.PlaybackState != PlaybackState.Playing || final.HasHeldVideoFrame
                    || audio.Snapshot.State != WindowsAudioPlaybackState.Playing
                    || audio.Snapshot.AudibleClock?.PlaybackTimeMs is not > 1000) return;
                readStarted = elapsed.Elapsed;
                clockRead = ReadClockPairAsync();
            }

            void Observe(TimeSpan period, string label)
            {
                var end = elapsed.Elapsed + period;
                UntilUserFiles(() => elapsed.Elapsed >= end, period + TimeSpan.FromSeconds(5), label, SampleClocks);
            }

            void AssertSource(int index, PlaybackState expected)
            {
                UntilUserFiles(() => pool.Snapshot.SourceMediaIndex == index && !final.HasHeldVideoFrame
                    && pool.Snapshot.PlaybackState == expected, TimeSpan.FromSeconds(20), $"source {index} {expected}", SampleClocks);
                Assert.AreEqual(originalPid, mpv.Snapshot.Runtime.Host.ProcessId, "Navigation must retain the same mpv process.");
                if (videoProcessing) Assert.AreEqual(MpvVideoProcessingMode.Gpu83, mpv.Snapshot.ActiveVideoProcessingMode);
                TestContext.WriteLine($"sourceReady: wallSeconds={elapsed.Elapsed.TotalSeconds:F2}; source={index}; state={expected}; hold={final.HasHeldVideoFrame}; pid={originalPid}");
            }

            Observe(TimeSpan.FromSeconds(20), "first source clocks through startup drift recovery");
            if (automatic)
            {
                var cycle = pool.Snapshot.PlaybackPoolCycle;
                UntilUserFiles(() => pool.Snapshot.SourceMediaIndex == 1 && !final.HasHeldVideoFrame,
                    TimeSpan.FromSeconds(90), "natural TS EOF to MP4", SampleClocks);
                AssertSource(1, PlaybackState.Playing);
                UntilUserFiles(() => pool.Snapshot.SourceMediaIndex == 0 && pool.Snapshot.PlaybackPoolCycle > cycle
                    && !final.HasHeldVideoFrame, TimeSpan.FromSeconds(245), "complete MP4 natural EOF back to TS", SampleClocks);
                AssertSource(0, PlaybackState.Playing);
                Observe(TimeSpan.FromSeconds(3), "clock after complete natural wrap");
            }
            else
            {
                Await(Invoke(window, "NavigateMediaCoreAsync", true));
                AssertSource(1, PlaybackState.Playing);
                Observe(TimeSpan.FromSeconds(3), "manual next clocks");
                Await(Invoke(window, "NavigateMediaCoreAsync", false));
                AssertSource(0, PlaybackState.Playing);
                Observe(TimeSpan.FromSeconds(3), "manual previous clocks");
                Await(Invoke(window, "TogglePlaybackCoreAsync"));
                Assert.AreEqual(PlaybackState.Paused, pool.Snapshot.PlaybackState);
                Await(Invoke(window, "NavigateMediaCoreAsync", true));
                AssertSource(1, PlaybackState.Paused);
                var pausedPosition = audio.Snapshot.AudibleClock?.PlaybackTimeMs;
                Observe(TimeSpan.FromSeconds(1), "paused next source remains paused");
                Assert.AreEqual(PlaybackState.Paused, pool.Snapshot.PlaybackState);
                Assert.AreEqual(pausedPosition, audio.Snapshot.AudibleClock?.PlaybackTimeMs);
                Await(Invoke(window, "TogglePlaybackCoreAsync"));
                AssertSource(1, PlaybackState.Playing);
                Observe(TimeSpan.FromSeconds(3), "resumed source clocks");
            }
            Assert.AreEqual(WindowsGraphicsCaptureWindowSessionCode.Running, capture.Snapshot.Code);
            Assert.IsFalse(final.HasHeldVideoFrame);
            Assert.IsNull(sampler.Snapshot.Failure);
            Assert.IsTrue(sampler.Snapshot.Samples > 0);
            var sourceP95 = new double[driftsBySource.Length];
            for (var sourceIndex = 0; sourceIndex < driftsBySource.Length; sourceIndex++)
            {
                var sorted = driftsBySource[sourceIndex].Order().ToArray();
                sourceP95[sourceIndex] = sorted.Length == 0 ? double.NaN : sorted[(int)Math.Ceiling(sorted.Length * 0.95) - 1];
                var maximum = sorted.Length == 0 ? double.NaN : sorted[^1];
                TestContext.WriteLine($"driftSummary: source={sourceIndex}; count={sorted.Length}; p95Ms={sourceP95[sourceIndex]:F1}; maxMs={maximum:F1}");
            }
            for (var sourceIndex = 0; sourceIndex < driftsBySource.Length; sourceIndex++)
            {
                Assert.IsTrue(driftsBySource[sourceIndex].Count >= 3, $"Source {sourceIndex} needs at least three audible/video paired clock samples.");
                Assert.IsTrue(sourceP95[sourceIndex] < 180, $"Source {sourceIndex} steady playback P95 drift is {sourceP95[sourceIndex]:F1}ms.");
            }
        }
        finally
        {
            var result = sampler.Snapshot;
            TestContext.WriteLine($"captureSummary: wallSeconds={elapsed.Elapsed.TotalSeconds:F2}; samples={result.Samples}; blackRoiFrames={result.BlackFrames}; maximumCaptureGapMs={result.MaximumGap100Ns / 10000.0:F2}; captureFailure={result.Failure ?? "none"}; black ROIs may be source content; capture gaps include paused/static frames and are not a standalone stutter verdict.");
            if (window is not null) TestContext.WriteLine($"finalStatus={Field<ShellState>(window, "_state").StatusMessage}; pool={Field<MediaPoolService>(window, "_mediaPool").Snapshot.PlaybackState}; mpv={Field<WindowsMpvPlaybackController>(window, "_mpvController").Snapshot.State}");
            try
            {
                if (capture is not null) { Await(capture.StopAsync()); Await(capture.DisposeAsync().AsTask()); }
            }
            finally
            {
                if (window is not null)
                {
                    window.Close();
                    if (window.ShutdownCompletion is { } shutdown) Await(shutdown);
                    if (clockRead is not null)
                    {
                        try { Await(clockRead); }
                        catch (OperationCanceledException) { }
                    }
                }
            }
        }
    }

    private static void UntilUserFiles(Func<bool> predicate, TimeSpan timeout, string operation, Action sample)
    {
        var deadline = Stopwatch.StartNew();
        var frame = new DispatcherFrame();
        var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(100) };
        Exception? failure = null;
        timer.Tick += (_, _) =>
        {
            try
            {
                if (predicate() || deadline.Elapsed >= timeout) frame.Continue = false;
                else sample();
            }
            catch (Exception exception) { failure = exception; frame.Continue = false; }
        };
        timer.Start();
        try { Dispatcher.PushFrame(frame); }
        finally { timer.Stop(); }
        if (failure is not null) System.Runtime.ExceptionServices.ExceptionDispatchInfo.Capture(failure).Throw();
        Assert.IsTrue(predicate(), "Timed out: " + operation);
    }
}
