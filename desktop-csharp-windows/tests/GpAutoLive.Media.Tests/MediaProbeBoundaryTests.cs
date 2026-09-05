using GpAutoLive.Contracts;
using GpAutoLive.Core.Processes;
using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class MediaProbeBoundaryTests
{
    [TestMethod]
    public void CatalogMatchesTheExistingImportAllowListCaseInsensitively()
    {
        Assert.IsTrue(MediaFormatCatalog.TryGetKind("sample.MP4", out var videoKind));
        Assert.AreEqual(MediaKind.Video, videoKind);
        Assert.IsTrue(MediaFormatCatalog.TryGetKind("sample.FLAC", out var audioKind));
        Assert.AreEqual(MediaKind.Audio, audioKind);
        Assert.IsFalse(MediaFormatCatalog.IsSupportedExtension("sample.exe"));
    }

    [TestMethod]
    public void PathPolicyRejectsRelativeAndUnsupportedPathsWithoutEchoingInput()
    {
        var relativeError = MediaPathPolicy.Validate("sample.mp4", requireExistingFile: false, out _);
        Assert.IsNotNull(relativeError);
        Assert.AreEqual(MediaProbeFailureCode.PathNotFullyQualified, relativeError.Code);
        Assert.IsFalse(relativeError.Message.Contains("sample.mp4", StringComparison.Ordinal));

        var unsupportedPath = Path.Combine(Path.GetTempPath(), "not-allowed.exe");
        var unsupportedError = MediaPathPolicy.Validate(unsupportedPath, requireExistingFile: false, out _);
        Assert.IsNotNull(unsupportedError);
        Assert.AreEqual(MediaProbeFailureCode.UnsupportedExtension, unsupportedError.Code);
        Assert.IsFalse(unsupportedError.Message.Contains(unsupportedPath, StringComparison.Ordinal));
    }

    [TestMethod]
    public void PathPolicyClassifiesAnExistingDirectorySeparately()
    {
        var root = Path.Combine(Path.GetTempPath(), "gpautolive-media-directory", Guid.NewGuid().ToString("N"));
        var directory = Path.Combine(root, "folder.mp4");
        Directory.CreateDirectory(directory);
        try
        {
            var error = MediaPathPolicy.Validate(directory, requireExistingFile: true, out _);
            Assert.IsNotNull(error);
            Assert.AreEqual(MediaProbeFailureCode.PathIsDirectory, error.Code);
        }
        finally
        {
            Directory.Delete(root, recursive: true);
        }
    }

    [TestMethod]
    public void CommandBuilderUsesFixedArgumentsAndHiddenNoShellPolicy()
    {
        var executable = Path.Combine(Path.GetTempPath(), "ffprobe.exe");
        var media = Path.Combine(Path.GetTempPath(), "sample.mp4");
        var plan = FfprobeCommandBuilder.Create(executable, media);

        CollectionAssert.AreEqual(
            new[] { "-v", "error", "-hide_banner", "-print_format", "json=compact=1", "-show_format", "-show_streams", "-show_chapters", Path.GetFullPath(media) },
            plan.Arguments.ToArray());
        Assert.IsFalse(plan.LaunchPolicy.UseShellExecute);
        Assert.IsTrue(plan.LaunchPolicy.CreateNoWindow);
        Assert.IsTrue(plan.LaunchPolicy.RedirectStandardOutput);
        Assert.IsTrue(plan.LaunchPolicy.RedirectStandardError);
        Assert.IsTrue(plan.LaunchPolicy.KillProcessTreeOnTimeout);
        Assert.IsTrue(plan.LaunchPolicy.KillProcessTreeOnCancellation);
    }

    [TestMethod]
    public async Task ProbeUsesInjectedRunnerAndReturnsBoundedSuccess()
    {
        using var fixture = MediaFixture.Create("sample.mp4");
        var runner = new CapturingRunner(new ExternalProcessResult(
            ExternalProcessRunStatus.Completed,
            ExitCode: 0,
            StandardOutput: "{\"format\":{},\"streams\":[]}",
            StandardError: string.Empty));
        var probe = new FfprobeMediaProbe(fixture.FfprobePath, runner);

        var result = await probe.ProbeAsync(fixture.MediaPath);

        Assert.IsTrue(result.IsSuccess);
        Assert.IsNotNull(result.MediaPath);
        Assert.AreEqual(Path.GetFullPath(fixture.MediaPath), result.MediaPath.Value.CanonicalPath);
        Assert.AreEqual(MediaKind.Video, result.MediaPath.Value.Kind);
        Assert.AreEqual(1, runner.CallCount);
        Assert.AreEqual(fixture.FfprobePath, runner.LastPlan?.ExecutablePath);
    }

    [TestMethod]
    public async Task ProbeMapsTimeoutAndOutputLimitToStableFailures()
    {
        using var fixture = MediaFixture.Create("sample.mp4");
        var timeoutProbe = new FfprobeMediaProbe(
            fixture.FfprobePath,
            new CapturingRunner(new ExternalProcessResult(
                ExternalProcessRunStatus.TimedOut, null, string.Empty, string.Empty)));
        var timeout = await timeoutProbe.ProbeAsync(fixture.MediaPath);
        Assert.IsFalse(timeout.IsSuccess);
        Assert.AreEqual(MediaProbeFailureCode.ProcessTimedOut, timeout.Error?.Code);

        var outputProbe = new FfprobeMediaProbe(
            fixture.FfprobePath,
            new CapturingRunner(new ExternalProcessResult(
                ExternalProcessRunStatus.StandardOutputLimitExceeded, null, "ignored", string.Empty)));
        var output = await outputProbe.ProbeAsync(fixture.MediaPath);
        Assert.IsFalse(output.IsSuccess);
        Assert.AreEqual(MediaProbeFailureCode.StandardOutputLimitExceeded, output.Error?.Code);
    }

    [TestMethod]
    public async Task ProbeMapsAnExplicitCancellationToAStableFailure()
    {
        using var fixture = MediaFixture.Create("sample.mp4");
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();
        var probe = new FfprobeMediaProbe(fixture.FfprobePath, new CancellingRunner());

        var result = await probe.ProbeAsync(fixture.MediaPath, cancellation.Token);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MediaProbeFailureCode.OperationCancelled, result.Error?.Code);
    }

    [TestMethod]
    public void SourceParserBuildsVideoDtoFromBoundedProbeJson()
    {
        var path = new ValidatedMediaPath(@"C:\media\sample.mp4", MediaKind.Video);
        const string json = """
            {
              "format": { "duration": "12.345" },
              "streams": [
                { "codec_type": "video", "codec_name": "h264", "width": 1280, "height": 720, "avg_frame_rate": "30000/1001" },
                { "codec_type": "audio", "codec_name": "aac", "sample_rate": "48000", "channels": 2 }
              ]
            }
            """;

        var parsed = FfprobeSourceParser.TryParse(path, 123, json, out var source, out var error);

        Assert.IsTrue(parsed);
        Assert.IsNull(error);
        Assert.IsNotNull(source);
        Assert.AreEqual(MediaKind.Video, source.MediaKind);
        Assert.AreEqual(12_345UL, source.DurationMs);
        Assert.AreEqual(1280U, source.Width);
        Assert.AreEqual(720U, source.Height);
        Assert.AreEqual(48_000U, source.AudioSampleRateHz);
        Assert.AreEqual((ushort)2, source.AudioChannelCount);
        Assert.AreEqual("h264", source.VideoCodecName);
    }

    [TestMethod]
    public void SourceParserFallsBackToRFrameRateWhenAverageFrameRateIsInvalid()
    {
        var path = new ValidatedMediaPath(@"C:\media\variable-rate.mp4", MediaKind.Video);
        const string json = """
            {
              "format": { "duration": "1.0" },
              "streams": [
                {
                  "codec_type": "video",
                  "codec_name": "h264",
                  "width": 1280,
                  "height": 720,
                  "avg_frame_rate": "0/0",
                  "r_frame_rate": "25/1"
                }
              ]
            }
            """;

        var parsed = FfprobeSourceParser.TryParse(path, 123, json, out var source, out var error);

        Assert.IsTrue(parsed);
        Assert.IsNull(error);
        Assert.IsNotNull(source?.FrameRateFps);
        Assert.AreEqual(25.0, source!.FrameRateFps!.Value, 0.0001);
    }

    [TestMethod]
    public void SourceParserIgnoresAttachedPictureWhenClassifyingAudio()
    {
        var path = new ValidatedMediaPath(@"C:\media\album.m4a", MediaKind.Audio);
        const string json = """
            {
              "format": { "duration": "1.0" },
              "streams": [
                {
                  "codec_type": "video",
                  "codec_name": "mjpeg",
                  "width": 600,
                  "height": 600,
                  "avg_frame_rate": "0/0",
                  "disposition": { "attached_pic": 1 }
                },
                { "codec_type": "audio", "codec_name": "aac", "sample_rate": "48000", "channels": 2 }
              ]
            }
            """;

        var parsed = FfprobeSourceParser.TryParse(path, 123, json, out var source, out var error);

        Assert.IsTrue(parsed);
        Assert.IsNull(error);
        Assert.AreEqual(MediaKind.Audio, source?.MediaKind);
        Assert.IsNull(source?.Width);
        Assert.AreEqual("aac", source?.AudioCodecName);
    }

    [TestMethod]
    public void SourceParserRejectsVideoStreamInPureAudioInput()
    {
        var path = new ValidatedMediaPath(@"C:\media\sample.flac", MediaKind.Audio);
        const string json = """
            {
              "format": { "duration": 1.0 },
              "streams": [
                { "codec_type": "video", "codec_name": "mjpeg", "width": 320, "height": 240, "avg_frame_rate": "1/1" },
                { "codec_type": "audio", "codec_name": "flac", "sample_rate": 44100, "channels": 2 }
              ]
            }
            """;

        var parsed = FfprobeSourceParser.TryParse(path, 1, json, out _, out var error);

        Assert.IsFalse(parsed);
        Assert.AreEqual(MediaProbeFailureCode.UnsupportedMediaStream, error?.Code);
    }

    private sealed class CapturingRunner(ExternalProcessResult result) : IExternalProcessRunner
    {
        public int CallCount { get; private set; }

        public ExternalProcessPlan? LastPlan { get; private set; }

        public Task<ExternalProcessResult> RunAsync(ExternalProcessPlan plan, CancellationToken cancellationToken)
        {
            CallCount++;
            LastPlan = plan;
            return Task.FromResult(result);
        }
    }

    private sealed class CancellingRunner : IExternalProcessRunner
    {
        public Task<ExternalProcessResult> RunAsync(ExternalProcessPlan plan, CancellationToken cancellationToken) =>
            throw new OperationCanceledException(cancellationToken);
    }

    private sealed class MediaFixture : IDisposable
    {
        private MediaFixture(string root, string mediaPath, string ffprobePath)
        {
            Root = root;
            MediaPath = mediaPath;
            FfprobePath = ffprobePath;
        }

        public string Root { get; }
        public string MediaPath { get; }
        public string FfprobePath { get; }

        public static MediaFixture Create(string fileName)
        {
            var root = Path.Combine(Path.GetTempPath(), "gpautolive-media-tests", Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(root);
            var mediaPath = Path.Combine(root, fileName);
            var ffprobePath = Path.Combine(root, "ffprobe.exe");
            File.WriteAllBytes(mediaPath, [0]);
            return new MediaFixture(root, mediaPath, ffprobePath);
        }

        public void Dispose()
        {
            if (Directory.Exists(Root))
            {
                Directory.Delete(Root, recursive: true);
            }
        }
    }
}
