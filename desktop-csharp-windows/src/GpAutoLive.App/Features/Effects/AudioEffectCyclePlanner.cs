using GpAutoLive.Core;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.App.Features.Effects;

public enum AudioEffectCycleAction
{
    None,
    Prepare,
    Commit,
}

/// <summary>
/// 以实际可听播放位置驱动声音候选周期，不拥有解码器、线程或输出设备。
/// </summary>
public sealed class AudioEffectCyclePlanner
{
    public const ulong PrepareLeadMs = 1_000;

    private readonly Random _random;
    private ulong _periodMinMs = EffectCycleSettings.Default.AudioPeriodMinMs;
    private ulong _periodMaxMs = EffectCycleSettings.Default.AudioPeriodMaxMs;
    private MediaPlaybackIdentity? _identity;
    private ulong? _cycleStartMs;
    private ulong? _targetPositionMs;
    private ulong? _currentPeriodMs;
    private bool _prepareRequested;

    public AudioEffectCyclePlanner(Random? random = null) => _random = random ?? Random.Shared;

    public ulong? CurrentCycleStartMs => _cycleStartMs;

    public ulong? CurrentCycleTargetMs => _targetPositionMs;

    public ulong? CurrentPeriodMs => _currentPeriodMs;

    public void Configure(ulong periodMinMs, ulong periodMaxMs)
    {
        var normalized = EffectCycleSettings.NormalizeRange(
            periodMinMs,
            periodMaxMs,
            EffectCycleSettings.Default.AudioPeriodMinMs,
            EffectCycleSettings.Default.AudioPeriodMaxMs);
        if (_periodMinMs == normalized.Minimum && _periodMaxMs == normalized.Maximum)
        {
            return;
        }

        _periodMinMs = normalized.Minimum;
        _periodMaxMs = normalized.Maximum;
        _cycleStartMs = null;
        _targetPositionMs = null;
        _currentPeriodMs = null;
        _prepareRequested = false;
    }

    public AudioEffectCycleAction GetAction(
        MediaPlaybackIdentity? identity,
        ulong? positionMs,
        ulong? durationMs,
        bool enabled,
        bool hasPreparedCandidate,
        out ulong targetPositionMs)
    {
        targetPositionMs = 0;
        if (identity is null || positionMs is not ulong position || durationMs is not ulong duration
            || duration == 0 || !enabled)
        {
            Reset();
            return AudioEffectCycleAction.None;
        }

        if (_identity != identity)
        {
            _identity = identity;
            _cycleStartMs = null;
            _targetPositionMs = null;
            _currentPeriodMs = null;
            _prepareRequested = false;
        }

        if (_targetPositionMs is null)
        {
            _currentPeriodMs = NextPeriod();
            if (ulong.MaxValue - position < _currentPeriodMs.Value
                || position + _currentPeriodMs.Value >= duration)
            {
                _cycleStartMs = null;
                _currentPeriodMs = null;
                return AudioEffectCycleAction.None;
            }

            _cycleStartMs = position;
            _targetPositionMs = position + _currentPeriodMs.Value;
        }

        targetPositionMs = _targetPositionMs.Value;
        if (hasPreparedCandidate)
        {
            return AudioEffectCycleAction.Commit;
        }

        if (_prepareRequested)
        {
            return AudioEffectCycleAction.None;
        }

        var prepareAt = targetPositionMs > PrepareLeadMs
            ? targetPositionMs - PrepareLeadMs
            : 0;
        if (position < prepareAt || position >= targetPositionMs)
        {
            return AudioEffectCycleAction.None;
        }

        _prepareRequested = true;
        return AudioEffectCycleAction.Prepare;
    }

    public void MarkCommitted(MediaPlaybackIdentity identity, ulong targetPositionMs)
    {
        if (_identity != identity || _targetPositionMs != targetPositionMs)
        {
            return;
        }

        _prepareRequested = false;
        _cycleStartMs = null;
        _targetPositionMs = null;
        _currentPeriodMs = null;
    }

    public void Reset()
    {
        _identity = null;
        _cycleStartMs = null;
        _targetPositionMs = null;
        _currentPeriodMs = null;
        _prepareRequested = false;
    }

    private ulong NextPeriod() => _periodMinMs == _periodMaxMs
        ? _periodMinMs
        : (ulong)_random.NextInt64((long)_periodMinMs, checked((long)_periodMaxMs + 1));
}
