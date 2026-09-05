using System.Text.Json;
using GpAutoLive.Contracts;

namespace GpAutoLive.Contracts.Tests;

[TestClass]
public sealed class MediaEffectContractTests
{
    [TestMethod]
    public void Default_media_effects_are_valid_and_use_wire_names()
    {
        var parameters = MediaEffectParams.Default;

        Assert.IsTrue(parameters.TryValidate(out var errors), string.Join("; ", errors.Select(error => error.Message)));
        Assert.AreEqual(12, parameters.Advanced.BandWeights.Count);
        CollectionAssert.AreEquivalent(
            new uint[] { 65, 92, 131, 188, 267, 381, 544, 777, 1_110, 1_585, 2_263, 20_000 },
            parameters.Advanced.BandWeights.Keys.ToArray());

        var json = JsonSerializer.Serialize(parameters, ContractJson.CreateOptions());

        StringAssert.Contains(json, "\"natural_voice_mode\":\"original\"");
        StringAssert.Contains(json, "\"random_change_period_ms\":4000");
        StringAssert.Contains(json, "\"band_weights\"");
        StringAssert.Contains(json, "\"picture_in_picture_timeline_locked\":true");
        Assert.IsFalse(json.Contains("PitchShiftSemitones", StringComparison.Ordinal));
    }

    [TestMethod]
    public void Video_audio_and_advanced_ranges_report_stable_fields()
    {
        var audio = AudioEffectParams.Default with
        {
            PitchShiftSemitones = 3,
            OutputBitrateKbps = 321
        };
        var video = VideoEffectParams.Default with { BrightnessPercent = double.NaN };
        var advanced = AdvancedEffectParams.Default with
        {
            TargetFrequencyHz = null,
            CoreFrequencyHz = 100,
            SliceLengthMs = 10_000,
            SliceTriggerIntervalMs = 5_000
        };

        Assert.IsFalse(audio.TryValidate(out var audioErrors));
        Assert.IsTrue(audioErrors.Any(error => error.Field == "audio.pitch_shift_semitones" && error.Code == "out_of_range"));
        Assert.IsTrue(audioErrors.Any(error => error.Field == "audio.output_bitrate_kbps" && error.Code == "out_of_range"));

        Assert.IsFalse(video.TryValidate(out var videoErrors));
        var nonFiniteError = videoErrors.Single(error => error.Field == "video.brightness_percent");
        Assert.AreEqual("out_of_range", nonFiniteError.Code);
        Assert.IsNull(nonFiniteError.Value);

        Assert.IsFalse(advanced.TryValidate(out var advancedErrors));
        Assert.IsTrue(advancedErrors.Any(error => error.Field == "advanced.core_frequency_hz" && error.Code == "invalid_relation"));
        Assert.IsTrue(advancedErrors.Any(error => error.Field == "advanced.slice_trigger_interval_ms" && error.Code == "invalid_relation"));
    }

    [TestMethod]
    public void Band_weights_and_identifiers_are_checked_as_contract_values()
    {
        var weights = MediaEffectContracts.VisualBandFrequenciesHz.ToDictionary(frequencyHz => frequencyHz, _ => 1.0);
        weights[65] = 2.0;
        weights[999] = 1.0;
        weights.Remove(92);

        var parameters = MediaEffectParams.Default with
        {
            Audio = AudioEffectParams.Default with { VoiceLibraryId = new string('密', 65) },
            Advanced = AdvancedEffectParams.Default with { BandWeights = weights }
        };

        Assert.IsFalse(parameters.TryValidate(out var errors));
        Assert.IsTrue(errors.Any(error => error.Field == "advanced.band_weights.65" && error.Code == "out_of_range"));
        Assert.IsTrue(errors.Any(error => error.Field == "advanced.band_weights" && error.Code == "unknown_frequency_band"));
        Assert.IsTrue(errors.Any(error => error.Field == "advanced.band_weights" && error.Code == "missing_frequency_band"));
        Assert.IsTrue(errors.Any(error => error.Field == "audio.voice_library_id" && error.Code == "invalid_identifier"));
    }
}
