using GpAutoLive.Contracts;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class FfmpegPcmDecodePlanTests
{
    [TestMethod]
    public void Missing_ffmpeg_path_is_rejected_without_touching_source()
    {
        var source = CreateSource(MediaKind.Audio, audioCodec: "aac");

        var ok = FfmpegPcmDecodePlanBuilder.TryCreate(
            @"C:\missing\ffmpeg.exe",
            source,
            48_000,
            2,
            out _,
            out var error);

        Assert.IsFalse(ok);
        Assert.AreEqual(FfmpegPcmDecodeFailureCode.FfmpegMissing, error?.Code);
    }

    [TestMethod]
    public void Source_without_audio_track_is_rejected()
    {
        var ffmpeg = Environment.ProcessPath!;
        var sourcePath = Path.Combine(Path.GetTempPath(), $"gpalive-{Guid.NewGuid():N}.mp4");
        File.WriteAllBytes(sourcePath, new byte[] { 1 });
        try
        {
            var source = CreateSource(MediaKind.Video, audioCodec: null, path: sourcePath);
            var ok = FfmpegPcmDecodePlanBuilder.TryCreate(ffmpeg, source, 48_000, 2, out _, out var error);

            Assert.IsFalse(ok);
            Assert.AreEqual(FfmpegPcmDecodeFailureCode.SourceTrackUnavailable, error?.Code);
        }
        finally
        {
            File.Delete(sourcePath);
        }
    }

    [TestMethod]
    public void Valid_plan_maps_only_first_audio_stream_to_bounded_f32le_output()
    {
        var ffmpeg = Environment.ProcessPath!;
        var sourcePath = Path.Combine(Path.GetTempPath(), $"gpalive-{Guid.NewGuid():N}.mp3");
        File.WriteAllBytes(sourcePath, new byte[] { 1 });
        try
        {
            var source = CreateSource(MediaKind.Audio, "aac", sourcePath);
            var ok = FfmpegPcmDecodePlanBuilder.TryCreate(ffmpeg, source, 44_100, 2, out var plan, out var error);

            Assert.IsTrue(ok, error?.Message);
            Assert.IsNotNull(plan);
            CollectionAssert.Contains(plan!.Arguments.ToArray(), "0:a:0");
            CollectionAssert.Contains(plan.Arguments.ToArray(), "f32le");
            CollectionAssert.Contains(plan.Arguments.ToArray(), "pipe:1");
            CollectionAssert.Contains(plan.Arguments.ToArray(), "-re");
            Assert.AreEqual(44_100, plan.SampleRateHz);
        }
        finally
        {
            File.Delete(sourcePath);
        }
    }

    [TestMethod]
    public void Invalid_audio_shape_is_rejected()
    {
        var sourcePath = Path.Combine(Path.GetTempPath(), $"gpalive-{Guid.NewGuid():N}.mp3");
        File.WriteAllBytes(sourcePath, new byte[] { 1 });
        try
        {
            var source = CreateSource(MediaKind.Audio, "aac", sourcePath);
            var ok = FfmpegPcmDecodePlanBuilder.TryCreate(Environment.ProcessPath, source, 96_000, 2, out _, out var error);

            Assert.IsFalse(ok);
            Assert.AreEqual(FfmpegPcmDecodeFailureCode.InvalidSampleRate, error?.Code);
        }
        finally
        {
            File.Delete(sourcePath);
        }
    }

    [TestMethod]
    public void Audio_effects_are_mapped_to_a_bounded_ffmpeg_filter_chain()
    {
        var ffmpeg = Environment.ProcessPath!;
        var sourcePath = Path.Combine(Path.GetTempPath(), $"gpalive-{Guid.NewGuid():N}.mp3");
        File.WriteAllBytes(sourcePath, new byte[] { 1 });
        try
        {
            var source = CreateSource(MediaKind.Audio, "aac", sourcePath);
            var effects = new AudioEffectParams
            {
                LoudnessAdjustmentDb = 2,
                LowEqDb = -3,
                MidEqDb = 2,
                HighEqDb = 4,
            };
            var ok = FfmpegPcmDecodePlanBuilder.TryCreate(
                ffmpeg,
                source,
                48_000,
                2,
                out var plan,
                out var error,
                effects);

            Assert.IsTrue(ok, error?.Message);
            Assert.IsNotNull(plan);
            var filterIndex = plan!.Arguments.IndexOf("-af");
            Assert.IsTrue(filterIndex >= 0 && filterIndex + 1 < plan.Arguments.Length);
            StringAssert.Contains(plan.Arguments[filterIndex + 1], "volume=2dB");
            StringAssert.Contains(plan.Arguments[filterIndex + 1], "equalizer=f=200");
            StringAssert.Contains(plan.Arguments[filterIndex + 1], "equalizer=f=1000");
            StringAssert.Contains(plan.Arguments[filterIndex + 1], "equalizer=f=8000");
        }
        finally
        {
            File.Delete(sourcePath);
        }
    }

    [TestMethod]
    public void Interlude_single_track_preset_is_consumed_by_the_decode_plan()
    {
        var ffmpeg = Environment.ProcessPath!;
        var sourcePath = Path.Combine(Path.GetTempPath(), $"gpalive-{Guid.NewGuid():N}.mp3");
        File.WriteAllBytes(sourcePath, new byte[] { 1 });
        try
        {
            var source = CreateSource(MediaKind.Audio, "aac", sourcePath);
            var effects = new AudioEffectParams { VoiceLibraryId = "p22" };
            var ok = FfmpegPcmDecodePlanBuilder.TryCreate(
                ffmpeg,
                source,
                48_000,
                2,
                out var plan,
                out var error,
                effects);

            Assert.IsTrue(ok, error?.Message);
            Assert.IsNotNull(plan);
            var filterIndex = plan!.Arguments.IndexOf("-af");
            Assert.IsTrue(filterIndex >= 0 && filterIndex + 1 < plan.Arguments.Length);
            StringAssert.Contains(plan.Arguments[filterIndex + 1], "equalizer=f=");
        }
        finally
        {
            File.Delete(sourcePath);
        }
    }

    [TestMethod]
    public void Audio_effects_with_an_unconsumed_formal_value_fail_closed()
    {
        var ffmpeg = Environment.ProcessPath!;
        var sourcePath = Path.Combine(Path.GetTempPath(), $"gpalive-{Guid.NewGuid():N}.mp3");
        File.WriteAllBytes(sourcePath, new byte[] { 1 });
        try
        {
            var source = CreateSource(MediaKind.Audio, "aac", sourcePath);
            var effects = AudioEffectParams.Default with { EnvironmentNoisePercent = 1 };

            var ok = FfmpegPcmDecodePlanBuilder.TryCreate(
                ffmpeg,
                source,
                48_000,
                2,
                out var plan,
                out var error,
                effects);

            Assert.IsFalse(ok, "未映射的正式字段不能在没有 -af 消费者时伪装成成功。");
            Assert.IsNull(plan);
            Assert.AreEqual(FfmpegPcmDecodeFailureCode.InvalidAudioEffects, error?.Code);
        }
        finally
        {
            File.Delete(sourcePath);
        }
    }

    [TestMethod]
    public void Fade_out_without_a_reliable_duration_fails_closed()
    {
        var ffmpeg = Environment.ProcessPath!;
        var sourcePath = Path.Combine(Path.GetTempPath(), $"gpalive-{Guid.NewGuid():N}.mp3");
        File.WriteAllBytes(sourcePath, new byte[] { 1 });
        try
        {
            var source = CreateSource(MediaKind.Audio, "aac", sourcePath) with { DurationMs = null };
            var effects = AudioEffectParams.Default with { FadeOutMs = 100 };

            var ok = FfmpegPcmDecodePlanBuilder.TryCreate(
                ffmpeg,
                source,
                48_000,
                2,
                out var plan,
                out var error,
                effects);

            Assert.IsFalse(ok, "无法计算淡出位置时不能静默省略淡出参数。");
            Assert.IsNull(plan);
            Assert.AreEqual(FfmpegPcmDecodeFailureCode.InvalidAudioEffects, error?.Code);
        }
        finally
        {
            File.Delete(sourcePath);
        }
    }

    [TestMethod]
    public void Start_position_is_encoded_as_a_bounded_input_seek()
    {
        var ffmpeg = Environment.ProcessPath!;
        var sourcePath = Path.Combine(Path.GetTempPath(), $"gpalive-{Guid.NewGuid():N}.mp3");
        File.WriteAllBytes(sourcePath, new byte[] { 1 });
        try
        {
            var source = CreateSource(MediaKind.Video, "aac", sourcePath) with { DurationMs = 60_000 };
            var ok = FfmpegPcmDecodePlanBuilder.TryCreate(
                ffmpeg,
                source,
                48_000,
                2,
                out var plan,
                out var error,
                sourceStartMs: 12_345);

            Assert.IsTrue(ok, error?.Message);
            Assert.IsNotNull(plan);
            Assert.AreEqual(12_345UL, plan!.SourceStartMs);
            var seekIndex = plan.Arguments.IndexOf("-ss");
            var inputIndex = plan.Arguments.IndexOf("-i");
            Assert.IsTrue(seekIndex >= 0 && inputIndex == seekIndex + 2);
            Assert.AreEqual("12.345", plan.Arguments[seekIndex + 1]);
        }
        finally
        {
            File.Delete(sourcePath);
        }
    }

    private static SourceMediaDto CreateSource(MediaKind kind, string? audioCodec, string? path = null) =>
        new(
            path ?? Path.Combine(Path.GetTempPath(), "not-checked.mp3"),
            "media://test",
            kind,
            MediaCompatibilityMode.Direct,
            "audio.mp3",
            1,
            1_000,
            0,
            1_000,
            kind == MediaKind.Video ? 1280u : null,
            kind == MediaKind.Video ? 720u : null,
            kind == MediaKind.Video ? 30d : null,
            48_000,
            2,
            kind == MediaKind.Video ? "h264" : null,
            audioCodec,
            null,
            "not-computed");
}
