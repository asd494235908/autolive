using GpAutoLive.Windows;

namespace GpAutoLive.App.Features.Performance;

/// <summary>把脱敏 Windows 性能快照投影为短文本，不在 UI 中重复推算指标。</summary>
public static class PerformanceMetricsFormatter
{
    public static string Format(WindowsPerformanceSampleResult? result)
    {
        if (result is null || !result.IsSuccess || result.Snapshot is null)
        {
            return result?.Status switch
            {
                WindowsPerformanceSampleStatus.Cancelled => "性能 · 已取消",
                WindowsPerformanceSampleStatus.Invalid => "性能 · 数据无效",
                _ => "性能 · 不可用",
            };
        }

        var snapshot = result.Snapshot;
        var cpu = snapshot.CpuStatus switch
        {
            WindowsCpuSampleStatus.Available when snapshot.CpuPercent is double value =>
                $"CPU {value:0.0}%",
            WindowsCpuSampleStatus.Warmup => "CPU 暖机",
            _ => "CPU —",
        };
        var workingMiB = Math.Max(0, snapshot.WorkingSetBytes / (1024d * 1024d));
        var gui = snapshot.GdiObjectCount is int gdi && snapshot.UserObjectCount is int user
            ? $" · GDI {gdi:N0} · User {user:N0}"
            : string.Empty;
        var gc = snapshot.Gen0CollectionCount is int gen0
            && snapshot.Gen1CollectionCount is int gen1
            && snapshot.Gen2CollectionCount is int gen2
            ? $" · GC {gen0:N0}/{gen1:N0}/{gen2:N0}"
            : string.Empty;
        return $"{cpu} · GPU — · RAM {workingMiB:0} MiB · 线程 {Math.Max(0, snapshot.ThreadCount):N0} · 句柄 {Math.Max(0, snapshot.HandleCount):N0}{gui}{gc}";
    }
}
