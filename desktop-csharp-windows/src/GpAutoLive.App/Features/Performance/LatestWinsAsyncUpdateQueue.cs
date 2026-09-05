namespace GpAutoLive.App.Features.Performance;

/// <summary>
/// 将后台快照更新压缩为单个有界的 UI 更新：新更新到达时替换尚未执行的旧更新。
/// </summary>
public sealed class LatestWinsAsyncUpdateQueue : IDisposable
{
    private readonly object _gate = new();
    private readonly Action<Action> _schedule;
    private readonly Action<Exception>? _onError;
    private Func<Task>? _pending;
    private bool _scheduled;
    private bool _disposed;

    public LatestWinsAsyncUpdateQueue(
        Action<Action> schedule,
        Action<Exception>? onError = null)
    {
        _schedule = schedule ?? throw new ArgumentNullException(nameof(schedule));
        _onError = onError;
    }

    /// <summary>提交一次更新；队列最多保留一个尚未执行的更新。</summary>
    public void Post(Func<Task> update)
    {
        ArgumentNullException.ThrowIfNull(update);

        var shouldSchedule = false;
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            _pending = update;
            if (!_scheduled)
            {
                _scheduled = true;
                shouldSchedule = true;
            }
        }

        if (shouldSchedule)
        {
            ScheduleNext();
        }
    }

    public void Dispose()
    {
        lock (_gate)
        {
            _disposed = true;
            _pending = null;
        }
    }

    private void ScheduleNext()
    {
        try
        {
            _schedule(RunScheduledUpdate);
        }
        catch (Exception exception) when (exception is InvalidOperationException or ObjectDisposedException)
        {
            lock (_gate)
            {
                _pending = null;
                _scheduled = false;
            }
        }
    }

    private void RunScheduledUpdate()
    {
        Func<Task>? update;
        lock (_gate)
        {
            if (_disposed)
            {
                _pending = null;
                _scheduled = false;
                return;
            }

            update = _pending;
            _pending = null;
        }

        if (update is null)
        {
            lock (_gate)
            {
                _scheduled = false;
            }

            return;
        }

        _ = ExecuteUpdateAsync(update);
    }

    private async Task ExecuteUpdateAsync(Func<Task> update)
    {
        try
        {
            await update().ConfigureAwait(true);
        }
        catch (Exception exception) when (exception is OperationCanceledException or InvalidOperationException)
        {
            _onError?.Invoke(exception);
        }
        catch (Exception exception)
        {
            _onError?.Invoke(exception);
        }

        var shouldSchedule = false;
        lock (_gate)
        {
            if (_disposed || _pending is null)
            {
                _scheduled = false;
            }
            else
            {
                shouldSchedule = true;
            }
        }

        if (shouldSchedule)
        {
            ScheduleNext();
        }
    }
}
