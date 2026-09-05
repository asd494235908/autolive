using System.IO;
using System.Windows;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.App.Features.Settings;

/// <summary>
/// WPF 壳层的低敏感偏好所有者。只负责窗口几何和非敏感 UI 偏好，
/// 不接触媒体池、播放位置、URL 或任何凭据。
/// </summary>
public sealed class DesktopPreferencesCoordinator
{
    private readonly IniUserPreferencesStore _store;
    private UserPreferences _current = UserPreferences.Defaults;
    private bool _loaded;

    public DesktopPreferencesCoordinator(IniUserPreferencesStore store)
    {
        _store = store ?? throw new ArgumentNullException(nameof(store));
    }

    public UserPreferences Current => _current;

    public bool IsLoaded => _loaded;

    /// <summary>从用户数据目录读取偏好；损坏或不可访问时回退默认值。</summary>
    public async Task<string?> LoadAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            _current = await _store.ReadAsync(cancellationToken).ConfigureAwait(false);
            _loaded = true;
            return null;
        }
        catch (OperationCanceledException)
        {
            throw;
        }
        catch (ConfigurationValidationException)
        {
            _current = UserPreferences.Defaults;
            _loaded = true;
            return "本地偏好格式无效，已使用默认设置";
        }
        catch (Exception exception) when (exception is IOException or UnauthorizedAccessException or NotSupportedException)
        {
            _current = UserPreferences.Defaults;
            _loaded = true;
            return "本地偏好暂时不可用，已使用默认设置";
        }
    }

    /// <summary>把已加载的窗口几何应用到 WPF 窗口，并限制在虚拟屏幕范围内。</summary>
    public void ApplyTo(Window window)
    {
        ArgumentNullException.ThrowIfNull(window);
        var preferences = _current;
        window.Width = preferences.WindowWidth;
        window.Height = preferences.WindowHeight;

        if (preferences.WindowLeft is not double left || preferences.WindowTop is not double top)
        {
            return;
        }

        var virtualLeft = SystemParameters.VirtualScreenLeft;
        var virtualTop = SystemParameters.VirtualScreenTop;
        var virtualRight = virtualLeft + SystemParameters.VirtualScreenWidth;
        var virtualBottom = virtualTop + SystemParameters.VirtualScreenHeight;
        var maxLeft = Math.Max(virtualLeft, virtualRight - Math.Min(window.Width, 96));
        var maxTop = Math.Max(virtualTop, virtualBottom - Math.Min(window.Height, 96));
        window.WindowStartupLocation = WindowStartupLocation.Manual;
        window.Left = Math.Clamp(left, virtualLeft, maxLeft);
        window.Top = Math.Clamp(top, virtualTop, maxTop);
    }

    /// <summary>保存当前窗口几何和既有非敏感偏好；写入由 Core 原子文件边界完成。</summary>
    public async Task<string?> SaveAsync(
        Window window,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(window);
        if (!_loaded)
        {
            _loaded = true;
        }

        var next = _current with
        {
            WindowWidth = ClampDimension(window.Width, 960, 7680, UserPreferences.Defaults.WindowWidth),
            WindowHeight = ClampDimension(window.Height, 680, 4320, UserPreferences.Defaults.WindowHeight),
            WindowLeft = ToFiniteCoordinate(window.Left),
            WindowTop = ToFiniteCoordinate(window.Top),
        };

        return await SaveCoreAsync(next, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>
    /// 保存设置窗口提交的非敏感偏好与主窗口几何；白名单和规范化由 Core 完成，
    /// 写入仍使用同一原子 INI 边界。
    /// </summary>
    public async Task<string?> SaveAsync(
        Window window,
        DesktopSettingsDraft draft,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(window);
        ArgumentNullException.ThrowIfNull(draft);
        if (!_loaded)
        {
            _loaded = true;
        }

        UserPreferences next;
        try
        {
            next = _current.WithUiSettings(
                draft.Theme,
                draft.Language,
                draft.EffectsPanelExpanded,
                draft.PerformanceSamplingEnabled,
                draft.LastOutputMode) with
            {
                WindowWidth = ClampDimension(window.Width, 960, 7680, UserPreferences.Defaults.WindowWidth),
                WindowHeight = ClampDimension(window.Height, 680, 4320, UserPreferences.Defaults.WindowHeight),
                WindowLeft = ToFiniteCoordinate(window.Left),
                WindowTop = ToFiniteCoordinate(window.Top),
            };
        }
        catch (Exception exception) when (exception is ConfigurationValidationException or ArgumentException)
        {
            return "设置值不受支持，本次修改未保存";
        }

        return await SaveCoreAsync(next, cancellationToken).ConfigureAwait(false);
    }

    private async Task<string?> SaveCoreAsync(
        UserPreferences next,
        CancellationToken cancellationToken)
    {

        try
        {
            await _store.SaveAsync(next, cancellationToken).ConfigureAwait(false);
            _current = next;
            return null;
        }
        catch (OperationCanceledException)
        {
            throw;
        }
        catch (Exception exception) when (exception is IOException or UnauthorizedAccessException or NotSupportedException)
        {
            return "本地偏好保存失败，本次关闭不影响媒体文件";
        }
    }

    /// <summary>创建默认用户配置位置；失败时返回脱敏错误，不暴露本地路径。</summary>
    public static bool TryCreateDefault(
        out DesktopPreferencesCoordinator? coordinator,
        out string? error)
    {
        coordinator = null;
        error = null;
        var localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        if (string.IsNullOrWhiteSpace(localAppData))
        {
            error = "无法定位 Windows 用户数据目录，已使用内存偏好";
            return false;
        }

        try
        {
            var path = Path.Combine(localAppData, "GpAutoLive", "config", "app.ini");
            coordinator = new DesktopPreferencesCoordinator(new IniUserPreferencesStore(path));
            return true;
        }
        catch (Exception exception) when (exception is ArgumentException or IOException or NotSupportedException)
        {
            error = "Windows 用户偏好路径不可用，已使用内存偏好";
            return false;
        }
    }

    private static int ClampDimension(double value, int minimum, int maximum, int fallback)
    {
        if (!double.IsFinite(value))
        {
            return fallback;
        }

        return Math.Clamp((int)Math.Round(value), minimum, maximum);
    }

    private static double? ToFiniteCoordinate(double value) =>
        double.IsFinite(value) ? Math.Clamp(value, -32768, 32767) : null;
}
