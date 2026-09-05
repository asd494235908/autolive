using System.Runtime.CompilerServices;
using GpAutoLive.Core;

namespace GpAutoLive.Media;

/// <summary>mpv 播放状态监视的稳定错误分类。</summary>
public enum MpvPlaybackStateMonitorFailureCode
{
    InvalidInput,
    InvalidConfiguration,
    RuntimeUnavailable,
    StalePlaybackIdentity,
    IpcFailure,
    InvalidPlaybackTime,
    InvalidEofValue,
    InvalidPausedValue,
    Cancelled,
}

/// <summary>不包含路径或 IPC 原始文本的播放状态监视错误。</summary>
public sealed record MpvPlaybackStateMonitorError(
    MpvPlaybackStateMonitorFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>一轮状态读取得到的最小、不可变播放快照。</summary>
public sealed record MpvPlaybackStateSnapshot(
    MediaPlaybackIdentity Identity,
    ulong? PlaybackTimeMs,
    bool EofReached,
    bool Paused);

/// <summary>状态读取或身份门禁的结果；Changed=false 表示重复快照的幂等 no-op。</summary>
public sealed record MpvPlaybackStateMonitorResult(
    bool IsSuccess,
    MpvPlaybackStateSnapshot? Snapshot,
    bool Changed,
    MpvPlaybackStateMonitorError? Error = null)
{
    public static MpvPlaybackStateMonitorResult Succeeded(
        MpvPlaybackStateSnapshot snapshot,
        bool changed = true) =>
        new(true, snapshot, changed);

    public static MpvPlaybackStateMonitorResult Failed(
        MpvPlaybackStateMonitorError error) =>
        new(false, null, false, error);
}

/// <summary>
/// 播放状态轮询的固定配置。三项属性按顺序读取，迭代器不会建立无界缓冲或无人管理任务。
/// </summary>
public sealed record MpvPlaybackStateMonitorOptions
{
    public static readonly TimeSpan MinPollInterval = TimeSpan.FromMilliseconds(50);
    public static readonly TimeSpan MaxPollInterval = TimeSpan.FromSeconds(5);

    public TimeSpan PollInterval { get; init; } = TimeSpan.FromMilliseconds(250);

    internal bool TryValidate(out MpvPlaybackStateMonitorError? error)
    {
        error = null;
        if (PollInterval < MinPollInterval || PollInterval > MaxPollInterval)
        {
            error = new(
                MpvPlaybackStateMonitorFailureCode.InvalidConfiguration,
                "mpv 播放状态轮询间隔超出 50ms..5s 范围。");
            return false;
        }

        return true;
    }
}

/// <summary>
/// Media 层的 mpv 播放状态监视边界。
/// 只依赖固定 IPC 网关和 Core 播放身份，不启动 mpv，也不引用 Windows 实现。
/// </summary>
public sealed class MpvPlaybackStateMonitor
{
    public const int PropertiesPerPoll = 3;

    private readonly MpvPlaybackIpcGateway _gateway;
    private readonly MpvPlaybackStateMonitorOptions _options;

    private MpvPlaybackStateMonitor(
        MpvPlaybackIpcGateway gateway,
        MpvPlaybackStateMonitorOptions options)
    {
        _gateway = gateway;
        _options = options;
    }

    /// <summary>供上层受管运行时复用的固定轮询间隔。</summary>
    public TimeSpan PollInterval => _options.PollInterval;

    public static bool TryCreate(
        MpvPlaybackIpcGateway? gateway,
        MpvPlaybackStateMonitorOptions? options,
        out MpvPlaybackStateMonitor? monitor,
        out MpvPlaybackStateMonitorError? error)
    {
        monitor = null;
        error = null;
        if (gateway is null)
        {
            error = new(
                MpvPlaybackStateMonitorFailureCode.InvalidInput,
                "mpv 播放状态监视网关不能为空。");
            return false;
        }

        var validatedOptions = options ?? new MpvPlaybackStateMonitorOptions();
        if (!validatedOptions.TryValidate(out error))
        {
            return false;
        }

        monitor = new MpvPlaybackStateMonitor(gateway, validatedOptions);
        return true;
    }

    /// <summary>
    /// 读取一轮 time-pos、eof-reached、pause。所有请求串行且使用现有 request_id/身份门禁。
    /// </summary>
    public async Task<MpvPlaybackStateMonitorResult> PollOnceAsync(
        MediaPlaybackIdentity? expectedIdentity,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Cancelled();
        }

        if (expectedIdentity is null)
        {
            return Failed(
                MpvPlaybackStateMonitorFailureCode.InvalidInput,
                "mpv 播放状态监视身份不能为空。");
        }

        if (!HasCurrentIdentity(expectedIdentity))
        {
            return StaleIdentity();
        }

        try
        {
            var timeResult = await ReadPropertyAsync(
                MpvIpcProperty.PlaybackTime,
                expectedIdentity,
                cancellationToken).ConfigureAwait(false);
            if (!TryGetFrame(timeResult, out var timeFrame, out var timeFailure))
            {
                return timeFailure!;
            }

            var eofResult = await ReadPropertyAsync(
                MpvIpcProperty.EofReached,
                expectedIdentity,
                cancellationToken).ConfigureAwait(false);
            if (!TryGetFrame(eofResult, out var eofFrame, out var eofFailure))
            {
                return eofFailure!;
            }

            var pausedResult = await ReadPropertyAsync(
                MpvIpcProperty.Paused,
                expectedIdentity,
                cancellationToken).ConfigureAwait(false);
            if (!TryGetFrame(pausedResult, out var pausedFrame, out var pausedFailure))
            {
                return pausedFailure!;
            }

            if (!HasCurrentIdentity(expectedIdentity))
            {
                return StaleIdentity();
            }

            if (!TryCreateSnapshot(
                    expectedIdentity,
                    _gateway.Session.Snapshot.ActiveSource?.Identity,
                    timeFrame!,
                    eofFrame!,
                    pausedFrame!,
                    out var snapshot,
                    out var snapshotError)
                || snapshot is null)
            {
                return MpvPlaybackStateMonitorResult.Failed(snapshotError ?? new(
                    MpvPlaybackStateMonitorFailureCode.InvalidInput,
                    "mpv 播放状态快照无效。"));
            }

            return MpvPlaybackStateMonitorResult.Succeeded(snapshot);
        }
        catch (OperationCanceledException)
        {
            return Cancelled();
        }
        catch (Exception exception) when (exception is IOException or InvalidOperationException or ObjectDisposedException)
        {
            return Failed(
                MpvPlaybackStateMonitorFailureCode.IpcFailure,
                "mpv 播放状态读取失败。",
                retryable: true);
        }
    }

    /// <summary>
    /// 有界的 latest-wins 监视迭代器。每次只保留当前快照，取消或首个错误后结束。
    /// </summary>
    public async IAsyncEnumerable<MpvPlaybackStateMonitorResult> WatchAsync(
        MediaPlaybackIdentity? expectedIdentity,
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        MpvPlaybackStateSnapshot? previous = null;
        while (true)
        {
            var polled = await PollOnceAsync(expectedIdentity, cancellationToken).ConfigureAwait(false);
            if (!polled.IsSuccess || polled.Snapshot is null)
            {
                yield return polled;
                yield break;
            }

            var applied = ApplySnapshot(
                previous,
                expectedIdentity,
                _gateway.Session.Snapshot.ActiveSource?.Identity,
                polled.Snapshot);
            yield return applied;
            if (!applied.IsSuccess)
            {
                yield break;
            }

            previous = applied.Snapshot;
            var cancelled = false;
            try
            {
                await Task.Delay(_options.PollInterval, cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                cancelled = true;
            }

            if (cancelled)
            {
                yield return Cancelled();
                yield break;
            }
        }
    }

    /// <summary>
    /// 将三项成功响应投影为状态快照。activeIdentity 不匹配时拒绝应用，避免旧源状态污染新源。
    /// </summary>
    public static bool TryCreateSnapshot(
        MediaPlaybackIdentity? expectedIdentity,
        MediaPlaybackIdentity? activeIdentity,
        MpvIpcFrame? playbackTimeFrame,
        MpvIpcFrame? eofFrame,
        MpvIpcFrame? pausedFrame,
        out MpvPlaybackStateSnapshot? snapshot,
        out MpvPlaybackStateMonitorError? error)
    {
        snapshot = null;
        error = null;
        if (expectedIdentity is null || activeIdentity is null)
        {
            error = new(
                MpvPlaybackStateMonitorFailureCode.StalePlaybackIdentity,
                "mpv 播放状态身份不可用，已拒绝应用。");
            return false;
        }

        if (expectedIdentity != activeIdentity)
        {
            error = new(
                MpvPlaybackStateMonitorFailureCode.StalePlaybackIdentity,
                "mpv 播放状态身份已过期，已拒绝应用。");
            return false;
        }

        if (playbackTimeFrame is null || eofFrame is null || pausedFrame is null)
        {
            error = new(
                MpvPlaybackStateMonitorFailureCode.InvalidInput,
                "mpv 播放状态响应帧不完整。");
            return false;
        }

        if (!MpvIpcValueReader.TryReadFiniteDouble(
                playbackTimeFrame,
                out var timeSeconds,
                out _)
            || !TryConvertPlaybackTime(timeSeconds, out var playbackTimeMs))
        {
            error = new(
                MpvPlaybackStateMonitorFailureCode.InvalidPlaybackTime,
                "mpv playback time 响应无效。");
            return false;
        }

        if (!MpvIpcValueReader.TryReadBoolean(eofFrame, out var eofReached, out _))
        {
            error = new(
                MpvPlaybackStateMonitorFailureCode.InvalidEofValue,
                "mpv EOF 响应无效。");
            return false;
        }

        if (!MpvIpcValueReader.TryReadBoolean(pausedFrame, out var paused, out _))
        {
            error = new(
                MpvPlaybackStateMonitorFailureCode.InvalidPausedValue,
                "mpv paused 响应无效。");
            return false;
        }

        snapshot = new MpvPlaybackStateSnapshot(
            expectedIdentity,
            playbackTimeMs,
            eofReached,
            paused);
        return true;
    }

    /// <summary>
    /// 应用快照时再次校验当前身份。相同快照是幂等 no-op；不同身份永远 fail-closed。
    /// </summary>
    public static MpvPlaybackStateMonitorResult ApplySnapshot(
        MpvPlaybackStateSnapshot? previous,
        MediaPlaybackIdentity? expectedIdentity,
        MediaPlaybackIdentity? activeIdentity,
        MpvPlaybackStateSnapshot? candidate)
    {
        if (expectedIdentity is null || activeIdentity is null || candidate is null)
        {
            return MpvPlaybackStateMonitorResult.Failed(new(
                MpvPlaybackStateMonitorFailureCode.StalePlaybackIdentity,
                "mpv 播放状态身份不可用，已拒绝应用。"));
        }

        if (expectedIdentity != activeIdentity || candidate.Identity != expectedIdentity)
        {
            return MpvPlaybackStateMonitorResult.Failed(new(
                MpvPlaybackStateMonitorFailureCode.StalePlaybackIdentity,
                "mpv 播放状态身份已过期，已拒绝应用。"));
        }

        if (previous is not null && previous.Identity != expectedIdentity)
        {
            return MpvPlaybackStateMonitorResult.Failed(new(
                MpvPlaybackStateMonitorFailureCode.StalePlaybackIdentity,
                "mpv 上一份播放状态身份已过期，已拒绝应用。"));
        }

        return MpvPlaybackStateMonitorResult.Succeeded(
            candidate,
            changed: previous is null || previous != candidate);
    }

    private async Task<MpvIpcDispatchResult> ReadPropertyAsync(
        MpvIpcProperty property,
        MediaPlaybackIdentity expectedIdentity,
        CancellationToken cancellationToken)
    {
        return await _gateway.DispatchAsync(
            MpvIpcCommand.GetProperty(property),
            expectedIdentity,
            cancellationToken).ConfigureAwait(false);
    }

    private static bool TryGetFrame(
        MpvIpcDispatchResult result,
        out MpvIpcFrame? frame,
        out MpvPlaybackStateMonitorResult? failure)
    {
        frame = null;
        failure = null;
        if (result.IsSuccess && result.Frame is not null)
        {
            frame = result.Frame;
            return true;
        }

        if (result.SessionError?.Code is MpvSessionFailureCode.StalePlaybackIdentity
            or MpvSessionFailureCode.ResponseDoesNotBelongToSession
            || result.IpcError?.Code is MpvIpcFailureCode.StalePlaybackIdentity)
        {
            failure = StaleIdentity();
            return false;
        }

        failure = MpvPlaybackStateMonitorResult.Failed(new(
            MpvPlaybackStateMonitorFailureCode.IpcFailure,
            "mpv 播放状态读取失败。",
            result.IpcError?.Retryable ?? result.SessionError?.Retryable ?? true));
        return false;
    }

    private bool HasCurrentIdentity(MediaPlaybackIdentity expectedIdentity) =>
        _gateway.Session.Snapshot.ActiveSource?.Identity == expectedIdentity;

    private static bool TryConvertPlaybackTime(
        double? seconds,
        out ulong? milliseconds)
    {
        milliseconds = null;
        if (seconds is null)
        {
            return true;
        }

        const ulong maxExactMilliseconds = 9_007_199_254_740_991UL;
        var value = seconds.Value;
        if (!double.IsFinite(value)
            || value < 0
            || value > maxExactMilliseconds / 1_000d)
        {
            return false;
        }

        var rounded = Math.Round(value * 1_000d, MidpointRounding.AwayFromZero);
        if (!double.IsFinite(rounded) || rounded < 0 || rounded > maxExactMilliseconds)
        {
            return false;
        }

        milliseconds = (ulong)rounded;
        return true;
    }

    private static MpvPlaybackStateMonitorResult StaleIdentity() =>
        Failed(
            MpvPlaybackStateMonitorFailureCode.StalePlaybackIdentity,
            "mpv 播放状态身份已过期，已拒绝应用。");

    private static MpvPlaybackStateMonitorResult Cancelled() =>
        Failed(
            MpvPlaybackStateMonitorFailureCode.Cancelled,
            "mpv 播放状态监视已取消。",
            retryable: true);

    private static MpvPlaybackStateMonitorResult Failed(
        MpvPlaybackStateMonitorFailureCode code,
        string message,
        bool retryable = false) =>
        MpvPlaybackStateMonitorResult.Failed(new(code, message, retryable));
}
