using GpAutoLive.Media;

namespace GpAutoLive.App.Features.Effects;

/// <summary>选择视频会话启动路径；处理关闭优先于运行时能力。</summary>
public static class VideoPlaybackModeSelector
{
    public static MpvLaunchMode Select(bool processingEnabled, bool fullGpu83Runtime) =>
        !processingEnabled
            ? MpvLaunchMode.Original
            : fullGpu83Runtime
                ? MpvLaunchMode.Gpu83
                : MpvLaunchMode.Cpu4;

    public static string DescribeActive(MpvVideoProcessingMode? mode) => mode switch
    {
        MpvVideoProcessingMode.Gpu83 => "GPU·GPU83",
        MpvVideoProcessingMode.Cpu4 => "CPU·CPU4",
        MpvVideoProcessingMode.Original => "未处理·Original",
        _ => "等待视频",
    };

    public static bool RequiresSessionRestart(
        MpvVideoProcessingMode? activeMode,
        MpvVideoProcessingMode requestedMode) =>
        activeMode is null || activeMode.Value != requestedMode;

    public static MpvLaunchMode ToLaunchMode(MpvVideoProcessingMode mode) => mode switch
    {
        MpvVideoProcessingMode.Gpu83 => MpvLaunchMode.Gpu83,
        MpvVideoProcessingMode.Cpu4 => MpvLaunchMode.Cpu4,
        MpvVideoProcessingMode.Original => MpvLaunchMode.Original,
        _ => throw new ArgumentOutOfRangeException(nameof(mode), mode, "未知的视频处理模式。"),
    };
}
