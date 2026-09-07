using GpAutoLive.Core;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.App.Features.Effects;

/// <summary>
/// 按当前视频播放位置计划有限的视频效果周期。
/// 该类只负责触发时机，不启动线程、不持有播放器，也不缓存效果快照。
/// </summary>
public sealed class VideoEffectCyclePlanner
{
    private const ulong SeekBackToleranceMs = 500;

    private readonly Random _random;
    private ulong _periodMinMs = EffectCycleSettings.Default.VideoPeriodMinMs;
    private ulong _periodMaxMs = EffectCycleSettings.Default.VideoPeriodMaxMs;
    private MediaPlaybackIdentity? _identity;
    private ulong _lastPositionMs;
    private ulong? _cycleStartMs;
    private ulong? _nextTargetMs;

    public VideoEffectCyclePlanner(Random? random = null) => _random = random ?? Random.Shared;

    public ulong? CurrentCycleStartMs => _cycleStartMs;

    public ulong? CurrentCycleTargetMs => _nextTargetMs;

    public void Configure(ulong periodMinMs, ulong periodMaxMs)
    {
        var normalized = EffectCycleSettings.NormalizeRange(
            periodMinMs,
            periodMaxMs,
            EffectCycleSettings.Default.VideoPeriodMinMs,
            EffectCycleSettings.Default.VideoPeriodMaxMs);
        if (_periodMinMs == normalized.Minimum && _periodMaxMs == normalized.Maximum)
        {
            return;
        }

        _periodMinMs = normalized.Minimum;
        _periodMaxMs = normalized.Maximum;
        _cycleStartMs = null;
        _nextTargetMs = null;
    }

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
            _cycleStartMs = null;
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

            _cycleStartMs = null;
            _nextTargetMs = null;
            return false;
        }

        if (position < _lastPositionMs
            && _lastPositionMs - position > SeekBackToleranceMs)
        {
            _cycleStartMs = null;
            _nextTargetMs = null;
        }

        _lastPositionMs = position;
        if (_nextTargetMs is not ulong target)
        {
            _cycleStartMs = position;
            _nextTargetMs = NextTarget(position, duration);
            return false;
        }

        if (position < target || target >= duration)
        {
            return false;
        }

        _cycleStartMs = position;
        _nextTargetMs = NextTarget(position, duration);
        return true;
    }

    private ulong NextTarget(ulong positionMs, ulong durationMs)
    {
        var period = (ulong)_random.NextInt64(
            checked((long)_periodMinMs),
            checked((long)_periodMaxMs + 1));
        return positionMs > durationMs - Math.Min(period, durationMs)
            ? durationMs
            : positionMs + period;
    }
}
