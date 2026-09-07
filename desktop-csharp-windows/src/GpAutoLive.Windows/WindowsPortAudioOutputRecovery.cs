using System.Runtime.InteropServices;

namespace GpAutoLive.Windows;

/// <summary>PortAudio 输出设备恢复的固定预算，避免设备丢失时无限重试。</summary>
public sealed class WindowsPortAudioRecoveryPolicy
{
    public WindowsPortAudioRecoveryPolicy(
        int maxAttempts = 3,
        TimeSpan? initialDelay = null,
        TimeSpan? maxDelay = null)
    {
        if (maxAttempts is < 1 or > 3)
        {
            throw new ArgumentOutOfRangeException(nameof(maxAttempts), "PortAudio 恢复尝试次数必须在 1 到 3 次之间。");
        }

        InitialDelay = initialDelay ?? TimeSpan.FromMilliseconds(250);
        MaxDelay = maxDelay ?? TimeSpan.FromSeconds(1);
        if (InitialDelay <= TimeSpan.Zero || MaxDelay < InitialDelay)
        {
            throw new ArgumentOutOfRangeException(nameof(initialDelay), "PortAudio 恢复延迟必须为正数且不超过最大延迟。");
        }

        MaxAttempts = maxAttempts;
    }

    public int MaxAttempts { get; }

    public TimeSpan InitialDelay { get; }

    public TimeSpan MaxDelay { get; }

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

/// <summary>
/// 重新打开同一设备配置的恢复器。恢复期间不清空 PCM 环缓，失败后由会话所有者终止播放。
/// </summary>
public sealed class WindowsPortAudioOutputRecovery
{
    private readonly WindowsPortAudioRecoveryPolicy _policy;

    public WindowsPortAudioOutputRecovery(WindowsPortAudioRecoveryPolicy? policy = null)
    {
        _policy = policy ?? new WindowsPortAudioRecoveryPolicy();
    }

    public async Task<WindowsPortAudioOutputResult> RecoverAsync(
        WindowsPortAudioOutputStream output,
        string? dllPath,
        WindowsPortAudioOutputConfig config,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(output);

        return await RecoverWithAsync(
                restart: token => output.RestartAsync(dllPath, config, token),
                snapshot: () => output.Snapshot,
                cancellationToken)
            .ConfigureAwait(false);
    }

    /// <summary>
    /// 使用单一恢复尝试入口执行有界重开；仅供 Windows 集成测试验证重试/取消边界。
    /// </summary>
    internal async Task<WindowsPortAudioOutputResult> RecoverWithAsync(
        Func<CancellationToken, Task<WindowsPortAudioOutputResult>> restart,
        Func<WindowsPortAudioOutputSnapshot> snapshot,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(restart);
        ArgumentNullException.ThrowIfNull(snapshot);

        WindowsPortAudioOutputResult? last = null;
        for (var attempt = 0; attempt < _policy.MaxAttempts; attempt++)
        {
            if (attempt > 0)
            {
                await Task.Delay(_policy.GetDelay(attempt - 1), cancellationToken).ConfigureAwait(false);
            }

            WindowsPortAudioOutputResult result;
            try
            {
                result = await restart(cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                return new(
                    false,
                    snapshot(),
                    new WindowsPortAudioStreamError(
                        WindowsPortAudioStreamFailureCode.Cancelled,
                        "PortAudio 输出流恢复已取消。",
                        Retryable: true));
            }
            catch (Exception exception) when (exception is AccessViolationException
                or InvalidOperationException
                or MarshalDirectiveException
                or ObjectDisposedException
                or SEHException)
            {
                result = new(
                    false,
                    snapshot(),
                    new WindowsPortAudioStreamError(
                        WindowsPortAudioStreamFailureCode.RestartFailed,
                        "PortAudio 输出流恢复发生原生错误。",
                        Retryable: true));
            }

            if (result.IsSuccess)
            {
                return result;
            }

            last = result;
            if (result.Error?.Retryable != true)
            {
                break;
            }
        }

        return last ?? new(
            false,
            snapshot(),
            new WindowsPortAudioStreamError(
                WindowsPortAudioStreamFailureCode.RestartFailed,
                "PortAudio 输出流恢复未执行。",
                Retryable: true));
    }
}
