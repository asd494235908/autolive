using GpAutoLive.Core.Configuration;

namespace GpAutoLive.App.Features.Settings;

/// <summary>设置窗口与偏好所有者之间的脱敏编辑快照；不包含路径、URL 或凭据。</summary>
public sealed record DesktopSettingsDraft(
    string Theme,
    string Language,
    bool EffectsPanelExpanded,
    bool PerformanceSamplingEnabled,
    string LastOutputMode)
{
    public static DesktopSettingsDraft From(UserPreferences preferences)
    {
        ArgumentNullException.ThrowIfNull(preferences);
        return new(
            preferences.Theme,
            preferences.Language,
            preferences.EffectsPanelExpanded,
            preferences.PerformanceSamplingEnabled,
            preferences.LastOutputMode);
    }
}
