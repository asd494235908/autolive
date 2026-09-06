using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Windows;

/// <summary>虚拟摄像头完整输出组合的稳定结果码。</summary>
public enum WindowsVirtualCameraOutputCoordinatorCode
{
    Started,
    AlreadyStarted,
    InvalidState,
    InvalidPlan,
    SidecarStartFailed,
    SidecarConnectFailed,
    CaptureFailed,
    WriterStartFailed,
    Cancelled,
    Stopped,
    CleanupFailed,
    Closed
}

/// <summary>虚拟摄像头完整输出组合的脱敏快照。</summary>
public sealed record WindowsVirtualCameraOutputCoordinatorSnapshot(
    VirtualCameraStatus Output,
    WindowsGraphicsCaptureWindowSessionSnapshot Capture,
    WindowsVirtualCameraSidecarHostSnapshot Sidecar,
    WindowsVirtualCameraSidecarClientSnapshot Client,
    WindowsVirtualCameraSidecarOutputWriterSnapshot Writer);

/// <summary>虚拟摄像头完整输出组合的操作结果。</summary>
public sealed record WindowsVirtualCameraOutputCoordinatorResult(
    bool IsSuccess,
    WindowsVirtualCameraOutputCoordinatorCode Code,
    WindowsVirtualCameraOutputCoordinatorSnapshot Snapshot,
    string? ErrorMessage = null);

/// <summary>
/// 收敛 AkVirtualCamera sidecar、Named Pipe、WGC/D3D11 GPU 转换和 30fps 输出泵的生命周期。
/// 该组合不安装 DirectShow 设备、不创建 sidecar ACL；安装器和 sidecar 继续拥有各自边界。
/// </summary>
public sealed class WindowsVirtualCameraOutputCoordinator : IAsyncDisposable
{
    private static readonly TimeSpan DefaultConnectTimeout = TimeSpan.FromSeconds(5);

    private readonly object _gate = new();
    private readonly SemaphoreSlim _lifecycle = new(1, 1);
    private readonly VirtualCameraOutputManager _output;
    private readonly WindowsVirtualCameraSidecarHost _sidecarHost;
    private readonly WindowsVirtualCameraSidecarClient _sidecarClient;
    private readonly WindowsVirtualCameraGpuOutputSession _gpuSession;
    private WindowsVirtualCameraSidecarOutputWriter? _writer;
    private CancellationTokenSource? _healthCancellation;
    private Task? _healthTask;
    private bool _started;
    private bool _starting;
    private bool _cleanupFailed;
    private bool _closed;

    /// <summary>创建完整输出组合；调用方仍需先把已安装设备标记为 Installed。</summary>
    public WindowsVirtualCameraOutputCoordinator(
        VirtualCameraOutputManager output,
        WindowsVirtualCameraSurfaceBinding binding)
    {
        _output = output ?? throw new ArgumentNullException(nameof(output));
        _sidecarHost = new WindowsVirtualCameraSidecarHost();
        _sidecarClient = new WindowsVirtualCameraSidecarClient();
        _gpuSession = new WindowsVirtualCameraGpuOutputSession(output, binding);
        _sidecarHost.DownstreamClientCountChanged += SidecarHost_DownstreamClientCountChanged;
    }

    /// <summary>运行时组合状态发生确定性变化时通知 UI；观察者异常不会影响资源回收。</summary>
    public event EventHandler<WindowsVirtualCameraOutputCoordinatorSnapshot>? SnapshotChanged;

    /// <summary>读取组合内所有受控组件的脱敏快照。</summary>
    public WindowsVirtualCameraOutputCoordinatorSnapshot Snapshot => new(
        _output.Snapshot,
        _gpuSession.Snapshot.Capture,
        _sidecarHost.Snapshot,
        _sidecarClient.Snapshot,
        _writer?.Snapshot ?? new(
            WindowsVirtualCameraSidecarOutputWriterState.Ready,
            0,
            0,
            0,
            0,
            null));

