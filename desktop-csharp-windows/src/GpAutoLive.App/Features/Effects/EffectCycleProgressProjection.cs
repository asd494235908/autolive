namespace GpAutoLive.App.Features.Effects;

/// <summary>把调度器已经确定的周期窗口投影为界面百分比，不拥有时钟或调度状态。</summary>
internal static class EffectCycleProgressProjection
{
    public static double Calculate(
        ulong? positionMs,
        ulong? cycleStartMs,
        ulong? cycleTargetMs)
    {
        if (positionMs is not ulong position
            || cycleStartMs is not ulong start
            || cycleTargetMs is not ulong target
            || target <= start)
        {
            return 0;
        }

        if (position <= start)
        {
            return 0;
        }

        if (position >= target)
        {
            return 100;
        }

        return (position - start) * 100d / (target - start);
    }
}
