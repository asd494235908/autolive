using System.Diagnostics;
using System.Threading;
using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Windows;

/// <summary>WGC/GPU 输出会话的稳定结果码。</summary>
public enum WindowsVirtualCameraGpuOutputSessionCode
{
    Started,
    AlreadyStarted,
    InvalidBinding,
    CaptureFailed,
    Cancelled,
    Stopped,
    Closed,
}

/// <summary>WGC/GPU 输出会话结果。</summary>
public sealed record WindowsVirtualCameraGpuOutputSessionResult(
    bool IsSuccess,
    WindowsVirtualCameraGpuOutputSessionCode Code,
    VirtualCameraStatus Status,
    WindowsGraphicsCaptureWindowSessionSnapshot Capture);

/// <summary>
/// 将最终效果 HWND 的 WGC 帧接入 GPU YUY2 转换器和既有虚拟摄像头逻辑所有者。
/// sidecar/DirectShow 仍由独立宿主负责，本会话不把本地播放和下游设备生命周期混在一起。
/// </summary>
public sealed class WindowsVirtualCameraGpuOutputSession : IAsyncDisposable
{
    private static readonly TimeSpan DefaultFirstFrameTimeout = TimeSpan.FromSeconds(5);
    private static readonly TimeSpan MaxFirstFrameTimeout = TimeSpan.FromSeconds(30);

    private readonly VirtualCameraOutputManager _output;
    private readonly WindowsVirtualCameraSurfaceBinding _binding;
    private readonly WindowsGraphicsCaptureWindowSession _capture;
    private readonly WindowsGraphicsCaptureGpuYuy2Converter _converter;
    private readonly object _gate = new();
    private ulong _sequence;
    private TaskCompletionSource<bool>? _firstFrameReady;
    private bool _started;
    private bool _closed;

    public WindowsVirtualCameraGpuOutputSession(
        VirtualCameraOutputManager output,
        WindowsVirtualCameraSurfaceBinding binding)
    {
        _output = output ?? throw new ArgumentNullException(nameof(output));
        _binding = binding ?? throw new ArgumentNullException(nameof(binding));
        _capture = new WindowsGraphicsCaptureWindowSession();
        _converter = new WindowsGraphicsCaptureGpuYuy2Converter(output.Snapshot.Config);
    }

    /// <summary>读取输出逻辑状态和 WGC 捕获状态。</summary>
    public (VirtualCameraStatus Output, WindowsGraphicsCaptureWindowSessionSnapshot Capture) Snapshot =>
        (_output.Snapshot, _capture.Snapshot);

    /// <summary>启动真实 WGC frame pool；首次成功 GPU 回读后才把逻辑状态置为 Ready。</summary>
    public async Task<WindowsVirtualCameraGpuOutputSessionResult> StartAsync(
        TimeSpan? timeout = null,
        CancellationToken cancellationToken = default)
    {
        lock (_gate)
        {
            if (_closed)
            {
                return Result(false, WindowsVirtualCameraGpuOutputSessionCode.Closed);
            }

            if (_started)
            {
                return Result(false, WindowsVirtualCameraGpuOutputSessionCode.AlreadyStarted);
            }

            _started = true;
            _firstFrameReady = new(TaskCreationOptions.RunContinuationsAsynchronously);
        }

        var begin = _output.BeginStart();
        if (!begin.IsSuccess)
        {
            ResetStartState();
            return Result(false, WindowsVirtualCameraGpuOutputSessionCode.CaptureFailed);
        }

        _output.SetOutputContext(_output.OutputContext with { HasValidFrame = false });

        var startupBudget = NormalizeFirstFrameTimeout(timeout);
        var startupStopwatch = Stopwatch.StartNew();
        var captureResult = await _capture.StartAsync(
                _binding,
                startupBudget,
                cancellationToken,
                OnCapturedFrame)
            .ConfigureAwait(false);
        if (!captureResult.IsSuccess)
        {
            await StopAsync().ConfigureAwait(false);
            var code = captureResult.Code == WindowsGraphicsCaptureWindowSessionCode.Cancelled
                ? WindowsVirtualCameraGpuOutputSessionCode.Cancelled
                : WindowsVirtualCameraGpuOutputSessionCode.CaptureFailed;
            return Result(false, code);
        }

        var remainingBudget = startupBudget - startupStopwatch.Elapsed;
        if (remainingBudget <= TimeSpan.Zero)
        {
            await StopAsync().ConfigureAwait(false);
            return Result(false, WindowsVirtualCameraGpuOutputSessionCode.CaptureFailed);
        }

        var firstFrameReady = GetFirstFrameWaiter(out var closed);
        if (firstFrameReady is null)
        {
            return Result(
                false,
                closed
                    ? WindowsVirtualCameraGpuOutputSessionCode.Closed
                    : WindowsVirtualCameraGpuOutputSessionCode.Cancelled);
        }

        try
        {
            await firstFrameReady.Task.WaitAsync(
                    remainingBudget,
                    cancellationToken)
                .ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            await StopAsync().ConfigureAwait(false);
            return Result(false, WindowsVirtualCameraGpuOutputSessionCode.Cancelled);
        }
        catch (TimeoutException)
        {
            await StopAsync().ConfigureAwait(false);
            return Result(false, WindowsVirtualCameraGpuOutputSessionCode.CaptureFailed);
        }

        lock (_gate)
        {
            if (_closed)
            {
                return Result(false, WindowsVirtualCameraGpuOutputSessionCode.Closed);
            }

            if (!_started)
            {
                return Result(false, WindowsVirtualCameraGpuOutputSessionCode.Cancelled);
            }

            if (ReferenceEquals(_firstFrameReady, firstFrameReady))
            {
                _firstFrameReady = null;
            }
        }
        return Result(true, WindowsVirtualCameraGpuOutputSessionCode.Started);
    }

