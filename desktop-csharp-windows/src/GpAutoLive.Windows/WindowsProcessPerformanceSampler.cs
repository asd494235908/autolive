using System.ComponentModel;
using System.Diagnostics;

namespace GpAutoLive.Windows;

/// <summary>
/// 只读取当前进程的无身份性能字段。实现不暴露 PID、路径、用户名、主机名或命令行。
/// </summary>
public interface IWindowsProcessPerformanceSource
{
    bool TryRead(out WindowsProcessPerformanceSample sample);
}

/// <summary>一次底层 Windows 进程计数器读取结果。</summary>
public readonly record struct WindowsProcessPerformanceSample(
    long PrivateWorkingSetBytes,
    long WorkingSetBytes,
    int ThreadCount,
    int HandleCount,
    TimeSpan? TotalProcessorTime,
    long MonotonicTimestampTicks,
    int? GdiObjectCount = null,
    int? UserObjectCount = null,
    int? Gen0CollectionCount = null,
    int? Gen1CollectionCount = null,
    int? Gen2CollectionCount = null)
{
    private const long MaximumMetricBytes = 1L << 50;
    private const int MaximumObjectCount = 1_000_000;

    public bool IsValid =>
        PrivateWorkingSetBytes is >= 0 and <= MaximumMetricBytes
        && WorkingSetBytes is >= 0 and <= MaximumMetricBytes
        && ThreadCount is >= 0 and <= MaximumObjectCount
        && HandleCount is >= 0 and <= MaximumObjectCount
        && (GdiObjectCount is null || GdiObjectCount.Value is >= 0 and <= MaximumObjectCount)
        && (UserObjectCount is null || UserObjectCount.Value is >= 0 and <= MaximumObjectCount)
        && (Gen0CollectionCount is null || Gen0CollectionCount.Value >= 0)
        && (Gen1CollectionCount is null || Gen1CollectionCount.Value >= 0)
        && (Gen2CollectionCount is null || Gen2CollectionCount.Value >= 0)
        && (TotalProcessorTime is null || TotalProcessorTime.Value >= TimeSpan.Zero)
        && MonotonicTimestampTicks > 0;
}

public enum WindowsPerformanceSampleStatus
{
    Captured,
    Cancelled,
    Unavailable,
    Invalid,
}

public enum WindowsCpuSampleStatus
{
    Available,
    Warmup,
    Unavailable,
}

/// <summary>
/// 当前进程的最小性能快照。该类型刻意不包含进程身份或用户数据。
/// </summary>
public sealed record WindowsProcessPerformanceSnapshot(
    DateTimeOffset CapturedAtUtc,
    long PrivateWorkingSetBytes,
    long WorkingSetBytes,
    int ThreadCount,
    int HandleCount,
    double? CpuPercent,
    WindowsCpuSampleStatus CpuStatus,
    int? GdiObjectCount = null,
    int? UserObjectCount = null,
    int? Gen0CollectionCount = null,
    int? Gen1CollectionCount = null,
    int? Gen2CollectionCount = null);

public sealed record WindowsPerformanceSampleResult(
    WindowsPerformanceSampleStatus Status,
    WindowsProcessPerformanceSnapshot? Snapshot)
{
    public bool IsSuccess => Status == WindowsPerformanceSampleStatus.Captured && Snapshot is not null;
}

/// <summary>
/// Windows 当前进程性能采样器。
/// 每次调用只读取一次本地计数器；CPU 使用相邻采样计算，首次采样明确返回 Warmup。
/// </summary>
public sealed class WindowsProcessPerformanceSampler
{
    private static readonly TimeSpan MaximumCpuSampleGap = TimeSpan.FromMinutes(5);
    private readonly IWindowsProcessPerformanceSource _source;
    private WindowsProcessPerformanceSample? _previousSample;

    public WindowsProcessPerformanceSampler(IWindowsProcessPerformanceSource source)
    {
        _source = source ?? throw new ArgumentNullException(nameof(source));
    }

    /// <summary>
    /// 采样不拥有后台循环，也不写文件；调用方应按不高于 2Hz 的 UI/诊断节奏调度。
    /// </summary>
    public ValueTask<WindowsPerformanceSampleResult> SampleAsync(
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return ValueTask.FromResult(Cancelled());
        }

        WindowsProcessPerformanceSample current;
        try
        {
            if (!_source.TryRead(out current))
            {
                return ValueTask.FromResult(
                    cancellationToken.IsCancellationRequested ? Cancelled() : Unavailable());
            }
        }
        catch (Exception exception) when (IsExpectedSourceFailure(exception))
        {
            return ValueTask.FromResult(
                cancellationToken.IsCancellationRequested ? Cancelled() : Unavailable());
        }

        if (cancellationToken.IsCancellationRequested)
        {
            return ValueTask.FromResult(Cancelled());
        }

        if (!current.IsValid)
        {
            return ValueTask.FromResult(Invalid());
        }

