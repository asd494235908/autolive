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
    private bool _started;
    private bool _starting;
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

            if (_started || _starting)
            {
                return Failure(WindowsVirtualCameraOutputCoordinatorCode.AlreadyStarted, "虚拟摄像头输出组合已经在运行");
            }

            _starting = true;
        }

        if (_output.Snapshot.State != VirtualCameraState.Installed)
        {
            ResetStarting();
            return Failure(WindowsVirtualCameraOutputCoordinatorCode.InvalidState, "虚拟摄像头设备尚未完成安装门禁");
        }

        if (plan is null)
        {
            ResetStarting();
            return Failure(WindowsVirtualCameraOutputCoordinatorCode.InvalidPlan, "虚拟摄像头 sidecar 启动计划无效");
        }

        var host = await _sidecarHost.StartAsync(plan, cancellationToken).ConfigureAwait(false);
        if (!host.IsSuccess)
        {
            ResetStarting();
            return Failure(
                host.Error?.Code == WindowsVirtualCameraSidecarHostErrorCode.Cancelled
                    ? WindowsVirtualCameraOutputCoordinatorCode.Cancelled
                    : WindowsVirtualCameraOutputCoordinatorCode.SidecarStartFailed,
                host.Error?.Message ?? "虚拟摄像头 sidecar 启动失败");
        }

        var client = await _sidecarHost.ConnectClientAsync(
                _sidecarClient,
                connectTimeout ?? DefaultConnectTimeout,
                cancellationToken)
            .ConfigureAwait(false);
        if (!client.IsSuccess)
        {
            await CleanupFailedStartAsync().ConfigureAwait(false);
            ResetStarting();
            return Failure(
                client.Error?.Code == WindowsVirtualCameraSidecarClientErrorCode.Cancelled
                    ? WindowsVirtualCameraOutputCoordinatorCode.Cancelled
                    : WindowsVirtualCameraOutputCoordinatorCode.SidecarConnectFailed,
                client.Error?.Message ?? "虚拟摄像头 sidecar 管道连接失败");
        }

        var capture = await _gpuSession.StartAsync(captureTimeout, cancellationToken).ConfigureAwait(false);
        if (!capture.IsSuccess)
        {
            await CleanupFailedStartAsync().ConfigureAwait(false);
            ResetStarting();
            return Failure(
                capture.Code == WindowsVirtualCameraGpuOutputSessionCode.Cancelled
                    ? WindowsVirtualCameraOutputCoordinatorCode.Cancelled
                    : WindowsVirtualCameraOutputCoordinatorCode.CaptureFailed,
                "虚拟摄像头 WGC/GPU 会话启动失败");
        }

        var writer = new WindowsVirtualCameraSidecarOutputWriter(_output, _sidecarClient);
        var writerResult = await writer.StartAsync(cancellationToken).ConfigureAwait(false);
        if (!writerResult.IsSuccess)
        {
            await _gpuSession.StopAsync().ConfigureAwait(false);
            await CleanupFailedStartAsync().ConfigureAwait(false);
            await writer.DisposeAsync().ConfigureAwait(false);
            ResetStarting();
            return Failure(
                writerResult.Error?.Code == WindowsVirtualCameraSidecarOutputWriterErrorCode.Cancelled
                    ? WindowsVirtualCameraOutputCoordinatorCode.Cancelled
                    : WindowsVirtualCameraOutputCoordinatorCode.WriterStartFailed,
                writerResult.Error?.Message ?? "虚拟摄像头输出泵启动失败");
        }

        lock (_gate)
        {
            _writer = writer;
            _starting = false;
            _started = true;
        }

        return Succeeded();
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
        lock (_gate)
        {
            if (_closed)
            {
                return Failure(WindowsVirtualCameraOutputCoordinatorCode.Closed, "虚拟摄像头输出组合已关闭");
            }

            writer = _writer;
            _starting = false;
            _started = false;
        }

        if (writer is not null)
        {
            await writer.StopAsync(cancellationToken).ConfigureAwait(false);
            await writer.DisposeAsync().ConfigureAwait(false);
        }

        await _gpuSession.StopAsync(cancellationToken).ConfigureAwait(false);
        await _sidecarClient.StopAsync(cancellationToken).ConfigureAwait(false);
        await _sidecarHost.StopAsync(cancellationToken).ConfigureAwait(false);
        lock (_gate)
        {
            _writer = null;
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
        lock (_gate)
        {
            if (_closed)
            {
                return;
            }

            _closed = true;
            _starting = false;
            _started = false;
        }

        _sidecarHost.DownstreamClientCountChanged -= SidecarHost_DownstreamClientCountChanged;

        WindowsVirtualCameraSidecarOutputWriter? writer;
        lock (_gate)
        {
            writer = _writer;
            _writer = null;
        }

        if (writer is not null)
        {
            await writer.DisposeAsync().ConfigureAwait(false);
        }

        await _gpuSession.DisposeAsync().ConfigureAwait(false);
        await _sidecarClient.DisposeAsync().ConfigureAwait(false);
        await _sidecarHost.DisposeAsync().ConfigureAwait(false);
    }

    private void SidecarHost_DownstreamClientCountChanged(object? sender, uint count)
    {
        var state = _output.Snapshot.State;
        if (state is VirtualCameraState.Ready or VirtualCameraState.Streaming)
        {
            _output.SetDownstreamClientCount(count);
        }
    }

    private async Task CleanupFailedStartAsync()
    {
        await _sidecarClient.StopAsync().ConfigureAwait(false);
        await _sidecarHost.StopAsync().ConfigureAwait(false);
    }

    private WindowsVirtualCameraOutputCoordinatorResult Succeeded() =>
        new(true, WindowsVirtualCameraOutputCoordinatorCode.Started, Snapshot);

    private WindowsVirtualCameraOutputCoordinatorResult Failure(
        WindowsVirtualCameraOutputCoordinatorCode code,
        string message) => new(false, code, Snapshot, message);

    private void ResetStarting()
    {
        lock (_gate)
        {
            _starting = false;
        }
    }
}
