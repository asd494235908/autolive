using GpAutoLive.Core.Configuration;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class UserPreferencesTests
{
    [TestMethod]
    public void WithUiSettings_normalizes_allowed_values_without_touching_window_geometry()
    {
        var current = UserPreferences.Defaults with
        {
            WindowWidth = 1440,
            WindowLeft = -12.5,
        };

        var next = current.WithUiSettings(
            "LIGHT",
            "en-us",
            effectsPanelExpanded: false,
            performanceSamplingEnabled: false,
            lastOutputMode: "RTMP");

        Assert.AreEqual("light", next.Theme);
        Assert.AreEqual("en-US", next.Language);
        Assert.IsFalse(next.EffectsPanelExpanded);
        Assert.IsFalse(next.PerformanceSamplingEnabled);
        Assert.AreEqual("rtmp", next.LastOutputMode);
        Assert.AreEqual(1440, next.WindowWidth);
        Assert.AreEqual(-12.5, next.WindowLeft);
    }

    [TestMethod]
    public void WithUiSettings_rejects_values_outside_the_persisted_whitelist()
    {
        try
        {
            _ = UserPreferences.Defaults.WithUiSettings(
                "neon",
                "zh-CN",
                effectsPanelExpanded: true,
                performanceSamplingEnabled: true,
                lastOutputMode: "preview");
            Assert.Fail("不受支持的主题必须拒绝");
        }
        catch (ConfigurationValidationException)
        {
            // expected
        }
    }
}
