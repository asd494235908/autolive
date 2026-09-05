using GpAutoLive.App.Features.Settings;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class DesktopSettingsDraftTests
{
    [TestMethod]
    public void From_copies_only_low_sensitivity_ui_preferences()
    {
        var preferences = UserPreferences.Defaults with
        {
            Theme = "light",
            Language = "en-US",
            EffectsPanelExpanded = false,
            PerformanceSamplingEnabled = false,
            LastOutputMode = "virtual_camera",
        };

        var draft = DesktopSettingsDraft.From(preferences);

        Assert.AreEqual("light", draft.Theme);
        Assert.AreEqual("en-US", draft.Language);
        Assert.IsFalse(draft.EffectsPanelExpanded);
        Assert.IsFalse(draft.PerformanceSamplingEnabled);
        Assert.AreEqual("virtual_camera", draft.LastOutputMode);
    }

    [TestMethod]
    public void Settings_window_loads_wpf_resources_on_sta()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new SettingsWindow(
                DesktopSettingsDraft.From(UserPreferences.Defaults));
            window.Close();
        });
    }
}
