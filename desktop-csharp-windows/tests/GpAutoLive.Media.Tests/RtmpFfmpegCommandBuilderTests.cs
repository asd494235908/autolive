using GpAutoLive.Contracts;
using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class RtmpFfmpegCommandBuilderTests
{
    [TestMethod]
    public void Cpu4_video_effects_are_translated_to_a_restricted_ffmpeg_filter()
    {
        using var fixture = MediaFixture.Create("sample.mp4");
        Assert.IsTrue(MpvVideoEffectSnapshot.TryCreate(
            MpvVideoProcessingMode.Cpu4,
            brightnessPercent: 12,
            contrastPercent: 88,
            saturationPercent: 110,
            hueRotationDegrees: 7,
            MpvShaderOptionsSnapshot.Empty,
            out var effects,
            out var effectError), effectError?.Message);

        var created = RtmpFfmpegCommandBuilder.TryCreate(
            RtmpOutputConfig.Default with { TargetUrl = "rtmp://127.0.0.1/live/stream" },
            fixture.Source,
            @"C:\media\ffmpeg.exe",
            "h264_amf",
            sourceIdentity: null,
            out var plan,
            out var error,
            videoEffects: effects);

        Assert.IsTrue(created, error?.Message);
        Assert.IsNotNull(plan);
        var vfIndex = plan!.ProcessPlan.Arguments.IndexOf("-vf");
        Assert.IsTrue(vfIndex >= 0);
        Assert.AreEqual(
            "eq=brightness=0.12:contrast=0.88:saturation=1.1,hue=h=7:s=1",
            plan.ProcessPlan.Arguments[vfIndex + 1]);
        Assert.IsFalse(plan.ProcessPlan.Arguments.Any(argument => argument.Contains("@autolive_cpu4", StringComparison.Ordinal)));
    }

    [TestMethod]
    public void Original_video_effects_do_not_add_an_ffmpeg_filter()
    {
        using var fixture = MediaFixture.Create("sample.mp4");
        var created = RtmpFfmpegCommandBuilder.TryCreate(
            RtmpOutputConfig.Default with { TargetUrl = "rtmp://127.0.0.1/live/stream" },
            fixture.Source,
            @"C:\media\ffmpeg.exe",
            preferredEncoder: null,
            sourceIdentity: null,
            out var plan,
            out var error,
            videoEffects: MpvVideoEffectSnapshot.Default);

        Assert.IsTrue(created, error?.Message);
        Assert.IsNotNull(plan);
        Assert.IsFalse(plan!.ProcessPlan.Arguments.Contains("-vf"));
    }

    [TestMethod]
    public void Rtmp_plan_enables_bounded_ffmpeg_progress_on_stderr()
    {
        using var fixture = MediaFixture.Create("sample.mp4");

        var created = RtmpFfmpegCommandBuilder.TryCreate(
            RtmpOutputConfig.Default with { TargetUrl = "rtmp://127.0.0.1/live/stream" },
            fixture.Source,
            @"C:\media\ffmpeg.exe",
            preferredEncoder: "h264_amf",
            sourceIdentity: null,
            out var plan,
            out var error);

        Assert.IsTrue(created, error?.Message);
        Assert.IsNotNull(plan);
        var progressIndex = plan!.ProcessPlan.Arguments.IndexOf("-progress");
        Assert.IsTrue(progressIndex >= 0);
        Assert.AreEqual("pipe:2", plan.ProcessPlan.Arguments[progressIndex + 1]);
    }

    [TestMethod]
    public void Video_and_audio_plan_uses_direct_source_and_final_pcm_pipe()
    {
        using var fixture = MediaFixture.Create("sample.mp4");
        var config = RtmpOutputConfig.Default with
        {
            TargetUrl = "rtmp://127.0.0.1:1935/live/stream?token=secret"
        };

        var created = RtmpFfmpegCommandBuilder.TryCreate(
            config,
            fixture.Source,
            @"C:\media\ffmpeg.exe",
            "h264_amf",
            new(2, 3, 1, 4, 1_250, 10_000),
            out var plan,
            out var error);

        Assert.IsTrue(created, error?.Message);
        Assert.IsNull(error);
        Assert.IsNotNull(plan);
        Assert.IsTrue(plan!.RequiresFinalPcmInput);
        CollectionAssert.Contains(plan.ProcessPlan.Arguments, "pipe:0");
        Assert.IsFalse(plan.ProcessPlan.Arguments.Contains("-nostdin"));
        CollectionAssert.Contains(plan.ProcessPlan.Arguments, "-c:v");
        CollectionAssert.Contains(plan.ProcessPlan.Arguments, "h264_amf");
        CollectionAssert.Contains(plan.ProcessPlan.Arguments, fixture.MediaPath);
        CollectionAssert.Contains(plan.ProcessPlan.Arguments, "-map");
        Assert.AreEqual(
            "rtmp://127.0.0.1:1935/<redacted>",
            plan.RedactedTargetUrl);
    }

    [TestMethod]
    public void Gpu83_video_effects_are_rejected_before_building_any_filter()
    {
        using var fixture = MediaFixture.Create("sample.mp4");
        Assert.IsTrue(MpvVideoEffectSnapshot.TryCreateGpu83BaselineColor(
            brightnessPercent: 1,
            contrastPercent: 100,
            saturationPercent: 100,
            hueRotationDegrees: 0,
            out var effects,
            out var effectError), effectError?.Message);

        var created = RtmpFfmpegCommandBuilder.TryCreate(
            RtmpOutputConfig.Default with { TargetUrl = "rtmp://127.0.0.1/live/stream" },
            fixture.Source,
            @"C:\media\ffmpeg.exe",
            preferredEncoder: "h264_amf",
            sourceIdentity: null,
            out var plan,
            out var error,
            videoEffects: effects);

        Assert.IsFalse(created);
        Assert.IsNull(plan);
        Assert.AreEqual(RtmpCommandFailureCode.InvalidVideoEffects, error?.Code);
    }

    [TestMethod]
    public void Audio_only_plan_does_not_add_video_encoder_or_source_loop()
    {
        using var fixture = MediaFixture.Create("sample.mp3");
        var config = RtmpOutputConfig.Default with
        {
            TargetUrl = "rtmps://media.example.com/live/audio",
            VideoEnabled = false,
            AudioEnabled = true,
            Width = 0,
            Height = 0,
            Fps = 0,
            VideoBitrateKbps = 0
        };

        var created = RtmpFfmpegCommandBuilder.TryCreate(
            config,
            fixture.Source,
            @"C:\media\ffmpeg.exe",
            preferredEncoder: null,
            sourceIdentity: null,
            out var plan,
            out var error);

        Assert.IsTrue(created, error?.Message);
        Assert.IsNotNull(plan);
        Assert.IsFalse(plan!.ProcessPlan.Arguments.Contains("-c:v"));
        Assert.IsFalse(plan.ProcessPlan.Arguments.Contains("-stream_loop"));
        CollectionAssert.Contains(plan.ProcessPlan.Arguments, "-map");
        CollectionAssert.Contains(plan.ProcessPlan.Arguments, "0:a:0");
        Assert.AreEqual(string.Empty, plan.Encoder);
    }

    [TestMethod]
    public void Rejects_wrong_source_kind_and_unknown_encoder()
    {
        using var fixture = MediaFixture.Create("sample.mp3");
        var config = RtmpOutputConfig.Default with
        {
            TargetUrl = "rtmp://127.0.0.1/live/stream"
        };

        var created = RtmpFfmpegCommandBuilder.TryCreate(
            config,
            fixture.Source,
            @"C:\media\ffmpeg.exe",
            "h264_amf",
            null,
            out var plan,
            out var error);

        Assert.IsFalse(created);
        Assert.IsNull(plan);
        Assert.AreEqual(RtmpCommandFailureCode.SourceTrackUnavailable, error?.Code);

        using var videoFixture = MediaFixture.Create("sample.mp4");
        created = RtmpFfmpegCommandBuilder.TryCreate(
            config,
            videoFixture.Source,
            @"C:\media\ffmpeg.exe",
            "h264_unknown",
            null,
            out plan,
            out error);

        Assert.IsFalse(created);
        Assert.IsNull(plan);
        Assert.AreEqual(RtmpCommandFailureCode.InvalidEncoder, error?.Code);
    }

    private static SourceMediaDto CreateSource(string path) => new(
        path,
        path,
        MediaPoolRules.TryGetMediaKind(path, out var kind) ? kind : MediaKind.Video,
        MediaCompatibilityMode.Direct,
        Path.GetFileName(path),
        1,
        10_000,
        null,
        null,
        1_280,
        720,
        30,
        48_000,
        2,
        "h264",
        "aac",
        null,
        "disabled");

    private sealed class MediaFixture : IDisposable
    {
        private MediaFixture(string directory, string mediaPath)
        {
            DirectoryPath = directory;
            MediaPath = mediaPath;
            Source = CreateSource(mediaPath);
        }

        public string DirectoryPath { get; }
        public string MediaPath { get; }
        public SourceMediaDto Source { get; }

        public static MediaFixture Create(string fileName)
        {
            var directory = Path.Combine(
                Path.GetTempPath(),
                "gpautolive-rtmp-command-tests",
                Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(directory);
            var mediaPath = Path.Combine(directory, fileName);
            File.WriteAllBytes(mediaPath, [0x00, 0x01]);
            return new(directory, mediaPath);
        }

        public void Dispose()
        {
            if (Directory.Exists(DirectoryPath))
            {
                Directory.Delete(DirectoryPath, recursive: true);
            }
        }
    }
}