    /// <summary>停止 WGC 线程和 GPU 转换资源；不会停止本地播放。</summary>
    public async Task<WindowsVirtualCameraGpuOutputSessionResult> StopAsync(
        CancellationToken cancellationToken = default)
    {
        TaskCompletionSource<bool>? firstFrameReady;
        lock (_gate)
        {
            firstFrameReady = _firstFrameReady;
            _firstFrameReady = null;
            _started = false;
        }

        firstFrameReady?.TrySetCanceled();
        await _capture.StopAsync(cancellationToken).ConfigureAwait(false);
        _output.Stop();
        _output.SetOutputContext(_output.OutputContext with { HasValidFrame = false });
        var captureStopped = _capture.Snapshot.Code is
            WindowsGraphicsCaptureWindowSessionCode.Stopped
            or WindowsGraphicsCaptureWindowSessionCode.Closed;
        return Result(
            captureStopped,
            captureStopped
                ? WindowsVirtualCameraGpuOutputSessionCode.Stopped
                : WindowsVirtualCameraGpuOutputSessionCode.CaptureFailed);
    }

    public async ValueTask DisposeAsync()
    {
        lock (_gate)
        {
            if (_closed)
            {
                return;
            }

            _closed = true;
            _started = false;
            _firstFrameReady?.TrySetCanceled();
            _firstFrameReady = null;
        }

        await _capture.DisposeAsync().ConfigureAwait(false);
        _converter.Dispose();
        _output.Stop();
        _output.SetOutputContext(_output.OutputContext with { HasValidFrame = false });
    }

    private void OnCapturedFrame(
        global::Windows.Graphics.Capture.Direct3D11CaptureFrame frame,
        WindowsGraphicsCaptureD3D11Context context)
    {
        lock (_gate)
        {
            if (_closed || !_started)
            {
                return;
            }
        }

        var status = _output.Snapshot;
        var conversion = _converter.TryConvert(
            frame,
            context,
            status.Generation,
            NextSequence());
        if (conversion.Frame is null)
        {
            return;
        }

        _output.RecordReadback(conversion.ReadbackDuration);
        if (_output.Snapshot.State == VirtualCameraState.Starting)
        {
            if (!WindowsD3D11HardwareContextFactory.TryBuildFacts(context.Device, out var facts)
                || facts is null
                || !_output.MarkReady(facts).IsSuccess)
            {
                _output.Fail("GPU 捕获事实无法验证");
                return;
            }
        }

        var submit = _output.SubmitFrame(conversion.Frame);
        if (!submit.IsSuccess)
        {
            return;
        }

        _output.SetOutputContext(_output.OutputContext with { HasValidFrame = true });
        lock (_gate)
        {
            _firstFrameReady?.TrySetResult(true);
        }
    }

    private TaskCompletionSource<bool>? GetFirstFrameWaiter(out bool closed)
    {
        lock (_gate)
        {
            closed = _closed;
            return _firstFrameReady;
        }
    }

    private ulong NextSequence()
    {
        lock (_gate)
        {
            _sequence = _sequence == ulong.MaxValue ? 1 : _sequence + 1;
            return _sequence;
        }
    }

    private void ResetStartState()
    {
        lock (_gate)
        {
            _firstFrameReady?.TrySetCanceled();
            _firstFrameReady = null;
            _started = false;
        }
    }

    private static TimeSpan NormalizeFirstFrameTimeout(TimeSpan? timeout)
    {
        var budget = timeout.GetValueOrDefault(DefaultFirstFrameTimeout);
        return budget <= TimeSpan.Zero || budget > MaxFirstFrameTimeout
            ? DefaultFirstFrameTimeout
            : budget;
    }

    private WindowsVirtualCameraGpuOutputSessionResult Result(
        bool isSuccess,
        WindowsVirtualCameraGpuOutputSessionCode code) =>
        new(isSuccess, code, _output.Snapshot, _capture.Snapshot);
}