    /// <summary>
    /// 判断是否仍有需要停止的输出资源，故障态也保留停止入口以完成幂等清理。
    /// </summary>
    public bool HasActiveResources
    {
        get
        {
            var snapshot = Snapshot;
            lock (_gate)
            {
                return _started
                    || _starting
                    || snapshot.Output.State == VirtualCameraState.Failed
                    || snapshot.Capture.Code is WindowsGraphicsCaptureWindowSessionCode.Starting
                        or WindowsGraphicsCaptureWindowSessionCode.Running
                    || snapshot.Sidecar.State is WindowsVirtualCameraSidecarHostState.Starting
                        or WindowsVirtualCameraSidecarHostState.Running
                        or WindowsVirtualCameraSidecarHostState.Exited
                        or WindowsVirtualCameraSidecarHostState.Failed
                    || snapshot.Client.State is WindowsVirtualCameraSidecarClientState.Connecting
                        or WindowsVirtualCameraSidecarClientState.Connected
                        or WindowsVirtualCameraSidecarClientState.Failed
                    || snapshot.Writer.State is WindowsVirtualCameraSidecarOutputWriterState.Starting
                        or WindowsVirtualCameraSidecarOutputWriterState.Running
                        or WindowsVirtualCameraSidecarOutputWriterState.Failed;
            }
        }
    }

