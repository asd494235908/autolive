using GpAutoLive.Contracts;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class FfmpegAudioFilterBuilderTests
{
    [TestMethod]
    public void Realtime_chain_maps_supported_rust_audio_effects_and_builtin_pitch_shift()
    {
        var parameters = new AudioEffectParams
        {
            InputGainDb = 1,
            OutputGainDb = -0.5,
            LoudnessAdjustmentDb = 0.5,
            LowEqDb = -2,
            MidEqDb = 1,
            HighEqDb = 3,
            PlaybackSpeed = 1.25,
            FadeInMs = 500,
            FadeOutMs = 1_000,
            ReverbWetPercent = 10,
            NoiseReductionPercent = 20,
            PhasePerturbationPercent = 10,
            VibratoFrequencyHz = 5,
            VibratoDepthPercent = 2,
            PitchShiftSemitones = 1,
        };

        var filter = FfmpegAudioFilterBuilder.Create(parameters);

        StringAssert.Contains(filter, "volume=1dB");
        StringAssert.Contains(filter, "equalizer=f=200");
        StringAssert.Contains(filter, "equalizer=f=1000");
        StringAssert.Contains(filter, "equalizer=f=8000");
        StringAssert.Contains(filter, "atempo=1.25");
        StringAssert.Contains(filter, "afade=t=in:st=0:d=0.5");
        StringAssert.Contains(filter, "aecho=1:1:80:0.1");
        StringAssert.Contains(filter, "afftdn=nr=19.4");
        StringAssert.Contains(filter, "aphaser=in_gain=0.4:out_gain=0.74:delay=3:decay=0.5:speed=0.5");
        StringAssert.Contains(filter, "vibrato=f=5:d=0.02");
        StringAssert.Contains(filter, "asetrate=50854");
        StringAssert.Contains(filter, "aresample=48000");
        StringAssert.Contains(filter, "atempo=0.944");
        Assert.IsFalse(filter.Contains("areverse", StringComparison.Ordinal));
    }

    [TestMethod]
    public void Pitch_shift_precedes_playback_speed_like_the_rust_realtime_chain()
    {
        var filter = FfmpegAudioFilterBuilder.Create(new AudioEffectParams
        {
            PitchShiftSemitones = 1,
            PlaybackSpeed = 1.25,
        });

        var pitchIndex = filter.IndexOf("asetrate=", StringComparison.Ordinal);
        var speedIndex = filter.IndexOf("atempo=1.25", StringComparison.Ordinal);

        Assert.IsTrue(pitchIndex >= 0, "音高变换没有进入 FFmpeg 链。");
        Assert.IsTrue(speedIndex >= 0, "播放速度没有进入 FFmpeg 链。");
        Assert.IsTrue(
            pitchIndex < speedIndex,
            "C# 应与 Rust 一样先完成音高微移，再应用播放速度；当前顺序会改变组合效果的时间语义。");
    }

    [TestMethod]
    public void Known_source_duration_maps_streamable_fade_out_and_unknown_duration_skips_it()
    {
        var parameters = new AudioEffectParams { FadeOutMs = 1_000 };

        var knownDuration = FfmpegAudioFilterBuilder.Create(parameters, sourceDurationMs: 2_500);
        StringAssert.Contains(knownDuration, "afade=t=out:st=1.5:d=1");

        var unknownDuration = FfmpegAudioFilterBuilder.Create(parameters);
        Assert.IsFalse(unknownDuration.Contains("afade=t=out", StringComparison.Ordinal));
    }

    [TestMethod]
    public void Realtime_chain_consumes_rust_natural_dynamic_and_local_voice_preset_fields()
    {
        var parameters = new AudioEffectParams
        {
            NaturalVoiceMode = NaturalVoiceMode.NaturalDynamic,
            RandomChangePeriodMs = 4_000,
            VoiceLibraryId = "local-voice-a",
        };

        var filter = FfmpegAudioFilterBuilder.Create(parameters);

        StringAssert.Contains(filter, "volume='1+0.012000*sin(2*PI*t/4.000000)':eval=frame");
        StringAssert.Contains(filter, "equalizer=f=");
        Assert.IsFalse(filter.Contains("loudnorm=", StringComparison.Ordinal));
    }

    [TestMethod]
    public void Realtime_chain_is_deterministic_for_the_same_local_voice_preset()
    {
        var parameters = new AudioEffectParams { VoiceLibraryId = "local-voice-a" };

        var first = FfmpegAudioFilterBuilder.Create(parameters);
        var second = FfmpegAudioFilterBuilder.Create(parameters);

        Assert.AreEqual(first, second);
    }

    [TestMethod]
    public void Non_empty_local_voice_library_id_is_consumed_into_the_managed_filter_chain()
    {
        var parameters = new AudioEffectParams { VoiceLibraryId = " " };

        var filter = FfmpegAudioFilterBuilder.Create(parameters);

        StringAssert.Contains(filter, "equalizer=f=");
    }

    [TestMethod]
    public void Realtime_chain_consumes_rust_verified_single_input_spectral_filters()
    {
        var parameters = new AudioEffectParams
        {
            SpectralPerturbationPercent = 10,
            SpectrumBlindSpotPercent = 2,
            HighFrequencyPerturbationEnabled = true,
            HighFrequencyPerturbationIntervalMs = 12_000,
            HighFrequencyPerturbationStrengthPercent = 4,
            HighFrequencyPerturbationLevelDb = -32,
        };

        var filter = FfmpegAudioFilterBuilder.Create(parameters);

        StringAssert.Contains(filter, "afftfilt=real='re*(1+0.100000*sin(2*PI*b/nb*7+ch*PI/3))':imag='im*(1+0.100000*sin(2*PI*b/nb*7+ch*PI/3))':win_size=4096:win_func=hann:overlap=0.75");
        StringAssert.Contains(filter, "bandreject=f=8000.000000:t=h:w=400.000000");
        StringAssert.Contains(filter, "gte(b/nb\\,0.25)");
        StringAssert.Contains(filter, "enable='lt(mod(t\\,12.000000)\\,6.000000)'");
    }

    [TestMethod]
    public async Task Packaged_ffmpeg_consumes_the_single_input_spectral_filter_chain()
    {
        var ffmpeg = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_FFMPEG");
        if (string.IsNullOrWhiteSpace(ffmpeg) || !File.Exists(ffmpeg))
        {
            return;
        }

        var filter = FfmpegAudioFilterBuilder.Create(
            new AudioEffectParams
            {
                SpectralPerturbationPercent = 10,
                SpectrumBlindSpotPercent = 2,
                HighFrequencyPerturbationEnabled = true,
                HighFrequencyPerturbationIntervalMs = 12_000,
                HighFrequencyPerturbationStrengthPercent = 4,
                HighFrequencyPerturbationLevelDb = -32,
            });

        using var process = new System.Diagnostics.Process
        {
            StartInfo = new System.Diagnostics.ProcessStartInfo
            {
                FileName = ffmpeg,
                UseShellExecute = false,
                CreateNoWindow = true,
                RedirectStandardError = false,
            },
        };
        process.StartInfo.ArgumentList.Add("-hide_banner");
        process.StartInfo.ArgumentList.Add("-loglevel");
        process.StartInfo.ArgumentList.Add("error");
        process.StartInfo.ArgumentList.Add("-f");
        process.StartInfo.ArgumentList.Add("lavfi");
        process.StartInfo.ArgumentList.Add("-i");
        process.StartInfo.ArgumentList.Add("sine=frequency=8000:sample_rate=48000:duration=0.25");
        process.StartInfo.ArgumentList.Add("-af");
        process.StartInfo.ArgumentList.Add(filter);
        process.StartInfo.ArgumentList.Add("-f");
        process.StartInfo.ArgumentList.Add("null");
        process.StartInfo.ArgumentList.Add("-");

        Assert.IsTrue(process.Start());
        await process.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(5));
        Assert.AreEqual(0, process.ExitCode);
    }
}
