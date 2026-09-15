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

[TestClass, DoNotParallelize]
public sealed partial class VideoTransitionPixelFixtureTests
{
    public TestContext TestContext { get; set; } = null!;

    [TestMethod] public void Explicit_manual_next_previous_and_paused_next_have_no_observed_black_frames() => Run("manual");
    [TestMethod] public void Explicit_gpu83_manual_and_paused_switch_keep_processed_picture() => Run("manual-gpu83");
    [TestMethod] public void Explicit_automatic_eof_and_wrap_have_no_observed_black_frames() => Run("automatic");
    [TestMethod] public void Explicit_manual_audio_video_switch_keeps_picture_and_clocks_aligned() => Run("manual-audio");
    [TestMethod] public void Explicit_automatic_audio_video_switch_keeps_picture_and_clocks_aligned() => Run("automatic-audio");
    [TestMethod] public void Explicit_missing_next_file_keeps_last_picture() => Run("missing");
    [TestMethod] public void Explicit_single_item_loop_keeps_picture_and_advances_loop_index() => Run("single");
    [TestMethod] public void Explicit_replace_all_retains_picture_until_new_pool_is_played() => Run("replace");

    private void Run(string scenario)
    {
        if (Environment.GetEnvironmentVariable("AUTOLIVE_TEST_VIDEO_TRANSITIONS") != "1")
            Assert.Inconclusive("Explicit Windows video transition fixture is disabled; no real-media verdict.");
        var runtime = Environment.GetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT");
        Assert.IsFalse(string.IsNullOrWhiteSpace(runtime), "Set AUTOLIVE_MEDIA_RUNTIME_ROOT to a verified media runtime.");
        var ffmpeg = Path.Combine(runtime!, "runtime", "media", "1.0.0", "bin", "ffmpeg.exe");
        Assert.IsTrue(File.Exists(ffmpeg));
        var directory = Path.Combine(Path.GetTempPath(), "gpautolive-transition-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            var red = Path.Combine(directory, "red.mp4");
            var green = Path.Combine(directory, "green.mp4");
            var automatic = scenario.StartsWith("automatic", StringComparison.Ordinal);
            var withAudio = scenario.EndsWith("-audio", StringComparison.Ordinal);
            Generate(ffmpeg, red, "red", "320x180", automatic || scenario == "single" ? 3 : 30, withAudio);
            Generate(ffmpeg, green, "lime", "640x360", automatic ? 3 : 30, withAudio);
            WpfTestApplicationHost.Run(() => RunWindow(red, green, scenario));
        }
        finally { Directory.Delete(directory, recursive: true); }
    }

