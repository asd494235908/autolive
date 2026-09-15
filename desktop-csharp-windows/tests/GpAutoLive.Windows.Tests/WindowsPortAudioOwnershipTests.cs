using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsPortAudioOwnershipTests
{
    private static readonly TimeSpan Budget = TimeSpan.FromMilliseconds(500);

    [TestMethod]
    [DataRow("Initialize")]
    [DataRow("Open")]
    public async Task Input_start_budget_covers_native_initialization_and_open(string operation)
    {
        var native = new FakeWindowsPortAudioStreamNative { BlockOperation = operation };
        var path = CreateResource();
        using var input = new WindowsPortAudioInputStream(new AudioPcmRingBuffer(256, 1), null, _ => native, Budget);
        try
        {
            var start = input.StartAsync(path, new(0));
            Assert.IsTrue(native.Entered.Wait(TimeSpan.FromSeconds(2)));
            Assert.IsFalse((await start).IsSuccess);
            Assert.IsTrue(input.HasPendingCleanup);
            Assert.AreEqual(0, native.DisposeCount);
            native.Release.Set();
            Assert.IsTrue((await input.StopAsync()).IsSuccess);
            Assert.IsFalse(input.HasPendingCleanup);
            Assert.AreEqual(1, native.DisposeCount);
            Assert.AreEqual(1, native.MaxConcurrent);
        }
        finally
        {
            native.Release.Set();
            DeleteResource(path);
        }
    }

    [TestMethod]
    [DataRow("Stop")]
    [DataRow("Terminate")]
    public async Task Output_stop_budget_covers_native_stop_and_terminate(string operation)
    {
        var native = new FakeWindowsPortAudioStreamNative();
        var path = CreateResource();
        using var output = new WindowsPortAudioOutputStream(
            new AudioPcmRingBufferOutputSource(new AudioPcmRingBuffer(256, 2)), _ => native, Budget);
        try
        {
            Assert.IsTrue((await output.StartAsync(path, new(0, 2, 48_000))).IsSuccess);
            native.BlockOperation = operation;
            Assert.IsFalse((await output.StopAsync()).IsSuccess);
            Assert.IsTrue(output.HasPendingCleanup);
            Assert.AreEqual(0, native.DisposeCount);
            native.Release.Set();
            Assert.IsTrue((await output.StopAsync()).IsSuccess);
            Assert.AreEqual(1, native.DisposeCount);
            Assert.AreEqual(1, native.MaxConcurrent);
        }
        finally
        {
            native.Release.Set();
            DeleteResource(path);
        }
    }

    [TestMethod]
    public async Task Cancelled_input_start_returns_without_unloading_an_inflight_call()
    {
        var native = new FakeWindowsPortAudioStreamNative { BlockOperation = "Open" };
        var path = CreateResource();
        using var input = new WindowsPortAudioInputStream(new AudioPcmRingBuffer(256, 1), null, _ => native, Budget);
        using var cancellation = new CancellationTokenSource();
        try
        {
            var start = input.StartAsync(path, new(0), cancellation.Token);
            Assert.IsTrue(native.Entered.Wait(TimeSpan.FromSeconds(2)));
            cancellation.Cancel();
            Assert.AreEqual(WindowsPortAudioInputFailureCode.Cancelled, (await start).Error?.Code);
            Assert.AreEqual(0, native.DisposeCount);
            Assert.IsTrue(input.HasPendingCleanup);
            native.Release.Set();
            Assert.IsTrue((await input.StopAsync()).IsSuccess);
            Assert.AreEqual(1, native.CloseCount);
        }
        finally
        {
            native.Release.Set();
            DeleteResource(path);
        }
    }

    [TestMethod]
    public async Task Cached_snapshot_schedules_health_in_background_and_stop_waits_for_that_query()
    {
        var native = new FakeWindowsPortAudioStreamNative();
        var path = CreateResource();
        using var output = new WindowsPortAudioOutputStream(
            new AudioPcmRingBufferOutputSource(new AudioPcmRingBuffer(256, 2)), _ => native, Budget);
        try
        {
            Assert.IsTrue((await output.StartAsync(path, new(0, 2, 48_000))).IsSuccess);
            native.BlockOperation = "Query";
            typeof(WindowsPortAudioOutputStream).GetField("_lastHealthQueryAt",
                System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!.SetValue(output, 0L);
            _ = output.Snapshot;
            Assert.IsTrue(native.Entered.Wait(TimeSpan.FromSeconds(2)));
            await Task.Run(() => output.Snapshot).WaitAsync(TimeSpan.FromSeconds(1));
            Assert.IsFalse((await output.StopAsync()).IsSuccess);
            Assert.AreEqual(0, native.CloseCount);
            native.Release.Set();
            Assert.IsTrue((await output.StopAsync()).IsSuccess);
            Assert.AreEqual(1, native.MaxConcurrent);
            Assert.IsFalse(output.Snapshot.IsRunning);
        }
        finally
        {
            native.Release.Set();
            DeleteResource(path);
        }
    }

    [TestMethod]
    public async Task Dispose_timeout_is_explicit_and_the_same_owner_can_finish_on_retry()
    {
        var native = new FakeWindowsPortAudioStreamNative();
        var path = CreateResource();
        var output = new WindowsPortAudioOutputStream(
            new AudioPcmRingBufferOutputSource(new AudioPcmRingBuffer(256, 2)), _ => native, Budget);
        try
        {
            Assert.IsTrue((await output.StartAsync(path, new(0, 2, 48_000))).IsSuccess);
            native.BlockOperation = "Close";
            Assert.ThrowsExactly<TimeoutException>(output.Dispose);
            Assert.IsTrue(output.HasPendingCleanup);
            Assert.AreEqual(0, native.DisposeCount);
            native.Release.Set();
            output.Dispose();
            Assert.IsFalse(output.HasPendingCleanup);
            Assert.AreEqual(1, native.DisposeCount);
        }
        finally
        {
            native.Release.Set();
            output.Dispose();
            DeleteResource(path);
        }
    }

    [TestMethod]
    public async Task Input_late_start_is_retained_then_closed_and_never_reactivated()
    {
        var native = new FakeWindowsPortAudioStreamNative { BlockOperation = "Start" };
        var path = CreateResource();
        using var input = new WindowsPortAudioInputStream(new AudioPcmRingBuffer(256, 1), null, _ => native, Budget);
        try
        {
            var start = input.StartAsync(path, new(0));
            Assert.IsTrue(native.Entered.Wait(TimeSpan.FromSeconds(2)));
            Assert.IsFalse((await start).IsSuccess);
            Assert.IsTrue(input.HasPendingCleanup);
            Assert.AreEqual(0, native.DisposeCount);
            Assert.IsFalse((await input.StartAsync(path, new(0))).IsSuccess);
            native.Release.Set();
            Assert.IsTrue((await input.StopAsync()).IsSuccess);
            Assert.IsFalse(input.HasPendingCleanup);
            Assert.IsFalse(input.Snapshot.IsRunning);
            Assert.AreEqual(1, native.CloseCount);
            Assert.AreEqual(1, native.DisposeCount);
            Assert.AreEqual(1, native.MaxConcurrent);
        }
        finally
        {
            native.Release.Set();
            DeleteResource(path);
        }
    }

    [TestMethod]
    public async Task Output_close_timeout_keeps_owner_and_repeated_stop_joins_same_cleanup()
    {
        var native = new FakeWindowsPortAudioStreamNative();
        var path = CreateResource();
        using var output = new WindowsPortAudioOutputStream(
            new AudioPcmRingBufferOutputSource(new AudioPcmRingBuffer(256, 2)), _ => native, Budget);
        try
        {
            Assert.IsTrue((await output.StartAsync(path, new(0, 2, 48_000))).IsSuccess);
            native.BlockOperation = "Close";
            Assert.IsFalse((await output.StopAsync()).IsSuccess);
            Assert.IsTrue(output.HasPendingCleanup);
            Assert.AreEqual(0, native.DisposeCount);
            Assert.IsFalse((await output.StopAsync()).IsSuccess);
            Assert.AreEqual(1, native.CloseCount);
            native.Release.Set();
            Assert.IsTrue((await output.StopAsync()).IsSuccess);
            Assert.IsFalse(output.HasPendingCleanup);
            Assert.AreEqual(1, native.CloseCount);
            Assert.AreEqual(1, native.DisposeCount);
            Assert.AreEqual(1, native.MaxConcurrent);
        }
        finally
        {
            native.Release.Set();
            DeleteResource(path);
        }
    }

    [TestMethod]
    public async Task Output_close_error_is_not_hidden_by_restart_and_can_be_retried()
    {
        var native = new FakeWindowsPortAudioStreamNative();
        var path = CreateResource();
        using var output = new WindowsPortAudioOutputStream(
            new AudioPcmRingBufferOutputSource(new AudioPcmRingBuffer(256, 2)), _ => native, Budget);
        try
        {
            Assert.IsTrue((await output.StartAsync(path, new(0, 2, 48_000))).IsSuccess);
            native.CloseError = -1;
            Assert.IsFalse((await output.RestartAsync(path, new(0, 2, 48_000))).IsSuccess);
            Assert.IsTrue(output.HasPendingCleanup);
            Assert.AreEqual(0, native.DisposeCount);
            native.CloseError = 0;
            Assert.IsTrue((await output.StopAsync()).IsSuccess);
            Assert.AreEqual(1, native.DisposeCount);
        }
        finally
        {
            DeleteResource(path);
        }
    }

    [TestMethod]
    public async Task Pending_health_query_never_blocks_snapshot_or_runs_alongside_close()
    {
        var native = new FakeWindowsPortAudioStreamNative { BlockOperation = "Query" };
        var path = CreateResource();
        using var output = new WindowsPortAudioOutputStream(
            new AudioPcmRingBufferOutputSource(new AudioPcmRingBuffer(256, 2)), _ => native, Budget);
        try
        {
            // 启动阶段健康查询也属于同一预算；阻塞不能转移到 Snapshot 的调用线程。
            var start = output.StartAsync(path, new(0, 2, 48_000));
            Assert.IsTrue(native.Entered.Wait(TimeSpan.FromSeconds(2)));
            Assert.IsFalse((await start).IsSuccess);
            var snapshotRead = Task.Run(() => output.Snapshot);
            var snapshot = await snapshotRead.WaitAsync(TimeSpan.FromSeconds(1));
            Assert.IsFalse(snapshot.IsRunning);
            Assert.AreEqual(0, native.CloseCount);
            Assert.AreEqual(0, native.DisposeCount);
            native.Release.Set();
            Assert.IsTrue((await output.StopAsync()).IsSuccess);
            Assert.AreEqual(1, native.MaxConcurrent);
            Assert.IsFalse(output.Snapshot.IsRunning);
        }
        finally
        {
            native.Release.Set();
            DeleteResource(path);
        }
    }

    private static void DeleteResource(string path)
    {
        File.Delete(path);
        Directory.Delete(Path.GetDirectoryName(path)!);
    }
    private static string CreateResource()
    {
        var directory = Path.Combine(Path.GetTempPath(), "gpautolive-native-owner", Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        var path = Path.Combine(directory, "portaudio_x64.dll");
        File.WriteAllBytes(path, []);
        return path;
    }
}
