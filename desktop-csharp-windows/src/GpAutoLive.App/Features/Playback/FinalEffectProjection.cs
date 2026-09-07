namespace GpAutoLive.App.Features.Playback;

/// <summary>最终效果窗口允许展示的表面类型。</summary>
public enum FinalEffectSurfaceKind
{
    None,
    VideoHwndReserved,
    AudioBlack
}

/// <summary>
/// 传给最终效果窗口的最小脱敏表面投影。
/// 播放控制和状态信息由主窗口负责，不在最终效果窗重复呈现。
/// </summary>
public sealed record FinalEffectSnapshot(
    FinalEffectSurfaceKind SurfaceKind,
    uint? VideoWidth = null,
    uint? VideoHeight = null)
{
    public static FinalEffectSnapshot Empty { get; } = new(FinalEffectSurfaceKind.None);

    public static FinalEffectSnapshot Create(
        FinalEffectSurfaceKind surfaceKind,
        uint? videoWidth = null,
        uint? videoHeight = null) =>
        new(surfaceKind, videoWidth, videoHeight);
}

/// <summary>最终效果窗口的纯状态控制器；窗口本身只负责投影和事件。</summary>
public sealed class FinalEffectWindowController
{
    private FinalEffectSnapshot _snapshot = FinalEffectSnapshot.Empty;

    public FinalEffectWindowState State { get; private set; } = FinalEffectWindowState.Closed;

    public FinalEffectSnapshot Snapshot => _snapshot;

    public bool IsOpen => State is FinalEffectWindowState.Open;

    public event EventHandler? StateChanged;

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

}

public enum FinalEffectWindowState
{
    Closed,
    Open
}
