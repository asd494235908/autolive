using GpAutoLive.Core;

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
    public const ulong CyclePeriodMs = 4_000;
    public const ulong PrepareLeadMs = 1_000;

    private MediaPlaybackIdentity? _identity;
    private ulong? _targetPositionMs;
    private bool _prepareRequested;

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
            _targetPositionMs = null;
            _prepareRequested = false;
        }

        if (_targetPositionMs is null)
        {
            if (ulong.MaxValue - position < CyclePeriodMs
                || position + CyclePeriodMs >= duration)
            {
                return AudioEffectCycleAction.None;
            }

            _targetPositionMs = position + CyclePeriodMs;
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
        _targetPositionMs = ulong.MaxValue - targetPositionMs < CyclePeriodMs
            ? null
            : targetPositionMs + CyclePeriodMs;
    }

    public void Reset()
    {
        _identity = null;
        _targetPositionMs = null;
        _prepareRequested = false;
    }
}
