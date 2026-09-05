using System.ComponentModel;
using System.Runtime.CompilerServices;
using GpAutoLive.Media;

namespace GpAutoLive.App.Features.Effects;

/// <summary>
/// 中央参数区首批已存在于 C# 播放契约中的视频参数草稿。
/// 草稿只服务于界面编辑，不冒充已经提交给 mpv 的运行时快照。
/// </summary>
public sealed class VideoEffectEditorState : INotifyPropertyChanged
{
    private double _brightnessPercent = MpvVideoEffectSnapshot.Default.BrightnessPercent;
    private double _contrastPercent = MpvVideoEffectSnapshot.Default.ContrastPercent;
    private double _saturationPercent = MpvVideoEffectSnapshot.Default.SaturationPercent;
    private double _hueRotationDegrees = MpvVideoEffectSnapshot.Default.HueRotationDegrees;

    public event PropertyChangedEventHandler? PropertyChanged;

    public double BrightnessPercent
    {
        get => _brightnessPercent;
        set => SetValue(ref _brightnessPercent, value, -100, 100);
    }

    public double ContrastPercent
    {
        get => _contrastPercent;
        set => SetValue(ref _contrastPercent, value, 0, 200);
    }

    public double SaturationPercent
    {
        get => _saturationPercent;
        set => SetValue(ref _saturationPercent, value, 0, 200);
    }

    public double HueRotationDegrees
    {
        get => _hueRotationDegrees;
        set => SetValue(ref _hueRotationDegrees, value, -180, 180);
    }

    /// <summary>恢复与现有 mpv 视频快照一致的默认草稿。</summary>
    public void Reset()
    {
        BrightnessPercent = MpvVideoEffectSnapshot.Default.BrightnessPercent;
        ContrastPercent = MpvVideoEffectSnapshot.Default.ContrastPercent;
        SaturationPercent = MpvVideoEffectSnapshot.Default.SaturationPercent;
        HueRotationDegrees = MpvVideoEffectSnapshot.Default.HueRotationDegrees;
    }

    /// <summary>
    /// 将草稿映射为已有的有限视频快照。调用方仍需决定模式并显式提交。
    /// </summary>
    public bool TryCreateSnapshot(
        MpvVideoProcessingMode mode,
        out MpvVideoEffectSnapshot? snapshot,
        out MpvVideoParameterError? error) =>
        mode is MpvVideoProcessingMode.Gpu83
            ? MpvVideoEffectSnapshot.TryCreateGpu83BaselineColor(
                BrightnessPercent,
                ContrastPercent,
                SaturationPercent,
                HueRotationDegrees,
                out snapshot,
                out error)
            : MpvVideoEffectSnapshot.TryCreate(
                mode,
                BrightnessPercent,
                ContrastPercent,
                SaturationPercent,
                HueRotationDegrees,
                MpvShaderOptionsSnapshot.Empty,
                out snapshot,
                out error);

    private void SetValue(
        ref double field,
        double value,
        double minimum,
        double maximum,
        [CallerMemberName] string? propertyName = null)
    {
        if (!double.IsFinite(value))
        {
            value = minimum;
        }

        var normalized = Math.Clamp(value, minimum, maximum);
        if (Math.Abs(field - normalized) < 0.0001)
        {
            return;
        }

        field = normalized;
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
    }
}
