using System.Collections.Immutable;
using GpAutoLive.Contracts;

namespace GpAutoLive.Core;

/// <summary>一次插话选择的脱敏结果；只携带预设 ID 和下次时间，不携带 PCM。</summary>
public sealed record InterludeAudioSelection(
    ImmutableArray<string> PresetIds,
    ulong NextIntervalMs,
    DateTimeOffset? PresetValidUntil);

/// <summary>
/// 插话预设/多轨/周期选择器。它只运行在控制线程，不触碰音频回调、文件或网络；
/// 这样 UI 可以先验证选择合同，后续 DSP 接入时复用同一选择结果。
/// </summary>
public sealed class InterludeAudioSelector
{
    private readonly Random _random;
    private ImmutableArray<string> _periodicPresetIds = [];
    private DateTimeOffset _periodicValidUntil = DateTimeOffset.MinValue;
    private InterludeAudioSelectionMode _periodicSelectionMode;
    private string _periodicFixedPresetId = string.Empty;
    private bool _periodicMixEnabled;
    private byte _periodicMixPickMin;
    private byte _periodicMixPickMax;
    private ImmutableArray<string> _periodicCandidates = [];
    private ulong _periodicPeriodMinMs;
    private ulong _periodicPeriodMaxMs;

    /// <summary>创建选择器；测试可注入带种子的 Random。</summary>
    public InterludeAudioSelector(Random? random = null)
    {
        _random = random ?? Random.Shared;
    }

    /// <summary>按配置生成下一次插话选择；配置无效时保持内部状态不变。</summary>
    public bool TrySelect(
        InterludeAudioConfig? config,
        DateTimeOffset now,
        out InterludeAudioSelection? selection,
        out InterludeAudioConfigError? error)
    {
        selection = null;
        if (!InterludeAudioRules.TryValidate(config, out error))
        {
            return false;
        }

        var effective = config!;
        ImmutableArray<string> presetIds;
        DateTimeOffset? validUntil = null;
        if (effective.AudioVariationMode is InterludeAudioVariationMode.Periodic
            && !_periodicPresetIds.IsDefaultOrEmpty
            && IsPeriodicConfigCompatible(effective)
            && now < _periodicValidUntil)
        {
            presetIds = _periodicPresetIds;
            validUntil = _periodicValidUntil;
        }
        else
        {
            presetIds = SelectPresetIds(effective);
            if (effective.AudioVariationMode is InterludeAudioVariationMode.Periodic)
            {
                var periodMs = NextInclusive(effective.AudioVariationPeriodMinMs, effective.AudioVariationPeriodMaxMs);
                _periodicPresetIds = presetIds;
                _periodicValidUntil = now.AddMilliseconds(periodMs);
                _periodicSelectionMode = effective.AudioSelectionMode;
                _periodicFixedPresetId = effective.AudioFixedPresetId;
                _periodicMixEnabled = effective.AudioMixEnabled;
                _periodicMixPickMin = effective.AudioMixPickMin;
                _periodicMixPickMax = effective.AudioMixPickMax;
                _periodicCandidates = effective.AudioPresetIds;
                _periodicPeriodMinMs = effective.AudioVariationPeriodMinMs;
                _periodicPeriodMaxMs = effective.AudioVariationPeriodMaxMs;
                validUntil = _periodicValidUntil;
            }
            else
            {
                _periodicPresetIds = [];
                _periodicValidUntil = DateTimeOffset.MinValue;
                _periodicCandidates = [];
            }
        }

        selection = new(
            presetIds,
            NextInclusive(effective.IntervalMinMs, effective.IntervalMaxMs),
            validUntil);
        error = null;
        return true;
    }

    private bool IsPeriodicConfigCompatible(InterludeAudioConfig config) =>
        _periodicSelectionMode == config.AudioSelectionMode
        && string.Equals(_periodicFixedPresetId, config.AudioFixedPresetId, StringComparison.Ordinal)
        && _periodicMixEnabled == config.AudioMixEnabled
        && _periodicMixPickMin == config.AudioMixPickMin
        && _periodicMixPickMax == config.AudioMixPickMax
        && _periodicPeriodMinMs == config.AudioVariationPeriodMinMs
        && _periodicPeriodMaxMs == config.AudioVariationPeriodMaxMs
        && _periodicCandidates.SequenceEqual(config.AudioPresetIds, StringComparer.Ordinal);

    private ImmutableArray<string> SelectPresetIds(InterludeAudioConfig config)
    {
        if (config.AudioSelectionMode is InterludeAudioSelectionMode.Fixed)
        {
            return [config.AudioFixedPresetId];
        }

        var requestedCount = config.AudioMixEnabled
            ? Math.Min(
                config.AudioPresetIds.Length,
                (int)NextInclusive(config.AudioMixPickMin, config.AudioMixPickMax))
            : 1;
        var candidates = config.AudioPresetIds.ToArray();
        for (var index = candidates.Length - 1; index > 0; index--)
        {
            var swapIndex = _random.Next(index + 1);
            (candidates[index], candidates[swapIndex]) = (candidates[swapIndex], candidates[index]);
        }

        return candidates.AsSpan(0, requestedCount).ToArray().ToImmutableArray();
    }

    private ulong NextInclusive(ulong minimum, ulong maximum)
    {
        if (minimum >= maximum)
        {
            return minimum;
        }

        return (ulong)_random.NextInt64((long)minimum, checked((long)maximum + 1));
    }
}
