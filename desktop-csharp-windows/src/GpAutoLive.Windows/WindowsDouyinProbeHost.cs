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
    private static readonly TimeSpan ReplyResponseTimeout = TimeSpan.FromSeconds(10);
    private static readonly TimeSpan ReplyMinimumInterval = TimeSpan.FromSeconds(3);
    private static readonly TimeSpan ReplyRateWindow = TimeSpan.FromMinutes(1);
    private static readonly TimeSpan ReplyRiskCooldown = TimeSpan.FromSeconds(60);
    private static readonly TimeSpan CommandResponseTimeout = TimeSpan.FromSeconds(10);
    private const int ReplyMaximumPerMinute = 5;
    private static readonly TimeSpan CleanupTimeout = TimeSpan.FromSeconds(2);
    private static readonly UTF8Encoding Utf8 = new(false, false);

    private readonly object _gate = new();
    private readonly SemaphoreSlim _lifecycle = new(1, 1);
    private readonly SemaphoreSlim _replySendGate = new(1, 1);
    private readonly SemaphoreSlim _replySignal = new(0);
    private readonly DouyinLiveManager _manager;
    private readonly Func<ProcessStartInfo, ProcessStartInfo>? _processStartInfoFactory;
    private Process? _process;
    private WindowsJobObject? _job;
    private CancellationTokenSource? _runCancellation;
    private Task? _monitorTask;
    private WindowsDouyinProbeHostState _state = WindowsDouyinProbeHostState.Ready;
    private int? _exitCode;
    private string? _qrPath;
    private string? _expectedRoomId;
    private string? _expectedSessionId;
    private ulong? _expectedGeneration;
    private PendingReplyResponse? _pendingReplyResponse;
    private DateTimeOffset? _lastReplyDispatchUtc;
    private DateTimeOffset? _replyBlockedUntilUtc;
    private readonly Queue<DateTimeOffset> _replyDispatchHistory = new();
    private string? _lastEvent;
    private int _invalidEventCount;
    private int _terminationReason;
    private bool _stopRequested;
    private bool _disposed;
    private WindowsDouyinProbeProtocol _protocol;
    private PendingCommandResponse? _pendingCommandResponse;
    private PendingLiveOpenResponse? _pendingLiveOpenResponse;
    private TaskCompletionSource<WindowsDouyinProbeEvent?>? _authConfirmation;

    /// <summary>创建使用指定核心状态所有者的宿主。</summary>
    public WindowsDouyinProbeHost(DouyinLiveManager manager)
        : this(manager, null)
    {
    }

    internal WindowsDouyinProbeHost(
        DouyinLiveManager manager,
        Func<ProcessStartInfo, ProcessStartInfo>? processStartInfoFactory)
    {
        _manager = manager ?? throw new ArgumentNullException(nameof(manager));
        _processStartInfoFactory = processStartInfoFactory;
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

            while (_replySignal.Wait(0))
            {
            }

            var process = new Process
            {
                StartInfo = _processStartInfoFactory is null
                    ? CreateStartInfo(plan, request.UpstreamRoot)
                    : _processStartInfoFactory(CreateStartInfo(plan, request.UpstreamRoot)),
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
                _protocol = request.Protocol;
                _state = WindowsDouyinProbeHostState.Running;
                _exitCode = null;
                _qrPath = null;
                _expectedRoomId = request.Config.RoomId;
                _expectedSessionId = null;
                _expectedGeneration = null;
                _pendingReplyResponse = null;
                _pendingCommandResponse = null;
                _pendingLiveOpenResponse = null;
                _authConfirmation = request.Protocol == WindowsDouyinProbeProtocol.CanonicalNdjson
                    ? new TaskCompletionSource<WindowsDouyinProbeEvent?>(TaskCreationOptions.RunContinuationsAsynchronously)
                    : null;
                _lastReplyDispatchUtc = null;
                _replyBlockedUntilUtc = null;
                _replyDispatchHistory.Clear();
                _lastEvent = "probe_started";
                _invalidEventCount = 0;
                _terminationReason = 0;
                _stopRequested = false;
            }

            var monitor = MonitorAsync(
                process,
                job,
                request.QrOutputPath,
                request.Config.RoomId,
                request.Protocol,
                _manager.Snapshot.Generation,
                cancellationToken,
                runCancellation);
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
                await TryGracefulStopAsync(process).ConfigureAwait(false);
            }

            CancelRun();
            CloseStandardInput(process);
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
                _protocol = WindowsDouyinProbeProtocol.LegacyEvents;
                _expectedRoomId = null;
                _expectedSessionId = null;
                _expectedGeneration = null;
                _pendingCommandResponse = null;
                _pendingLiveOpenResponse = null;
                _authConfirmation = null;
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

            CloseStandardInput(process);
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
                _protocol = WindowsDouyinProbeProtocol.LegacyEvents;
                _expectedRoomId = null;
                _expectedSessionId = null;
                _expectedGeneration = null;
                _pendingCommandResponse = null;
                _pendingLiveOpenResponse = null;
                _authConfirmation = null;
            }
        }
        finally
        {
            _lifecycle.Release();
            _lifecycle.Dispose();
            _replySignal.Dispose();
            _replySendGate.Dispose();
            GC.SuppressFinalize(this);
        }
    }

    private async Task MonitorAsync(
        Process process,
        WindowsJobObject? job,
        string qrPath,
        string webRid,
        WindowsDouyinProbeProtocol protocol,
        ulong generation,
        CancellationToken callerCancellation,
        CancellationTokenSource runCancellation)
    {
        var processExitTask = process.WaitForExitAsync(runCancellation.Token);
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
        using var replyCancellation = CancellationTokenSource.CreateLinkedTokenSource(runCancellation.Token);
        var replyTask = DrainRepliesAsync(process, processExitTask, replyCancellation.Token);
        var canonicalTask = protocol == WindowsDouyinProbeProtocol.CanonicalNdjson
            ? RunCanonicalSessionAsync(process, webRid, generation, runCancellation.Token)
            : Task.CompletedTask;
        var naturalExit = false;
        try
        {
            try
            {
                await processExitTask.ConfigureAwait(false);
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

            replyCancellation.Cancel();
            CancelPendingReplyResponse();
            try
            {
                await Task.WhenAll(stdoutTask, stderrTask, replyTask, canonicalTask)
                    .WaitAsync(CleanupTimeout)
                    .ConfigureAwait(false);
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

    private async Task TryGracefulStopAsync(Process process)
    {
        WindowsDouyinProbeProtocol protocol;
        DouyinLiveState state;
        string? sessionId;
        ulong? generation;
        lock (_gate)
        {
            protocol = _protocol;
            sessionId = _expectedSessionId;
            generation = _expectedGeneration;
        }

        if (protocol != WindowsDouyinProbeProtocol.CanonicalNdjson)
        {
            return;
        }

        state = _manager.Snapshot.State;
        var cancelRequestId = CreateRequestId("cancel");
        if (state == DouyinLiveState.WaitingQr
            && WindowsDouyinSidecarProtocol.TrySerializeAuthCancel(
                cancelRequestId,
                out var cancelLine,
                out _))
        {
            await SendGracefulCommandAsync(process, cancelRequestId, cancelLine).ConfigureAwait(false);
        }
        else
        {
            var closeRequestId = CreateRequestId("close");
            if (state is (DouyinLiveState.RoomResolved or DouyinLiveState.Listening or DouyinLiveState.Paused)
                && sessionId is not null
                && generation is > 0
                && WindowsDouyinSidecarProtocol.TrySerializeLiveClose(
                    closeRequestId,
                    sessionId,
                    generation.Value,
                    out var closeLine,
                    out _))
            {
                await SendGracefulCommandAsync(process, closeRequestId, closeLine).ConfigureAwait(false);
            }

            var logoutRequestId = CreateRequestId("logout");
            if (WindowsDouyinSidecarProtocol.TrySerializeAuthLogout(
                    logoutRequestId,
                    out var logoutLine,
                    out _))
            {
                await SendGracefulCommandAsync(process, logoutRequestId, logoutLine).ConfigureAwait(false);
            }
        }

        var shutdownRequestId = CreateRequestId("shutdown");
        if (WindowsDouyinSidecarProtocol.TrySerializeShutdown(
                shutdownRequestId,
                out var shutdownLine,
                out _))
        {
            await SendGracefulCommandAsync(process, shutdownRequestId, shutdownLine).ConfigureAwait(false);
        }
    }

    private async Task SendGracefulCommandAsync(
        Process process,
        string requestId,
        string line)
    {
        using var timeout = new CancellationTokenSource(TimeSpan.FromMilliseconds(750));
        await SendCommandAsync(process, requestId, line, timeout.Token).ConfigureAwait(false);
    }

    private async Task DrainRepliesAsync(
        Process process,
        Task processExitTask,
        CancellationToken cancellationToken)
    {
        try
        {
            while (!cancellationToken.IsCancellationRequested)
            {
                var signalTask = _replySignal.WaitAsync(cancellationToken);
                var completed = await Task.WhenAny(signalTask, processExitTask).ConfigureAwait(false);
                if (completed == processExitTask)
                {
                    return;
                }

                await signalTask.ConfigureAwait(false);
                while (TryGetCanonicalIdentity(out var sessionId, out var generation))
                {
                    if (!await WaitForReplyBudgetAsync(cancellationToken).ConfigureAwait(false))
                    {
                        break;
                    }

                    if (!TryGetCanonicalIdentity(out sessionId, out generation)
                        || !_manager.TryDequeue(
                            DateTimeOffset.UtcNow,
                            out var task,
                            expectedGeneration: generation)
                        || task is null)
                    {
                        break;
                    }

                    var outcome = await SendReplyTaskAsync(
                        process,
                        sessionId,
                        generation,
                        task,
                        cancellationToken).ConfigureAwait(false);
                    _manager.RecordSendOutcome(outcome);
                    PublishSnapshot();
                }
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
    }

    private async Task RunCanonicalSessionAsync(
        Process process,
        string webRid,
        ulong generation,
        CancellationToken cancellationToken)
    {
        try
        {
            var qrRequestId = CreateRequestId("qr");
            if (!WindowsDouyinSidecarProtocol.TrySerializeAuthQrStart(
                    qrRequestId,
                    out var qrLine,
                    out _))
            {
                FailCanonicalSession("sidecar 二维码登录请求无效");
                return;
            }

            var qrResponse = await SendCommandAsync(
                    process,
                    qrRequestId,
                    qrLine,
                    cancellationToken)
                .ConfigureAwait(false);
            if (qrResponse is null || !qrResponse.IsSuccess)
            {
                FailCanonicalSession("sidecar 二维码登录启动失败");
                return;
            }

            var confirmed = await WaitForAuthConfirmationAsync(cancellationToken).ConfigureAwait(false);
            if (confirmed is null)
            {
                return;
            }

            if (_manager.Snapshot.State != DouyinLiveState.LoggedIn)
            {
                FailCanonicalSession("sidecar 登录状态未确认");
                return;
            }

            var openRequestId = CreateRequestId("open");
            if (!WindowsDouyinSidecarProtocol.TrySerializeLiveOpen(
                    openRequestId,
                    webRid,
                    generation,
                    out var openLine,
                    out _))
            {
                FailCanonicalSession("sidecar 直播间请求无效");
                return;
            }

            var openResponse = await SendLiveOpenAsync(
                    process,
                    openRequestId,
                    generation,
                    openLine,
                    cancellationToken)
                .ConfigureAwait(false);
            if (openResponse is null || !openResponse.IsSuccess)
            {
                FailCanonicalSession("sidecar 直播间打开失败");
                return;
            }

            if (openResponse.LiveStatus is "room_ended" or "failed")
            {
                _manager.MarkInconclusive("直播间当前不可监听");
                CancelRun();
                PublishSnapshot();
                return;
            }

            var managerState = _manager.Snapshot.State;
            if (managerState == DouyinLiveState.LoggedIn
                && !_manager.MarkRoomResolved().IsSuccess)
            {
                FailCanonicalSession("sidecar 直播间状态未能接入");
                return;
            }

            if (_manager.Snapshot.State is not (DouyinLiveState.RoomResolved or DouyinLiveState.Listening))
            {
                FailCanonicalSession("sidecar 直播间状态未能接入");
                return;
            }

            PublishSnapshot();
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch (IOException)
        {
            FailCanonicalSession("sidecar 命令通道不可用");
        }
        catch (InvalidOperationException)
        {
            FailCanonicalSession("sidecar 命令通道不可用");
        }
    }

    private async Task<WindowsDouyinCommandResponse?> SendCommandAsync(
        Process process,
        string requestId,
        string line,
        CancellationToken cancellationToken)
    {
        var completion = new TaskCompletionSource<WindowsDouyinCommandResponse?>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        lock (_gate)
        {
            if (_pendingCommandResponse is not null || _pendingLiveOpenResponse is not null)
            {
                return null;
            }

            _pendingCommandResponse = new(requestId, completion);
        }

        try
        {
            using var ioCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            ioCancellation.CancelAfter(CommandResponseTimeout);
            await process.StandardInput.WriteLineAsync(line.AsMemory(), ioCancellation.Token).ConfigureAwait(false);
            await process.StandardInput.FlushAsync(ioCancellation.Token).ConfigureAwait(false);
            return await completion.Task
                .WaitAsync(CommandResponseTimeout, cancellationToken)
                .ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return null;
        }
        catch (OperationCanceledException)
        {
            return null;
        }
        catch (TimeoutException)
        {
            return null;
        }
        catch (IOException)
        {
            return null;
        }
        catch (InvalidOperationException)
        {
            return null;
        }
        finally
        {
            lock (_gate)
            {
                if (_pendingCommandResponse?.RequestId == requestId)
                {
                    _pendingCommandResponse = null;
                }
            }
        }
    }

    private async Task<WindowsDouyinLiveOpenResponse?> SendLiveOpenAsync(
        Process process,
        string requestId,
        ulong generation,
        string line,
        CancellationToken cancellationToken)
    {
        var completion = new TaskCompletionSource<WindowsDouyinLiveOpenResponse?>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        lock (_gate)
        {
            if (_pendingCommandResponse is not null || _pendingLiveOpenResponse is not null)
            {
                return null;
            }

            _pendingLiveOpenResponse = new(requestId, generation, completion);
        }

        try
        {
            using var ioCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            ioCancellation.CancelAfter(CommandResponseTimeout);
            await process.StandardInput.WriteLineAsync(line.AsMemory(), ioCancellation.Token).ConfigureAwait(false);
            await process.StandardInput.FlushAsync(ioCancellation.Token).ConfigureAwait(false);
            return await completion.Task
                .WaitAsync(CommandResponseTimeout, cancellationToken)
                .ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return null;
        }
        catch (OperationCanceledException)
        {
            return null;
        }
        catch (TimeoutException)
        {
            return null;
        }
        catch (IOException)
        {
            return null;
        }
        catch (InvalidOperationException)
        {
            return null;
        }
        finally
        {
            lock (_gate)
            {
                if (_pendingLiveOpenResponse?.RequestId == requestId)
                {
                    _pendingLiveOpenResponse = null;
                }
            }
        }
    }

    private async Task<WindowsDouyinProbeEvent?> WaitForAuthConfirmationAsync(
        CancellationToken cancellationToken)
    {
        TaskCompletionSource<WindowsDouyinProbeEvent?>? completion;
        lock (_gate)
        {
            completion = _authConfirmation;
        }

        if (completion is null)
        {
            return null;
        }

        try
        {
            return await completion.Task.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return null;
        }
    }

    private void FailCanonicalSession(string reason)
    {
        _manager.Fail(reason);
        CancelRun();
        PublishSnapshot();
    }

    private static string CreateRequestId(string prefix) => $"{prefix}-{Guid.NewGuid():N}";

    private async Task<bool> WaitForReplyBudgetAsync(CancellationToken cancellationToken)
    {
        while (true)
        {
            TimeSpan delay;
            lock (_gate)
            {
                var now = DateTimeOffset.UtcNow;
                if ((_replyBlockedUntilUtc is { } blockedUntil && now < blockedUntil)
                    || _manager.Snapshot.ReplySendingBlocked)
                {
                    return false;
                }

                while (_replyDispatchHistory.Count > 0
                    && now - _replyDispatchHistory.Peek() >= ReplyRateWindow)
                {
                    _replyDispatchHistory.Dequeue();
                }

                var nextAllowed = now;
                if (_lastReplyDispatchUtc is { } lastDispatch)
                {
                    nextAllowed = Max(nextAllowed, lastDispatch + ReplyMinimumInterval);
                }

                if (_replyDispatchHistory.Count >= ReplyMaximumPerMinute)
                {
                    nextAllowed = Max(nextAllowed, _replyDispatchHistory.Peek() + ReplyRateWindow);
                }

                delay = nextAllowed - now;
            }

            if (delay <= TimeSpan.Zero)
            {
                return true;
            }

            await Task.Delay(delay, cancellationToken).ConfigureAwait(false);
        }
    }

    private async Task<DouyinSendOutcome> SendReplyTaskAsync(
        Process process,
        string sessionId,
        ulong generation,
        DouyinReplyTask task,
        CancellationToken cancellationToken)
    {
        var clientActionId = task.ClientActionId;
        var request = new WindowsDouyinChatSendRequest(
            sessionId,
            generation,
            clientActionId,
            task.ReplyText);
        if (!WindowsDouyinSidecarProtocol.TrySerializeChatSend(request, out var line, out _))
        {
            return DouyinSendOutcome.NotSent;
        }

        try
        {
            await _replySendGate.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return DouyinSendOutcome.NotSent;
        }

        var writeStarted = false;
        try
        {
            if (HasExited(process)
                || !TryGetCanonicalIdentity(out var currentSessionId, out var currentGeneration)
                || !string.Equals(currentSessionId, sessionId, StringComparison.Ordinal)
                || currentGeneration != generation
                || task.Generation != generation)
            {
                return DouyinSendOutcome.NotSent;
            }

            var completion = new TaskCompletionSource<WindowsDouyinSidecarResponse?>(
                TaskCreationOptions.RunContinuationsAsynchronously);
            lock (_gate)
            {
                if (_pendingReplyResponse is not null)
                {
                    return DouyinSendOutcome.OutcomeUnknown;
                }

                _pendingReplyResponse = new(clientActionId, completion);
            }

            try
            {
                using var ioCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
                ioCancellation.CancelAfter(ReplyResponseTimeout);
                writeStarted = true;
                await process.StandardInput.WriteLineAsync(line.AsMemory(), ioCancellation.Token).ConfigureAwait(false);
                await process.StandardInput.FlushAsync(ioCancellation.Token).ConfigureAwait(false);
                MarkReplyDispatched();

                var response = await completion.Task
                    .WaitAsync(ReplyResponseTimeout, cancellationToken)
                    .ConfigureAwait(false);
                if (response is null
                    || !string.Equals(response.RequestId, clientActionId, StringComparison.Ordinal)
                    || (response.ClientActionId is not null
                        && !string.Equals(response.ClientActionId, clientActionId, StringComparison.Ordinal)))
                {
                    return DouyinSendOutcome.OutcomeUnknown;
                }

                if (!response.IsSuccess
                    && response.ErrorCode is "auth_expired" or "rate_limited" or "risk_controlled")
                {
                    BlockReplySending();
                }

                return response.Outcome;
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                return writeStarted ? DouyinSendOutcome.OutcomeUnknown : DouyinSendOutcome.NotSent;
            }
            catch (OperationCanceledException)
            {
                return writeStarted ? DouyinSendOutcome.OutcomeUnknown : DouyinSendOutcome.NotSent;
            }
            catch (TimeoutException)
            {
                return writeStarted ? DouyinSendOutcome.OutcomeUnknown : DouyinSendOutcome.NotSent;
            }
            catch (IOException)
            {
                return writeStarted ? DouyinSendOutcome.OutcomeUnknown : DouyinSendOutcome.NotSent;
            }
            catch (InvalidOperationException)
            {
                return writeStarted ? DouyinSendOutcome.OutcomeUnknown : DouyinSendOutcome.NotSent;
            }
        }
        finally
        {
            lock (_gate)
            {
                if (_pendingReplyResponse?.RequestId == clientActionId)
                {
                    _pendingReplyResponse = null;
                }
            }

            try
            {
                _replySendGate.Release();
            }
            catch (ObjectDisposedException)
            {
                // 仅在有界清理与未受控底层写入竞态同时发生时兜底。
            }
        }
    }

    private bool TryGetCanonicalIdentity(out string sessionId, out ulong generation)
    {
        lock (_gate)
        {
            sessionId = _expectedSessionId ?? string.Empty;
            generation = _expectedGeneration ?? 0;
        }

        var managerSnapshot = _manager.Snapshot;
        return sessionId.Length > 0
            && generation > 0
            && !managerSnapshot.ReplySendingBlocked
            && managerSnapshot.State == DouyinLiveState.Listening;
    }

    private void BlockReplySending()
    {
        lock (_gate)
        {
            _replyBlockedUntilUtc = Max(
                _replyBlockedUntilUtc ?? DateTimeOffset.MinValue,
                DateTimeOffset.UtcNow + ReplyRiskCooldown);
        }

        _manager.BlockReplySending("sidecar 进入风控/限流状态");
    }

    private void MarkReplyDispatched()
    {
        lock (_gate)
        {
            var now = DateTimeOffset.UtcNow;
            _lastReplyDispatchUtc = now;
            _replyDispatchHistory.Enqueue(now);
        }
    }

    private static DateTimeOffset Max(DateTimeOffset left, DateTimeOffset right) =>
        left >= right ? left : right;

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

        string? expectedRoomId;
        string? expectedSessionId;
        ulong? expectedGeneration;
        lock (_gate)
        {
            expectedRoomId = _expectedRoomId;
            expectedSessionId = _expectedSessionId;
            expectedGeneration = _expectedGeneration;
        }

        PendingCommandResponse? pendingCommand;
        PendingLiveOpenResponse? pendingLiveOpen;
        lock (_gate)
        {
            pendingCommand = _pendingCommandResponse;
            pendingLiveOpen = _pendingLiveOpenResponse;
        }

        if (pendingLiveOpen is not null
            && WindowsDouyinSidecarProtocol.TryParseLiveOpenResponse(
                line,
                pendingLiveOpen.RequestId,
                out var liveOpenResponse,
                out _))
        {
            if (CompletePendingLiveOpenResponse(pendingLiveOpen, liveOpenResponse!))
            {
                return true;
            }

            return true;
        }

        if (pendingCommand is not null
            && WindowsDouyinSidecarProtocol.TryParseCommandResponse(
                line,
                pendingCommand.RequestId,
                out var commandResponse,
                out _))
        {
            if (pendingCommand.Completion.TrySetResult(commandResponse))
            {
                lock (_gate)
                {
                    if (ReferenceEquals(_pendingCommandResponse, pendingCommand))
                    {
                        _pendingCommandResponse = null;
                    }
                }
            }

            return true;
        }

        if (WindowsDouyinSidecarProtocol.TryParseResponse(line, out var response, out _))
        {
            if (CompletePendingReplyResponse(response!))
            {
                return true;
            }

            // 合法但迟到/未知请求的 response 不属于事件，也不能累计为无效事件。
            return true;
        }

        if (WindowsDouyinProbeEventParser.TryParse(
                line,
                out var probeEvent,
                out _,
                expectedRoomId,
                expectedSessionId,
                expectedGeneration))
        {
            if (IsCanonicalUnboundEvent(probeEvent!))
            {
                SetTerminationReasonIfUnset(4);
                CancelRun();
                return false;
            }

            if (probeEvent!.SessionId is not null)
            {
                lock (_gate)
                {
                    _expectedSessionId ??= probeEvent.SessionId;
                    _expectedGeneration ??= probeEvent.Generation;
                }
            }

            ApplyEvent(probeEvent!, qrPath);
            if (probeEvent.Kind == WindowsDouyinProbeEventKind.AuthState
                && probeEvent.State == "confirmed"
                && _manager.Snapshot.State == DouyinLiveState.LoggedIn)
            {
                lock (_gate)
                {
                    _authConfirmation?.TrySetResult(probeEvent);
                }
            }
        }
        else if (Interlocked.Increment(ref _invalidEventCount) > MaxInvalidEvents)
        {
            SetTerminationReasonIfUnset(4);
            CancelRun();
            return false;
        }

        return true;
    }

    private bool CompletePendingReplyResponse(WindowsDouyinSidecarResponse response)
    {
        PendingReplyResponse? pending;
        lock (_gate)
        {
            pending = _pendingReplyResponse;
        }

        return pending is not null
            && string.Equals(pending.RequestId, response.RequestId, StringComparison.Ordinal)
            && pending.Completion.TrySetResult(response);
    }

    private bool CompletePendingLiveOpenResponse(
        PendingLiveOpenResponse pending,
        WindowsDouyinLiveOpenResponse response)
    {
        if (!response.IsSuccess
            || string.IsNullOrWhiteSpace(response.SessionId))
        {
            return pending.Completion.TrySetResult(response);
        }

        var shouldResolveRoom = response.LiveStatus is not ("room_ended" or "failed");
        lock (_gate)
        {
            if (!ReferenceEquals(_pendingLiveOpenResponse, pending))
            {
                return false;
            }

            _expectedSessionId = response.SessionId;
            _expectedGeneration = pending.Generation;
        }

        if (shouldResolveRoom
            && !_manager.MarkRoomResolved().IsSuccess
            && _manager.Snapshot.State != DouyinLiveState.RoomResolved)
        {
            return pending.Completion.TrySetResult(new(
                response.RequestId,
                false,
                ErrorCode: "protocol_invalid",
                IsRetryable: false,
                Outcome: DouyinSendOutcome.NotSent));
        }

        PublishSnapshot();
        return pending.Completion.TrySetResult(response);
    }

    private bool IsCanonicalUnboundEvent(WindowsDouyinProbeEvent probeEvent)
    {
        lock (_gate)
        {
            return _protocol == WindowsDouyinProbeProtocol.CanonicalNdjson
                && probeEvent.Kind is (WindowsDouyinProbeEventKind.LiveState
                    or WindowsDouyinProbeEventKind.ChatReceived)
                && (_expectedSessionId is null || _expectedGeneration is null);
        }
    }

    private void CancelPendingReplyResponse()
    {
        PendingReplyResponse? pending;
        lock (_gate)
        {
            pending = _pendingReplyResponse;
        }

        pending?.Completion.TrySetResult(null);
        lock (_gate)
        {
            _pendingReplyResponse = null;
            _pendingCommandResponse?.Completion.TrySetResult(null);
            _pendingLiveOpenResponse?.Completion.TrySetResult(null);
            _pendingCommandResponse = null;
            _pendingLiveOpenResponse = null;
            _authConfirmation?.TrySetResult(null);
        }
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
        var qrPathReady = probeEvent.QrPngBytes is { Length: > 0 }
            && probeEvent.QrExpiresAtUtc is { } qrExpiresAtUtc
            && TryWriteQrPng(qrPath, probeEvent.QrPngBytes, qrExpiresAtUtc);
        lock (_gate)
        {
            _lastEvent = probeEvent.Name;
            if (probeEvent.Kind == WindowsDouyinProbeEventKind.QrIssued
                && (qrPathReady
                    || (probeEvent.QrPngBytes is null
                        && File.Exists(qrPath)
                        && !HasReparsePoint(qrPath))))
            {
                _qrPath = qrPath;
            }
        }

        var bridgeResult = WindowsDouyinProbeEventBridge.Apply(_manager, probeEvent);
        if (bridgeResult is { IsSuccess: false })
        {
            if (_manager.Snapshot.State != DouyinLiveState.Failed)
            {
                var failed = _manager.Fail("sidecar 事件未通过本地状态校验");
                bridgeResult = new(false, failed.Snapshot, bridgeResult.Error);
            }

            lock (_gate)
            {
                _lastEvent = bridgeResult.Snapshot.LastEvent;
            }
        }

        if (probeEvent.SessionId is not null || probeEvent.Kind == WindowsDouyinProbeEventKind.ChatReceived)
        {
            try
            {
                _replySignal.Release();
            }
            catch (ObjectDisposedException)
            {
                // 宿主已开始释放；不再唤醒发送循环。
            }
        }

        PublishSnapshot();
    }

    internal static bool TryWriteQrPng(string path, byte[] bytes, DateTimeOffset expiresAtUtc)
    {
        if (expiresAtUtc <= DateTimeOffset.UtcNow
            || bytes.Length is 0 or > 256 * 1024
            || !IsPng(bytes))
        {
            return false;
        }

        try
        {
            var fullPath = Path.GetFullPath(path);
            if (File.Exists(fullPath) || Directory.Exists(fullPath))
            {
                return false;
            }

            var temporaryPath = $"{fullPath}.{Guid.NewGuid():N}.partial";
            try
            {
                using (var stream = new FileStream(
                    temporaryPath,
                    FileMode.CreateNew,
                    FileAccess.Write,
                    FileShare.Read,
                    bufferSize: 4096,
                    options: FileOptions.SequentialScan))
                {
                    stream.Write(bytes, 0, bytes.Length);
                    stream.Flush(flushToDisk: true);
                }

                File.Move(temporaryPath, fullPath);
                return File.Exists(fullPath) && !HasReparsePoint(fullPath);
            }
            finally
            {
                try
                {
                    if (File.Exists(temporaryPath))
                    {
                        File.Delete(temporaryPath);
                    }
                }
                catch (IOException)
                {
                }
                catch (UnauthorizedAccessException)
                {
                }
            }
        }
        catch (ArgumentException)
        {
            return false;
        }
        catch (IOException)
        {
            return false;
        }
        catch (UnauthorizedAccessException)
        {
            return false;
        }
        catch (System.Security.SecurityException)
        {
            return false;
        }
        catch (NotSupportedException)
        {
            return false;
        }
    }

    private static bool IsPng(byte[] bytes) =>
        bytes.Length >= 8
        && bytes[0] == 0x89
        && bytes[1] == 0x50
        && bytes[2] == 0x4E
        && bytes[3] == 0x47
        && bytes[4] == 0x0D
        && bytes[5] == 0x0A
        && bytes[6] == 0x1A
        && bytes[7] == 0x0A;

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
        CancelPendingReplyResponse();
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
            RedirectStandardInput = true,
            StandardOutputEncoding = Utf8,
            StandardErrorEncoding = Utf8,
            StandardInputEncoding = Utf8
        };

        foreach (var argument in plan.Arguments)
        {
            startInfo.ArgumentList.Add(argument);
        }

        return startInfo;
    }

    private static void CloseStandardInput(Process? process)
    {
        try
        {
            process?.StandardInput.Close();
        }
        catch (InvalidOperationException)
        {
        }
        catch (IOException)
        {
        }
    }

    private sealed record PendingReplyResponse(
        string RequestId,
        TaskCompletionSource<WindowsDouyinSidecarResponse?> Completion);

    private sealed record PendingCommandResponse(
        string RequestId,
        TaskCompletionSource<WindowsDouyinCommandResponse?> Completion);

    private sealed record PendingLiveOpenResponse(
        string RequestId,
        ulong Generation,
        TaskCompletionSource<WindowsDouyinLiveOpenResponse?> Completion);

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
