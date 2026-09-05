using GpAutoLive.Contracts;

namespace GpAutoLive.Windows;

/// <summary>RTMP 重连策略的固定预算；不访问网络，也不创建后台任务。</summary>
public sealed class WindowsRtmpReconnectPolicy
{
    public WindowsRtmpReconnectPolicy(
        int maxAttempts = 3,
        TimeSpan? initialDelay = null,
        TimeSpan? maxDelay = null)
    {
        if (maxAttempts is < 1 or > 3)
        {
            throw new ArgumentOutOfRangeException(nameof(maxAttempts), "RTMP 重连尝试次数必须在 1 到 3 次之间。");
        }

        InitialDelay = initialDelay ?? TimeSpan.FromMilliseconds(250);
        MaxDelay = maxDelay ?? TimeSpan.FromSeconds(1);
        if (InitialDelay <= TimeSpan.Zero || MaxDelay < InitialDelay)
        {
            throw new ArgumentOutOfRangeException(nameof(initialDelay), "RTMP 重连延迟必须为正数且不超过最大延迟。");
        }

        MaxAttempts = maxAttempts;
    }

    /// <summary>一次断开事件最多允许执行的重连调用次数。</summary>
    public int MaxAttempts { get; }

    /// <summary>第一次重试前的退避时长。</summary>
    public TimeSpan InitialDelay { get; }

    /// <summary>指数退避的上限。</summary>
    public TimeSpan MaxDelay { get; }

    /// <summary>按 0 基重试序号计算有界指数退避，不加入随机抖动。</summary>
    public TimeSpan GetDelay(int retryOrdinal)
    {
        if (retryOrdinal < 0)
        {
            throw new ArgumentOutOfRangeException(nameof(retryOrdinal));
        }

        var multiplier = Math.Pow(2, Math.Min(retryOrdinal, 10));
        var milliseconds = Math.Min(MaxDelay.TotalMilliseconds, InitialDelay.TotalMilliseconds * multiplier);
        return TimeSpan.FromMilliseconds(milliseconds);
    }
}

/// <summary>一次底层重连尝试的脱敏结果；不携带地址、路径、命令行或外部错误正文。</summary>
public sealed record WindowsRtmpReconnectAttempt(
    bool IsSuccess,
    WindowsRtmpFailureCode? FailureCode = null,
    bool Retryable = false)
{
    public static WindowsRtmpReconnectAttempt Succeeded() => new(true);

    public static WindowsRtmpReconnectAttempt Failed(
        WindowsRtmpFailureCode failureCode,
        bool retryable) =>
        new(false, failureCode, retryable);
}

/// <summary>RTMP 有限重连协调器的脱敏状态。</summary>
public sealed record WindowsRtmpReconnectSnapshot(
    RtmpOutputState State,
    int RetryCount,
    string? ErrorCode,
    string? Error);

/// <summary>RTMP 有限重连协调器结果。</summary>
public sealed record WindowsRtmpReconnectResult(
    bool IsSuccess,
    WindowsRtmpReconnectSnapshot Snapshot,
    WindowsRtmpError? Error = null);

/// <summary>
/// 在底层宿主已确认连接断开后，执行一次有界、可取消的 RTMP 重连序列。
/// 每次尝试由调用方提供，协调器只拥有顺序、退避和脱敏终态，不自行访问网络。
/// </summary>
public sealed class WindowsRtmpReconnectCoordinator
{
    private readonly object _gate = new();
    private readonly WindowsRtmpReconnectPolicy _policy;
    private readonly Func<TimeSpan, CancellationToken, Task> _delayAsync;
    private WindowsRtmpReconnectSnapshot _snapshot = new(
        RtmpOutputState.Idle,
        0,
        null,
        null);
    private bool _running;

    public WindowsRtmpReconnectCoordinator(WindowsRtmpReconnectPolicy? policy = null)
        : this(policy ?? new WindowsRtmpReconnectPolicy(), Task.Delay)
    {
    }

    /// <summary>仅测试替身使用的延迟注入入口；生产路径固定使用 Task.Delay。</summary>
    internal WindowsRtmpReconnectCoordinator(
        WindowsRtmpReconnectPolicy policy,
        Func<TimeSpan, CancellationToken, Task> delayAsync)
    {
        _policy = policy ?? throw new ArgumentNullException(nameof(policy));
        _delayAsync = delayAsync ?? throw new ArgumentNullException(nameof(delayAsync));
    }

