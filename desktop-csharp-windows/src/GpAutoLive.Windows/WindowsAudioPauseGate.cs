namespace GpAutoLive.Windows;

/// <summary>
/// 解码线程的暂停门控。暂停只阻止继续读取/写入新的 PCM，不创建额外队列；
/// 关闭会释放等待者，确保取消和退出可以有界完成。
/// </summary>
public sealed class WindowsAudioPauseGate
{
    private readonly object _gate = new();
    private TaskCompletionSource<bool>? _resumeSignal;
    private bool _paused;
    private bool _closed;

    public bool IsPaused
    {
        get
        {
            lock (_gate)
            {
                return _paused && !_closed;
            }
        }
    }

    public void Pause()
    {
        lock (_gate)
        {
            if (_closed || _paused)
            {
                return;
            }

            _paused = true;
            _resumeSignal = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        }
    }

    public void Resume()
    {
        TaskCompletionSource<bool>? signal;
        lock (_gate)
        {
            if (_closed)
            {
                return;
            }

            _paused = false;
            signal = _resumeSignal;
            _resumeSignal = null;
        }

        signal?.TrySetResult(true);
    }

    public void Close()
    {
        TaskCompletionSource<bool>? signal;
        lock (_gate)
        {
            if (_closed)
            {
                return;
            }

            _closed = true;
            _paused = false;
            signal = _resumeSignal;
            _resumeSignal = null;
        }

        signal?.TrySetResult(true);
    }

    public async ValueTask WaitIfPausedAsync(CancellationToken cancellationToken = default)
    {
        while (true)
        {
            Task? waitTask;
            lock (_gate)
            {
                if (!_paused || _closed)
                {
                    return;
                }

                waitTask = _resumeSignal?.Task;
            }

            if (waitTask is null)
            {
                continue;
            }

            await waitTask.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
    }
}
