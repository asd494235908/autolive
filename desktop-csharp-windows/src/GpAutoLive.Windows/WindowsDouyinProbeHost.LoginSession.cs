using System.Diagnostics;
using GpAutoLive.Contracts;

namespace GpAutoLive.Windows;

public sealed partial class WindowsDouyinProbeHost
{
    private bool _authenticated;
    private WindowsDouyinLoginClearReason _loginClearReason = WindowsDouyinLoginClearReason.NotAuthenticatedThisRun;
    private (string RoomId, string SessionId, ulong Generation)? _retiredSession;
    private bool _roomCloseObserved;

    private void ClearAuthentication(WindowsDouyinLoginClearReason reason)
    {
        lock (_gate)
        {
            _authenticated = false;
            _loginClearReason = reason;
        }
    }

    private bool HasConfirmedRoomConnection(Process process, ulong generation)
    {
        lock (_gate)
        {
            return ReferenceEquals(_process, process) && !HasExited(process) && _authenticated
                && _runCancellation is { IsCancellationRequested: false }
                && _state == WindowsDouyinProbeHostState.Running && _expectedGeneration == generation
                && _expectedSessionId is not null && _manager.Snapshot.State == DouyinLiveState.Listening;
        }
    }

    /// <summary>仅断开直播间；已确认的登录继续保存在本次运行的 sidecar 内存中。</summary>
    public async Task<WindowsDouyinProbeHostResult> DisconnectAsync(CancellationToken cancellationToken = default)
    {
        if (!Snapshot.Authenticated)
        {
            return await StopAsync(cancellationToken).ConfigureAwait(false);
        }
        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsDouyinProbeHostFailureCode.Cancelled, "直播间断开已取消。", true);
        }
        try
        {
            Process? process;
            string? sessionId;
            ulong? generation;
            lock (_gate)
            {
                process = _process;
                sessionId = _expectedSessionId;
                generation = _expectedGeneration;
                if (_disposed || process is null || HasExited(process) || !_authenticated)
                {
                    return Failure(WindowsDouyinProbeHostFailureCode.ProcessExited, "抖音登录进程已结束，请重新扫码。", true);
                }
                if (_state == WindowsDouyinProbeHostState.Ready)
                {
                    return Succeeded();
                }
                _state = WindowsDouyinProbeHostState.Stopping;
                _roomCloseObserved = false;
            }
            // 先切断旧任务的发送资格；在途写入由原发送锁等待结束，再发送 live.close。
            _manager.DisconnectRoom();
            PublishSnapshot();
            await _replySendGate.WaitAsync(cancellationToken).ConfigureAwait(false);
            try
            {
                if (sessionId is null || generation is null)
                {
                    FailCanonicalSession("直播间尚未建立完成，请重新连接");
                    return Failure(WindowsDouyinProbeHostFailureCode.StartFailed, "直播间尚未建立完成。", true);
                }
                var requestId = CreateRequestId("close");
                if (!WindowsDouyinSidecarProtocol.TrySerializeLiveClose(requestId, sessionId, generation.Value,
                        out var line, out _))
                {
                    FailCanonicalSession("直播间断开请求无效");
                    return Failure(WindowsDouyinProbeHostFailureCode.InvalidPlan, "直播间断开请求无效。", false);
                }
                var response = await SendCommandAsync(process, requestId, line, cancellationToken).ConfigureAwait(false);
                bool closeConfirmed;
                lock (_gate)
                {
                    if (!_authenticated)
                    {
                        return Failure(WindowsDouyinProbeHostFailureCode.ProcessExited, "登录会话已结束，请查看登录状态。", true);
                    }
                    closeConfirmed = (response is { IsSuccess: true } || (response is null && _roomCloseObserved))
                        && _runCancellation is { IsCancellationRequested: false }
                        && ReferenceEquals(_process, process) && !HasExited(process)
                        && _expectedSessionId == sessionId && _expectedGeneration == generation;
                }
                if (!closeConfirmed)
                {
                    FailCanonicalSession("直播间断开未得到确认，请重新连接");
                    return Failure(WindowsDouyinProbeHostFailureCode.StopTimedOut, "直播间断开未得到确认。", true);
                }
                CompleteRoomDisconnect();
                return Succeeded();
            }
            finally
            {
                _replySendGate.Release();
            }
        }
        catch (OperationCanceledException)
        {
            FailCanonicalSession("直播间断开已取消");
            return Failure(WindowsDouyinProbeHostFailureCode.Cancelled, "直播间断开已取消。", true);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    private async Task<WindowsDouyinProbeHostResult> ReopenAuthenticatedRoomAsync(
        WindowsDouyinProbeLaunchRequest request, Process process, CancellationToken cancellationToken)
    {
        var started = _manager.TryStart(request.Config, reuseAuthenticatedSession: true);
        if (!started.IsSuccess)
        {
            return Failure(WindowsDouyinProbeHostFailureCode.AlreadyRunning,
                started.Error?.Message ?? "直播间会话已在运行。", false);
        }
        CancellationToken runToken;
        lock (_gate)
        {
            if (_runCancellation is null || _runCancellation.IsCancellationRequested)
            {
                return Failure(WindowsDouyinProbeHostFailureCode.ProcessExited, "抖音登录进程已结束，请重新扫码。", true);
            }
            _state = WindowsDouyinProbeHostState.Running;
            _expectedRoomId = request.Config.RoomId;
            _expectedSessionId = null;
            _expectedGeneration = null;
            _chatMessages.Reset(started.Snapshot.Generation);
            _lastEvent = "live_opening";
            _invalidEventCount = 0;
            ArmStartupTimer(_runCancellation, request.Timeout);
            runToken = _runCancellation.Token;
        }
        PublishSnapshot();
        using var opening = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, runToken);
        await RunCanonicalSessionAsync(process, request.Config.RoomId, started.Snapshot.Generation,
            opening.Token, reuseAuthentication: true).ConfigureAwait(false);
        return Snapshot.Douyin.State is DouyinLiveState.RoomResolved or DouyinLiveState.Listening
            ? Succeeded()
            : Failure(WindowsDouyinProbeHostFailureCode.StartFailed, "直播间连接未成功，请查看当前状态。", true);
    }

    private void CompleteRoomDisconnect(string? reason = null)
    {
        lock (_gate)
        {
            if (!_authenticated || _stopRequested || _state == WindowsDouyinProbeHostState.Ready)
            {
                return;
            }
            if (_expectedRoomId is { } room && _expectedSessionId is { } session && _expectedGeneration is { } generation)
            {
                _retiredSession = (room, session, generation);
            }
            _expectedSessionId = null;
            _expectedGeneration = null;
            _startupTimer?.Dispose();
            _startupTimer = null;
            _pendingReplyResponse?.Completion.TrySetResult(null);
            _pendingReplyResponse = null;
            _state = WindowsDouyinProbeHostState.Ready;
            _lastEvent = "live_disconnected";
            _manager.DisconnectRoom(reason);
        }
        PublishSnapshot();
    }

    private void ArmStartupTimer(CancellationTokenSource runCancellation, TimeSpan timeout)
    {
        _startupTimer?.Dispose();
        var generation = _manager.Snapshot.Generation;
        _startupTimer = _timeProvider.CreateTimer(_ =>
        {
            lock (_gate)
            {
                if (_startupTimer is not null && ReferenceEquals(_runCancellation, runCancellation)
                    && _manager.Snapshot.Generation == generation)
                {
                    CancelRun();
                }
            }
        }, null, timeout, Timeout.InfiniteTimeSpan);
    }

    private bool IsRetiredLiveEvent(WindowsDouyinProbeEvent probeEvent)
    {
        lock (_gate)
        {
            return _retiredSession is { } retired && probeEvent.SessionId == retired.SessionId
                && probeEvent.Generation == retired.Generation;
        }
    }

    private bool IsRetiredLiveEvent(string line)
    {
        (string RoomId, string SessionId, ulong Generation)? retired;
        lock (_gate) { retired = _retiredSession; }
        return retired is { } old && WindowsDouyinProbeEventParser.TryParse(line, out var parsed, out _,
            old.RoomId, old.SessionId, old.Generation) && IsRetiredLiveEvent(parsed!);
    }
}