    /// <summary>读取当前有限重连状态。</summary>
    public WindowsRtmpReconnectSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return _snapshot;
            }
        }
    }

    /// <summary>
    /// 执行一次重连序列。attempt 为从 1 开始的尝试序号；成功后进入 Publishing，
    /// 取消后回到 Idle，重试耗尽或不可重试失败进入 Failed。
    /// </summary>
    public async Task<WindowsRtmpReconnectResult> ReconnectAsync(
        Func<int, CancellationToken, Task<WindowsRtmpReconnectAttempt>> attemptAsync,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(attemptAsync);

        lock (_gate)
        {
            if (_running)
            {
                var current = _snapshot;
                return new(
                    false,
                    current,
                    new(
                        WindowsRtmpFailureCode.AlreadyRunning,
                        "RTMP 重连已经在运行。",
                        Retryable: false));
            }

            _running = true;
            _snapshot = new(RtmpOutputState.Reconnecting, 0, null, null);
        }

        try
        {
            for (var attempt = 1; attempt <= _policy.MaxAttempts; attempt++)
            {
                SetReconnecting(attempt);

                if (attempt > 1)
                {
                    try
                    {
                        await _delayAsync(_policy.GetDelay(attempt - 2), cancellationToken)
                            .ConfigureAwait(false);
                    }
                    catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
                    {
                        return Cancelled(attempt - 1);
                    }
                }

                if (cancellationToken.IsCancellationRequested)
                {
                    return Cancelled(attempt - 1);
                }

                WindowsRtmpReconnectAttempt result;
                try
                {
                    var attemptResult = await attemptAsync(attempt, cancellationToken).ConfigureAwait(false);
                    result = attemptResult ?? WindowsRtmpReconnectAttempt.Failed(
                        WindowsRtmpFailureCode.StartFailed,
                        retryable: true);
                }
                catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
                {
                    return Cancelled(attempt - 1);
                }
                catch (Exception)
                {
                    // 对外只投影固定错误分类；不把异常正文（可能含路径/地址）传播到 UI。
                    result = WindowsRtmpReconnectAttempt.Failed(
                        WindowsRtmpFailureCode.StartFailed,
                        retryable: true);
                }

                if (cancellationToken.IsCancellationRequested)
                {
                    return Cancelled(attempt);
                }

                if (result.IsSuccess)
                {
                    lock (_gate)
                    {
                        _snapshot = new(
                            RtmpOutputState.Publishing,
                            attempt,
                            null,
                            null);
                    }

                    return Success();
                }

                var failureCode = result.FailureCode ?? WindowsRtmpFailureCode.StartFailed;
                if (failureCode == WindowsRtmpFailureCode.Cancelled)
                {
                    return Cancelled(attempt);
                }

                if (!result.Retryable)
                {
                    return Failure(
                        failureCode,
                        "RTMP 重连被拒绝。",
                        retryable: false,
                        RtmpOutputState.Failed,
                        attempt);
                }

                if (attempt == _policy.MaxAttempts)
                {
                    return Failure(
                        WindowsRtmpFailureCode.ReconnectExhausted,
                        "RTMP 重连次数已用尽。",
                        retryable: false,
                        RtmpOutputState.Failed,
                        attempt);
                }

                SetRetryableFailure(failureCode, attempt);
            }

            return Failure(
                WindowsRtmpFailureCode.ReconnectExhausted,
                "RTMP 重连次数已用尽。",
                retryable: false,
                RtmpOutputState.Failed,
                _policy.MaxAttempts);
        }
        finally
        {
            lock (_gate)
            {
                _running = false;
            }
        }
    }

    private void SetReconnecting(int retryCount)
    {
        lock (_gate)
        {
            _snapshot = _snapshot with
            {
                State = RtmpOutputState.Reconnecting,
                RetryCount = retryCount,
            };
        }
    }

    private void SetRetryableFailure(WindowsRtmpFailureCode failureCode, int retryCount)
    {
        lock (_gate)
        {
            _snapshot = new(
                RtmpOutputState.Reconnecting,
                retryCount,
                failureCode.ToString(),
                "RTMP 连接已断开，正在准备有限重连。");
        }
    }

    private WindowsRtmpReconnectResult Cancelled(int retryCount) =>
        Failure(
            WindowsRtmpFailureCode.Cancelled,
            "RTMP 重连已取消。",
            retryable: true,
            RtmpOutputState.Idle,
            retryCount);

    private WindowsRtmpReconnectResult Success() => new(true, Snapshot);

    private WindowsRtmpReconnectResult Failure(
        WindowsRtmpFailureCode code,
        string message,
        bool retryable,
        RtmpOutputState state,
        int retryCount)
    {
        var snapshot = new WindowsRtmpReconnectSnapshot(
            state,
            retryCount,
            code.ToString(),
            message);
        lock (_gate)
        {
            _snapshot = snapshot;
        }

        return new(false, snapshot, new(code, message, retryable));
    }
}
