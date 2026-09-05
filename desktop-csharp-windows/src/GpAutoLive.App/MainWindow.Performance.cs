using System.Windows;
using GpAutoLive.App.Features.Performance;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private void PerformanceTimer_Tick(object? sender, EventArgs e) =>
        _ = SamplePerformanceAsync();

    private void ApplyRuntimePreferences()
    {
        var preferences = _preferences?.Current ?? UserPreferences.Defaults;
        QuickParamsCard.Visibility = preferences.EffectsPanelExpanded
            ? Visibility.Visible
            : Visibility.Collapsed;
        QuickParamsRow.Height = preferences.EffectsPanelExpanded
            ? new GridLength(1.04, GridUnitType.Star)
            : new GridLength(0);
        LastOutputModeText.Text = $"下次入口：{FormatLastOutputMode(preferences.LastOutputMode)}";

        if (preferences.PerformanceSamplingEnabled && !_isClosing)
        {
            _performanceTimer.Start();
            _ = SamplePerformanceAsync();
        }
        else
        {
            _performanceTimer.Stop();
            SetPerformanceText("CPU / GPU / RAM · 已关闭");
        }
    }

    private static string FormatLastOutputMode(string outputMode) => outputMode switch
    {
        "rtmp" => "RTMP / RTMPS",
        "virtual_camera" => "虚拟摄像头",
        _ => "本地预览",
    };

    private async Task SamplePerformanceAsync()
    {
        if (_performanceSampleInFlight || _isClosing)
        {
            return;
        }

        _performanceSampleInFlight = true;
        try
        {
            var result = await _performanceSampler
                .SampleAsync(_windowCancellation.Token)
                .ConfigureAwait(true);
            if (!_isClosing)
            {
                SetPerformanceText(PerformanceMetricsFormatter.Format(result));
            }
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 关闭窗口时取消当前采样，不再更新 UI。
        }
        finally
        {
            _performanceSampleInFlight = false;
        }
    }

    private void SetPerformanceText(string text)
    {
        PerformanceText.Text = text;
        FooterPerformanceText.Text = text;
    }
}
