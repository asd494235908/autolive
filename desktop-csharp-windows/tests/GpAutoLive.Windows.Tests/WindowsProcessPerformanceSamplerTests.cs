using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsProcessPerformanceSamplerTests
{
    [TestMethod]
    public async Task First_sample_is_bounded_and_cpu_is_warmup()
    {
        var source = new FakeSource(new WindowsProcessPerformanceSample(
            10 * 1024,
            20 * 1024,
            4,
            12,
            TimeSpan.FromMilliseconds(10),
            1,
            31,
            17,
            4,
            5,
            6));
        var sampler = new WindowsProcessPerformanceSampler(source);

        var result = await sampler.SampleAsync();

        Assert.IsTrue(result.IsSuccess);
        Assert.IsNotNull(result.Snapshot);
        Assert.AreEqual(10 * 1024, result.Snapshot.PrivateWorkingSetBytes);
        Assert.AreEqual(20 * 1024, result.Snapshot.WorkingSetBytes);
        Assert.AreEqual(4, result.Snapshot.ThreadCount);
        Assert.AreEqual(12, result.Snapshot.HandleCount);
        Assert.AreEqual(31, result.Snapshot.GdiObjectCount);
        Assert.AreEqual(17, result.Snapshot.UserObjectCount);
        Assert.AreEqual(4, result.Snapshot.Gen0CollectionCount);
        Assert.AreEqual(5, result.Snapshot.Gen1CollectionCount);
        Assert.AreEqual(6, result.Snapshot.Gen2CollectionCount);
        Assert.AreEqual(WindowsCpuSampleStatus.Warmup, result.Snapshot.CpuStatus);
        Assert.IsNull(result.Snapshot.CpuPercent);
        Assert.AreEqual(1, source.ReadCount);
    }

    [TestMethod]
    public async Task Cancelled_sample_does_not_read_source()
    {
        var source = new FakeSource(default);
        var sampler = new WindowsProcessPerformanceSampler(source);
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();

        var result = await sampler.SampleAsync(cancellation.Token);

        Assert.AreEqual(WindowsPerformanceSampleStatus.Cancelled, result.Status);
        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(0, source.ReadCount);
    }

    [TestMethod]
    public async Task Invalid_counter_values_are_rejected_without_snapshot()
    {
        var source = new FakeSource(new WindowsProcessPerformanceSample(
            -1,
            20 * 1024,
            4,
            12,
            TimeSpan.Zero,
            1));
        var sampler = new WindowsProcessPerformanceSampler(source);

        var result = await sampler.SampleAsync();

        Assert.AreEqual(WindowsPerformanceSampleStatus.Invalid, result.Status);
        Assert.IsNull(result.Snapshot);
    }

    [TestMethod]
    public async Task Invalid_gui_resource_counts_are_rejected_without_snapshot()
    {
        var source = new FakeSource(new WindowsProcessPerformanceSample(
            10,
            20,
            4,
            12,
            TimeSpan.Zero,
            1,
            -1,
            17));
        var sampler = new WindowsProcessPerformanceSampler(source);

        var result = await sampler.SampleAsync();

        Assert.AreEqual(WindowsPerformanceSampleStatus.Invalid, result.Status);
        Assert.IsNull(result.Snapshot);
    }

    [TestMethod]
    public async Task Invalid_gc_counts_are_rejected_without_snapshot()
    {
        var source = new FakeSource(new WindowsProcessPerformanceSample(
            10,
            20,
            4,
            12,
            TimeSpan.Zero,
            1,
            null,
            null,
            -1,
            0,
            0));
        var sampler = new WindowsProcessPerformanceSampler(source);

        var result = await sampler.SampleAsync();

        Assert.AreEqual(WindowsPerformanceSampleStatus.Invalid, result.Status);
        Assert.IsNull(result.Snapshot);
    }

    [TestMethod]
    public async Task Source_exception_is_mapped_without_leaking_exception_details()
    {
        var sampler = new WindowsProcessPerformanceSampler(new ThrowingSource());

        var result = await sampler.SampleAsync();

        Assert.AreEqual(WindowsPerformanceSampleStatus.Unavailable, result.Status);
        Assert.IsNull(result.Snapshot);
    }

    [TestMethod]
    public void Cpu_calculation_is_normalized_and_bounded()
    {
        var ok = WindowsProcessPerformanceSampler.TryCalculateCpuPercentage(
            TimeSpan.FromSeconds(2),
            TimeSpan.FromSeconds(1),
            processorCount: 4,
            out var percentage);
        var clamped = WindowsProcessPerformanceSampler.TryCalculateCpuPercentage(
            TimeSpan.FromSeconds(1),
            TimeSpan.FromSeconds(20),
            processorCount: 1,
            out var clampedPercentage);

        Assert.IsTrue(ok);
        Assert.AreEqual(12.5d, percentage, 0.0001d);
        Assert.IsTrue(clamped);
        Assert.AreEqual(100d, clampedPercentage);
        Assert.IsFalse(WindowsProcessPerformanceSampler.TryCalculateCpuPercentage(
            TimeSpan.Zero,
            TimeSpan.FromSeconds(1),
            1,
            out _));
    }

    [TestMethod]
    public void Current_process_source_returns_only_bounded_counters()
    {
        var source = new WindowsCurrentProcessPerformanceSource();

        Assert.IsTrue(source.TryRead(out var sample));
        Assert.IsTrue(sample.IsValid);
        Assert.IsTrue(sample.PrivateWorkingSetBytes > 0);
        Assert.IsTrue(sample.WorkingSetBytes > 0);
        Assert.IsTrue(sample.ThreadCount > 0);
        Assert.IsTrue(sample.HandleCount > 0);
        Assert.IsTrue(sample.GdiObjectCount is null or >= 0);
        Assert.IsTrue(sample.UserObjectCount is null or >= 0);
        Assert.IsTrue(sample.Gen0CollectionCount is null or >= 0);
        Assert.IsTrue(sample.Gen1CollectionCount is null or >= 0);
        Assert.IsTrue(sample.Gen2CollectionCount is null or >= 0);
    }

    private sealed class FakeSource(WindowsProcessPerformanceSample sample) : IWindowsProcessPerformanceSource
    {
        public int ReadCount { get; private set; }

        public bool TryRead(out WindowsProcessPerformanceSample current)
        {
            ReadCount++;
            current = sample;
            return true;
        }
    }

    private sealed class ThrowingSource : IWindowsProcessPerformanceSource
    {
        public bool TryRead(out WindowsProcessPerformanceSample sample)
        {
            sample = default;
            throw new InvalidOperationException("sensitive path must not escape");
        }
    }
}
