namespace GpAutoLive.App.Features.Playback;

/// <summary>将播放器的毫秒位置安全格式化为 GUI 可读时长。</summary>
public static class PlaybackTimeFormatter
{
    public static string Format(ulong? milliseconds)
    {
        if (milliseconds is not ulong value)
        {
            return "—";
        }

        var bounded = Math.Min(
            value,
            (ulong)(TimeSpan.MaxValue.Ticks / TimeSpan.TicksPerMillisecond));
        var time = TimeSpan.FromMilliseconds(bounded);
        return time.ToString(@"hh\:mm\:ss");
    }
}
