using GpAutoLive.Core.Configuration;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class EffectCycleSettingsTests
{
    [TestMethod]
    public void Creates_valid_ranges_and_rejects_reversed_or_out_of_bounds_values()
    {
        Assert.IsTrue(
            EffectCycleSettings.TryCreate(1_500, 4_500, 2_000, 6_000, out var settings, out var error),
            error);
        Assert.IsNotNull(settings);
        Assert.AreEqual(1_500UL, settings.VideoPeriodMinMs);
        Assert.AreEqual(4_500UL, settings.VideoPeriodMaxMs);
        Assert.AreEqual(2_000UL, settings.AudioPeriodMinMs);
        Assert.AreEqual(6_000UL, settings.AudioPeriodMaxMs);

        Assert.IsFalse(EffectCycleSettings.TryCreate(5_000, 4_000, 2_000, 3_000, out _, out error));
        StringAssert.Contains(error, "视频周期最小值不能大于最大值");

        Assert.IsFalse(EffectCycleSettings.TryCreate(1_000, 4_000, 2_000, 60_001, out _, out error));
        StringAssert.Contains(error, "声音周期必须在 1 到 60 秒之间");
    }

    [TestMethod]
    public void Applies_ranges_to_low_sensitivity_user_preferences()
    {
        var preferences = UserPreferences.Defaults;
        var settings = new EffectCycleSettings(1_500, 4_500, 2_000, 6_000);

        var updated = settings.ApplyTo(preferences);

        Assert.AreEqual(1_500UL, updated.VideoCyclePeriodMinMs);
        Assert.AreEqual(4_500UL, updated.VideoCyclePeriodMaxMs);
        Assert.AreEqual(2_000UL, updated.AudioCyclePeriodMinMs);
        Assert.AreEqual(6_000UL, updated.AudioCyclePeriodMaxMs);
    }
}
