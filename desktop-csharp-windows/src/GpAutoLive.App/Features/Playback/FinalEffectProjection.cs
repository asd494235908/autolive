using GpAutoLive.Contracts;

namespace GpAutoLive.App.Features.Playback;

/// <summary>最终效果窗口允许展示的表面类型。</summary>
public enum FinalEffectSurfaceKind
{
    None,
    VideoHwndReserved,
    AudioBlack
}

/// <summary>最终效果窗口允许发出的最小播放控制。</summary>
public enum FinalEffectPlaybackCommand
{
    TogglePlayPause,
    Stop
}

/// <summary>
/// 传给最终效果窗口的脱敏播放快照。
/// 不携带路径、文件名、凭据、HTTP 信息或媒体处理参数。
/// </summary>
public sealed record FinalEffectSnapshot(
    PlaybackState PlaybackState,
    FinalEffectSurfaceKind SurfaceKind,
    TimeSpan Position,
    TimeSpan Duration,
    ulong PlaybackGeneration,
    ulong SourceRevision,
    ulong LoopIndex)
{
    public static FinalEffectSnapshot Empty { get; } = new(
        PlaybackState.Stopped,
        FinalEffectSurfaceKind.None,
        TimeSpan.Zero,
        TimeSpan.Zero,
        0,
        0,
        0);

    public bool HasMedia => SurfaceKind is not FinalEffectSurfaceKind.None;

    public bool IsPureAudio => SurfaceKind is FinalEffectSurfaceKind.AudioBlack;

    public string SurfaceLabel => SurfaceKind switch
    {
        FinalEffectSurfaceKind.VideoHwndReserved => "视频表面 · Windows HWND",
        FinalEffectSurfaceKind.AudioBlack => "纯音频 · 黑色表面",
        _ => "无活动媒体"
    };

    public string PlaybackLabel => PlaybackState switch
    {
        PlaybackState.Playing => "播放中",
        PlaybackState.Paused => "已暂停",
        PlaybackState.Ready => "待播放",
        _ => "已停止"
    };

    public double Progress => Duration <= TimeSpan.Zero
        ? 0
        : Math.Clamp(Position.TotalMilliseconds / Duration.TotalMilliseconds, 0, 1);

    public static FinalEffectSnapshot Create(
        PlaybackState playbackState,
        FinalEffectSurfaceKind surfaceKind,
        TimeSpan position,
        TimeSpan duration,
        ulong playbackGeneration,
        ulong sourceRevision,
        ulong loopIndex)
    {
        if (position < TimeSpan.Zero || duration < TimeSpan.Zero)
        {
            throw new ArgumentOutOfRangeException(nameof(position), "播放位置和总时长必须为非负值。");
        }

        return new FinalEffectSnapshot(
            playbackState,
            surfaceKind,
            position,
            duration,
            playbackGeneration,
            sourceRevision,
            loopIndex);
    }
}

/// <summary>最终效果窗口的纯状态控制器；窗口本身只负责投影和事件。</summary>
public sealed class FinalEffectWindowController
{
    private FinalEffectSnapshot _snapshot = FinalEffectSnapshot.Empty;

    public FinalEffectWindowState State { get; private set; } = FinalEffectWindowState.Closed;

    public FinalEffectSnapshot Snapshot => _snapshot;

    public bool IsOpen => State is FinalEffectWindowState.Open;

    public event EventHandler? StateChanged;

    public event EventHandler<FinalEffectCommandRequestedEventArgs>? CommandRequested;

    public bool Open(FinalEffectSnapshot snapshot)
    {
        ArgumentNullException.ThrowIfNull(snapshot);

        var wasOpen = IsOpen;
        _snapshot = snapshot;
        State = FinalEffectWindowState.Open;
        if (!wasOpen)
        {
            StateChanged?.Invoke(this, EventArgs.Empty);
        }

        return !wasOpen;
    }

    public bool Update(FinalEffectSnapshot snapshot)
    {
        ArgumentNullException.ThrowIfNull(snapshot);
        if (!IsOpen)
        {
            return false;
        }

        _snapshot = snapshot;
        StateChanged?.Invoke(this, EventArgs.Empty);
        return true;
    }

    public bool Close()
    {
        if (!IsOpen)
        {
            return false;
        }

        State = FinalEffectWindowState.Closed;
        _snapshot = FinalEffectSnapshot.Empty;
        StateChanged?.Invoke(this, EventArgs.Empty);
        return true;
    }

    public bool RequestCommand(FinalEffectPlaybackCommand command)
    {
        if (!IsOpen)
        {
            return false;
        }

        CommandRequested?.Invoke(this, new FinalEffectCommandRequestedEventArgs(command));
        return true;
    }
}

public enum FinalEffectWindowState
{
    Closed,
    Open
}

public sealed class FinalEffectCommandRequestedEventArgs(FinalEffectPlaybackCommand command) : EventArgs
{
    public FinalEffectPlaybackCommand Command { get; } = command;
}
