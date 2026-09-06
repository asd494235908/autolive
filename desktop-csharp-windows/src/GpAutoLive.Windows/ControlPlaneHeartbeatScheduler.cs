using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.Windows;

/// <summary>后台心跳调度配置；生产默认与 Rust/Tauri 参考端保持 30 秒周期。</summary>
public sealed record ControlPlaneHeartbeatSchedulerOptions
{
    public TimeSpan Interval { get; init; } = TimeSpan.FromSeconds(30);

    internal void Validate()
    {
        if (Interval <= TimeSpan.Zero || Interval > TimeSpan.FromHours(24))
        {
            throw new ArgumentOutOfRangeException(nameof(Interval), "心跳周期必须在 0 秒到 24 小时之间。");
        }
    }
}

/// <summary>心跳状态的唯一采集边界；采集器不得返回路径、凭据或媒体正文。</summary>
public delegate HeartbeatStatusDto HeartbeatStatusProvider();

/// <summary>
/// 单一、可取消、可 Join 的控制面心跳所有者。
/// 网络失败只持久化一条最新、无凭据的心跳 outbox；认证和重试状态仍由
/// <see cref="ControlPlaneAuthCoordinator"/> 与 Core 状态机拥有。
/// </summary>
public sealed class ControlPlaneHeartbeatScheduler : IAsyncDisposable, IDisposable
{
    private readonly ControlPlaneAuthCoordinator _auth;
    private readonly HeartbeatStatusProvider _statusProvider;
    private readonly HeartbeatOutboxStore _outbox;
    private readonly IControlPlaneClock _clock;
    private readonly ControlPlaneHeartbeatSchedulerOptions _options;
    private readonly Action<AuthTransition>? _transitionObserver;
    private readonly object _gate = new();
    private readonly SemaphoreSlim _sendSerial = new(1, 1);
    private Task? _runTask;
    private Task? _stopTask;
    private CancellationTokenSource? _runCancellation;
    private bool _stopping;
    private bool _disposed;

    public ControlPlaneHeartbeatScheduler(
        ControlPlaneAuthCoordinator auth,
        HeartbeatStatusProvider statusProvider,
        HeartbeatOutboxStore outbox,
        IControlPlaneClock? clock = null,
        ControlPlaneHeartbeatSchedulerOptions? options = null,
        Action<AuthTransition>? transitionObserver = null)
    {
        _auth = auth ?? throw new ArgumentNullException(nameof(auth));
        _statusProvider = statusProvider ?? throw new ArgumentNullException(nameof(statusProvider));
        _outbox = outbox ?? throw new ArgumentNullException(nameof(outbox));
        _clock = clock ?? SystemControlPlaneClock.Instance;
        _options = options ?? new ControlPlaneHeartbeatSchedulerOptions();
        _transitionObserver = transitionObserver;
        _options.Validate();
    }

    public bool IsRunning
    {
        get
        {
            lock (_gate)
            {
                return _runTask is not null && !_stopping;
            }
        }
    }

