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
}