        var cpuStatus = WindowsCpuSampleStatus.Unavailable;
        double? cpuPercent = null;
        if (current.TotalProcessorTime is not null)
        {
            if (_previousSample is not { } previous
                || previous.TotalProcessorTime is null)
            {
                cpuStatus = WindowsCpuSampleStatus.Warmup;
            }
            else
            {
                var elapsed = Stopwatch.GetElapsedTime(
                    previous.MonotonicTimestampTicks,
                    current.MonotonicTimestampTicks);
                if (elapsed > TimeSpan.Zero
                    && elapsed <= MaximumCpuSampleGap
                    && current.TotalProcessorTime.Value >= previous.TotalProcessorTime.Value
                    && TryCalculateCpuPercentage(
                        elapsed,
                        current.TotalProcessorTime.Value - previous.TotalProcessorTime.Value,
                        Environment.ProcessorCount,
                        out var calculated))
                {
                    cpuPercent = calculated;
                    cpuStatus = WindowsCpuSampleStatus.Available;
                }
            }
        }

        _previousSample = current;
        return ValueTask.FromResult(
            new WindowsPerformanceSampleResult(
                WindowsPerformanceSampleStatus.Captured,
                new WindowsProcessPerformanceSnapshot(
                    DateTimeOffset.UtcNow,
                    current.PrivateWorkingSetBytes,
                    current.WorkingSetBytes,
                    current.ThreadCount,
                    current.HandleCount,
                    cpuPercent,
                    cpuStatus,
                    current.GdiObjectCount,
                    current.UserObjectCount,
                    current.Gen0CollectionCount,
                    current.Gen1CollectionCount,
                    current.Gen2CollectionCount)));
    }

    /// <summary>丢弃 CPU 相邻样本基线；下一次采样会回到 Warmup。</summary>
    public void ResetCpuBaseline() => _previousSample = null;

    /// <summary>纯逻辑 CPU 计算，结果按整机逻辑处理器数归一到 0～100。</summary>
    public static bool TryCalculateCpuPercentage(
        TimeSpan elapsed,
        TimeSpan processorTimeDelta,
        int processorCount,
        out double cpuPercent)
    {
        cpuPercent = 0;
        if (elapsed <= TimeSpan.Zero
            || processorTimeDelta < TimeSpan.Zero
            || processorCount is < 1 or > 512
            || elapsed > MaximumCpuSampleGap)
        {
            return false;
        }

        var percentage = processorTimeDelta.TotalSeconds
            / (elapsed.TotalSeconds * processorCount)
            * 100d;
        if (double.IsNaN(percentage) || double.IsInfinity(percentage))
        {
            return false;
        }

        cpuPercent = Math.Clamp(percentage, 0d, 100d);
        return true;
    }

    private static bool IsExpectedSourceFailure(Exception exception) =>
        exception is InvalidOperationException
            or Win32Exception
            or NotSupportedException
            or UnauthorizedAccessException
            or ObjectDisposedException;

    private static WindowsPerformanceSampleResult Cancelled() =>
        new(WindowsPerformanceSampleStatus.Cancelled, null);

    private static WindowsPerformanceSampleResult Unavailable() =>
        new(WindowsPerformanceSampleStatus.Unavailable, null);

    private static WindowsPerformanceSampleResult Invalid() =>
        new(WindowsPerformanceSampleStatus.Invalid, null);
}

/// <summary>Windows 原生当前进程计数器源。</summary>
public sealed class WindowsCurrentProcessPerformanceSource : IWindowsProcessPerformanceSource
{
    private const uint GdiObjects = 0;
    private const uint UserObjects = 1;

    [System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true)]
    private static extern uint GetGuiResources(nint processHandle, uint flags);

    public bool TryRead(out WindowsProcessPerformanceSample sample)
    {
        sample = default;
        if (!OperatingSystem.IsWindows())
        {
            return false;
        }

        using var process = Process.GetCurrentProcess();
        process.Refresh();

        TimeSpan? totalProcessorTime;
        try
        {
            totalProcessorTime = process.TotalProcessorTime;
        }
        catch (InvalidOperationException)
        {
            totalProcessorTime = null;
        }
        catch (Win32Exception)
        {
            totalProcessorTime = null;
        }

        var gdiObjectCount = TryReadGuiResourceCount(process, GdiObjects);
        var userObjectCount = TryReadGuiResourceCount(process, UserObjects);
        var gen0CollectionCount = TryReadCollectionCount(0);
        var gen1CollectionCount = TryReadCollectionCount(1);
        var gen2CollectionCount = TryReadCollectionCount(2);

        sample = new WindowsProcessPerformanceSample(
            process.PrivateMemorySize64,
            process.WorkingSet64,
            process.Threads.Count,
            process.HandleCount,
            totalProcessorTime,
            Stopwatch.GetTimestamp(),
            gdiObjectCount,
            userObjectCount,
            gen0CollectionCount,
            gen1CollectionCount,
            gen2CollectionCount);
        return true;
    }

    private static int? TryReadCollectionCount(int generation)
    {
        try
        {
            return GC.CollectionCount(generation);
        }
        catch (ArgumentOutOfRangeException)
        {
            return null;
        }
    }

    private static int? TryReadGuiResourceCount(Process process, uint flags)
    {
        try
        {
            var count = GetGuiResources(process.Handle, flags);
            return count == 0 ? null : checked((int)count);
        }
        catch (InvalidOperationException)
        {
            return null;
        }
        catch (Win32Exception)
        {
            return null;
        }
        catch (OverflowException)
        {
            return null;
        }
    }
}
