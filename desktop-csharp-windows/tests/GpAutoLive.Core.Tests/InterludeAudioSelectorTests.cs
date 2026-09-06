using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class InterludeAudioSelectorTests
{
    [TestMethod]
    public void Schedule_starts_immediately_then_waits_and_avoids_adjacent_duplicate_files()
    {
        var planner = new InterludeSchedulePlanner(new Random(7));

        var first = planner.Observe("generation-1:source-a", 100, 3, 500, 500, true, false, false, false);
        var active = planner.Observe("generation-1:source-a", 599, 3, 500, 500, true, true, false, false);
        var afterCompletion = planner.Observe("generation-1:source-a", 600, 3, 500, 500, true, false, false, false);
        var waiting = planner.Observe("generation-1:source-a", 1_099, 3, 500, 500, true, false, false, false);
        var second = planner.Observe("generation-1:source-a", 1_100, 3, 500, 500, true, false, false, false);

        Assert.IsTrue(first.ShouldStart);
        Assert.IsNotNull(first.FileIndex);
        Assert.IsFalse(active.ShouldStart);
        Assert.IsNull(first.NextDueTimestampMs);
        Assert.IsFalse(afterCompletion.ShouldStart);
        Assert.IsFalse(waiting.ShouldStart);
        Assert.AreEqual(99d, waiting.ProgressPercent, 0.001d);
        Assert.IsTrue(second.ShouldStart);
        Assert.AreNotEqual(first.FileIndex, second.FileIndex);
    }

    [TestMethod]
    public void Schedule_resets_to_immediate_on_source_generation_change_and_pauses_without_triggering()
    {
        var planner = new InterludeSchedulePlanner(new Random(11));

        var first = planner.Observe("generation-1:source-a", 0, 2, 500, 500, true, false, false, false);
        var paused = planner.Observe("generation-1:source-a", 5_000, 2, 500, 500, true, true, false, true);
        var changed = planner.Observe("generation-2:source-b", 5_000, 2, 500, 500, true, false, false, false);

        Assert.IsTrue(first.ShouldStart);
        Assert.IsFalse(paused.ShouldStart);
        Assert.IsTrue(changed.ShouldStart);
        Assert.AreEqual(0, changed.ProgressPercent);
    }

    [TestMethod]
    public void Schedule_does_not_trigger_without_ready_files_or_schedule_key()
    {
        var planner = new InterludeSchedulePlanner();

        var noFiles = planner.Observe("generation-1:source-a", 0, 0, 500, 500, true, false, false, false);
        var noKey = planner.Observe(null, 0, 2, 500, 500, true, false, false, false);

        Assert.IsFalse(noFiles.ShouldStart);
        Assert.IsFalse(noKey.ShouldStart);
        Assert.IsNull(noKey.FileIndex);
    }

    [TestMethod]
    public void Manual_start_restarts_the_bounded_wait_without_immediate_retrigger()
    {
        var planner = new InterludeSchedulePlanner(new Random(19));

        planner.MarkPlaybackStarted("generation-1:source-a", 1);
        var active = planner.Observe("generation-1:source-a", 599, 3, 500, 500, true, true, false, false);
        var afterCompletion = planner.Observe("generation-1:source-a", 600, 3, 500, 500, true, false, false, false);
        var waiting = planner.Observe("generation-1:source-a", 1_099, 3, 500, 500, true, false, false, false);
        var next = planner.Observe("generation-1:source-a", 1_100, 3, 500, 500, true, false, false, false);

        Assert.IsFalse(active.ShouldStart);
        Assert.IsFalse(afterCompletion.ShouldStart);
        Assert.IsFalse(waiting.ShouldStart);
        Assert.AreEqual(99d, waiting.ProgressPercent, 0.001d);
        Assert.IsTrue(next.ShouldStart);
        Assert.AreNotEqual(1, next.FileIndex);
    }

    [TestMethod]
    public void Schedule_keeps_projecting_the_next_wait_until_the_audio_host_recovers()
    {
        var planner = new InterludeSchedulePlanner(new Random(23));

        var first = planner.Observe("generation-1:source-a", 0, 2, 500, 500, true, false, false, false);
        var active = planner.Observe("generation-1:source-a", 100, 2, 500, 500, true, true, false, false);
        var afterCompletion = planner.Observe(
            "generation-1:source-a",
            200,
            2,
            500,
            500,
            true,
            false,
            false,
            false,
            canStartPlayback: false);
        var waiting = planner.Observe(
            "generation-1:source-a",
            450,
            2,
            500,
            500,
            true,
            false,
            false,
            false,
            canStartPlayback: false);
        var dueButUnavailable = planner.Observe(
            "generation-1:source-a",
            700,
            2,
            500,
            500,
            true,
            false,
            false,
            false,
            canStartPlayback: false);
        var recovered = planner.Observe(
            "generation-1:source-a",
            701,
            2,
            500,
            500,
            true,
            false,
            false,
            false,
            canStartPlayback: true);

        Assert.IsTrue(first.ShouldStart);
        Assert.IsFalse(active.ShouldStart);
        Assert.IsFalse(afterCompletion.ShouldStart);
        Assert.AreEqual(0d, afterCompletion.ProgressPercent);
        Assert.AreEqual(50d, waiting.ProgressPercent);
        Assert.IsFalse(dueButUnavailable.ShouldStart);
        Assert.AreEqual(100d, dueButUnavailable.ProgressPercent);
        Assert.IsTrue(recovered.ShouldStart);
    }

    [TestMethod]
    public void Fixed_mode_returns_only_fixed_preset_and_bounded_interval()
    {
        var config = InterludeAudioConfig.Default with
        {
            AudioSelectionMode = InterludeAudioSelectionMode.Fixed,
            AudioFixedPresetId = "p22",
            IntervalMinMs = 700,
            IntervalMaxMs = 700
        };
        var selector = new InterludeAudioSelector(new Random(7));

        Assert.IsTrue(selector.TrySelect(config, DateTimeOffset.UnixEpoch, out var selection, out var error), error?.Message);
        Assert.IsNotNull(selection);
        CollectionAssert.AreEqual(new[] { "p22" }, selection!.PresetIds.ToArray());
        Assert.AreEqual(700UL, selection.NextIntervalMs);
        Assert.IsNull(selection.PresetValidUntil);
    }

    [TestMethod]
    public void Fixed_single_track_selection_creates_bounded_audio_effect_parameters()
    {
        var config = InterludeAudioConfig.Default with
        {
            AudioSelectionMode = InterludeAudioSelectionMode.Fixed,
            AudioFixedPresetId = "p22",
            IntervalMinMs = 500,
            IntervalMaxMs = 500,
        };
        var selector = new InterludeAudioSelector(new Random(7));

        Assert.IsTrue(selector.TrySelect(config, DateTimeOffset.UnixEpoch, out var selection, out var error), error?.Message);
        Assert.IsNotNull(selection);
        Assert.IsTrue(
            selection!.TryCreateBoundedAudioEffectParams(out var parameters, out var projectionError),
            projectionError);
        Assert.IsNotNull(parameters);
        Assert.AreEqual("p22", parameters!.VoiceLibraryId);
        Assert.AreEqual(NaturalVoiceMode.Original, parameters.NaturalVoiceMode);
    }

    [TestMethod]
    public void Random_single_track_selection_creates_bounded_audio_effect_parameters()
    {
        var config = InterludeAudioConfig.Default with
        {
            AudioSelectionMode = InterludeAudioSelectionMode.Random,
            AudioMixEnabled = false,
            IntervalMinMs = 500,
            IntervalMaxMs = 500,
        };
        var selector = new InterludeAudioSelector(new Random(11));

        Assert.IsTrue(selector.TrySelect(config, DateTimeOffset.UnixEpoch, out var selection, out var error), error?.Message);
        Assert.IsNotNull(selection);
        Assert.AreEqual(1, selection!.PresetIds.Length);
        Assert.IsTrue(
            selection.TryCreateBoundedAudioEffectParams(out var parameters, out var projectionError),
            projectionError);
        Assert.IsNotNull(parameters);
        Assert.IsTrue(InterludeAudioRules.IsAllowedPresetId(parameters!.VoiceLibraryId));
    }

    [TestMethod]
    public void Multi_track_selection_is_rejected_by_the_single_input_projection()
    {
        var config = InterludeAudioConfig.Default with
        {
            AudioMixEnabled = true,
            AudioMixPickMin = 2,
            AudioMixPickMax = 2,
            IntervalMinMs = 500,
            IntervalMaxMs = 500,
        };
        var selector = new InterludeAudioSelector(new Random(13));

        Assert.IsTrue(selector.TrySelect(config, DateTimeOffset.UnixEpoch, out var selection, out var error), error?.Message);
        Assert.IsNotNull(selection);
        Assert.IsFalse(
            selection!.TryCreateBoundedAudioEffectParams(out var parameters, out var projectionError));
        Assert.IsNull(parameters);
        StringAssert.Contains(projectionError, "多轨");
    }

    [TestMethod]
    public void Empty_or_invalid_selection_is_rejected_before_ffmpeg_projection()
    {
        var empty = new InterludeAudioSelection([], 500, null);
        Assert.IsFalse(empty.TryCreateBoundedAudioEffectParams(out var emptyParameters, out var emptyError));
        Assert.IsNull(emptyParameters);
        StringAssert.Contains(emptyError, "为空");

        var invalid = new InterludeAudioSelection(["p99"], 500, null);
        Assert.IsFalse(invalid.TryCreateBoundedAudioEffectParams(out var invalidParameters, out var invalidError));
        Assert.IsNull(invalidParameters);
        StringAssert.Contains(invalidError, "不受支持");
    }

    [TestMethod]
    public void Random_mix_is_unique_and_respects_pick_bounds()
    {
        var config = InterludeAudioConfig.Default with
        {
            AudioMixEnabled = true,
            AudioMixPickMin = 3,
            AudioMixPickMax = 4,
            IntervalMinMs = 500,
            IntervalMaxMs = 500
        };
        var selector = new InterludeAudioSelector(new Random(11));

        Assert.IsTrue(selector.TrySelect(config, DateTimeOffset.UnixEpoch, out var selection, out var error), error?.Message);
        Assert.IsNotNull(selection);
        Assert.IsTrue(selection!.PresetIds.Length is >= 3 and <= 4);
        Assert.AreEqual(selection.PresetIds.Length, selection.PresetIds.Distinct(StringComparer.Ordinal).Count());
        Assert.AreEqual(500UL, selection.NextIntervalMs);
    }

    [TestMethod]
    public void Periodic_mode_reuses_selection_until_period_expires()
    {
        var config = InterludeAudioConfig.Default with
        {
            AudioVariationMode = InterludeAudioVariationMode.Periodic,
            AudioVariationPeriodMinMs = 1_000,
            AudioVariationPeriodMaxMs = 1_000,
            IntervalMinMs = 500,
            IntervalMaxMs = 500
        };
        var selector = new InterludeAudioSelector(new Random(13));
        var start = DateTimeOffset.UnixEpoch;

        Assert.IsTrue(selector.TrySelect(config, start, out var first, out var firstError), firstError?.Message);
        Assert.IsTrue(selector.TrySelect(config, start.AddMilliseconds(999), out var second, out var secondError), secondError?.Message);
        Assert.IsTrue(selector.TrySelect(config, start.AddMilliseconds(1_000), out var third, out var thirdError), thirdError?.Message);
        CollectionAssert.AreEqual(first!.PresetIds.ToArray(), second!.PresetIds.ToArray());
        Assert.AreNotEqual(first.PresetValidUntil, third!.PresetValidUntil);
    }

    [TestMethod]
    public void Invalid_config_does_not_mutate_selector_state()
    {
        var selector = new InterludeAudioSelector(new Random(17));
        var invalid = InterludeAudioConfig.Default with { AudioPresetIds = ["p01", "p01"] };

        Assert.IsFalse(selector.TrySelect(invalid, DateTimeOffset.UnixEpoch, out var selection, out var error));
        Assert.IsNull(selection);
        Assert.AreEqual(InterludeAudioConfigFailureCode.AudioPresetDuplicate, error?.Code);
    }

    [TestMethod]
    public void Periodic_selection_resets_when_candidate_configuration_changes()
    {
        var config = InterludeAudioConfig.Default with
        {
            AudioVariationMode = InterludeAudioVariationMode.Periodic,
            AudioVariationPeriodMinMs = 1_000,
            AudioVariationPeriodMaxMs = 1_000
        };
        var changed = config with { AudioPresetIds = ["p22"] };
        var selector = new InterludeAudioSelector(new Random(19));
        var now = DateTimeOffset.UnixEpoch;

        Assert.IsTrue(selector.TrySelect(config, now, out var first, out var firstError), firstError?.Message);
        Assert.IsTrue(selector.TrySelect(changed, now.AddMilliseconds(100), out var next, out var nextError), nextError?.Message);
        CollectionAssert.AreEqual(new[] { "p22" }, next!.PresetIds.ToArray());
        Assert.AreNotEqual(first!.PresetIds[0], next.PresetIds[0]);
    }

    [TestMethod]
    public async Task Config_store_round_trips_versioned_json_atomically()
    {
        var root = Path.Combine(Path.GetTempPath(), $"gpautolive-interlude-{Guid.NewGuid():N}");
        var path = Path.Combine(root, "profiles", "interlude", "default.json");
        try
        {
            var store = new InterludeAudioConfigStore(path);
            var expected = InterludeAudioConfig.Default with
            {
                Enabled = true,
                Directory = root,
                AudioSelectionMode = InterludeAudioSelectionMode.Fixed,
                AudioFixedPresetId = "p22",
                VolumeDb = -18,
            };

            await store.WriteAsync(expected);
            var actual = await store.ReadAsync();

            Assert.AreEqual(expected.Enabled, actual.Enabled);
            Assert.AreEqual(expected.Directory, actual.Directory);
            Assert.AreEqual(expected.AudioSelectionMode, actual.AudioSelectionMode);
            Assert.AreEqual(expected.AudioFixedPresetId, actual.AudioFixedPresetId);
            Assert.AreEqual(expected.AudioMixEnabled, actual.AudioMixEnabled);
            Assert.AreEqual(expected.AudioMixPickMin, actual.AudioMixPickMin);
            Assert.AreEqual(expected.AudioMixPickMax, actual.AudioMixPickMax);
            Assert.AreEqual(expected.AudioVariationMode, actual.AudioVariationMode);
            Assert.AreEqual(expected.AudioVariationPeriodMinMs, actual.AudioVariationPeriodMinMs);
            Assert.AreEqual(expected.AudioVariationPeriodMaxMs, actual.AudioVariationPeriodMaxMs);
            Assert.AreEqual(expected.IntervalMinMs, actual.IntervalMinMs);
            Assert.AreEqual(expected.IntervalMaxMs, actual.IntervalMaxMs);
            Assert.AreEqual(expected.VolumeDb, actual.VolumeDb);
            Assert.AreEqual(expected.DuckingDepthDb, actual.DuckingDepthDb);
            Assert.AreEqual(expected.DuckingAttackMs, actual.DuckingAttackMs);
            Assert.AreEqual(expected.DuckingReleaseMs, actual.DuckingReleaseMs);
            CollectionAssert.AreEqual(expected.AudioPresetIds.ToArray(), actual.AudioPresetIds.ToArray());
            StringAssert.Contains(await File.ReadAllTextAsync(path), "schema_version");
        }
        finally
        {
            if (Directory.Exists(root))
            {
                Directory.Delete(root, recursive: true);
            }
        }
    }
}
