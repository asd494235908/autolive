using System.Buffers;
using System.Diagnostics;
using System.Security.Cryptography;
using System.Text;
using GpAutoLive.Contracts;

namespace GpAutoLive.Windows;

/// <summary>AkVirtualCamera sidecar 受管宿主状态。</summary>
public enum WindowsVirtualCameraSidecarHostState
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

/// <summary>AkVirtualCamera sidecar 宿主稳定错误分类。</summary>
public enum WindowsVirtualCameraSidecarHostErrorCode
{
    NotWindows,
    InvalidPlan,
    AlreadyRunning,
    StartFailed,
    JobObjectUnavailable,
    TokenWriteFailed,
    StartupTimedOut,
    ProcessExited,
    Cancelled,
    OutputLimitExceeded,
    OutputReadFailed,
    StopTimedOut,
    Closed
}

/// <summary>宿主错误；不包含路径、令牌、管道名或原始异常正文。</summary>
public sealed record WindowsVirtualCameraSidecarHostError(
    WindowsVirtualCameraSidecarHostErrorCode Code,
    string Message,
    bool Retryable = false);

/// <summary>sidecar 宿主脱敏快照。</summary>
public sealed record WindowsVirtualCameraSidecarHostSnapshot(
    WindowsVirtualCameraSidecarHostState State,
    int? ProcessId,
    int? ExitCode,
    WindowsVirtualCameraSidecarHostErrorCode? LastErrorCode);

/// <summary>sidecar 宿主启动/停止结果。</summary>
public sealed record WindowsVirtualCameraSidecarHostResult(
    bool IsSuccess,
    WindowsVirtualCameraSidecarHostSnapshot Snapshot,
    WindowsVirtualCameraSidecarHostError? Error = null);

/// <summary>
/// 受管 AkVirtualCamera sidecar 宿主。sidecar 自己创建当前用户 ACL 的 Named Pipe，
/// 本宿主只负责隐藏启动、stdin 令牌交付、Job Object、有限输出读取和有界退出。
/// </summary>
public sealed class WindowsVirtualCameraSidecarHost : IAsyncDisposable
{
    private const int ReadBufferBytes = 4 * 1024;
    private const int MaxStandardOutputBytes = 64 * 1024;
    private const int MaxStandardErrorBytes = 64 * 1024;
    private static readonly TimeSpan CleanupTimeout = TimeSpan.FromSeconds(2);
    private static readonly UTF8Encoding Utf8 = new(false, false);
    private static readonly byte[] HexDigits = "0123456789abcdef"u8.ToArray();

    private readonly object _gate = new();
    private readonly SemaphoreSlim _lifecycle = new(1, 1);
    private Process? _process;
    private WindowsJobObject? _job;
    private CancellationTokenSource? _runCancellation;
    private Task? _monitorTask;
    private WindowsVirtualCameraSidecarHostState _state = WindowsVirtualCameraSidecarHostState.Ready;
    private int? _exitCode;
    private WindowsVirtualCameraSidecarHostErrorCode? _lastErrorCode;
    private int _terminationReason;
    private bool _stopRequested;
    private bool _disposed;
    private string? _pipeName;

