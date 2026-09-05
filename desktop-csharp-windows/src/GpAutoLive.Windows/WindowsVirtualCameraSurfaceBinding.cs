namespace GpAutoLive.Windows;

/// <summary>最终效果视频表面 HWND 绑定操作结果。</summary>
public enum WindowsVirtualCameraSurfaceBindingCode
{
    Bound,
    Unchanged,
    Unbound,
    InvalidHandle,
    Closed
}

/// <summary>脱敏的最终效果表面绑定快照；代际用于拒绝迟到捕获结果。</summary>
public sealed record WindowsVirtualCameraSurfaceBindingSnapshot(
    uint? WindowId,
    ulong Generation,
    bool IsBound,
    bool IsClosed);

/// <summary>绑定操作的稳定结果。</summary>
public sealed record WindowsVirtualCameraSurfaceBindingResult(
    bool IsSuccess,
    WindowsVirtualCameraSurfaceBindingCode Code,
    WindowsVirtualCameraSurfaceBindingSnapshot Snapshot);

/// <summary>
/// 虚拟摄像头只允许绑定唯一最终效果窗口。它不捕获像素、不创建 WGC 资源，
/// 只维护 HWND 变化与代际，供后续捕获实现拒绝旧会话结果。
/// </summary>
public sealed class WindowsVirtualCameraSurfaceBinding : IDisposable
{
    private readonly object _gate = new();
    private uint? _windowId;
    private ulong _generation = 1;
    private bool _closed;

    /// <summary>当前绑定快照。</summary>
    public WindowsVirtualCameraSurfaceBindingSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshot();
            }
        }
    }

    /// <summary>绑定有效的 Win32 HWND；同一 HWND 重复绑定不会递增代际。</summary>
    public WindowsVirtualCameraSurfaceBindingResult Bind(uint windowId)
    {
        lock (_gate)
        {
            if (_closed)
            {
                return Failure(WindowsVirtualCameraSurfaceBindingCode.Closed);
            }

            if (windowId == 0)
            {
                return Failure(WindowsVirtualCameraSurfaceBindingCode.InvalidHandle);
            }

            if (_windowId == windowId)
            {
                return Success(WindowsVirtualCameraSurfaceBindingCode.Unchanged);
            }

            _windowId = windowId;
            AdvanceGeneration();
            return Success(WindowsVirtualCameraSurfaceBindingCode.Bound);
        }
    }

    /// <summary>解除当前 HWND；有实际绑定时递增代际使旧捕获结果失效。</summary>
    public WindowsVirtualCameraSurfaceBindingResult Unbind()
    {
        lock (_gate)
        {
            if (_closed)
            {
                return Failure(WindowsVirtualCameraSurfaceBindingCode.Closed);
            }

            if (_windowId is null)
            {
                return Success(WindowsVirtualCameraSurfaceBindingCode.Unbound);
            }

            _windowId = null;
            AdvanceGeneration();
            return Success(WindowsVirtualCameraSurfaceBindingCode.Unbound);
        }
    }

    /// <summary>判断捕获回调是否仍属于当前 HWND 和代际。</summary>
    public bool IsCurrent(uint windowId, ulong generation)
    {
        lock (_gate)
        {
            return !_closed
                && _windowId == windowId
                && _generation == generation;
        }
    }

    /// <inheritdoc />
    public void Dispose()
    {
        lock (_gate)
        {
            if (_closed)
            {
                return;
            }

            if (_windowId is not null)
            {
                _windowId = null;
                AdvanceGeneration();
            }

            _closed = true;
        }

        GC.SuppressFinalize(this);
    }

    private WindowsVirtualCameraSurfaceBindingResult Success(WindowsVirtualCameraSurfaceBindingCode code) =>
        new(true, code, CreateSnapshot());

    private WindowsVirtualCameraSurfaceBindingResult Failure(WindowsVirtualCameraSurfaceBindingCode code) =>
        new(false, code, CreateSnapshot());

    private WindowsVirtualCameraSurfaceBindingSnapshot CreateSnapshot() =>
        new(_windowId, _generation, _windowId is not null, _closed);

    private void AdvanceGeneration() =>
        _generation = _generation == ulong.MaxValue ? 1 : _generation + 1;
}
