namespace GpAutoLive.Windows;

/// <summary>
/// 一个 PortAudio 流只有一条在途原生调用链。超时只结束调用方等待，
/// 收尾任务持有并等待旧任务后再关闭资源，绝不与旧原生调用并行。
/// </summary>
internal sealed class WindowsPortAudioOperationOwner<TResult>(Func<TResult> cleanup, TimeSpan budget)
{
    private readonly object _gate = new();
    private Task<TResult>? _pending;
    private Task<TResult>? _cleanupTask;
    private long _startedAt;
    private bool _healthQuery;
    private int _stopRequested;
    private int _timedOut;

    internal bool StopRequested => Volatile.Read(ref _stopRequested) != 0;
    internal bool TimedOut => Volatile.Read(ref _timedOut) != 0;

    internal bool IsBusy
    {
        get
        {
            lock (_gate)
            {
                return _pending is { IsCompleted: false };
            }
        }
    }

    internal async Task<TResult> RunAsync(Func<TResult> operation, CancellationToken cancellationToken = default)
    {
        cancellationToken.ThrowIfCancellationRequested();
        var deadline = Environment.TickCount64 + (long)budget.TotalMilliseconds;
        Task<TResult>? task = null;
        lock (_gate)
        {
            if (_pending is { IsCompleted: false })
            {
                if (!_healthQuery)
                {
                    throw new InvalidOperationException("PortAudio 上一次原生操作尚未回收。");
                }

                task = _pending;
            }
        }

        try
        {
            // 正常的短健康查询不应让用户的暂停/恢复失败，但等待也计入同一预算。
            if (task is not null)
            {
                await task.WaitAsync(Remaining(deadline), cancellationToken).ConfigureAwait(false);
            }

            lock (_gate)
            {
                if (_pending is { IsCompleted: false })
                {
                    throw new InvalidOperationException("PortAudio 上一次原生操作尚未回收。");
                }

                Volatile.Write(ref _stopRequested, 0);
                Volatile.Write(ref _timedOut, 0);
                _cleanupTask = null;
                _healthQuery = false;
                _startedAt = Environment.TickCount64;
                task = Task.Run(operation);
                _pending = task;
            }

            return await task.WaitAsync(Remaining(deadline), cancellationToken).ConfigureAwait(false);
        }
        catch (TimeoutException)
        {
            CancelOperation(task, timedOut: true);
            throw;
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            CancelOperation(task, timedOut: false);
            throw;
        }
    }

    internal async Task<TResult> StopAsync()
    {
        var task = RequestCleanup();
        try
        {
            return await task.WaitAsync(budget).ConfigureAwait(false);
        }
        catch (TimeoutException)
        {
            Volatile.Write(ref _timedOut, 1);
            throw;
        }
    }

    /// <summary>快照只调度刷新，不在读快照的线程执行或等待 P/Invoke。</summary>
    internal bool TryRefresh(Func<TResult> refresh)
    {
        lock (_gate)
        {
            if (StopRequested || _pending is { IsCompleted: false })
            {
                return false;
            }

            _startedAt = Environment.TickCount64;
            _healthQuery = true;
            _pending = Task.Run(refresh);
            return true;
        }
    }

    /// <summary>健康查询没有等待者；读快照时观察预算并交给同一 owner 收尾。</summary>
    internal bool CheckHealthTimeout()
    {
        lock (_gate)
        {
            if (!StopRequested && _pending is { IsFaulted: true })
            {
                _ = _pending.Exception;
                Volatile.Write(ref _timedOut, 1);
                _ = RequestCleanup();
            }
            else if (_healthQuery && !StopRequested
                && _pending is { IsCompleted: false }
                && Environment.TickCount64 - _startedAt >= budget.TotalMilliseconds)
            {
                Volatile.Write(ref _timedOut, 1);
                _ = RequestCleanup();
            }

            return TimedOut;
        }
    }

    private Task<TResult> RequestCleanup()
    {
        lock (_gate)
        {
            Volatile.Write(ref _stopRequested, 1);
            if (_cleanupTask is { IsCompleted: false })
            {
                return _cleanupTask;
            }

            var previous = _pending;
            _healthQuery = false;
            // 这条收尾链继续拥有 previous；超时返回不丢弃旧任务或其原生资源。
            _cleanupTask = Task.Run(async () =>
            {
                if (previous is not null)
                {
                    try
                    {
                        await previous.ConfigureAwait(false);
                    }
                    catch (Exception)
                    {
                        // 原调用的等待者会收到失败；即使原调用异常，资源仍必须尝试回收。
                        _ = previous.Exception;
                    }
                }

                return cleanup();
            });
            _pending = _cleanupTask;
            return _cleanupTask;
        }
    }

    private void CancelOperation(Task<TResult>? task, bool timedOut)
    {
        lock (_gate)
        {
            // 一个已结束调用的迟到取消，不能取消后面已准入的新操作。
            if (task is null || !ReferenceEquals(task, _pending))
            {
                return;
            }

            if (timedOut)
            {
                Volatile.Write(ref _timedOut, 1);
            }

            _ = RequestCleanup();
        }
    }

    private static TimeSpan Remaining(long deadline) =>
        TimeSpan.FromMilliseconds(Math.Max(0, deadline - Environment.TickCount64));
}
