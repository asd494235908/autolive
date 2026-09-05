using GpAutoLive.Core;

namespace GpAutoLive.App.Features.Effects;

/// <summary>
/// 按当前视频播放位置计划有限的视频效果周期。
/// 该类只负责触发时机，不启动线程、不持有播放器，也不缓存效果快照。
/// </summary>
public sealed class VideoEffectCyclePlanner
{
    private const ulong MinimumPeriodMs = 5_000;
    private const ulong MaximumPeriodMs = 8_000;
    private const ulong SeekBackToleranceMs = 500;

    private readonly Random _random;
    private MediaPlaybackIdentity? _identity;
    private ulong _lastPositionMs;
    private ulong? _nextTargetMs;

    public VideoEffectCyclePlanner(Random? random = null) => _random = random ?? Random.Shared;

    /// <summary>
    /// 仅在播放位置跨过当前目标时触发一次；切源、回绕、回退或关闭处理均会重新布置目标。
    /// </summary>
    public bool ShouldRegenerate(
        MediaPlaybackIdentity identity,
        ulong? positionMs,
        ulong? durationMs,
        bool enabled)
    {
        ArgumentNullException.ThrowIfNull(identity);
        if (!Equals(_identity, identity))
        {
            _identity = identity;
            _nextTargetMs = null;
        }

        if (positionMs is not ulong position
            || !enabled
            || durationMs is not ulong duration
            || duration == 0)
        {
            if (positionMs is ulong knownPosition)
            {
                _lastPositionMs = knownPosition;
            }

            _nextTargetMs = null;
            return false;
        }

        if (position < _lastPositionMs
            && _lastPositionMs - position > SeekBackToleranceMs)
        {
            _nextTargetMs = null;
        }

        _lastPositionMs = position;
        if (_nextTargetMs is not ulong target)
        {
            _nextTargetMs = NextTarget(position, duration);
            return false;
        }

        if (position < target || target >= duration)
        {
            return false;
        }

        _nextTargetMs = NextTarget(position, duration);
        return true;
    }

    private ulong NextTarget(ulong positionMs, ulong durationMs)
    {
        var period = (ulong)_random.NextInt64(
            checked((long)MinimumPeriodMs),
            checked((long)MaximumPeriodMs + 1));
        return positionMs > durationMs - Math.Min(period, durationMs)
            ? durationMs
            : positionMs + period;
    }
}
