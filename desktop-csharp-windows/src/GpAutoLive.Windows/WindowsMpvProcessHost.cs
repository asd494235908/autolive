using System.Diagnostics;
using GpAutoLive.Contracts;
using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>受管 mpv 进程宿主的稳定生命周期状态。</summary>
public enum WindowsMpvHostState
{
    Ready,
    Starting,
    Running,
    Exited,
    Stopping,
    Stopped,
    Faulted,
    Closed,
}

/// <summary>mpv 进程宿主的稳定错误分类。</summary>
public enum WindowsMpvHostFailureCode
{
    NotWindows,
    InvalidPlan,
    AlreadyRunning,
    StartFailed,
    ProcessExited,
    StopTimedOut,
    Cancelled,
}

/// <summary>不回显执行文件、参数或底层异常文本的宿主错误。</summary>
public sealed record WindowsMpvHostError(
    WindowsMpvHostFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>mpv 进程宿主的脱敏快照。</summary>
public sealed record WindowsMpvHostSnapshot(
    WindowsMpvHostState State,
    int? ProcessId,
    int? ExitCode);

/// <summary>启动/停止结果；成功时只返回状态快照和进程 ID。</summary>
public sealed record WindowsMpvHostResult(
    bool IsSuccess,
    WindowsMpvHostSnapshot Snapshot,
    WindowsMpvHostError? Error = null);

/// <summary>
/// Windows 专用的长生命周期 mpv 宿主。
/// 只消费 <see cref="MpvLaunchPlan"/>，不接受任意命令行，不读取媒体正文，
/// 并用 Job Object 优先、Process.Kill 兜底的方式回收进程树。
/// </summary>
public sealed class WindowsMpvProcessHost : IAsyncDisposable
{
    private static readonly TimeSpan CleanupTimeout = TimeSpan.FromSeconds(2);
    private readonly object _gate = new();
    private readonly SemaphoreSlim _lifecycle = new(1, 1);
    private Process? _process;
    private WindowsJobObject? _job;
    private WindowsMpvHostState _state = WindowsMpvHostState.Ready;
    private int? _exitCode;
    private bool _disposed;

    public WindowsMpvHostSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return new(_state, SafeProcessId(_process), _exitCode);
            }
        }
    }

    /// <summary>启动一条受管 mpv 会话；启动成功后由调用方连接对应 IPC 端点。</summary>
    public async Task<WindowsMpvHostResult> StartAsync(
        MpvLaunchPlan? plan,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(
                WindowsMpvHostFailureCode.Cancelled,
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
                WindowsMpvHostFailureCode.Cancelled,
                "mpv 启动已取消。",
                retryable: true);
        }

        try
        {
            if (_disposed)
            {
                return Failure(
                    WindowsMpvHostFailureCode.InvalidPlan,
                    "mpv 进程宿主已关闭。",
                    retryable: false);
            }

            if (!OperatingSystem.IsWindows())
            {
                return Failure(
                    WindowsMpvHostFailureCode.NotWindows,
                    "mpv 进程宿主只支持 Windows。",
                    retryable: false);
            }

            if (!TryValidatePlan(plan))
            {
                return Failure(
                    WindowsMpvHostFailureCode.InvalidPlan,
                    "mpv 启动计划无效。",
                    retryable: false);
            }

            if (_process is not null && !HasExited(_process))
            {
                return Failure(
                    WindowsMpvHostFailureCode.AlreadyRunning,
                    "mpv 会话已经在运行。",
                    retryable: false);
            }

            ReleaseExitedProcess();
            var process = new Process
            {
                StartInfo = CreateStartInfo(plan!),
                EnableRaisingEvents = true,
            };
            process.Exited += Process_Exited;
            lock (_gate)
            {
                _process = process;
                _job = null;
                _exitCode = null;
                _state = WindowsMpvHostState.Starting;
            }

            try
            {
                if (!process.Start())
                {
                    return await FailStartAsync(process).ConfigureAwait(false);
                }

                WindowsJobObject? createdJob = null;
                if (WindowsJobObject.TryCreate(out var candidateJob)
                    && candidateJob is not null
                    && candidateJob.TryAssign(process))
                {
                    createdJob = candidateJob;
                }
                else
                {
                    candidateJob?.Dispose();
                }

                lock (_gate)
                {
                    _job = createdJob;
                    _state = WindowsMpvHostState.Running;
                }

                if (HasExited(process))
                {
                    Process_Exited(process, EventArgs.Empty);
                    ReleaseImmediatelyExitedProcess(process);
                    return Failure(
                        WindowsMpvHostFailureCode.ProcessExited,
                        "mpv 启动后立即退出。",
                        retryable: true);
                }

                if (cancellationToken.IsCancellationRequested)
                {
                    await StopCoreAsync().ConfigureAwait(false);
                    return Failure(
                        WindowsMpvHostFailureCode.Cancelled,
                        "mpv 启动已取消。",
                        retryable: true);
                }

                return Succeeded();
            }
            catch (Exception exception) when (IsStartFailure(exception))
            {
                return await FailStartAsync(process).ConfigureAwait(false);
            }
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>停止 mpv 并在有界时间内回收进程、Job Object 和事件句柄。</summary>
    public async Task<WindowsMpvHostResult> StopAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(
                WindowsMpvHostFailureCode.Cancelled,
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
            lock (_gate)
            {
                _state = WindowsMpvHostState.Closed;
            }
        }
        finally
        {
            _lifecycle.Release();
        }

        GC.SuppressFinalize(this);
    }

    private async Task<WindowsMpvHostResult> StopCoreAsync()
    {
        Process? process;
        WindowsJobObject? job;
        lock (_gate)
        {
            process = _process;
            job = _job;
            if (process is null)
            {
                _state = _disposed ? WindowsMpvHostState.Closed : WindowsMpvHostState.Stopped;
                return Succeeded();
            }

            _state = WindowsMpvHostState.Stopping;
        }

        var exited = HasExited(process);
        if (!exited)
        {
            KillProcessTree(process, job);
            exited = await WaitForExitBoundedAsync(process).ConfigureAwait(false);
        }

        var exitCode = SafeExitCode(process);
        process.Exited -= Process_Exited;
        process.Dispose();
        job?.Dispose();
        lock (_gate)
        {
            _process = null;
            _job = null;
            _exitCode = exitCode;
            _state = _disposed ? WindowsMpvHostState.Closed : WindowsMpvHostState.Stopped;
        }

        return exited
            ? Succeeded()
            : Failure(
                WindowsMpvHostFailureCode.StopTimedOut,
                "mpv 未能在停止预算内退出。",
                retryable: false);
    }

    private async Task<WindowsMpvHostResult> FailStartAsync(Process process)
    {
        WindowsJobObject? job;
        lock (_gate)
        {
            job = _job;
            _state = WindowsMpvHostState.Faulted;
        }

        KillProcessTree(process, job);
        await WaitForExitBoundedAsync(process).ConfigureAwait(false);
        var exitCode = SafeExitCode(process);
        process.Exited -= Process_Exited;
        process.Dispose();
        job?.Dispose();
        lock (_gate)
        {
            _process = null;
            _job = null;
            _exitCode = exitCode;
        }

        return Failure(
            WindowsMpvHostFailureCode.StartFailed,
            "mpv 进程无法启动。",
            retryable: true);
    }

    private void ReleaseExitedProcess()
    {
        Process? process;
        WindowsJobObject? job;
        lock (_gate)
        {
            process = _process;
            job = _job;
            if (process is null || !HasExited(process))
            {
                return;
            }

            _process = null;
            _job = null;
            _state = WindowsMpvHostState.Stopped;
        }

        process.Exited -= Process_Exited;
        process.Dispose();
        job?.Dispose();
    }

    private void Process_Exited(object? sender, EventArgs e)
    {
        if (sender is not Process process)
        {
            return;
        }

        lock (_gate)
        {
            if (!ReferenceEquals(_process, process))
            {
                return;
            }

            _exitCode = SafeExitCode(process);
            if (_state is not (WindowsMpvHostState.Stopping or WindowsMpvHostState.Closed))
            {
                _state = WindowsMpvHostState.Exited;
            }
        }
    }

    private static ProcessStartInfo CreateStartInfo(MpvLaunchPlan plan)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = plan.ExecutablePath,
            WorkingDirectory = Path.GetDirectoryName(plan.ExecutablePath)!,
            UseShellExecute = false,
            CreateNoWindow = true,
            WindowStyle = ProcessWindowStyle.Hidden,
            RedirectStandardOutput = false,
            RedirectStandardError = false,
        };

        foreach (var argument in plan.Arguments)
        {
            startInfo.ArgumentList.Add(argument);
        }

        return startInfo;
    }

    private static bool TryValidatePlan(MpvLaunchPlan? plan)
    {
        if (plan is null
            || string.IsNullOrWhiteSpace(plan.ExecutablePath)
            || !Path.IsPathFullyQualified(plan.ExecutablePath)
            || !File.Exists(plan.ExecutablePath)
            || plan.Arguments.IsDefaultOrEmpty
            || plan.Arguments.Length < 2
            || plan.Arguments.Length > MpvLaunchPlan.MaxArgumentCount
            || plan.IpcEndpoint is null
            || plan.HostWindowId is 0
            || plan.MediaPath.Kind is not MediaKind.Video
            || !string.Equals(plan.Arguments[0], "--no-config", StringComparison.Ordinal)
            || !string.Equals(plan.Arguments[^2], "--", StringComparison.Ordinal)
            || !string.Equals(plan.Arguments[^1], plan.MediaPath.CanonicalPath, StringComparison.Ordinal))
        {
            return false;
        }

        try
        {
            var commandLength = plan.ExecutablePath.Length;
            foreach (var argument in plan.Arguments)
            {
                if (argument.Any(char.IsControl) || argument.Length > MpvLaunchPlan.MaxArgumentCharacters)
                {
                    return false;
                }

                commandLength = checked(commandLength + argument.Length + 1);
            }

            return commandLength <= 32_767;
        }
        catch (ArgumentException)
        {
            return false;
        }
    }

    private void ReleaseImmediatelyExitedProcess(Process process)
    {
        WindowsJobObject? job;
        var exitCode = SafeExitCode(process);
        lock (_gate)
        {
            if (!ReferenceEquals(_process, process))
            {
                return;
            }

            job = _job;
            _process = null;
            _job = null;
            _exitCode = exitCode;
            _state = WindowsMpvHostState.Exited;
        }

        process.Exited -= Process_Exited;
        process.Dispose();
        job?.Dispose();
    }

    private static void KillProcessTree(Process process, WindowsJobObject? job)
    {
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

    private static async Task<bool> WaitForExitBoundedAsync(Process process)
    {
        try
        {
            await process.WaitForExitAsync()
                .WaitAsync(CleanupTimeout)
                .ConfigureAwait(false);
            return true;
        }
        catch (InvalidOperationException)
        {
            return true;
        }
        catch (TimeoutException)
        {
            return HasExited(process);
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
        if (process is null)
        {
            return null;
        }

        try
        {
            return process.HasExited ? process.ExitCode : null;
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

    private WindowsMpvHostResult Succeeded() =>
        new(true, Snapshot);

    private WindowsMpvHostResult Failure(
        WindowsMpvHostFailureCode code,
        string message,
        bool retryable) =>
        new(false, Snapshot, new(code, message, retryable));

    private static bool IsStartFailure(Exception exception) =>
        exception is InvalidOperationException
            or System.ComponentModel.Win32Exception
            or ArgumentException
            or NotSupportedException
            or UnauthorizedAccessException
            or System.Security.SecurityException;
}