    /// <summary>启动唯一后台任务；重复启动不会创建第二个循环。</summary>
    public void Start()
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_disposed, this);
            if (_stopping)
            {
                throw new InvalidOperationException("心跳调度器已经停止。");
            }

            if (_runTask is not null)
            {
                return;
            }

            _runCancellation = new CancellationTokenSource();
            _runTask = RunAsync(_runCancellation.Token);
        }
    }

    /// <summary>请求当前已授权会话立即发送一轮，不会创建额外后台任务。</summary>
    public async Task SendNowAsync(CancellationToken cancellationToken = default)
    {
        CancellationTokenSource? linked = null;
        try
        {
            CancellationToken runToken;
            lock (_gate)
            {
                if (_disposed || _stopping || _runTask is null || _runCancellation is null)
                {
                    return;
                }

                runToken = _runCancellation.Token;
            }

            linked = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, runToken);
            await SendCycleAsync(linked.Token).ConfigureAwait(false);
        }
        finally
        {
            linked?.Dispose();
        }
    }

    /// <summary>取消循环并等待其唯一任务退出；真实传输的取消/超时保证 Join 有界。</summary>
    public Task StopAsync()
    {
        lock (_gate)
        {
            if (_stopTask is not null)
            {
                return _stopTask;
            }

            _stopping = true;
            var cancellation = _runCancellation;
            var runTask = _runTask;
            _stopTask = StopCoreAsync(cancellation, runTask);
            return _stopTask;
        }
    }

    public void Dispose()
    {
        DisposeAsync().AsTask().GetAwaiter().GetResult();
    }

    public async ValueTask DisposeAsync()
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
        }

        await StopAsync().ConfigureAwait(false);
        _sendSerial.Dispose();
    }

    private async Task RunAsync(CancellationToken cancellationToken)
    {
        try
        {
            while (!cancellationToken.IsCancellationRequested)
            {
                if (IsHeartbeatEligible(_auth.Snapshot))
                {
                    await SendCycleAsync(cancellationToken).ConfigureAwait(false);
                }

                var delay = NextDelay();
                await _clock.DelayAsync(delay, cancellationToken).ConfigureAwait(false);
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            // StopAsync 已发出取消；由调用方继续等待该任务完成。
        }
    }

    private async Task StopCoreAsync(
        CancellationTokenSource? cancellation,
        Task? runTask)
    {
        cancellation?.Cancel();
        if (runTask is not null)
        {
            await runTask.ConfigureAwait(false);
        }

        cancellation?.Dispose();
        lock (_gate)
        {
            _runTask = null;
            _runCancellation = null;
        }
    }

    private async Task SendCycleAsync(CancellationToken cancellationToken)
    {
        await _sendSerial.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var expiryTransition = await _auth
                .ExpireActivationIfNeededAsync(cancellationToken)
                .ConfigureAwait(false);
            if (expiryTransition is not null)
            {
                Observe(expiryTransition);
                return;
            }

            var snapshot = _auth.Snapshot;
            if (!IsHeartbeatEligible(snapshot)
                || string.IsNullOrWhiteSpace(snapshot.UserId)
                || string.IsNullOrWhiteSpace(snapshot.DeviceId))
            {
                return;
            }

            if (snapshot.AccessExpiresAt is { } accessExpiresAt
                && accessExpiresAt <= _clock.UtcNow + _options.Interval)
            {
                var refresh = await _auth.RefreshAsync(cancellationToken).ConfigureAwait(false);
                Observe(refresh);
                if (!refresh.IsSuccess || !refresh.Snapshot.CanEnterWorkbench)
                {
                    return;
                }

                snapshot = _auth.Snapshot;
            }

            if (!IsHeartbeatEligible(snapshot)
                || string.IsNullOrWhiteSpace(snapshot.UserId)
                || string.IsNullOrWhiteSpace(snapshot.DeviceId))
            {
                return;
            }

            var userId = snapshot.UserId;
            var deviceId = snapshot.DeviceId;
            var now = _clock.UtcNow;
            var queued = await _outbox.ReadAsync(userId, deviceId, now, cancellationToken).ConfigureAwait(false);
            if (queued is not null)
            {
                var queuedTransition = await _auth.HeartbeatAsync(
                    queued.Request,
                    cancellationToken,
                    queued.IdempotencyKey).ConfigureAwait(false);
                Observe(queuedTransition);
                if (queuedTransition.IsSuccess)
                {
                    await _outbox.ClearAsync(
                        userId,
                        deviceId,
                        queued.IdempotencyKey,
                        cancellationToken,
                        now).ConfigureAwait(false);
                }
                else if (ShouldKeepForRetry(queuedTransition))
                {
                    return;
                }
                else
                {
                    await _outbox.ClearAsync(
                        userId,
                        deviceId,
                        queued.IdempotencyKey,
                        cancellationToken,
                        now).ConfigureAwait(false);
                    return;
                }
            }

            HeartbeatStatusDto status;
            try
            {
                status = _statusProvider();
            }
            catch (Exception) when (!cancellationToken.IsCancellationRequested)
            {
                // 采集失败不伪造成功，也不把旧运行摘要重复写入 outbox。
                return;
            }

            var request = new HeartbeatRequestDto(
                ControlPlaneContractValues.Product,
                deviceId,
                _clock.UtcNow,
                status);
            var transition = await _auth.HeartbeatAsync(request, cancellationToken).ConfigureAwait(false);
            Observe(transition);
            if (transition.IsSuccess)
            {
                return;
            }

            if (!ShouldKeepForRetry(transition))
            {
                await _outbox.ClearAsync(
                    userId,
                    deviceId,
                    cancellationToken: cancellationToken,
                    now: _clock.UtcNow).ConfigureAwait(false);
                return;
            }

            try
            {
                await _outbox.SaveLatestAsync(
                    new HeartbeatOutboxEntry(
                        userId,
                        deviceId,
                        transition.OperationKey,
                        request,
                        _clock.UtcNow),
                    cancellationToken).ConfigureAwait(false);
            }
            catch (ConfigurationValidationException)
            {
                // outbox 不可写时保留内存状态；本轮失败不会伪装为在线。
            }
            catch (IOException)
            {
                // 同上，下一轮重新采集并尝试发送。
            }
            catch (UnauthorizedAccessException)
            {
                // 同上，不向日志暴露本地路径。
            }
        }
        finally
        {
            _sendSerial.Release();
        }
    }

    private TimeSpan NextDelay()
    {
        var delay = _options.Interval;
        var nextRetryAt = _auth.Snapshot.NextRetryAt;
        if (nextRetryAt is { } retryAt)
        {
            var retryDelay = retryAt - _clock.UtcNow;
            if (retryDelay > TimeSpan.Zero && retryDelay < delay)
            {
                delay = retryDelay;
            }
            else if (retryDelay <= TimeSpan.Zero)
            {
                delay = TimeSpan.FromMilliseconds(1);
            }
        }

        return delay;
    }

    private static bool IsHeartbeatEligible(AuthSessionSnapshot snapshot) =>
        snapshot.State is AuthSessionState.Activated or AuthSessionState.Offline;

    private static bool ShouldKeepForRetry(AuthTransition transition) =>
        transition.ShouldRetry || transition.Error?.IsTransient == true;

    private void Observe(AuthTransition transition)
    {
        try
        {
            _transitionObserver?.Invoke(transition);
        }
        catch (Exception)
        {
            // UI 投影失败不能终止唯一心跳所有者；核心状态仍保持 fail-closed。
        }
    }
}