    private void RunWindow(string red, string green, string scenario)
    {
        MainWindow? window = null;
        WindowsGraphicsCaptureWindowSession? capture = null;
        using var binding = new WindowsVirtualCameraSurfaceBinding();
        using var sampler = new VideoTransitionFrameSampler();
        try
        {
            window = new MainWindow();
            window.Show();
            Field<ShellState>(window, "_state").VideoProcessing = scenario == "manual-gpu83";
            if (scenario.EndsWith("-audio", StringComparison.Ordinal))
                Field<ShellState>(window, "_state").AudioProcessing = false;
            Field<LoginViewModel>(window, "_login").ApplyActivated("transition-fixture");
            Func<Task<MediaImportRequest?>> request = () => Task.FromResult<MediaImportRequest?>(new(MediaImportOperation.ReplaceAll, scenario == "single" ? [red] : [red, green]));
            Await(Invoke(window, "RunImportAsync", request));
            window.GetType().GetField("_interludeConfig", BindingFlags.Instance | BindingFlags.NonPublic)!
                .SetValue(window, InterludeAudioConfig.Default with { Enabled = false });
            Await(Invoke(window, "TogglePlaybackCoreAsync"));
            var pool = Field<MediaPoolService>(window, "_mediaPool");
            Assert.AreEqual(PlaybackState.Playing, pool.Snapshot.PlaybackState);
            var mpv = Field<WindowsMpvPlaybackController>(window, "_mpvController");
            var originalPid = mpv.Snapshot.Runtime.Host.ProcessId;
            Assert.IsNotNull(originalPid);
            if (scenario.EndsWith("-audio", StringComparison.Ordinal)) AssertClocksAligned(window);
            if (scenario == "manual-gpu83")
                Assert.AreEqual(MpvVideoProcessingMode.Gpu83, mpv.Snapshot.ActiveVideoProcessingMode);
            var final = Field<FinalEffectWindow>(window, "_finalEffectWindow");
            Assert.IsTrue(final.TryGetCaptureWindowHandle(out var hwnd));
            Assert.IsTrue(binding.Bind(hwnd).IsSuccess);
            capture = new WindowsGraphicsCaptureWindowSession();
            var start = capture.StartAsync(binding, TimeSpan.FromSeconds(10), frameConsumer: sampler.Accept);
            Await(start);
            Assert.IsTrue(start.Result.IsSuccess, start.Result.Code.ToString());
            Until(() => sampler.Snapshot.Colors.Contains("red"), "initial red frame");
            sampler.Arm();
            if (scenario.StartsWith("automatic", StringComparison.Ordinal))
            {
                Until(() => sampler.Snapshot.Colors.Contains("green"), "automatic next source");
                var count = sampler.Snapshot.Samples;
                Until(() => pool.Snapshot.SourceMediaIndex == 0 && pool.Snapshot.PlaybackPoolCycle > 0
                    && !final.HasHeldVideoFrame, "automatic wrap completed");
                Until(() => sampler.Snapshot.Colors.Skip(count).Contains("red"), "automatic wrap to first source");
                if (scenario.EndsWith("-audio", StringComparison.Ordinal)) AssertClocksAligned(window);
            }
            else if (scenario == "single")
            {
                var loopIndex = pool.Snapshot.LoopIndex;
                Until(() => pool.Snapshot.LoopIndex > loopIndex && !final.HasHeldVideoFrame,
                    "single item automatic loop completed");
                var count = sampler.Snapshot.Samples;
                Until(() => sampler.Snapshot.Colors.Skip(count).Contains("red"), "red picture after single item loop");
                Assert.AreEqual(1, pool.Snapshot.SourceMediaPool.Length);
                Assert.AreEqual(PlaybackState.Playing, pool.Snapshot.PlaybackState);
                Assert.IsFalse(sampler.Snapshot.Colors.Contains("green"));
                TestContext.WriteLine($"loopIndexBefore={loopIndex}; loopIndexAfter={pool.Snapshot.LoopIndex}");
            }
            else if (scenario == "replace")
            {
                Func<Task<MediaImportRequest?>> replacement = () => Task.FromResult<MediaImportRequest?>(new(MediaImportOperation.ReplaceAll, [green]));
                Await(Invoke(window, "RunImportAsync", replacement));
                Assert.AreEqual(PlaybackState.Ready, pool.Snapshot.PlaybackState);
                Assert.AreEqual(1, pool.Snapshot.SourceMediaPool.Length);
                Assert.AreEqual(green, pool.Snapshot.SourceMediaPool[0].SourcePath);
                var holdObservation = Stopwatch.StartNew();
                Until(() => holdObservation.Elapsed >= TimeSpan.FromSeconds(1), "replacement Ready picture retention");
                Assert.IsFalse(sampler.Snapshot.Colors.Contains("green"), "New pool must await the explicit play action.");
                Await(Invoke(window, "TogglePlaybackCoreAsync"));
                Until(() => sampler.Snapshot.Colors.Contains("green"), "replacement pool actual new picture");
                Assert.AreEqual(PlaybackState.Playing, pool.Snapshot.PlaybackState);
            }
            else if (scenario == "missing")
            {
                File.Delete(green);
                Await(Invoke(window, "NavigateMediaCoreAsync", true));
                var until = Stopwatch.StartNew();
                Until(() => until.Elapsed >= TimeSpan.FromSeconds(1), "failure retention observation");
                Assert.IsFalse(sampler.Snapshot.Colors.Contains("green"));
                Assert.AreEqual(PlaybackState.Stopped, pool.Snapshot.PlaybackState);
                Assert.IsTrue(final.HasHeldVideoFrame);
            }
            else
            {
                Await(Invoke(window, "NavigateMediaCoreAsync", true));
                Until(() => sampler.Snapshot.Colors.Contains("green"), "manual next source");
                if (scenario.EndsWith("-audio", StringComparison.Ordinal)) AssertClocksAligned(window);
                var count = sampler.Snapshot.Samples;
                Await(Invoke(window, "NavigateMediaCoreAsync", false));
                Until(() => sampler.Snapshot.Colors.Skip(count).Contains("red"), "manual previous source");
                Await(Invoke(window, "TogglePlaybackCoreAsync"));
                Assert.AreEqual(PlaybackState.Paused, pool.Snapshot.PlaybackState);
                count = sampler.Snapshot.Samples;
                Await(Invoke(window, "NavigateMediaCoreAsync", true));
                Until(() => sampler.Snapshot.Colors.Skip(count).Contains("green"), "paused next source frame");
                Assert.AreEqual(PlaybackState.Paused, pool.Snapshot.PlaybackState);
            }
            var result = sampler.Snapshot;
            if (scenario is not "missing" and not "replace")
                Assert.AreEqual(originalPid, mpv.Snapshot.Runtime.Host.ProcessId, "Switches must reuse one mpv process.");
            Assert.AreEqual(WindowsGraphicsCaptureWindowSessionCode.Running, capture.Snapshot.Code,
                "Continuous capture must remain active throughout the transition.");
            TestContext.WriteLine($"scenario={scenario}; samples={result.Samples}; blackFrames={result.BlackFrames}; maximumCaptureGapMs={result.MaximumGap100Ns / 10_000.0:F2}; colors={string.Join(",", result.Colors.Distinct())}; failure={result.Failure ?? "none"}");
            Assert.IsNull(result.Failure);
            Assert.IsTrue(result.Samples > 0);
            Assert.AreEqual(0, result.BlackFrames, "Transition introduced a black center ROI in a delivered WGC frame.");
        }
        finally
        {
            if (window is not null)
                TestContext.WriteLine($"finalStatus={Field<ShellState>(window, "_state").StatusMessage}; pool={Field<MediaPoolService>(window, "_mediaPool").Snapshot.PlaybackState}; mpv={Field<WindowsMpvPlaybackController>(window, "_mpvController").Snapshot.State}; samples={sampler.Snapshot.Samples}; black={sampler.Snapshot.BlackFrames}; colors={string.Join(",", sampler.Snapshot.Colors.Distinct())}; captureFailure={sampler.Snapshot.Failure}");
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
                }
            }
        }
    }

    private void AssertClocksAligned(MainWindow window)
    {
        var audio = Field<WindowsAudioPlaybackController>(window, "_audioPlaybackController");
        Until(() => audio.Snapshot.AudibleClock?.PlaybackTimeMs > 100, "audible audio after switching");
        var pool = Field<MediaPoolService>(window, "_mediaPool");
        var duration = pool.Snapshot.SourceMediaPool[pool.Snapshot.SourceMediaIndex].DurationMs!.Value;
        var position = (Task<ulong?>)Invoke(window, "ReadCurrentVideoPositionAsync", pool.CurrentIdentity, duration);
        Await(position);
        var audible = audio.Snapshot.AudibleClock?.PlaybackTimeMs;
        Assert.IsNotNull(position.Result);
        Assert.IsNotNull(audible);
        var drift = Math.Abs((double)position.Result.Value - audible.Value);
        TestContext.WriteLine($"audioVideoDriftMs={drift:F0}; audioPosition={audible}; videoPosition={position.Result}");
        Assert.IsTrue(drift < 180, $"Source transition left {drift:F0}ms audio/video drift.");
    }

    private static void Generate(string executable, string output, string color, string size, int seconds, bool withAudio = false)
    {
        var info = new ProcessStartInfo(executable) { UseShellExecute = false, CreateNoWindow = true, RedirectStandardError = true };
        foreach (var argument in new[] { "-hide_banner", "-loglevel", "error", "-nostdin", "-f", "lavfi", "-i", $"color=c={color}:s={size}:r=30" }) info.ArgumentList.Add(argument);
        if (withAudio)
            foreach (var argument in new[] { "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-c:a", "aac" }) info.ArgumentList.Add(argument);
        else info.ArgumentList.Add("-an");
        foreach (var argument in new[] { "-t", seconds.ToString(System.Globalization.CultureInfo.InvariantCulture), "-c:v", "mpeg4", "-y", output }) info.ArgumentList.Add(argument);
        using var process = Process.Start(info) ?? throw new InvalidOperationException("Unable to start fixture FFmpeg.");
        var errors = process.StandardError.ReadToEndAsync();
        if (!process.WaitForExit(15_000)) { process.Kill(entireProcessTree: true); process.WaitForExit(); Assert.Fail("Fixture generation timed out."); }
        Assert.AreEqual(0, process.ExitCode, errors.GetAwaiter().GetResult());
    }
    private static T Field<T>(object instance, string name) => (T)(instance.GetType().GetField(name, BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(instance) ?? throw new MissingFieldException(name));
    private static Task Invoke(object instance, string name, params object[] arguments) => (Task)(instance.GetType().GetMethod(name, BindingFlags.Instance | BindingFlags.NonPublic)?.Invoke(instance, arguments) ?? throw new MissingMethodException(name));
    private static void Await(Task task) { Until(() => task.IsCompleted, "asynchronous operation"); task.GetAwaiter().GetResult(); }
    private static void Until(Func<bool> predicate, string operation)
    {
        if (predicate()) return;
        var deadline = Stopwatch.StartNew();
        var frame = new DispatcherFrame();
        var timer = new DispatcherTimer(TimeSpan.FromMilliseconds(10), DispatcherPriority.Background, (_, _) => { if (predicate() || deadline.Elapsed > TimeSpan.FromSeconds(15)) frame.Continue = false; }, Dispatcher.CurrentDispatcher);
        timer.Start();
        try { Dispatcher.PushFrame(frame); }
        finally { timer.Stop(); }
        Assert.IsTrue(predicate(), "Timed out: " + operation);
    }
}



