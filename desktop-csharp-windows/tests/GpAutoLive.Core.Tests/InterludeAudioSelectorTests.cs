using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class InterludeAudioSelectorTests
{
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
                AudioFixedPresetId = "p22"
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
