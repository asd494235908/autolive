using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace GpAutoLive.App.Features.Settings;

/// <summary>桌面端低敏感偏好编辑窗口；不读取或展示任何凭据和媒体路径。</summary>
public partial class SettingsWindow : Window
{
    private readonly DesktopSettingsDraft _initial;

    public SettingsWindow(DesktopSettingsDraft initial)
    {
        ArgumentNullException.ThrowIfNull(initial);
        _initial = initial;
        InitializeComponent();
        PerformanceSamplingCheckBox.IsChecked = initial.PerformanceSamplingEnabled;
        EffectsPanelExpandedCheckBox.IsChecked = initial.EffectsPanelExpanded;
        SelectOutputMode(initial.LastOutputMode);
    }

    public DesktopSettingsDraft? Draft { get; private set; }

    private void SelectOutputMode(string outputMode)
    {
        foreach (var item in LastOutputModeComboBox.Items.OfType<ComboBoxItem>())
        {
            if (string.Equals(item.Tag?.ToString(), outputMode, StringComparison.OrdinalIgnoreCase))
            {
                LastOutputModeComboBox.SelectedItem = item;
                return;
            }
        }

        LastOutputModeComboBox.SelectedIndex = 0;
    }

    private void SaveButton_Click(object sender, RoutedEventArgs e)
    {
        if (LastOutputModeComboBox.SelectedItem is not ComboBoxItem selected
            || string.IsNullOrWhiteSpace(selected.Tag?.ToString()))
        {
            StatusText.Text = "请选择下次输出入口";
            return;
        }

        Draft = new DesktopSettingsDraft(
            _initial.Theme,
            _initial.Language,
            EffectsPanelExpandedCheckBox.IsChecked == true,
            PerformanceSamplingCheckBox.IsChecked == true,
            selected.Tag.ToString()!);
        DialogResult = true;
    }

    private void CancelButton_Click(object sender, RoutedEventArgs e) =>
        DialogResult = false;

    private void Window_PreviewKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.Escape)
        {
            DialogResult = false;
            e.Handled = true;
        }
    }
}
