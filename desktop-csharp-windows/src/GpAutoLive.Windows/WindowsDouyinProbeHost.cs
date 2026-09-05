using System.Buffers;
using System.Diagnostics;
using System.Text;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Processes;

namespace GpAutoLive.Windows;

/// <summary>受管抖音探针宿主状态。</summary>
public enum WindowsDouyinProbeHostState
{
    Ready,
    Starting,
    Running,
    Exited,
    Stopping,
    Stopped,
    Failed,
    Closed
}

/// <summary>抖音探针宿主的稳定错误分类。</summary>
public enum WindowsDouyinProbeHostFailureCode
{
    NotWindows,
    InvalidPlan,
    AlreadyRunning,
    StartFailed,
    ProcessExited,
    TimedOut,
    Cancelled,
    OutputLimitExceeded,
    OutputReadFailed,
    StopTimedOut,
    Closed
}

/// <summary>不回显路径、命令行、凭据或 sidecar 原始输出的宿主错误。</summary>
public sealed record WindowsDouyinProbeHostError(
    WindowsDouyinProbeHostFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>抖音探针宿主脱敏快照。</summary>
public sealed record WindowsDouyinProbeHostSnapshot(
    WindowsDouyinProbeHostState State,
    int? ProcessId,
    int? ExitCode,
    DouyinLiveStatus Douyin,
    string? QrPath,
    string? LastEvent,
    int InvalidEventCount);

/// <summary>探针启动/停止结果。</summary>
public sealed record WindowsDouyinProbeHostResult(
    bool IsSuccess,
    WindowsDouyinProbeHostSnapshot Snapshot,
    WindowsDouyinProbeHostError? Error = null);

/// <summary>
/// Windows 专用抖音 Conda sidecar 宿主。stdout 只接受脱敏 JSON 事件，
/// 进程树优先由 Job Object 回收，停止、超时和输出越界均有界结束。
/// </summary>
public sealed class WindowsDouyinProbeHost : IAsyncDisposable
{
    private const int MaxInvalidEvents = 16;
    private const int ReadBufferChars = 8 * 1024;
    private static readonly TimeSpan CleanupTimeout = TimeSpan.FromSeconds(2);
    private static readonly UTF8Encoding Utf8 = new(false, false);

    private readonly object _gate = new();
    private readonly SemaphoreSlim _lifecycle = new(1, 1);
    private readonly DouyinLiveManager _manager;
    private Process? _process;
    private WindowsJobObject? _job;
    private CancellationTokenSource? _runCancellation;
    private Task? _monitorTask;
    private WindowsDouyinProbeHostState _state = WindowsDouyinProbeHostState.Ready;
    private int? _exitCode;
    private string? _qrPath;
    private string? _lastEvent;
    private int _invalidEventCount;
    private int _terminationReason;
    private bool _stopRequested;
    private bool _disposed;

    /// <summary>创建使用指定核心状态所有者的宿主。</summary>
    public WindowsDouyinProbeHost(DouyinLiveManager manager)
    {
        _manager = manager ?? throw new ArgumentNullException(nameof(manager));
    }

    /// <summary>当前宿主和 M1 状态快照。</summary>
    public WindowsDouyinProbeHostSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshot();
            }
        }
    }

    /// <summary>收到脱敏状态变化时触发；事件处理器异常不会影响宿主回收。</summary>
    public event EventHandler<WindowsDouyinProbeHostSnapshot>? SnapshotChanged;

    /// <summary>启动已验证的 Conda 探针；不会把 stdout 原文返回给调用方。</summary>
    public async Task<WindowsDouyinProbeHostResult> StartAsync(
        WindowsDouyinProbeLaunchRequest? request,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(WindowsDouyinProbeHostFailureCode.Cancelled, "抖音探针启动已取消。", true);
        }

        if (!WindowsDouyinProbeLaunchPlanBuilder.TryCreate(request, out var plan, out _)
            || request is null
            || plan is null)
        {
            return Failure(WindowsDouyinProbeHostFailureCode.InvalidPlan, "抖音探针启动计划无效。", false);
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsDouyinProbeHostFailureCode.Cancelled, "抖音探针启动已取消。", true);
        }

        try
        {
            if (_disposed)
            {
                return Failure(WindowsDouyinProbeHostFailureCode.Closed, "抖音探针宿主已关闭。", false);
            }

            if (!OperatingSystem.IsWindows())
            {
                return Failure(WindowsDouyinProbeHostFailureCode.NotWindows, "抖音探针仅支持 Windows。", false);
            }

            if (_process is not null && !HasExited(_process))
            {
                return Failure(WindowsDouyinProbeHostFailureCode.AlreadyRunning, "抖音探针已经在运行。", false);
            }

            await ReleaseExitedProcessAsync().ConfigureAwait(false);
            var managerStart = _manager.TryStart(request.Config);
            if (!managerStart.IsSuccess)
            {
                return Failure(
                    WindowsDouyinProbeHostFailureCode.AlreadyRunning,
                    managerStart.Error?.Message ?? "抖音 M1 会话已经在运行。",
                    false);
            }

            var process = new Process
            {
                StartInfo = CreateStartInfo(plan, request.UpstreamRoot),
                EnableRaisingEvents = false
            };
            try
            {
                if (!process.Start())
                {
                    _manager.Fail("探针进程无法启动");
                    process.Dispose();
                    return Failure(WindowsDouyinProbeHostFailureCode.StartFailed, "抖音探针进程无法启动。", true);
                }
            }
            catch (Exception exception) when (IsProcessStartFailure(exception))
            {
                _manager.Fail("探针进程无法启动");
                process.Dispose();
                return Failure(WindowsDouyinProbeHostFailureCode.StartFailed, "抖音探针进程无法启动。", true);
            }

            WindowsJobObject? job = null;
            if (WindowsJobObject.TryCreate(out var candidateJob)
                && candidateJob is not null
                && candidateJob.TryAssign(process))
            {
                job = candidateJob;
            }
            else
            {
                candidateJob?.Dispose();
            }

            var runCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            runCancellation.CancelAfter(request.Timeout);
            lock (_gate)
            {
                _process = process;
                _job = job;
                _runCancellation = runCancellation;
                _monitorTask = null;
                _state = WindowsDouyinProbeHostState.Running;
                _exitCode = null;
                _qrPath = null;
                _lastEvent = "probe_started";
                _invalidEventCount = 0;
                _terminationReason = 0;
                _stopRequested = false;
            }

            var monitor = MonitorAsync(process, job, request.QrOutputPath, cancellationToken, runCancellation);
            lock (_gate)
            {
                _monitorTask = monitor;
            }
            PublishSnapshot();

            if (HasExited(process))
            {
                SetTerminationReasonIfUnset(5);
                await monitor.ConfigureAwait(false);
                return Failure(WindowsDouyinProbeHostFailureCode.ProcessExited, "抖音探针启动后立即退出。", true);
            }

            return Succeeded();
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>停止探针并在有限预算内回收 stdout、进程和 Job Object。</summary>
    public async Task<WindowsDouyinProbeHostResult> StopAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsDouyinProbeHostFailureCode.Cancelled, "抖音探针停止已取消。", true);
        }

        try
        {
            if (_disposed)
            {
                return Succeeded();
            }

            lock (_gate)
            {
                _stopRequested = true;
                _state = _process is null
                    ? WindowsDouyinProbeHostState.Stopped
                    : WindowsDouyinProbeHostState.Stopping;
            }

            CancelRun();
            Process? process;
            WindowsJobObject? job;
            Task? monitor;
            lock (_gate)
            {
                process = _process;
                job = _job;
                monitor = _monitorTask;
            }

            if (process is not null && !HasExited(process))
            {
                KillProcessTree(process, job);
            }

            if (monitor is not null)
            {
                try
                {
                    await monitor.WaitAsync(CleanupTimeout).ConfigureAwait(false);
                }
                catch (TimeoutException)
                {
                    KillProcessTree(process, job);
                }
            }

            var exited = process is null || HasExited(process);
            var exitCode = SafeExitCode(process);
            DisposeProcess(process, job);
            lock (_gate)
            {
                _process = null;
                _job = null;
                _runCancellation?.Dispose();
                _runCancellation = null;
                _monitorTask = null;
                _exitCode = exitCode;
                _state = _disposed ? WindowsDouyinProbeHostState.Closed : WindowsDouyinProbeHostState.Stopped;
                _qrPath = null;
            }
            _manager.Stop();
            PublishSnapshot();

            return exited
                ? Succeeded()
                : Failure(WindowsDouyinProbeHostFailureCode.StopTimedOut, "抖音探针未能在停止预算内退出。", false);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>释放宿主；释放过程等价于停止并清理本地 M1 状态。</summary>
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
            lock (_gate)
            {
                _stopRequested = true;
            }
            CancelRun();
            Process? process;
            WindowsJobObject? job;
            Task? monitor;
            lock (_gate)
            {
                process = _process;
                job = _job;
                monitor = _monitorTask;
            }

            KillProcessTree(process, job);
            if (monitor is not null)
            {
                try
                {
                    await monitor.WaitAsync(CleanupTimeout).ConfigureAwait(false);
                }
                catch (TimeoutException)
                {
                    // 进程和管道句柄仍由下方 Dispose 关闭，避免无限等待异常子进程。
                }
            }

            DisposeProcess(process, job);
            _manager.Stop();
            lock (_gate)
            {
                _process = null;
                _job = null;
                _runCancellation?.Dispose();
                _runCancellation = null;
                _monitorTask = null;
                _state = WindowsDouyinProbeHostState.Closed;
                _qrPath = null;
            }
        }
        finally
        {
            _lifecycle.Release();
            _lifecycle.Dispose();
            GC.SuppressFinalize(this);
        }
    }

    private async Task MonitorAsync(
        Process process,
        WindowsJobObject? job,
        string qrPath,
        CancellationToken callerCancellation,
        CancellationTokenSource runCancellation)
    {
        var stdoutTask = ReadStdoutAsync(process.StandardOutput.BaseStream, qrPath, runCancellation.Token);
        var stderrTask = DrainStreamAsync(
            process.StandardError.BaseStream,
            runCancellation.Token,
            () =>
            {
                SetTerminationReasonIfUnset(3);
                runCancellation.Cancel();
            },
            () =>
            {
                SetTerminationReasonIfUnset(4);
                runCancellation.Cancel();
            });
        var naturalExit = false;
        try
        {
            try
            {
                await process.WaitForExitAsync(runCancellation.Token).ConfigureAwait(false);
                naturalExit = true;
            }
            catch (OperationCanceledException) when (runCancellation.IsCancellationRequested)
            {
                if (Volatile.Read(ref _terminationReason) == 0)
                {
                    SetTerminationReasonIfUnset(IsStopRequested() || callerCancellation.IsCancellationRequested ? 2 : 1);
                }
                KillProcessTree(process, job);
                await WaitForExitBoundedAsync(process).ConfigureAwait(false);
            }

            try
            {
                await Task.WhenAll(stdoutTask, stderrTask).WaitAsync(CleanupTimeout).ConfigureAwait(false);
            }
            catch (TimeoutException)
            {
                SetTerminationReasonIfUnset(4);
                runCancellation.Cancel();
            }
            catch (OperationCanceledException)
            {
                SetTerminationReasonIfUnset(4);
            }
            catch (IOException)
            {
                SetTerminationReasonIfUnset(4);
                runCancellation.Cancel();
            }

            var exitCode = SafeExitCode(process);
            var reason = Volatile.Read(ref _terminationReason);
            var stopRequested = IsStopRequested();
            if (stopRequested || reason == 2)
            {
                _manager.Stop();
                SetHostTerminal(WindowsDouyinProbeHostState.Stopped, exitCode);
            }
            else if (reason == 1)
            {
                _manager.Fail("探针超过时间预算");
                SetHostTerminal(WindowsDouyinProbeHostState.Failed, exitCode);
            }
            else if (reason is 3 or 4)
            {
                _manager.Fail(reason == 3 ? "探针标准输出超出上限" : "探针输出读取失败");
                SetHostTerminal(WindowsDouyinProbeHostState.Failed, exitCode);
            }
            else if (_manager.Snapshot.State is not (
                DouyinLiveState.Passed
                or DouyinLiveState.Failed
                or DouyinLiveState.Inconclusive))
            {
                _manager.MarkInconclusive(naturalExit && exitCode == 0
                    ? "探针在完成前退出"
                    : "探针进程意外退出");
                SetHostTerminal(WindowsDouyinProbeHostState.Exited, exitCode);
            }
            else
            {
                SetHostTerminal(
                    _manager.Snapshot.State == DouyinLiveState.Passed
                        ? WindowsDouyinProbeHostState.Exited
                        : WindowsDouyinProbeHostState.Failed,
                    exitCode);
            }

            PublishSnapshot();
        }
        finally
        {
            // The lifecycle owner disposes Process/Job; monitor only drains and projects state.
        }
    }

    private async Task ReadStdoutAsync(Stream stream, string qrPath, CancellationToken cancellationToken)
    {
        using var reader = new StreamReader(stream, Utf8, detectEncodingFromByteOrderMarks: false, ReadBufferChars, leaveOpen: true);
        var buffer = new char[ReadBufferChars];
        var line = new StringBuilder(capacity: 256);
        var totalBytes = 0;
        try
        {
            while (true)
            {
                var read = await reader.ReadAsync(buffer.AsMemory(), cancellationToken).ConfigureAwait(false);
                if (read == 0)
                {
                    if (line.Length > 0 && !ProcessStdoutLine(line.ToString(), qrPath, ref totalBytes))
                    {
                        return;
                    }

                    return;
                }

                for (var index = 0; index < read; index++)
                {
                    var character = buffer[index];
                    if (character == '\n')
                    {
                        if (!ProcessStdoutLine(line.ToString(), qrPath, ref totalBytes))
                        {
                            return;
                        }

                        line.Clear();
                        continue;
                    }

                    line.Append(character == '\r' ? '\r' : character);
                    if (line.Length > WindowsDouyinProbeEventParser.MaxLineBytes)
                    {
                        SetTerminationReasonIfUnset(3);
                        CancelRun();
                        return;
                    }
                }
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch (IOException)
        {
            SetTerminationReasonIfUnset(4);
            CancelRun();
        }
        catch (ObjectDisposedException)
        {
            SetTerminationReasonIfUnset(4);
            CancelRun();
        }
    }

    private bool ProcessStdoutLine(string line, string qrPath, ref int totalBytes)
    {
        var lineBytes = Utf8.GetByteCount(line) + 1;
        totalBytes = checked(totalBytes + lineBytes);
        if (totalBytes > WindowsDouyinProbeLaunchPlanBuilder.MaxStandardOutputBytes
            || lineBytes > WindowsDouyinProbeEventParser.MaxLineBytes)
        {
            SetTerminationReasonIfUnset(3);
            CancelRun();
            return false;
        }

        if (WindowsDouyinProbeEventParser.TryParse(line, out var probeEvent, out _))
        {
            ApplyEvent(probeEvent!, qrPath);
        }
        else if (Interlocked.Increment(ref _invalidEventCount) > MaxInvalidEvents)
        {
            SetTerminationReasonIfUnset(4);
            CancelRun();
            return false;
        }

        return true;
    }

    private static async Task DrainStreamAsync(
        Stream stream,
        CancellationToken cancellationToken,
        Action onLimit,
        Action onFailure)
    {
        var buffer = ArrayPool<byte>.Shared.Rent(4 * 1024);
        var totalBytes = 0;
        try
        {
            while (true)
            {
                var read = await stream.ReadAsync(buffer.AsMemory(), cancellationToken).ConfigureAwait(false);
                if (read == 0)
                {
                    return;
                }

                totalBytes = checked(totalBytes + read);
                if (totalBytes > WindowsDouyinProbeLaunchPlanBuilder.MaxStandardErrorBytes)
                {
                    onLimit();
                    return;
                }
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch (IOException)
        {
            onFailure();
        }
        catch (ObjectDisposedException)
        {
            onFailure();
        }
        finally
        {
            ArrayPool<byte>.Shared.Return(buffer);
        }
    }

    private void ApplyEvent(WindowsDouyinProbeEvent probeEvent, string qrPath)
    {
        lock (_gate)
        {
            _lastEvent = probeEvent.Name;
            if (probeEvent.Kind == WindowsDouyinProbeEventKind.QrIssued
                && File.Exists(qrPath)
                && !HasReparsePoint(qrPath))
            {
                _qrPath = qrPath;
            }
        }

        WindowsDouyinProbeEventBridge.Apply(_manager, probeEvent);

        PublishSnapshot();
    }

    private async Task ReleaseExitedProcessAsync()
    {
        Process? process;
        WindowsJobObject? job;
        Task? monitor;
        lock (_gate)
        {
            process = _process;
            job = _job;
            monitor = _monitorTask;
        }

        if (process is null || !HasExited(process))
        {
            return;
        }

        if (monitor is not null)
        {
            try
            {
                await monitor.WaitAsync(CleanupTimeout).ConfigureAwait(false);
            }
            catch (TimeoutException)
            {
                // 下方 Dispose 仍会关闭有限的进程句柄和管道。
            }
        }

        DisposeProcess(process, job);
        lock (_gate)
        {
            if (ReferenceEquals(_process, process))
            {
                _process = null;
                _job = null;
                _runCancellation?.Dispose();
                _runCancellation = null;
                _monitorTask = null;
            }
        }
    }

    private void SetHostTerminal(WindowsDouyinProbeHostState state, int? exitCode)
    {
        lock (_gate)
        {
            _state = _disposed ? WindowsDouyinProbeHostState.Closed : state;
            _exitCode = exitCode;
        }
    }

    private bool IsStopRequested()
    {
        lock (_gate)
        {
            return _stopRequested;
        }
    }

    private void SetTerminationReasonIfUnset(int reason) =>
        Interlocked.CompareExchange(ref _terminationReason, reason, 0);

    private void CancelRun()
    {
        try
        {
            _runCancellation?.Cancel();
        }
        catch (ObjectDisposedException)
        {
            // 生命周期所有者已经完成有界清理。
        }
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
            // UI observers are not allowed to break process cleanup.
        }
    }

    private WindowsDouyinProbeHostSnapshot CreateSnapshot() => new(
        _state,
        SafeProcessId(_process),
        _exitCode,
        _manager.Snapshot,
        _qrPath,
        _lastEvent,
        _invalidEventCount);

    private WindowsDouyinProbeHostResult Succeeded() => new(true, Snapshot);

    private WindowsDouyinProbeHostResult Failure(
        WindowsDouyinProbeHostFailureCode code,
        string message,
        bool retryable) => new(false, Snapshot, new(code, message, retryable));

    private static ProcessStartInfo CreateStartInfo(ExternalProcessPlan plan, string upstreamRoot)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = plan.ExecutablePath,
            WorkingDirectory = Path.GetFullPath(upstreamRoot),
            UseShellExecute = false,
            CreateNoWindow = true,
            WindowStyle = ProcessWindowStyle.Hidden,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            StandardOutputEncoding = Utf8,
            StandardErrorEncoding = Utf8
        };

        foreach (var argument in plan.Arguments)
        {
            startInfo.ArgumentList.Add(argument);
        }

        return startInfo;
    }

    private static void DisposeProcess(Process? process, WindowsJobObject? job)
    {
        try
        {
            process?.Dispose();
        }
        finally
        {
            job?.Dispose();
        }
    }

    private static void KillProcessTree(Process? process, WindowsJobObject? job)
    {
        if (process is null)
        {
            return;
        }

        if (job is not null && job.TryTerminate())
        {
            return;
        }

        try
        {
            if (!process.HasExited)
            {
                process.Kill(entireProcessTree: true);
            }
        }
        catch (InvalidOperationException)
        {
        }
        catch (System.ComponentModel.Win32Exception)
        {
        }
        catch (PlatformNotSupportedException)
        {
        }
    }

    private static async Task WaitForExitBoundedAsync(Process process)
    {
        try
        {
            await process.WaitForExitAsync().WaitAsync(CleanupTimeout).ConfigureAwait(false);
        }
        catch (InvalidOperationException)
        {
        }
        catch (TimeoutException)
        {
        }
    }

    private static bool HasExited(Process process)
    {
        try
        {
            return process.HasExited;
        }
        catch (InvalidOperationException)
        {
            return true;
        }
        catch (System.ComponentModel.Win32Exception)
        {
            return true;
        }
    }

    private static int? SafeExitCode(Process? process)
    {
        try
        {
            return process is not null && process.HasExited ? process.ExitCode : null;
        }
        catch (InvalidOperationException)
        {
            return null;
        }
        catch (System.ComponentModel.Win32Exception)
        {
            return null;
        }
    }

    private static int? SafeProcessId(Process? process)
    {
        try
        {
            return process?.Id;
        }
        catch (InvalidOperationException)
        {
            return null;
        }
    }

    private static bool HasReparsePoint(string path)
    {
        try
        {
            return (File.GetAttributes(path) & FileAttributes.ReparsePoint) != 0;
        }
        catch (IOException)
        {
            return true;
        }
        catch (UnauthorizedAccessException)
        {
            return true;
        }
    }

    private static bool IsProcessStartFailure(Exception exception) =>
        exception is InvalidOperationException
            or System.ComponentModel.Win32Exception
            or ArgumentException
            or NotSupportedException
            or UnauthorizedAccessException
            or System.Security.SecurityException;
}