    /// <summary>当前脱敏宿主快照。</summary>
    public WindowsVirtualCameraSidecarHostSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshot();
            }
        }
    }

    /// <summary>宿主状态变化事件；观察者异常不会影响进程回收。</summary>
    public event EventHandler<WindowsVirtualCameraSidecarHostSnapshot>? SnapshotChanged;

    /// <summary>sidecar stdout 报告的下游客户端数量；不暴露管道、令牌或原始日志。</summary>
    public event EventHandler<uint>? DownstreamClientCountChanged;

    /// <summary>
    /// 启动已验证 sidecar。令牌按 32 个 ASCII 十六进制字符加换行写入 stdin，随后立即关闭 stdin。
    /// </summary>
    public async Task<WindowsVirtualCameraSidecarHostResult> StartAsync(
        WindowsVirtualCameraSidecarLaunchPlan? plan,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(WindowsVirtualCameraSidecarHostErrorCode.Cancelled, "虚拟摄像头 sidecar 启动已取消。", true);
        }

        if (!TryValidatePlan(plan))
        {
            return Failure(WindowsVirtualCameraSidecarHostErrorCode.InvalidPlan, "虚拟摄像头 sidecar 启动计划无效。", false);
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsVirtualCameraSidecarHostErrorCode.Cancelled, "虚拟摄像头 sidecar 启动已取消。", true);
        }

        try
        {
            if (_disposed)
            {
                return Failure(WindowsVirtualCameraSidecarHostErrorCode.Closed, "虚拟摄像头 sidecar 宿主已关闭。", false);
            }

            if (!OperatingSystem.IsWindows())
            {
                return Failure(WindowsVirtualCameraSidecarHostErrorCode.NotWindows, "虚拟摄像头 sidecar 仅支持 Windows。", false);
            }

            if (_process is not null && !HasExited(_process))
            {
                return Failure(WindowsVirtualCameraSidecarHostErrorCode.AlreadyRunning, "虚拟摄像头 sidecar 已经在运行。", false);
            }

            await ReleaseExitedProcessAsync().ConfigureAwait(false);

            if (!WindowsJobObject.TryCreate(out var candidateJob) || candidateJob is null)
            {
                candidateJob?.Dispose();
                return Failure(WindowsVirtualCameraSidecarHostErrorCode.JobObjectUnavailable, "虚拟摄像头 sidecar 缺少 Job Object 进程树边界。", false);
            }

            var process = new Process
            {
                StartInfo = CreateStartInfo(plan!),
                EnableRaisingEvents = false
            };
            try
            {
                if (!process.Start())
                {
                    process.Dispose();
                    candidateJob.Dispose();
                    return Failure(WindowsVirtualCameraSidecarHostErrorCode.StartFailed, "虚拟摄像头 sidecar 进程无法启动。", true);
                }
            }
            catch (Exception exception) when (IsProcessStartFailure(exception))
            {
                process.Dispose();
                candidateJob.Dispose();
                return Failure(WindowsVirtualCameraSidecarHostErrorCode.StartFailed, "虚拟摄像头 sidecar 进程无法启动。", true);
            }

            if (!candidateJob.TryAssign(process))
            {
                KillProcessTree(process, candidateJob);
                await WaitForExitBoundedAsync(process).ConfigureAwait(false);
                process.Dispose();
                candidateJob.Dispose();
                return Failure(WindowsVirtualCameraSidecarHostErrorCode.JobObjectUnavailable, "虚拟摄像头 sidecar 无法加入 Job Object。", false);
            }

            try
            {
                await WriteSessionTokenAsync(process, plan!.SessionToken, plan.StartupTimeout, cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                KillProcessTree(process, candidateJob);
                await WaitForExitBoundedAsync(process).ConfigureAwait(false);
                process.Dispose();
                candidateJob.Dispose();
                return cancellationToken.IsCancellationRequested
                    ? Failure(WindowsVirtualCameraSidecarHostErrorCode.Cancelled, "虚拟摄像头 sidecar 令牌交付已取消。", true)
                    : Failure(WindowsVirtualCameraSidecarHostErrorCode.StartupTimedOut, "虚拟摄像头 sidecar 启动超时。", true);
            }
            catch (Exception exception) when (exception is IOException or InvalidOperationException or ObjectDisposedException)
            {
                KillProcessTree(process, candidateJob);
                await WaitForExitBoundedAsync(process).ConfigureAwait(false);
                process.Dispose();
                candidateJob.Dispose();
                return Failure(WindowsVirtualCameraSidecarHostErrorCode.TokenWriteFailed, "虚拟摄像头 sidecar 令牌交付失败。", true);
            }

            var runCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            lock (_gate)
            {
                _process = process;
                _job = candidateJob;
                _runCancellation = runCancellation;
                _monitorTask = null;
                _state = WindowsVirtualCameraSidecarHostState.Running;
                _exitCode = null;
                _lastErrorCode = null;
                _terminationReason = 0;
                _stopRequested = false;
                _pipeName = plan.PipeName;
            }

            var monitor = MonitorAsync(process, candidateJob, cancellationToken, runCancellation);
            lock (_gate)
            {
                _monitorTask = monitor;
            }
            PublishSnapshot();

            if (HasExited(process))
            {
                SetTerminationReasonIfUnset((int)TerminationReason.ProcessExited);
                await monitor.ConfigureAwait(false);
                return Failure(WindowsVirtualCameraSidecarHostErrorCode.ProcessExited, "虚拟摄像头 sidecar 启动后立即退出。", true);
            }

            return Succeeded();
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>通过宿主内部受保护的管道名连接现有传输客户端，不向 UI 暴露令牌或管道名。</summary>
    public async Task<WindowsVirtualCameraSidecarClientResult> ConnectClientAsync(
        WindowsVirtualCameraSidecarClient client,
        TimeSpan timeout,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(client);
        string? pipeName;
        lock (_gate)
        {
            pipeName = _state == WindowsVirtualCameraSidecarHostState.Running ? _pipeName : null;
        }

        return await client.ConnectAsync(pipeName, timeout, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>停止并在有限预算内终止 sidecar 进程树；可重复调用。</summary>
    public async Task<WindowsVirtualCameraSidecarHostResult> StopAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsVirtualCameraSidecarHostErrorCode.Cancelled, "虚拟摄像头 sidecar 停止已取消。", true);
        }

        try
        {
            if (_disposed)
            {
                return Succeeded();
            }

            Process? process;
            WindowsJobObject? job;
            Task? monitor;
            lock (_gate)
            {
                _stopRequested = true;
                _state = _process is null
                    ? WindowsVirtualCameraSidecarHostState.Stopped
                    : WindowsVirtualCameraSidecarHostState.Stopping;
                process = _process;
                job = _job;
                monitor = _monitorTask;
            }

            CancelRun();
            KillProcessTree(process, job);
            var monitorCompleted = true;
            if (monitor is not null)
            {
                try
                {
                    await monitor.WaitAsync(CleanupTimeout).ConfigureAwait(false);
                }
                catch (TimeoutException)
                {
                    monitorCompleted = false;
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
                _pipeName = null;
                _exitCode = exitCode;
                _lastErrorCode = monitorCompleted && exited ? null : WindowsVirtualCameraSidecarHostErrorCode.StopTimedOut;
                _state = _disposed ? WindowsVirtualCameraSidecarHostState.Closed : WindowsVirtualCameraSidecarHostState.Stopped;
            }
            PublishSnapshot();

            return monitorCompleted && exited
                ? Succeeded()
                : Failure(WindowsVirtualCameraSidecarHostErrorCode.StopTimedOut, "虚拟摄像头 sidecar 未能在停止预算内退出。", false);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <inheritdoc />
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
            Process? process;
            WindowsJobObject? job;
            Task? monitor;
            lock (_gate)
            {
                _stopRequested = true;
                process = _process;
                job = _job;
                monitor = _monitorTask;
            }

            CancelRun();
            KillProcessTree(process, job);
            if (monitor is not null)
            {
                try
                {
                    await monitor.WaitAsync(CleanupTimeout).ConfigureAwait(false);
                }
                catch (TimeoutException)
                {
                    // 进程句柄和 Job Object 由下方释放，避免异常 sidecar 阻塞宿主退出。
                }
            }

            DisposeProcess(process, job);
            lock (_gate)
            {
                _process = null;
                _job = null;
                _runCancellation?.Dispose();
                _runCancellation = null;
                _monitorTask = null;
                _pipeName = null;
                _state = WindowsVirtualCameraSidecarHostState.Closed;
            }
        }
        finally
        {
            _lifecycle.Release();
            _lifecycle.Dispose();
            GC.SuppressFinalize(this);
        }
    }

    private static bool TryValidatePlan(WindowsVirtualCameraSidecarLaunchPlan? plan)
    {
        if (plan is null
            || !OperatingSystem.IsWindows()
            || plan.Arguments.IsDefaultOrEmpty
            || plan.Arguments.Length != 1
            || !string.Equals(plan.Arguments[0], "--session-token-stdin", StringComparison.Ordinal)
            || plan.Config is null
            || plan.SessionToken is null
            || plan.PipeName is null
            || !plan.Config.TryValidateFixedOutput(out _)
            || plan.SessionToken.Length != 16
            || !WindowsVirtualCameraSidecarProtocol.TryCreatePipeName(plan.SessionToken, out var expectedPipe, out _)
            || !string.Equals(expectedPipe, plan.PipeName, StringComparison.Ordinal)
            || !WindowsVirtualCameraSidecarLaunchPlanBuilder.TryResolveValidatedSidecar(plan.ExecutablePath, out var executablePath))
        {
            return false;
        }

        try
        {
            return string.Equals(executablePath, plan.ExecutablePath, StringComparison.Ordinal)
                && string.Equals(
                    Path.GetDirectoryName(executablePath),
                    plan.WorkingDirectory,
                    StringComparison.Ordinal)
                && plan.StartupTimeout >= WindowsVirtualCameraSidecarLaunchPlanBuilder.MinStartupTimeout
                && plan.StartupTimeout <= WindowsVirtualCameraSidecarLaunchPlanBuilder.MaxStartupTimeout;
        }
        catch (ArgumentException)
        {
            return false;
        }
    }

    private static ProcessStartInfo CreateStartInfo(WindowsVirtualCameraSidecarLaunchPlan plan)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = plan.ExecutablePath,
            WorkingDirectory = plan.WorkingDirectory,
            UseShellExecute = false,
            CreateNoWindow = true,
            WindowStyle = ProcessWindowStyle.Hidden,
            RedirectStandardInput = true,
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

    private static async Task WriteSessionTokenAsync(
        Process process,
        byte[] sessionToken,
        TimeSpan timeout,
        CancellationToken cancellationToken)
    {
        var payload = ArrayPool<byte>.Shared.Rent(33);
        try
        {
            for (var index = 0; index < sessionToken.Length; index++)
            {
                var value = sessionToken[index];
                payload[index * 2] = HexDigits[value >> 4];
                payload[index * 2 + 1] = HexDigits[value & 0x0f];
            }

            payload[32] = (byte)'\n';
            using var timeoutSource = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            timeoutSource.CancelAfter(timeout);
            await process.StandardInput.BaseStream.WriteAsync(payload.AsMemory(0, 33), timeoutSource.Token).ConfigureAwait(false);
            await process.StandardInput.BaseStream.FlushAsync(timeoutSource.Token).ConfigureAwait(false);
            process.StandardInput.Close();
        }
        finally
        {
            CryptographicOperations.ZeroMemory(payload.AsSpan(0, 33));
            ArrayPool<byte>.Shared.Return(payload);
        }
    }

    private async Task MonitorAsync(
        Process process,
        WindowsJobObject job,
        CancellationToken callerCancellation,
        CancellationTokenSource runCancellation)
    {
        var outputLimit = 0;
        void SignalOutputLimit(int value)
        {
            if (Interlocked.CompareExchange(ref outputLimit, value, 0) == 0)
            {
                SetTerminationReasonIfUnset((int)TerminationReason.OutputLimit);
                CancelMonitor(runCancellation);
            }
        }

        void SignalOutputReadFailure()
        {
            SetTerminationReasonIfUnset((int)TerminationReason.OutputReadFailed);
            CancelMonitor(runCancellation);
        }

        var stdoutTask = DrainBoundedStdoutAsync(process.StandardOutput.BaseStream, MaxStandardOutputBytes, runCancellation.Token, () => SignalOutputLimit(1), SignalOutputReadFailure, PublishDownstreamClientCount);
        var stderrTask = DrainBoundedAsync(process.StandardError.BaseStream, MaxStandardErrorBytes, runCancellation.Token, () => SignalOutputLimit(2), SignalOutputReadFailure);
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
                    SetTerminationReasonIfUnset(_stopRequested || callerCancellation.IsCancellationRequested
                        ? (int)TerminationReason.Stop
                        : (int)TerminationReason.Cancelled);
                }

                KillProcessTree(process, job);
                await WaitForExitBoundedAsync(process).ConfigureAwait(false);
            }

            if (naturalExit)
            {
                try
                {
                    runCancellation.CancelAfter(Timeout.InfiniteTimeSpan);
                }
                catch (ObjectDisposedException)
                {
                }
            }

            try
            {
                await Task.WhenAll(stdoutTask, stderrTask).WaitAsync(CleanupTimeout).ConfigureAwait(false);
            }
            catch (TimeoutException)
            {
                SetTerminationReasonIfUnset((int)TerminationReason.OutputReadFailed);
                CancelMonitor(runCancellation);
            }
            catch (OperationCanceledException)
            {
                SetTerminationReasonIfUnset((int)TerminationReason.OutputReadFailed);
            }

            var exitCode = SafeExitCode(process);
            var reason = (TerminationReason)Volatile.Read(ref _terminationReason);
            WindowsVirtualCameraSidecarHostState state;
            WindowsVirtualCameraSidecarHostErrorCode? errorCode;
            switch (reason)
            {
                case TerminationReason.Stop:
                    state = WindowsVirtualCameraSidecarHostState.Stopped;
                    errorCode = null;
                    break;
                case TerminationReason.Cancelled:
                    state = WindowsVirtualCameraSidecarHostState.Failed;
                    errorCode = WindowsVirtualCameraSidecarHostErrorCode.Cancelled;
                    break;
                case TerminationReason.OutputLimit:
                    state = WindowsVirtualCameraSidecarHostState.Failed;
                    errorCode = WindowsVirtualCameraSidecarHostErrorCode.OutputLimitExceeded;
                    break;
                case TerminationReason.OutputReadFailed:
                    state = WindowsVirtualCameraSidecarHostState.Failed;
                    errorCode = WindowsVirtualCameraSidecarHostErrorCode.OutputReadFailed;
                    break;
                default:
                    state = WindowsVirtualCameraSidecarHostState.Exited;
                    errorCode = WindowsVirtualCameraSidecarHostErrorCode.ProcessExited;
                    break;
            }

            lock (_gate)
            {
                _state = _disposed ? WindowsVirtualCameraSidecarHostState.Closed : state;
                _exitCode = exitCode;
                _lastErrorCode = errorCode;
            }
            PublishSnapshot();
        }
        finally
        {
            // Process and Job Object handles remain owned by the lifecycle methods.
        }
    }

    private static async Task DrainBoundedAsync(
        Stream stream,
        int maximumBytes,
        CancellationToken cancellationToken,
        Action onLimit,
        Action onFailure)
    {
        var buffer = ArrayPool<byte>.Shared.Rent(ReadBufferBytes);
        var totalBytes = 0;
        try
        {
            while (true)
            {
                var requested = Math.Min(buffer.Length, maximumBytes - totalBytes + 1);
                var read = await stream.ReadAsync(buffer.AsMemory(0, Math.Max(1, requested)), cancellationToken).ConfigureAwait(false);
                if (read == 0)
                {
                    return;
                }

                totalBytes = checked(totalBytes + read);
                if (totalBytes > maximumBytes)
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

    private static async Task DrainBoundedStdoutAsync(
        Stream stream,
        int maximumBytes,
        CancellationToken cancellationToken,
        Action onLimit,
        Action onFailure,
        Action<uint> onClientCount)
    {
        var buffer = ArrayPool<byte>.Shared.Rent(ReadBufferBytes);
        var line = ArrayPool<byte>.Shared.Rent(WindowsVirtualCameraSidecarLaunchPlanBuilder.MaxStatusLineBytes + 1);
        var totalBytes = 0;
        var lineLength = 0;
        var lineTooLong = false;
        try
        {
            while (true)
            {
                var requested = Math.Min(buffer.Length, maximumBytes - totalBytes + 1);
                var read = await stream.ReadAsync(buffer.AsMemory(0, Math.Max(1, requested)), cancellationToken).ConfigureAwait(false);
                if (read == 0)
                {
                    return;
                }

                totalBytes = checked(totalBytes + read);
                if (totalBytes > maximumBytes)
                {
                    onLimit();
                    return;
                }

                for (var index = 0; index < read; index++)
                {
                    var value = buffer[index];
                    if (value == (byte)'\n')
                    {
                        if (!lineTooLong
                            && WindowsVirtualCameraSidecarStatusParser.TryParseClientCount(
                                line.AsSpan(0, lineLength),
                                out var clientCount))
                        {
                            onClientCount(clientCount);
                        }

                        lineLength = 0;
                        lineTooLong = false;
                        continue;
                    }

                    if (lineTooLong)
                    {
                        continue;
                    }

                    if (lineLength < WindowsVirtualCameraSidecarLaunchPlanBuilder.MaxStatusLineBytes)
                    {
                        line[lineLength++] = value;
                    }
                    else
                    {
                        lineLength = 0;
                        lineTooLong = true;
                    }
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
            ArrayPool<byte>.Shared.Return(line);
        }
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
                _pipeName = null;
            }
        }
    }

    private void CancelRun()
    {
        CancellationTokenSource? cancellation;
        lock (_gate)
        {
            cancellation = _runCancellation;
        }

        CancelMonitor(cancellation);
    }

    private static void CancelMonitor(CancellationTokenSource? cancellation)
    {
        if (cancellation is null)
        {
            return;
        }

        try
        {
            cancellation.Cancel();
        }
        catch (ObjectDisposedException)
        {
            // Stop/Dispose 与输出读取回调可能并发；已释放的 CTS 等价于已取消。
        }
    }

    private WindowsVirtualCameraSidecarHostSnapshot CreateSnapshot() =>
        new(_state, SafeProcessId(_process), _exitCode, _lastErrorCode);

    private void PublishSnapshot()
    {
        var snapshot = Snapshot;
        try
        {
            SnapshotChanged?.Invoke(this, snapshot);
        }
        catch
        {
            // UI 观察者不能破坏 sidecar 清理。
        }
    }

    private void PublishDownstreamClientCount(uint count)
    {
        try
        {
            DownstreamClientCountChanged?.Invoke(this, count);
        }
        catch
        {
            // UI/业务观察者不能破坏 stdout drain 和 sidecar 回收。
        }
    }

    private WindowsVirtualCameraSidecarHostResult Succeeded() => new(true, Snapshot);

    private WindowsVirtualCameraSidecarHostResult Failure(
        WindowsVirtualCameraSidecarHostErrorCode code,
        string message,
        bool retryable) => new(false, Snapshot, new(code, message, retryable));

    private void SetTerminationReasonIfUnset(int reason) =>
        Interlocked.CompareExchange(ref _terminationReason, reason, 0);

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

    private static bool IsProcessStartFailure(Exception exception) =>
        exception is InvalidOperationException
            or System.ComponentModel.Win32Exception
            or ArgumentException
            or NotSupportedException
            or UnauthorizedAccessException
            or System.Security.SecurityException;

    private enum TerminationReason
    {
        None,
        Stop,
        Cancelled,
        OutputLimit,
        OutputReadFailed,
        ProcessExited
    }
}
