namespace GpAutoLive.Media;

/// <summary>N/N+1 音频候选的受限生命周期。</summary>
public enum AudioCycleCandidateStatus
{
    Planned,
    Preparing,
    Prepared,
    Committing,
}

/// <summary>当前候选调度器允许的单步动作。</summary>
public enum AudioCycleCoordinatorAction
{
    None,
    Prepare,
    Commit,
    Expire,
}

/// <summary>
/// 一个 N+1 候选计划。计划只保存调用方提供的候选引用和绝对媒体时间，
/// 不拥有解码任务、PCM 队列或无限期缓存。
/// </summary>
public sealed record AudioCycleCandidatePlan<T>(
    ulong CandidateId,
    T Sample,
    long TargetAbsolutePositionMs,
    AudioCycleCandidateStatus Status);

/// <summary>
/// 依据绝对媒体时间协调 N+1 的准备、提交和过期；N+2 不在此处创建计划。
/// </summary>
public static class AudioCyclePrewarmCoordinator
{
    public const long CommitGraceMediaMs = 500;

    /// <summary>创建计划时把基准和周期限制为有限的非负毫秒值。</summary>
    public static AudioCycleCandidatePlan<T> CreateCandidatePlan<T>(
        ulong candidateId,
        T sample,
        double baseAbsolutePositionMs,
        double periodMediaMs)
    {
        if (candidateId == 0)
        {
            throw new ArgumentOutOfRangeException(nameof(candidateId), "音频候选 ID 必须大于 0。");
        }

        var safeBase = NormalizeNonNegativeMilliseconds(baseAbsolutePositionMs, fallback: 0);
        var safePeriod = NormalizeNonNegativeMilliseconds(periodMediaMs, fallback: 1);
        safePeriod = Math.Max(1, safePeriod);
        var target = safeBase > long.MaxValue - safePeriod
            ? long.MaxValue
            : safeBase + safePeriod;
        return new(candidateId, sample, target, AudioCycleCandidateStatus.Planned);
    }

    /// <summary>返回新状态计划，不修改已有快照。</summary>
    public static AudioCycleCandidatePlan<T> WithStatus<T>(
        AudioCycleCandidatePlan<T> plan,
        AudioCycleCandidateStatus status)
    {
        ArgumentNullException.ThrowIfNull(plan);
        if (!Enum.IsDefined(status))
        {
            throw new ArgumentOutOfRangeException(nameof(status), "音频候选状态无效。");
        }

        return plan with { Status = status };
    }

    /// <summary>
    /// 计算单步调度动作：planned 只触发 prepare；prepared 在环缓待播时间覆盖目标前触发 commit；
    /// 超过目标宽限期触发 expire；committing 永不重复提交。
    /// </summary>
    public static AudioCycleCoordinatorAction GetAction<T>(
        AudioCycleCandidatePlan<T>? plan,
        double currentAbsolutePositionMs,
        double playbackRate,
        double queuedPlaybackMs = 0)
    {
        if (plan is null)
        {
            return AudioCycleCoordinatorAction.None;
        }

        var safeCurrent = NormalizeNonNegativePosition(currentAbsolutePositionMs);
        var safeRate = double.IsFinite(playbackRate) && playbackRate > 0
            ? Math.Min(playbackRate, 16d)
            : 1d;
        var safeQueued = double.IsFinite(queuedPlaybackMs) && queuedPlaybackMs > 0
            ? Math.Min(queuedPlaybackMs, double.MaxValue / safeRate)
            : 0d;
        var grace = CommitGraceMediaMs * safeRate;
        var target = (double)plan.TargetAbsolutePositionMs;

        if (safeCurrent > target + grace)
        {
            return plan.Status is AudioCycleCandidateStatus.Committing
                ? AudioCycleCoordinatorAction.None
                : AudioCycleCoordinatorAction.Expire;
        }

        if (plan.Status is AudioCycleCandidateStatus.Committing)
        {
            return AudioCycleCoordinatorAction.None;
        }

        if (plan.Status is AudioCycleCandidateStatus.Planned)
        {
            return AudioCycleCoordinatorAction.Prepare;
        }

        var mediaLead = target - safeCurrent;
        var commitLead = safeQueued * safeRate;
        return plan.Status is AudioCycleCandidateStatus.Prepared
            && mediaLead <= commitLead
            ? AudioCycleCoordinatorAction.Commit
            : AudioCycleCoordinatorAction.None;
    }

    private static long NormalizeNonNegativeMilliseconds(double value, long fallback)
    {
        if (!double.IsFinite(value) || value <= 0)
        {
            return fallback;
        }

        return value >= long.MaxValue
            ? long.MaxValue
            : Math.Max(1, checked((long)Math.Round(value, MidpointRounding.AwayFromZero)));
    }

    private static double NormalizeNonNegativePosition(double value) =>
        double.IsFinite(value) && value > 0 ? value : 0;
}
