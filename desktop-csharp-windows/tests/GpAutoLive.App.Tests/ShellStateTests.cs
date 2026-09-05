using GpAutoLive.App;
using GpAutoLive.App.Features.Effects;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class ShellStateTests
{
    [TestMethod]
    public void Empty_pool_does_not_enter_playing_state()
    {
        var state = new ShellState();

        state.TogglePlayback();

        Assert.IsFalse(state.IsPlaying);
        StringAssert.Contains(state.StatusMessage, "媒体池为空");
    }

    [TestMethod]
    public void Stop_resets_progress_and_playback()
    {
        var state = new ShellState();
        state.PlaybackProgress = 0.75;

        state.StopPlayback();

        Assert.AreEqual(0d, state.PlaybackProgress);
        Assert.IsFalse(state.IsPlaying);
    }

    [TestMethod]
    public void Video_effect_editor_uses_the_existing_mpv_ranges_and_defaults()
    {
        var state = new ShellState();

        Assert.AreEqual(0d, state.VideoEffects.BrightnessPercent);
        Assert.AreEqual(100d, state.VideoEffects.ContrastPercent);
        Assert.AreEqual(100d, state.VideoEffects.SaturationPercent);
        Assert.AreEqual(0d, state.VideoEffects.HueRotationDegrees);

        state.VideoEffects.BrightnessPercent = -1000;
        state.VideoEffects.ContrastPercent = 1000;
        state.VideoEffects.SaturationPercent = 1000;
        state.VideoEffects.HueRotationDegrees = -1000;

        Assert.AreEqual(-100d, state.VideoEffects.BrightnessPercent);
        Assert.AreEqual(200d, state.VideoEffects.ContrastPercent);
        Assert.AreEqual(200d, state.VideoEffects.SaturationPercent);
        Assert.AreEqual(-180d, state.VideoEffects.HueRotationDegrees);
    }

    [TestMethod]
    public void Video_effect_editor_can_create_a_valid_cpu4_snapshot()
    {
        var state = new ShellState();
        state.VideoEffects.BrightnessPercent = -25;
        state.VideoEffects.ContrastPercent = 125;
        state.VideoEffects.SaturationPercent = 80;
        state.VideoEffects.HueRotationDegrees = 12;

        var created = state.VideoEffects.TryCreateSnapshot(
            GpAutoLive.Media.MpvVideoProcessingMode.Cpu4,
            out var snapshot,
            out var error);

        Assert.IsTrue(created);
        Assert.IsNull(error);
        Assert.IsNotNull(snapshot);
        Assert.AreEqual(-25d, snapshot!.BrightnessPercent);
        Assert.AreEqual(125d, snapshot.ContrastPercent);
        Assert.AreEqual(80d, snapshot.SaturationPercent);
        Assert.AreEqual(12d, snapshot.HueRotationDegrees);
    }

    [TestMethod]
    public void Generated_effect_snapshots_are_bounded_and_separate_from_editor_draft()
    {
        var random = new Random(17);
        var video = GeneratedVideoEffectSnapshot.Create(random);
        var audio = GeneratedAudioEffectSnapshot.Create(random);

        Assert.AreEqual(1, video.Generation);
        Assert.IsTrue(video.BrightnessPercent is >= -6 and <= 6);
        Assert.IsTrue(video.ContrastPercent is >= 96 and <= 104);
        Assert.IsTrue(video.SaturationPercent is >= 96 and <= 106);
        Assert.IsTrue(video.HueRotationDegrees is >= -4 and <= 4);
        Assert.IsTrue(video.SharpnessPercent is >= 0 and <= 6);
        Assert.IsTrue(video.Gamma is >= 0.96 and <= 1.04);
        Assert.IsTrue(video.ExposurePercent is >= -2 and <= 3);
        StringAssert.StartsWith(audio.PresetId, "p");
        Assert.IsTrue(int.Parse(audio.PresetId[1..]) is >= 1 and <= 20);
        Assert.AreEqual("随机预设", audio.ProcessingMode);
        Assert.IsTrue(audio.GainDb is >= -1 and <= 2);
        Assert.IsTrue(audio.DynamicRangeDb is >= 6 and <= 12);
        Assert.IsTrue(audio.LowEqDb is >= -3 and <= 3);
        Assert.IsTrue(audio.MidEqDb is >= -3 and <= 3);
        Assert.IsTrue(audio.HighEqDb is >= -3 and <= 3);
        Assert.IsTrue(audio.PitchShiftSemitones is >= -0.5 and <= 0.5);
        Assert.IsTrue(audio.SpectralPerturbationPercent is >= 0.5 and <= 2.0);
        Assert.IsTrue(audio.SpectrumBlindSpotPercent is >= 0.5 and <= 2.0);
        Assert.IsTrue(audio.HighFrequencyPerturbationEnabled);
        Assert.IsTrue(audio.HighFrequencyPerturbationIntervalMs is >= 8_000 and <= 12_000);
        Assert.IsTrue(audio.HighFrequencyPerturbationStrengthPercent is >= 1 and <= 4);
        Assert.AreEqual(-32d, audio.HighFrequencyPerturbationLevelDb);
    }

    [TestMethod]
    public void Generated_video_snapshot_maps_supported_values_to_formal_contract()
    {
        var generated = new GeneratedVideoEffectSnapshot(
            Generation: 7,
            BrightnessPercent: -5,
            ContrastPercent: 108,
            SaturationPercent: 94,
            HueRotationDegrees: -3,
            SharpnessPercent: 8,
            Gamma: 1.02,
            ExposurePercent: 2,
            NoiseReduction: "关闭");

        var video = generated.ToVideoEffectParams();

        Assert.AreEqual(-5d, video.BrightnessPercent);
        Assert.AreEqual(108d, video.ContrastPercent);
        Assert.AreEqual(94d, video.SaturationPercent);
        Assert.AreEqual(-3d, video.HueRotationDegrees);
        Assert.AreEqual(8d, video.SharpenPercent);
        Assert.IsTrue(video.TryValidate(out var videoErrors), string.Join("; ", videoErrors.Select(error => error.Message)));
    }

    [TestMethod]
    public void Generated_video_snapshot_projects_automatic_geometry_and_visual_values_into_gpu83()
    {
        var generated = GeneratedVideoEffectSnapshot.Create(new Random(17));

        var video = generated.ToVideoEffectParams();
        var advanced = generated.ToAdvancedEffectParams();

        Assert.AreNotEqual(0d, video.BlurRadiusPx);
        Assert.AreNotEqual(100d, video.PixelScalePercent);
        Assert.AreNotEqual(0d, video.RotationDegrees);
        Assert.AreNotEqual(1d, advanced.BandWeights[1_110]);
        Assert.IsNotNull(advanced.TargetFrequencyHz);
        Assert.IsTrue(advanced.RandomGraphicEnabled);
        Assert.AreEqual(
            VideoEffectParams.Default.ColorSpaceConversionStrengthPercent,
            video.ColorSpaceConversionStrengthPercent);
        Assert.AreEqual(
            AdvancedEffectParams.Default.SliceMinLengthMs,
            advanced.SliceMinLengthMs);
        Assert.AreEqual(
            AdvancedEffectParams.Default.PictureInPictureTimelineLocked,
            advanced.PictureInPictureTimelineLocked);
        Assert.IsTrue(
            GpAutoLive.Media.MpvGpu83ShaderSnapshot.TryCreate(
                video,
                advanced,
                sourceFps: 30,
                epochStartSeconds: 0,
                randomSeed: 17,
                out var gpu,
                out var error),
            error?.Message);
        Assert.IsNotNull(gpu);
        Assert.AreNotEqual("0", gpu!.ShaderOptions.Values["al_blur_radius_px"]);
        Assert.AreNotEqual("100", gpu.ShaderOptions.Values["al_pixel_scale_percent"]);
        Assert.AreNotEqual("0", gpu.ShaderOptions.Values["al_rotation_degrees"]);
        Assert.AreNotEqual("1", gpu.ShaderOptions.Values["al_band_1110"]);
        Assert.IsTrue(gpu.ShaderOptions.Values.ContainsKey("al_random_graphic_enabled"));
        CollectionAssert.Contains(gpu.UnavailableFields.ToArray(), "video.color_space_conversion_enabled");
        CollectionAssert.Contains(gpu.UnavailableFields.ToArray(), "advanced.slice_min_length_ms");
        CollectionAssert.Contains(gpu.UnavailableFields.ToArray(), "advanced.picture_in_picture_timeline_locked");
    }

    [TestMethod]
    public void Generated_audio_snapshot_maps_formal_frequency_fields_into_the_runtime_filter_chain()
    {
        var generated = new GeneratedAudioEffectSnapshot(
            Generation: 7,
            PresetId: "p07",
            ProcessingMode: "随机预设",
            GainDb: 1,
            DynamicRangeDb: 8,
            LowEqDb: -1,
            MidEqDb: 2,
            HighEqDb: 3,
            PitchShiftSemitones: 0.25,
            Reverb: "关闭",
            Compression: "轻度",
            NoiseReduction: "低",
            Tone: "明亮",
            SpectralPerturbationPercent: 1,
            SpectrumBlindSpotPercent: 2,
            HighFrequencyPerturbationEnabled: true,
            HighFrequencyPerturbationIntervalMs: 12_000,
            HighFrequencyPerturbationStrengthPercent: 4,
            HighFrequencyPerturbationLevelDb: -32);

        var audio = generated.ToAudioEffectParams();
        var filter = GpAutoLive.Media.FfmpegAudioFilterBuilder.Create(audio);

        Assert.AreEqual(1d, audio.SpectralPerturbationPercent);
        Assert.AreEqual(2d, audio.SpectrumBlindSpotPercent);
        Assert.IsTrue(audio.HighFrequencyPerturbationEnabled);
        Assert.AreEqual(12_000ul, audio.HighFrequencyPerturbationIntervalMs);
        Assert.AreEqual(4d, audio.HighFrequencyPerturbationStrengthPercent);
        Assert.AreEqual(-32d, audio.HighFrequencyPerturbationLevelDb);
        StringAssert.Contains(filter, "afftfilt=");
        StringAssert.Contains(filter, "bandreject=f=8000.000000");
        StringAssert.Contains(filter, "gte(b/nb\\,0.25)");
    }

    [TestMethod]
    public void Video_resume_retries_audio_after_a_previous_audio_start_failure()
    {
        Assert.IsTrue(MainWindow.ShouldRetryVideoAudioOnResume(
            PlaybackState.Paused,
            hasAudioTrack: true,
            WindowsAudioPlaybackState.Idle));
        Assert.IsTrue(MainWindow.ShouldRetryVideoAudioOnResume(
            PlaybackState.Paused,
            hasAudioTrack: true,
            WindowsAudioPlaybackState.Failed));
        Assert.IsTrue(MainWindow.ShouldRetryVideoAudioOnResume(
            PlaybackState.Paused,
            hasAudioTrack: true,
            WindowsAudioPlaybackState.Completed));

        Assert.IsFalse(MainWindow.ShouldRetryVideoAudioOnResume(
            PlaybackState.Playing,
            hasAudioTrack: true,
            WindowsAudioPlaybackState.Failed));
        Assert.IsFalse(MainWindow.ShouldRetryVideoAudioOnResume(
            PlaybackState.Paused,
            hasAudioTrack: false,
            WindowsAudioPlaybackState.Failed));
        Assert.IsFalse(MainWindow.ShouldRetryVideoAudioOnResume(
            PlaybackState.Paused,
            hasAudioTrack: true,
            WindowsAudioPlaybackState.Paused));
    }

    [TestMethod]
    [DataRow(false, false, GpAutoLive.Media.MpvLaunchMode.Original)]
    [DataRow(false, true, GpAutoLive.Media.MpvLaunchMode.Original)]
    [DataRow(true, false, GpAutoLive.Media.MpvLaunchMode.Cpu4)]
    [DataRow(true, true, GpAutoLive.Media.MpvLaunchMode.Gpu83)]
    public void Video_launch_mode_selection_honors_processing_switch_and_runtime_capability(
        bool processingEnabled,
        bool fullGpu83Runtime,
        GpAutoLive.Media.MpvLaunchMode expected)
    {
        Assert.AreEqual(
            expected,
            VideoPlaybackModeSelector.Select(processingEnabled, fullGpu83Runtime));
    }

    [TestMethod]
    public void Regenerating_parameter_snapshots_only_advances_the_system_generation()
    {
        var state = new ShellState();
        var initialVideo = state.VideoParameterSnapshot;
        var initialAudio = state.AudioParameterSnapshot;

        state.RegenerateParameterSnapshots();

        Assert.AreEqual(initialVideo.Generation + 1, state.VideoParameterSnapshot.Generation);
        Assert.AreEqual(initialAudio.Generation + 1, state.AudioParameterSnapshot.Generation);
        StringAssert.Contains(state.StatusMessage, "只读参数快照");
    }

    [TestMethod]
    public void Media_owner_snapshot_is_projected_without_shell_state_guessing()
    {
        var state = new ShellState();
        var source = new SourceMediaDto(
            @"C:\media\sample.mp4",
            @"C:\media\sample.mp4",
            MediaKind.Video,
            MediaCompatibilityMode.Direct,
            "sample.mp4",
            1,
            1_000,
            null,
            null,
            1280,
            720,
            30,
            null,
            null,
            "h264",
            null,
            null,
            "disabled");

        state.ApplyMediaSnapshot(new AppState
        {
            PlaybackState = PlaybackState.Playing,
            SourceMediaPool = [source],
        });

        Assert.IsTrue(state.HasMedia);
        Assert.AreEqual(1, state.MediaItems.Count);
        Assert.AreEqual("1 / 100", state.MediaCountLabel);
        Assert.IsTrue(state.IsPlaying);
        Assert.AreEqual("播放中", state.PlaybackLabel);
        Assert.AreEqual("00:00:01", state.MediaItems[0].DurationLabel);

        state.ApplyMediaSnapshot(AppState.Initial);

        Assert.IsFalse(state.HasMedia);
        Assert.AreEqual(0, state.MediaItems.Count);
        Assert.AreEqual("0 / 100", state.MediaCountLabel);
        Assert.IsFalse(state.IsPlaying);
        Assert.AreEqual(0d, state.PlaybackProgress);
    }
}
