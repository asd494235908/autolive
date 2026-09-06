using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using GpAutoLive.Contracts;
using GpAutoLive.Media;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private double _outputVolumePercent = 80;

    private double GetOutputVolumeGainDb() =>
        AudioPcmMixer.TryGetOutputVolumeGainDb(_outputVolumePercent, out var decibels)
            ? decibels
            : 0;

    private void OutputVolumeSlider_ValueChanged(object sender, RoutedPropertyChangedEventArgs<double> e)
    {
        _outputVolumePercent = Math.Clamp(e.NewValue, 0, 100);
        if (OutputVolumeText is not null)
        {
            OutputVolumeText.Text = $"{_outputVolumePercent:0}%";
        }
    }

    private void InterludeVolumeSlider_ValueChanged(object sender, RoutedPropertyChangedEventArgs<double> e)
    {
        var volumePercent = Math.Clamp(e.NewValue, 0, 100);
        var volumeDb = InterludeVolumePercentToDb(volumePercent);
        if (InterludeVolumeText is not null)
        {
            InterludeVolumeText.Text = $"{volumePercent:0}%";
        }

        if (!IsInitialized || _isClosing)
        {
            return;
        }

        var next = _interludeConfig with { VolumeDb = volumeDb };
        if (!InterludeAudioRules.TryValidate(next, out var error))
        {
            _state.SetStatus(error?.Message ?? "插话音量无效");
            return;
        }

        _interludeConfig = next;
        InterludeAudioConfigStatusText.Text = FormatInterludeAudioConfig(_interludeConfig);
    }

    private async void InterludeVolumeSlider_PreviewMouseLeftButtonUp(object sender, MouseButtonEventArgs e) =>
        await PersistInterludeAudioConfigAsync().ConfigureAwait(true);

    private async void InterludeVolumeSlider_KeyUp(object sender, KeyEventArgs e)
    {
        if (e.Key is Key.Left or Key.Right or Key.Up or Key.Down or Key.Home or Key.End or Key.PageUp or Key.PageDown)
        {
            await PersistInterludeAudioConfigAsync().ConfigureAwait(true);
        }
    }

    internal static double InterludeVolumePercentToDb(double volumePercent) =>
        InterludeAudioRules.MinVolumeDb
        + Math.Clamp(volumePercent, 0, 100)
        * (0 - InterludeAudioRules.MinVolumeDb)
        / 100;

    internal static double InterludeVolumeDbToPercent(double volumeDb) =>
        Math.Clamp(
            (volumeDb - InterludeAudioRules.MinVolumeDb)
            * 100
            / (0 - InterludeAudioRules.MinVolumeDb),
            0,
            100);
}
