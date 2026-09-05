using GpAutoLive.App.Features.Performance;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class PerformanceMetricsFormatterTests
{
    [TestMethod]
    public void Captured_snapshot_formats_cpu_memory_threads_and_handles()
    {
        var result = new WindowsPerformanceSampleResult(
            WindowsPerformanceSampleStatus.Captured,
            new WindowsProcessPerformanceSnapshot(
                DateTimeOffset.UtcNow,
                64 * 1024 * 1024,
                128 * 1024 * 1024,
                22,
                1_079,
                3.74,
                WindowsCpuSampleStatus.Available,
                31,
                17,
                4,
                5,
                6));

        var text = PerformanceMetricsFormatter.Format(result);

        StringAssert.Contains(text, "CPU 3.7%");
        StringAssert.Contains(text, "RAM 128 MiB");
        StringAssert.Contains(text, "线程 22");
        StringAssert.Contains(text, "句柄 1,079");
        StringAssert.Contains(text, "GDI 31");
        StringAssert.Contains(text, "User 17");
        StringAssert.Contains(text, "GC 4/5/6");
    }

    [TestMethod]
    public void Warmup_snapshot_does_not_claim_cpu_is_available()
    {
        var result = new WindowsPerformanceSampleResult(
            WindowsPerformanceSampleStatus.Captured,
            new WindowsProcessPerformanceSnapshot(
                DateTimeOffset.UtcNow,
                1,
                1,
                1,
                1,
                null,
                WindowsCpuSampleStatus.Warmup));

        StringAssert.Contains(PerformanceMetricsFormatter.Format(result), "CPU 暖机");
    }

    [TestMethod]
    public void Unavailable_snapshot_is_presented_as_unavailable()
    {
        var result = new WindowsPerformanceSampleResult(
            WindowsPerformanceSampleStatus.Unavailable,
            null);

        Assert.AreEqual("性能 · 不可用", PerformanceMetricsFormatter.Format(result));
    }
}
