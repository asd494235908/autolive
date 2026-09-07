using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private readonly float[] _videoSpectrumLevels = new float[PcmSpectrumAnalyzer.BandCount];
    private readonly float[] _interludeSpectrumLevels = new float[PcmSpectrumAnalyzer.BandCount];
    private Border[] _videoSpectrumBars = Array.Empty<Border>();
    private Border[] _interludeSpectrumBars = Array.Empty<Border>();

    private void InitializeAudioDiagnostics()
    {
        _videoSpectrumBars = CreateSpectrumBars(VideoSpectrumBarGrid, "#159F7C", "#78FFDA");
        _interludeSpectrumBars = CreateSpectrumBars(InterludeSpectrumBarGrid, "#367FC1", "#86D2FF");
    }

    private void SpectrumTimer_Tick(object? sender, EventArgs e) =>
        UpdateAudioDiagnosticsProjection();

    private void UpdateAudioDiagnosticsProjection()
    {
        var bus = _audioPlaybackController.ActiveFinalPcmBus;
        if (bus is null || bus.Snapshot.IsClosed)
        {
            Array.Clear(_videoSpectrumLevels);
            Array.Clear(_interludeSpectrumLevels);
            VideoSpectrumStatusText.Text = "等待视频 / 主音频 PCM";
            InterludeSpectrumStatusText.Text = "等待插话 PCM";
            AudioDiagnosticsStatusText.Text = "未启动音频会话";
        }
        else
        {
            var priority = _audioPriority.Snapshot;
            ProjectAudioDiagnosticsSpectrumLevels(
                bus,
                priority,
                _videoSpectrumLevels,
                _interludeSpectrumLevels);
            VideoSpectrumStatusText.Text = priority.InterludeActive
                ? "插话播放中 · 主音频频谱已隐藏"
                : bus.OutputSpectrum.HasSignal
                    ? "正在接收主音频 PCM"
                    : "主音频当前无有效信号";
            InterludeSpectrumStatusText.Text = priority.InterludeActive
                ? bus.OverlaySpectrum.HasSignal
                    ? "正在接收插话 PCM"
                    : "插话已启动 · 等待 PCM"
                : "当前没有插话信号";
            AudioDiagnosticsStatusText.Text = "音谱来自实际 PCM · 不消费输出缓冲";
        }

        UpdateSpectrumBars(_videoSpectrumBars, _videoSpectrumLevels);
        UpdateSpectrumBars(_interludeSpectrumBars, _interludeSpectrumLevels);
    }

    internal static void ProjectAudioDiagnosticsSpectrumLevels(
        FinalPcmBus bus,
        AudioPrioritySnapshot priority,
        Span<float> mainLevels,
        Span<float> interludeLevels)
    {
        if (priority.InterludeActive)
        {
            mainLevels.Clear();
            bus.OverlaySpectrum.CopyTo(interludeLevels);
            return;
        }

        bus.OutputSpectrum.CopyTo(mainLevels);
        interludeLevels.Clear();
    }

    private static Border[] CreateSpectrumBars(Panel panel, string baseColor, string peakColor)
    {
        var brush = new LinearGradientBrush
        {
            StartPoint = new Point(0.5, 1),
            EndPoint = new Point(0.5, 0),
            GradientStops =
            {
                new GradientStop((Color)ColorConverter.ConvertFromString(baseColor), 0),
                new GradientStop((Color)ColorConverter.ConvertFromString(peakColor), 1),
            },
        };
        brush.Freeze();
        var bars = new Border[PcmSpectrumAnalyzer.BandCount];
        for (var index = 0; index < bars.Length; index++)
        {
            bars[index] = new Border
            {
                Width = 7,
                Height = 3,
                Margin = new Thickness(2, 0, 2, 0),
                Background = brush,
                CornerRadius = new CornerRadius(3, 3, 1, 1),
                Opacity = 0.28,
                VerticalAlignment = VerticalAlignment.Bottom,
            };
            panel.Children.Add(bars[index]);
        }

        return bars;
    }

    private static void UpdateSpectrumBars(Border[] bars, float[] levels)
    {
        for (var index = 0; index < bars.Length; index++)
        {
            var displayLevel = CalculateSpectrumDisplayLevel(levels[index]);
            bars[index].Height = 3 + (displayLevel * 33);
            bars[index].Opacity = 0.28 + (displayLevel * 0.72);
        }
    }

    /// <summary>对实际 PCM 能量做对数显示增益；零输入保持零，不制造信号。</summary>
    internal static double CalculateSpectrumDisplayLevel(float level) =>
        Math.Log10(1 + (99 * Math.Clamp(level, 0, 1))) / 2;
}
