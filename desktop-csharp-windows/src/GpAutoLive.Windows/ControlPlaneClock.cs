namespace GpAutoLive.Windows;

/// <summary>控制面调度使用的最小时钟边界，测试可替换时间和等待。</summary>
public interface IControlPlaneClock
{
    DateTimeOffset UtcNow { get; }

    Task DelayAsync(TimeSpan delay, CancellationToken cancellationToken);
}

/// <summary>生产环境使用系统 UTC 时钟和可取消延迟。</summary>
public sealed class SystemControlPlaneClock : IControlPlaneClock
{
    public static SystemControlPlaneClock Instance { get; } = new();

    private SystemControlPlaneClock()
    {
    }

    public DateTimeOffset UtcNow => DateTimeOffset.UtcNow;

    public Task DelayAsync(TimeSpan delay, CancellationToken cancellationToken) =>
        Task.Delay(delay, cancellationToken);
}
