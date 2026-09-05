using System.Windows;
using System.Windows.Controls;
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
}