    /// <summary>
    /// 启动受管 sidecar、连接其 Named Pipe、建立 WGC/GPU 会话并开始固定 30fps 输出。
    /// sidecar 计划必须由固定路径、x64 PE 和令牌校验构造器生成。
    /// </summary>
    public async Task<WindowsVirtualCameraOutputCoordinatorResult> StartAsync(
        WindowsVirtualCameraSidecarLaunchPlan? plan,
        TimeSpan? connectTimeout = null,
        TimeSpan? captureTimeout = null,
        CancellationToken cancellationToken = default)
    {
        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsVirtualCameraOutputCoordinatorCode.Cancelled, "虚拟摄像头输出启动已取消");
        }

        try
        {
            return await StartCoreAsync(plan, connectTimeout, captureTimeout, cancellationToken).ConfigureAwait(false);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    private async Task<WindowsVirtualCameraOutputCoordinatorResult> StartCoreAsync(
        WindowsVirtualCameraSidecarLaunchPlan? plan,
        TimeSpan? connectTimeout,
        TimeSpan? captureTimeout,
        CancellationToken cancellationToken)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(WindowsVirtualCameraOutputCoordinatorCode.Cancelled, "虚拟摄像头输出启动已取消");
        }

        lock (_gate)
        {
            if (_closed)
            {
                return Failure(WindowsVirtualCameraOutputCoordinatorCode.Closed, "虚拟摄像头输出组合已关闭");
            }

            if (_cleanupFailed)
            {
                return Failure(
                    WindowsVirtualCameraOutputCoordinatorCode.CleanupFailed,
                    "虚拟摄像头输出上一次清理未完成；请先重试停止");
            }

            if (_started || _starting)
            {
                return Failure(WindowsVirtualCameraOutputCoordinatorCode.AlreadyStarted, "虚拟摄像头输出组合已经在运行");
            }

            _starting = true;
        }

        WindowsVirtualCameraSidecarOutputWriter? writer = null;
        var hostAttempted = false;
        var clientAttempted = false;
        var gpuAttempted = false;
        try
        {
            if (_output.Snapshot.State != VirtualCameraState.Installed)
            {
                return Failure(WindowsVirtualCameraOutputCoordinatorCode.InvalidState, "虚拟摄像头设备尚未完成安装门禁");
            }

            if (plan is null)
            {
                return Failure(WindowsVirtualCameraOutputCoordinatorCode.InvalidPlan, "虚拟摄像头 sidecar 启动计划无效");
            }

            hostAttempted = true;
            var host = await _sidecarHost.StartAsync(plan, cancellationToken).ConfigureAwait(false);
            if (!host.IsSuccess)
            {
                var cleanup = await CleanupAsync(null, false, false, hostAttempted, CancellationToken.None).ConfigureAwait(false);
                return FailureAfterCleanup(
                    host.Error?.Code == WindowsVirtualCameraSidecarHostErrorCode.Cancelled
                        ? WindowsVirtualCameraOutputCoordinatorCode.Cancelled
                        : WindowsVirtualCameraOutputCoordinatorCode.SidecarStartFailed,
                    host.Error?.Message ?? "虚拟摄像头 sidecar 启动失败",
                    cleanup);
            }

            clientAttempted = true;
            var client = await _sidecarHost.ConnectClientAsync(
                    _sidecarClient,
                    connectTimeout ?? DefaultConnectTimeout,
                    cancellationToken)
                .ConfigureAwait(false);
            if (!client.IsSuccess)
            {
                var cleanup = await CleanupAsync(null, false, clientAttempted, hostAttempted, CancellationToken.None).ConfigureAwait(false);
                return FailureAfterCleanup(
                    client.Error?.Code == WindowsVirtualCameraSidecarClientErrorCode.Cancelled
                        ? WindowsVirtualCameraOutputCoordinatorCode.Cancelled
                        : WindowsVirtualCameraOutputCoordinatorCode.SidecarConnectFailed,
                    client.Error?.Message ?? "虚拟摄像头 sidecar 管道连接失败",
                    cleanup);
            }

            gpuAttempted = true;
            var capture = await _gpuSession.StartAsync(captureTimeout, cancellationToken).ConfigureAwait(false);
            if (!capture.IsSuccess)
            {
                var cleanup = await CleanupAsync(null, gpuAttempted, clientAttempted, hostAttempted, CancellationToken.None).ConfigureAwait(false);
                return FailureAfterCleanup(
                    capture.Code == WindowsVirtualCameraGpuOutputSessionCode.Cancelled
                        ? WindowsVirtualCameraOutputCoordinatorCode.Cancelled
                        : WindowsVirtualCameraOutputCoordinatorCode.CaptureFailed,
                    "虚拟摄像头 WGC/GPU 会话启动失败",
                    cleanup);
            }

            writer = new WindowsVirtualCameraSidecarOutputWriter(_output, _sidecarClient);
            var writerResult = await writer.StartAsync(cancellationToken).ConfigureAwait(false);
            if (!writerResult.IsSuccess)
            {
                var cleanup = await CleanupAsync(writer, gpuAttempted, clientAttempted, hostAttempted, CancellationToken.None).ConfigureAwait(false);
                return FailureAfterCleanup(
                    writerResult.Error?.Code == WindowsVirtualCameraSidecarOutputWriterErrorCode.Cancelled
                        ? WindowsVirtualCameraOutputCoordinatorCode.Cancelled
                        : WindowsVirtualCameraOutputCoordinatorCode.WriterStartFailed,
                    writerResult.Error?.Message ?? "虚拟摄像头输出泵启动失败",
                    cleanup);
            }

            lock (_gate)
            {
                _writer = writer;
                _starting = false;
                _started = true;
                StartHealthMonitorNoLock();
            }

            return Succeeded();
        }
        catch (OperationCanceledException)
        {
            var cleanup = await CleanupAsync(writer, gpuAttempted, clientAttempted, hostAttempted, CancellationToken.None).ConfigureAwait(false);
            return FailureAfterCleanup(
                WindowsVirtualCameraOutputCoordinatorCode.Cancelled,
                "虚拟摄像头输出启动已取消",
                cleanup);
        }
        catch (Exception)
        {
            var cleanup = await CleanupAsync(writer, gpuAttempted, clientAttempted, hostAttempted, CancellationToken.None).ConfigureAwait(false);
            return FailureAfterCleanup(
                WindowsVirtualCameraOutputCoordinatorCode.SidecarStartFailed,
                "虚拟摄像头输出启动失败",
                cleanup);
        }
        finally
        {
            ResetStartingIfNeeded();
        }
    }

    /// <summary>按 writer→GPU→client→sidecar→Core 顺序停止完整输出组合。</summary>
    public async Task<WindowsVirtualCameraOutputCoordinatorResult> StopAsync(
        CancellationToken cancellationToken = default)
    {
        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsVirtualCameraOutputCoordinatorCode.Cancelled, "虚拟摄像头输出停止已取消");
        }

        try
        {
            return await StopCoreAsync(cancellationToken).ConfigureAwait(false);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    private async Task<WindowsVirtualCameraOutputCoordinatorResult> StopCoreAsync(
        CancellationToken cancellationToken)
    {
        WindowsVirtualCameraSidecarOutputWriter? writer;
        Task? healthTask;
        CancellationTokenSource? healthCancellation;
        lock (_gate)
        {
            if (_closed)
            {
                return Failure(WindowsVirtualCameraOutputCoordinatorCode.Closed, "虚拟摄像头输出组合已关闭");
            }

            writer = _writer;
            _starting = false;
            _started = false;
            healthTask = _healthTask;
            healthCancellation = _healthCancellation;
            _healthTask = null;
            _healthCancellation = null;
        }

        await StopHealthMonitorAsync(healthTask, healthCancellation).ConfigureAwait(false);

        if (writer?.Snapshot.State == WindowsVirtualCameraSidecarOutputWriterState.Closed)
        {
            writer = null;
        }

        var cleanup = await CleanupAsync(writer, true, true, true, cancellationToken).ConfigureAwait(false);

        if (cleanup.Failures.Count > 0)
        {
            lock (_gate)
            {
                _cleanupFailed = true;
            }

            return Failure(
                WindowsVirtualCameraOutputCoordinatorCode.CleanupFailed,
                $"虚拟摄像头输出停止清理失败：{string.Join(",", cleanup.Failures)}");
        }

        if (cleanup.WasCancelled)
        {
            return Failure(WindowsVirtualCameraOutputCoordinatorCode.Cancelled, "虚拟摄像头输出停止已取消");
        }

        lock (_gate)
        {
            _cleanupFailed = false;
        }

        return new(true, WindowsVirtualCameraOutputCoordinatorCode.Stopped, Snapshot);
    }

    /// <inheritdoc />
    public async ValueTask DisposeAsync()
    {
        await _lifecycle.WaitAsync().ConfigureAwait(false);
        try
        {
            await DisposeCoreAsync().ConfigureAwait(false);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    private async Task DisposeCoreAsync()
    {
        Task? healthTask;
        CancellationTokenSource? healthCancellation;
        lock (_gate)
        {
            if (_closed)
            {
                return;
            }

            _closed = true;
            _starting = false;
            _started = false;
            healthTask = _healthTask;
            healthCancellation = _healthCancellation;
            _healthTask = null;
            _healthCancellation = null;
        }

        await StopHealthMonitorAsync(healthTask, healthCancellation).ConfigureAwait(false);

        _sidecarHost.DownstreamClientCountChanged -= SidecarHost_DownstreamClientCountChanged;

        WindowsVirtualCameraSidecarOutputWriter? writer;
        lock (_gate)
        {
            writer = _writer;
        }

        if (writer?.Snapshot.State == WindowsVirtualCameraSidecarOutputWriterState.Closed)
        {
            writer = null;
        }

        var cleanup = await CleanupAsync(writer, true, true, true, CancellationToken.None).ConfigureAwait(false);
        var failures = new List<string>(cleanup.Failures);

        async Task DisposeComponent(string name, Func<ValueTask> dispose)
        {
            try
            {
                await dispose().ConfigureAwait(false);
            }
            catch (Exception)
            {
                failures.Add($"{name}:Exception");
            }
        }

        await DisposeComponent("gpu.dispose", _gpuSession.DisposeAsync).ConfigureAwait(false);
        await DisposeComponent("client.dispose", _sidecarClient.DisposeAsync).ConfigureAwait(false);
        await DisposeComponent("sidecar.dispose", _sidecarHost.DisposeAsync).ConfigureAwait(false);
        if (failures.Count > 0)
        {
            throw new InvalidOperationException($"虚拟摄像头输出关闭清理失败：{string.Join(",", failures)}");
        }
    }

    private void SidecarHost_DownstreamClientCountChanged(object? sender, uint count)
    {
        var state = _output.Snapshot.State;
        if (state is VirtualCameraState.Ready or VirtualCameraState.Streaming)
        {
            var result = _output.SetDownstreamClientCount(count);
            if (result.IsSuccess)
            {
                PublishSnapshot();
            }
        }
    }

    /// <summary>主动刷新一次运行时健康状态，供 UI 或测试在需要时立即核对。</summary>
    public void RefreshHealth()
    {
        string? reason = null;
        lock (_gate)
        {
            if (!_started)
            {
                return;
            }

            var capture = _gpuSession.Snapshot.Capture;
            var sidecar = _sidecarHost.Snapshot;
            var client = _sidecarClient.Snapshot;
            var writer = _writer?.Snapshot;
            if (capture.Code is not (WindowsGraphicsCaptureWindowSessionCode.Starting
                or WindowsGraphicsCaptureWindowSessionCode.Running))
            {
                reason = $"WGC 会话已进入 {capture.Code} 状态";
            }
            else if (sidecar.State is WindowsVirtualCameraSidecarHostState.Exited
                or WindowsVirtualCameraSidecarHostState.Failed)
            {
                reason = $"sidecar 已进入 {sidecar.State} 状态";
            }
            else if (client.State == WindowsVirtualCameraSidecarClientState.Failed)
            {
                reason = $"sidecar 管道已进入 Failed 状态（{client.LastErrorCode}）";
            }
            else if (writer?.State == WindowsVirtualCameraSidecarOutputWriterState.Failed)
            {
                reason = $"虚拟摄像头输出泵已进入 Failed 状态（{writer.LastErrorCode}）";
            }

            if (reason is null)
            {
                return;
            }

            _started = false;
            _healthCancellation?.Cancel();
        }

        _output.Fail(reason);
        PublishSnapshot();
    }

    private void StartHealthMonitorNoLock()
    {
        _healthCancellation?.Dispose();
        _healthCancellation = new CancellationTokenSource();
        _healthTask = MonitorHealthAsync(_healthCancellation.Token);
    }

    private async Task MonitorHealthAsync(CancellationToken cancellationToken)
    {
        using var timer = new PeriodicTimer(TimeSpan.FromMilliseconds(250));
        try
        {
            while (await timer.WaitForNextTickAsync(cancellationToken).ConfigureAwait(false))
            {
                RefreshHealth();
                lock (_gate)
                {
                    if (!_started)
                    {
                        return;
                    }
                }
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            // StopAsync/DisposeAsync owns cancellation and waits for this task.
        }
    }

    private static async Task StopHealthMonitorAsync(
        Task? healthTask,
        CancellationTokenSource? healthCancellation)
    {
        if (healthCancellation is null)
        {
            return;
        }

        healthCancellation.Cancel();
        if (healthTask is not null)
        {
            try
            {
                await healthTask.WaitAsync(TimeSpan.FromSeconds(1)).ConfigureAwait(false);
            }
            catch (TimeoutException)
            {
                // 健康监视不是资源拥有者；超时不阻塞 writer/GPU/sidecar 的有界清理。
            }
        }

        healthCancellation.Dispose();
    }

    private void PublishSnapshot()
    {
        var snapshot = Snapshot;
        try
        {
            SnapshotChanged?.Invoke(this, snapshot);
        }
        catch
        {
            // UI 观察者异常不得中断输出链的状态收敛或后续清理。
        }
    }

    private async Task<CleanupReport> CleanupAsync(
        WindowsVirtualCameraSidecarOutputWriter? writer,
        bool stopGpu,
        bool stopClient,
        bool stopHost,
        CancellationToken cancellationToken)
    {
        var failures = new List<string>();
        var wasCancelled = cancellationToken.IsCancellationRequested;

        async Task Run(string name, Func<CancellationToken, Task<CleanupStep>> operation)
        {
            CleanupStep first;
            try
            {
                first = await operation(cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                first = new(false, "Cancelled", true);
            }
            catch (Exception)
            {
                first = new(false, "Exception", false);
            }

            if (first.IsSuccess)
            {
                return;
            }

            failures.Add($"{name}:{first.Code}");
            wasCancelled |= first.WasCancelled;
            if (cancellationToken == CancellationToken.None)
            {
                return;
            }

            CleanupStep retry;
            try
            {
                retry = await operation(CancellationToken.None).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                retry = new(false, "Cancelled", true);
            }
            catch (Exception)
            {
                retry = new(false, "Exception", false);
            }

            if (!retry.IsSuccess)
            {
                failures.Add($"{name}:{retry.Code}");
                wasCancelled |= retry.WasCancelled;
            }
        }

        if (writer is not null)
        {
            await Run("writer.stop", async token =>
            {
                var result = await writer.StopAsync(token).ConfigureAwait(false);
                return new(
                    result.IsSuccess,
                    result.Error?.Code.ToString() ?? "Failed",
                    result.Error?.Code == WindowsVirtualCameraSidecarOutputWriterErrorCode.Cancelled);
            }).ConfigureAwait(false);
            await Run("writer.dispose", async _ =>
            {
                try
                {
                    await writer.DisposeAsync().ConfigureAwait(false);
                    return new(true, "", false);
                }
                catch (Exception)
                {
                    return new(false, "Exception", false);
                }
            }).ConfigureAwait(false);
        }

        if (stopGpu)
        {
            await Run("gpu.stop", async token =>
            {
                var result = await _gpuSession.StopAsync(token).ConfigureAwait(false);
                return new(
                    result.IsSuccess,
                    result.Code.ToString(),
                    result.Code == WindowsVirtualCameraGpuOutputSessionCode.Cancelled);
            }).ConfigureAwait(false);
        }

        if (stopClient)
        {
            await Run("client.stop", async token =>
            {
                var result = await _sidecarClient.StopAsync(token).ConfigureAwait(false);
                return new(
                    result.IsSuccess,
                    result.Error?.Code.ToString() ?? "Failed",
                    result.Error?.Code == WindowsVirtualCameraSidecarClientErrorCode.Cancelled);
            }).ConfigureAwait(false);
        }

        if (stopHost)
        {
            await Run("sidecar.stop", async token =>
            {
                var result = await _sidecarHost.StopAsync(token).ConfigureAwait(false);
                return new(
                    result.IsSuccess,
                    result.Error?.Code.ToString() ?? "Failed",
                    result.Error?.Code == WindowsVirtualCameraSidecarHostErrorCode.Cancelled);
            }).ConfigureAwait(false);
        }

        return new(failures, wasCancelled);
    }

    private WindowsVirtualCameraOutputCoordinatorResult Succeeded() =>
        new(true, WindowsVirtualCameraOutputCoordinatorCode.Started, Snapshot);

    private WindowsVirtualCameraOutputCoordinatorResult Failure(
        WindowsVirtualCameraOutputCoordinatorCode code,
        string message) => new(false, code, Snapshot, message);

    private WindowsVirtualCameraOutputCoordinatorResult FailureAfterCleanup(
        WindowsVirtualCameraOutputCoordinatorCode primaryCode,
        string message,
        CleanupReport cleanup)
    {
        if (cleanup.Failures.Count > 0)
        {
            lock (_gate)
            {
                _cleanupFailed = true;
            }

            return Failure(
                WindowsVirtualCameraOutputCoordinatorCode.CleanupFailed,
                $"{message}；清理失败：{string.Join(",", cleanup.Failures)}");
        }

        return Failure(
            cleanup.WasCancelled
                ? WindowsVirtualCameraOutputCoordinatorCode.Cancelled
                : primaryCode,
            message);
    }

    private void ResetStartingIfNeeded()
    {
        lock (_gate)
        {
            if (!_started)
            {
                _starting = false;
            }
        }
    }

    private sealed record CleanupReport(IReadOnlyList<string> Failures, bool WasCancelled);

    private readonly record struct CleanupStep(bool IsSuccess, string Code, bool WasCancelled);
}
