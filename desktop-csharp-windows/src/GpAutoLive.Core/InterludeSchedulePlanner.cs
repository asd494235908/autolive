using GpAutoLive.Contracts;

namespace GpAutoLive.Core;

/// <summary>一次插话调度观察的最小结果；不携带文件路径、PCM 或 UI 状态。</summary>
public readonly record struct InterludeScheduleDecision(
    bool ShouldStart,
    int? FileIndex,
    double ProgressPercent,
    long? NextDueTimestampMs);

/// <summary>
/// 按 Rust 播放端语义规划插话触发时机。只拥有调度状态，不启动任务、不访问文件或设备。
/// </summary>
public sealed class InterludeSchedulePlanner
{
    private readonly Random _random;
    private string? _scheduleKey;
    private long? _nextDueTimestampMs;
    private long? _intervalStartedTimestampMs;
    private int? _lastFileIndex;
    private bool _interludeActiveObserved;

    /// <summary>创建调度器；测试可注入带种子的随机源。</summary>
    public InterludeSchedulePlanner(Random? random = null)
    {
        _random = random ?? Random.Shared;
    }

    /// <summary>当前已选择的上一文件索引；仅用于调度诊断和测试。</summary>
    public int? LastFileIndex => _lastFileIndex;

    /// <summary>切换媒体代次或停止播放时重置调度；默认同时允许新源立即触发。</summary>
    public void Reset(string? scheduleKey, bool resetLastFileIndex = true)
    {
        _scheduleKey = scheduleKey;
        _nextDueTimestampMs = null;
        _intervalStartedTimestampMs = null;
        _interludeActiveObserved = false;
        if (resetLastFileIndex)
        {
            _lastFileIndex = null;
        }
    }

    /// <summary>记录手动启动的文件；当前插话结束后再建立下一次自动等待。</summary>
    public void MarkPlaybackStarted(
        string? scheduleKey,
        int fileIndex)
    {
        if (!string.Equals(_scheduleKey, scheduleKey, StringComparison.Ordinal))
        {
            Reset(scheduleKey);
        }

        _lastFileIndex = Math.Max(0, fileIndex);
        _intervalStartedTimestampMs = null;
        _nextDueTimestampMs = null;
        _interludeActiveObserved = true;
    }

    /// <summary>
    /// 观察一次当前播放状态。首次就绪观察立即触发；当前插话结束后，下一轮按配置间隔等待。
    /// </summary>
    public InterludeScheduleDecision Observe(
        string? scheduleKey,
        long nowTimestampMs,
        int fileCount,
        ulong intervalMinMs,
        ulong intervalMaxMs,
        bool enabled,
        bool playbackActive,
        bool playbackStarting,
        bool paused,
        bool canStartPlayback = true)
    {
        var now = Math.Max(0, nowTimestampMs);
        if (!string.Equals(_scheduleKey, scheduleKey, StringComparison.Ordinal))
        {
            Reset(scheduleKey);
        }

        if (string.IsNullOrWhiteSpace(scheduleKey)
            || fileCount <= 0
            || !enabled
            )
        {
            return Waiting(now);
        }

        if (playbackActive || playbackStarting)
        {
            _interludeActiveObserved = true;
            return Waiting(now);
        }

        if (_interludeActiveObserved)
        {
            _interludeActiveObserved = false;
            _nextDueTimestampMs = null;
            _intervalStartedTimestampMs = null;
        }

        if (paused)
        {
            return Waiting(now);
        }

        if (_nextDueTimestampMs is null)
        {
            _intervalStartedTimestampMs = now;
            var intervalMs = _lastFileIndex is null
                ? 0UL
                : NextIntervalMs(intervalMinMs, intervalMaxMs);
            _nextDueTimestampMs = SaturatingAdd(now, intervalMs);
        }

        if (now < _nextDueTimestampMs.Value)
        {
            return Waiting(now);
        }

        if (!canStartPlayback)
        {
            return Waiting(now);
        }

        var selectedIndex = ChooseNextFileIndex(fileCount, _lastFileIndex);
        _lastFileIndex = selectedIndex;
        _interludeActiveObserved = true;
        _nextDueTimestampMs = null;
        _intervalStartedTimestampMs = null;
        return new(true, selectedIndex, 0, null);
    }

    private InterludeScheduleDecision Waiting(long now)
    {
        var start = _intervalStartedTimestampMs;
        var due = _nextDueTimestampMs;
        var progress = start is null || due is null || due <= start
            ? 0
            : Math.Floor(Math.Clamp((now - start.Value) * 100d / (due.Value - start.Value), 0, 100));
        return new(false, null, progress, due);
    }

    private ulong NextIntervalMs(ulong minimum, ulong maximum)
    {
        var lower = Math.Clamp(Math.Min(minimum, maximum), InterludeAudioRules.MinIntervalMs, InterludeAudioRules.MaxIntervalMs);
        var upper = Math.Clamp(Math.Max(minimum, maximum), InterludeAudioRules.MinIntervalMs, InterludeAudioRules.MaxIntervalMs);
        return lower >= upper
            ? lower
            : (ulong)_random.NextInt64((long)lower, checked((long)upper + 1));
    }

    private int ChooseNextFileIndex(int fileCount, int? previousIndex)
    {
        if (fileCount == 1)
        {
            return 0;
        }

        var candidate = _random.Next(fileCount - 1);
        if (previousIndex is int previous
            && previous >= 0
            && previous < fileCount
            && candidate >= previous)
        {
            candidate++;
        }

        return candidate;
    }

    private static long SaturatingAdd(long timestampMs, ulong intervalMs) =>
        intervalMs >= long.MaxValue - (ulong)timestampMs
            ? long.MaxValue
            : timestampMs + (long)intervalMs;
}
