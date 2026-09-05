using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>Windows mpv 组合运行时的生命周期状态。</summary>
public enum WindowsMpvPlaybackRuntimeState
{
    Ready,
    Starting,
    Running,
    Stopping,
    Stopped,
    Faulted,
    Closed,
}

/// <summary>Windows mpv 组合运行时的稳定错误分类。</summary>
public enum WindowsMpvPlaybackRuntimeFailureCode
{
    InvalidBinding,
    AlreadyRunning,
    HostStartFailed,
    IpcConnectFailed,
    NotRunning,
    StopFailed,
    Cancelled,
}

/// <summary>不包含路径、命令行或外部异常正文的组合运行时错误。</summary>
public sealed record WindowsMpvPlaybackRuntimeError(
    WindowsMpvPlaybackRuntimeFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>进程宿主与 IPC 连接的最小脱敏快照。</summary>
public sealed record WindowsMpvPlaybackRuntimeSnapshot(
    WindowsMpvPlaybackRuntimeState State,
    WindowsMpvHostSnapshot Host,
    MpvIpcPipeState IpcState,
    GpAutoLive.Core.MediaPlaybackIdentity? ActiveIdentity);

/// <summary>组合运行时的启动/停止结果。</summary>
public sealed record WindowsMpvPlaybackRuntimeResult(
    bool IsSuccess,
    WindowsMpvPlaybackRuntimeSnapshot Snapshot,
    WindowsMpvPlaybackRuntimeError? Error = null);

/// <summary>
/// 收敛 mpv 的受管进程、命名管道和会话身份。
/// 启动计划只能来自 <see cref="MpvLaunchPlan.TryCreate"/>；本类型负责顺序、取消和释放，
/// 不接受任意命令行，也不把 IPC 原始帧传播到 GUI。
/// </summary>
public sealed class WindowsMpvPlaybackRuntime : IAsyncDisposable
{
    private static readonly TimeSpan CleanupTimeout = TimeSpan.FromSeconds(2);
    private readonly object _gate = new();
    private readonly SemaphoreSlim _lifecycle = new(1, 1);
    private readonly WindowsMpvProcessHost _host = new();
    private MpvPlaybackIpcGateway? _gateway;
    private MpvPlaybackStateMonitor? _stateMonitor;
    private MpvPlaybackSession? _session;
    private WindowsMpvPlaybackRuntimeState _state = WindowsMpvPlaybackRuntimeState.Ready;
    private bool _disposed;

    public WindowsMpvPlaybackRuntimeSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                var hostSnapshot = _host.Snapshot;
                var projectedState = _state is WindowsMpvPlaybackRuntimeState.Running
                    && hostSnapshot.State is not WindowsMpvHostState.Running
                    ? WindowsMpvPlaybackRuntimeState.Faulted
                    : _state;
                return new(
                    projectedState,
                    hostSnapshot,
                    _gateway?.Client.State ?? MpvIpcPipeState.Disconnected,
                    _session?.Snapshot.ActiveSource?.Identity);
            }
        }
    }

    /// <summary>
    /// 启动 mpv，连接启动计划中指定的命名管道，并保持会话身份绑定。
    /// mpv 的启动参数包含首个媒体源，因此连接成功后不会重复发送 loadfile。
    /// </summary>
    public async Task<WindowsMpvPlaybackRuntimeResult> StartAsync(
        MpvLaunchPlan? plan,
        MpvPlaybackSession? session,
        MpvIpcPipeOptions? pipeOptions = null,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(
                WindowsMpvPlaybackRuntimeFailureCode.Cancelled,
                "mpv 启动已取消。",
                retryable: true);
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(
                WindowsMpvPlaybackRuntimeFailureCode.Cancelled,
                "mpv 启动已取消。",
                retryable: true);
        }

        try
        {
            if (_disposed)
            {
                return Failure(
                    WindowsMpvPlaybackRuntimeFailureCode.InvalidBinding,
                    "mpv 组合运行时已关闭。",
                    retryable: false);
            }

            if (_gateway is not null || _host.Snapshot.State is WindowsMpvHostState.Starting or WindowsMpvHostState.Running)
            {
                return Failure(
                    WindowsMpvPlaybackRuntimeFailureCode.AlreadyRunning,
                    "mpv 会话已经在运行。",
                    retryable: false);
            }

            if (!TryValidateBinding(plan, session))
            {
                return Failure(
                    WindowsMpvPlaybackRuntimeFailureCode.InvalidBinding,
                    "mpv 启动计划与活动会话不匹配。",
                    retryable: false);
            }

            lock (_gate)
            {
                _state = WindowsMpvPlaybackRuntimeState.Starting;
            }

            var gateway = new MpvPlaybackIpcGateway(
                session!,
                new MpvNamedPipeClient(plan!.IpcEndpoint, pipeOptions));
            var hostResult = await _host.StartAsync(plan, cancellationToken).ConfigureAwait(false);
            if (!hostResult.IsSuccess)
            {
                await gateway.DisposeAsync().ConfigureAwait(false);
                lock (_gate)
                {
                    _state = WindowsMpvPlaybackRuntimeState.Faulted;
                }

                return Failure(
                    WindowsMpvPlaybackRuntimeFailureCode.HostStartFailed,
                    hostResult.Error?.Message ?? "mpv 进程无法启动。",
                    retryable: hostResult.Error?.Retryable ?? true);
            }

            var connection = await gateway.ConnectAsync(cancellationToken).ConfigureAwait(false);
            if (!connection.IsSuccess)
            {
                await gateway.DisposeAsync().ConfigureAwait(false);
                await _host.StopAsync().ConfigureAwait(false);
                lock (_gate)
                {
                    _state = WindowsMpvPlaybackRuntimeState.Faulted;
                }

                return Failure(
                    WindowsMpvPlaybackRuntimeFailureCode.IpcConnectFailed,
                    connection.Error?.Message ?? "mpv IPC 无法连接。",
                    retryable: connection.Error?.Retryable ?? true);
            }

            lock (_gate)
            {
                _gateway = gateway;
                MpvPlaybackStateMonitor.TryCreate(
                    gateway,
                    options: null,
                    out _stateMonitor,
                    out _);
                _session = session;
                _state = WindowsMpvPlaybackRuntimeState.Running;
            }

            return Succeeded();
        }
        catch (ArgumentException)
        {
            lock (_gate)
            {
                _state = WindowsMpvPlaybackRuntimeState.Faulted;
            }

            return Failure(
                WindowsMpvPlaybackRuntimeFailureCode.InvalidBinding,
                "mpv IPC 配置无效。",
                retryable: false);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>在当前活动源身份下发送一条已固定的 IPC 命令。</summary>
    public async Task<MpvIpcDispatchResult> DispatchAsync(
        MpvIpcCommand? command,
        GpAutoLive.Core.MediaPlaybackIdentity expectedIdentity,
        CancellationToken cancellationToken = default)
    {
        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return MpvIpcDispatchResult.Failed(sessionError: new(
                MpvSessionFailureCode.InvalidStateTransition,
                "mpv IPC 操作已取消。",
                Retryable: true));
        }

        try
        {
            MpvPlaybackIpcGateway? gateway;
            lock (_gate)
            {
                gateway = _gateway;
                if (_state is not WindowsMpvPlaybackRuntimeState.Running
                    || _host.Snapshot.State is not WindowsMpvHostState.Running
                    || gateway is null)
                {
                    return MpvIpcDispatchResult.Failed(sessionError: new(
                        MpvSessionFailureCode.SessionClosed,
                        "mpv 会话当前不可用。",
                        Retryable: true));
                }
            }

            return await gateway.DispatchAsync(command, expectedIdentity, cancellationToken).ConfigureAwait(false);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>
    /// 在当前受管 mpv 会话内读取一轮播放状态。
    /// 生命周期信号量覆盖整个 IPC 读取，Stop/Dispose 因而不会与监视器并发释放同一个网关。
    /// </summary>
    public Task<MpvPlaybackStateMonitorResult> PollPlaybackStateAsync(
        MediaPlaybackIdentity? expectedIdentity,
        CancellationToken cancellationToken = default) =>
        PollMonitorOnceAsync(expectedMonitor: null, expectedIdentity, cancellationToken);

    /// <summary>
    /// 以调用方持有的取消令牌消费状态快照。迭代器不创建后台任务或无界队列；
    /// 停止/关闭运行时后，下一轮在接触网关前结束，避免使用已释放的 IPC 对象。
    /// </summary>
    public async IAsyncEnumerable<MpvPlaybackStateMonitorResult> WatchPlaybackStateAsync(
        MediaPlaybackIdentity? expectedIdentity,
        [System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        if (expectedIdentity is null)
        {
            yield return MonitorFailure(
                MpvPlaybackStateMonitorFailureCode.InvalidInput,
                "mpv 播放状态监视身份不能为空。",
                retryable: false);
            yield break;
        }

        if (cancellationToken.IsCancellationRequested)
        {
            yield return MonitorFailure(
                MpvPlaybackStateMonitorFailureCode.Cancelled,
                "mpv 播放状态监视已取消。",
                retryable: true);
            yield break;
        }

        MpvPlaybackStateMonitor? monitor;
        var captureCancelled = false;
        try
        {
            monitor = await CaptureStateMonitorAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            monitor = null;
            captureCancelled = true;
        }

        if (captureCancelled)
        {
            yield return MonitorFailure(
                MpvPlaybackStateMonitorFailureCode.Cancelled,
                "mpv 播放状态监视已取消。",
                retryable: true);
            yield break;
        }

        if (monitor is null)
        {
            yield return RuntimeUnavailable();
            yield break;
        }

        MpvPlaybackStateSnapshot? previous = null;
        while (true)
        {
            var polled = await PollMonitorOnceAsync(
                monitor,
                expectedIdentity,
                cancellationToken).ConfigureAwait(false);
            if (!polled.IsSuccess || polled.Snapshot is null)
            {
                yield return polled;
                yield break;
            }

            var boundary = await VerifyMonitorBoundaryAsync(
                monitor,
                cancellationToken).ConfigureAwait(false);
            if (boundary is not null)
            {
                yield return boundary;
                yield break;
            }

            var applied = MpvPlaybackStateMonitor.ApplySnapshot(
                previous,
                expectedIdentity,
                GetActiveIdentity(),
                polled.Snapshot);
            yield return applied;
            if (!applied.IsSuccess)
            {
                yield break;
            }

            previous = applied.Snapshot;
            var delayCancelled = false;
            try
            {
                await Task.Delay(monitor.PollInterval, cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                delayCancelled = true;
            }

            if (delayCancelled)
            {
                yield return MonitorFailure(
                    MpvPlaybackStateMonitorFailureCode.Cancelled,
                    "mpv 播放状态监视已取消。",
                    retryable: true);
                yield break;
            }
        }
    }

    /// <summary>先发送固定 quit（若连接仍可用），再有界停止宿主并释放 IPC。</summary>
    public async Task<WindowsMpvPlaybackRuntimeResult> StopAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(
                WindowsMpvPlaybackRuntimeFailureCode.Cancelled,
                "mpv 停止已取消。",
                retryable: true);
        }

        try
        {
            if (_disposed)
            {
                return Succeeded();
            }

            return await StopCoreAsync().ConfigureAwait(false);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    public async ValueTask DisposeAsync()
    {
        try
        {
            await _lifecycle.WaitAsync().ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return;
        }

        try
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            await StopCoreAsync().ConfigureAwait(false);
            await _host.DisposeAsync().ConfigureAwait(false);
            lock (_gate)
            {
                _state = WindowsMpvPlaybackRuntimeState.Closed;
            }
        }
        finally
        {
            _lifecycle.Release();
        }

        GC.SuppressFinalize(this);
    }

    private async Task<WindowsMpvPlaybackRuntimeResult> StopCoreAsync()
    {
        using var cleanupCancellation = new CancellationTokenSource(CleanupTimeout);
        MpvPlaybackIpcGateway? gateway;
        MpvPlaybackSession? session;
        lock (_gate)
        {
            gateway = _gateway;
            session = _session;
            _stateMonitor = null;
            _state = WindowsMpvPlaybackRuntimeState.Stopping;
        }

        if (gateway is not null)
        {
            var identity = session?.Snapshot.ActiveSource?.Identity;
            if (identity is GpAutoLive.Core.MediaPlaybackIdentity activeIdentity)
            {
                _ = await gateway.DispatchAsync(
                    MpvIpcCommand.Quit(),
                    activeIdentity,
                    cleanupCancellation.Token).ConfigureAwait(false);
                _ = session?.Close();
            }

            await gateway.DisposeAsync().ConfigureAwait(false);
        }

        lock (_gate)
        {
            _gateway = null;
            _session = null;
        }

        var stopped = await _host.StopAsync().ConfigureAwait(false);
        lock (_gate)
        {
            _state = stopped.IsSuccess
                ? (_disposed ? WindowsMpvPlaybackRuntimeState.Closed : WindowsMpvPlaybackRuntimeState.Stopped)
                : WindowsMpvPlaybackRuntimeState.Faulted;
        }

        return stopped.IsSuccess
            ? Succeeded()
            : Failure(
                WindowsMpvPlaybackRuntimeFailureCode.StopFailed,
                stopped.Error?.Message ?? "mpv 未能在停止预算内退出。",
                retryable: false);
    }

    private static bool TryValidateBinding(MpvLaunchPlan? plan, MpvPlaybackSession? session)
    {
        var active = session?.Snapshot.ActiveSource;
        return plan is not null
            && session is not null
            && active is not null
            && string.Equals(
                active.MediaPath.CanonicalPath,
                plan.MediaPath.CanonicalPath,
                StringComparison.OrdinalIgnoreCase)
            && active.MediaPath.Kind is MediaKind.Video
            && plan.IpcEndpoint.PipePath.Length > 0;
    }

    private WindowsMpvPlaybackRuntimeResult Succeeded() => new(true, Snapshot);

    private WindowsMpvPlaybackRuntimeResult Failure(
        WindowsMpvPlaybackRuntimeFailureCode code,
        string message,
        bool retryable) =>
        new(false, Snapshot, new(code, message, retryable));

    private async Task<MpvPlaybackStateMonitorResult> PollMonitorOnceAsync(
        MpvPlaybackStateMonitor? expectedMonitor,
        MediaPlaybackIdentity? expectedIdentity,
        CancellationToken cancellationToken)
    {
        if (expectedIdentity is null)
        {
            return MonitorFailure(
                MpvPlaybackStateMonitorFailureCode.InvalidInput,
                "mpv 播放状态监视身份不能为空。",
                retryable: false);
        }

        if (cancellationToken.IsCancellationRequested)
        {
            return MonitorFailure(
                MpvPlaybackStateMonitorFailureCode.Cancelled,
                "mpv 播放状态监视已取消。",
                retryable: true);
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return MonitorFailure(
                MpvPlaybackStateMonitorFailureCode.Cancelled,
                "mpv 播放状态监视已取消。",
                retryable: true);
        }

        try
        {
            MpvPlaybackStateMonitor? monitor;
            lock (_gate)
            {
                monitor = _stateMonitor;
                if (_disposed || _state is WindowsMpvPlaybackRuntimeState.Closed)
                {
                    return RuntimeUnavailable(closed: true);
                }

                if (_state is not WindowsMpvPlaybackRuntimeState.Running
                    || _host.Snapshot.State is not WindowsMpvHostState.Running
                    || monitor is null
                    || expectedMonitor is not null && !ReferenceEquals(expectedMonitor, monitor))
                {
                    return RuntimeUnavailable();
                }
            }

            return await monitor.PollOnceAsync(expectedIdentity, cancellationToken)
                .ConfigureAwait(false);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    private async Task<MpvPlaybackStateMonitor?> CaptureStateMonitorAsync(
        CancellationToken cancellationToken)
    {
        await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            lock (_gate)
            {
                return !_disposed
                    && _state is WindowsMpvPlaybackRuntimeState.Running
                    && _host.Snapshot.State is WindowsMpvHostState.Running
                    ? _stateMonitor
                    : null;
            }
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    private async Task<MpvPlaybackStateMonitorResult?> VerifyMonitorBoundaryAsync(
        MpvPlaybackStateMonitor expectedMonitor,
        CancellationToken cancellationToken)
    {
        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return MonitorFailure(
                MpvPlaybackStateMonitorFailureCode.Cancelled,
                "mpv 播放状态监视已取消。",
                retryable: true);
        }

        try
        {
            lock (_gate)
            {
                if (_disposed || _state is WindowsMpvPlaybackRuntimeState.Closed)
                {
                    return RuntimeUnavailable(closed: true);
                }

                if (_state is not WindowsMpvPlaybackRuntimeState.Running
                    || _host.Snapshot.State is not WindowsMpvHostState.Running
                    || !ReferenceEquals(expectedMonitor, _stateMonitor))
                {
                    return RuntimeUnavailable();
                }

                if (_session?.Snapshot.ActiveSource?.Identity is not MediaPlaybackIdentity)
                {
                    return RuntimeUnavailable();
                }
            }

            return null;
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    private MediaPlaybackIdentity? GetActiveIdentity()
    {
        lock (_gate)
        {
            return _session?.Snapshot.ActiveSource?.Identity;
        }
    }

    private static MpvPlaybackStateMonitorResult RuntimeUnavailable(bool closed = false) =>
        MonitorFailure(
            MpvPlaybackStateMonitorFailureCode.RuntimeUnavailable,
            closed
                ? "mpv 播放状态监视运行时已关闭。"
                : "mpv 播放状态监视运行时当前不可用。",
            retryable: !closed);

    private static MpvPlaybackStateMonitorResult MonitorFailure(
        MpvPlaybackStateMonitorFailureCode code,
        string message,
        bool retryable) =>
        MpvPlaybackStateMonitorResult.Failed(new(code, message, retryable));
}
