using System.Collections.Immutable;
using System.Diagnostics;
using GpAutoLive.Media;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsFfmpegPcmDecoderLifecycleTests
{
    [TestMethod]
    public async Task Decode_completion_confirms_native_process_handle_is_signaled()
    {
        using var fixture = new DecoderFixture(continuous: true, outputFrames: 4_096);
        for (var iteration = 0; iteration < 12; iteration++)
        {
            await using var decoder = new WindowsFfmpegPcmDecoder();
            using var cancellation = new CancellationTokenSource();
            var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var decode = decoder.DecodeAsync(fixture.Plan, new AudioPcmRingBuffer(128, 1), cancellation.Token,
                beforePublishWaiter: (_, token) =>
                {
                    entered.TrySetResult();
                    return new(Task.Delay(Timeout.Infinite, token));
                });
            await entered.Task.WaitAsync(TimeSpan.FromSeconds(5));
            using var observer = Process.GetProcessById(decoder.Snapshot.ProcessId!.Value);
            _ = observer.SafeHandle;
            cancellation.Cancel();
            try
            {
                var result = await decode.WaitAsync(TimeSpan.FromSeconds(5));
                Assert.AreEqual(WindowsFfmpegPcmDecoderFailureCode.Cancelled, result.Error?.Code);
                Assert.IsTrue(observer.WaitForExit(0), "Decode 返回后原生进程句柄仍未完成退出。");
            }
            finally
            {
                // 即使旧实现断言失败，也按真实进程事件回收夹具；不以睡眠或忽略文件错误掩盖泄漏。
                Assert.IsTrue(await Task.Run(() => observer.WaitForExit(2_000)));
            }
        }
    }

    [TestMethod]
    public async Task File_overlay_moves_to_current_bus_after_candidate_promotion()
    {
        using var fixture = new DecoderFixture(continuous: false, outputFrames: 8_192);
        using var firstBus = new FinalPcmBus(16_384, 1);
        using var nextBus = new FinalPcmBus(16_384, 1);
        var currentBus = firstBus;
        var publishedChunks = 0;
        await using var decoder = new WindowsFfmpegPcmDecoder();
        var result = await decoder.DecodeAsync(fixture.Plan, destination: null,
            finalPcmBus: firstBus, finalPcmOverlay: true,
            beforePublishWaiter: (_, _) =>
            {
                if (++publishedChunks == 2)
                {
                    currentBus = nextBus;
                    firstBus.Close();
                }
                return ValueTask.CompletedTask;
            },
            finalPcmBusProvider: () => currentBus);
        Assert.IsTrue(result.IsSuccess, result.Error?.Message);
        Assert.AreEqual(4_096, firstBus.OutputOverlayBuffer.Snapshot.AvailableFrames);
        Assert.AreEqual(4_096, nextBus.OutputOverlayBuffer.Snapshot.AvailableFrames);
        Assert.AreEqual(8_192UL, result.Snapshot.DecodedFrames);
    }

    [TestMethod]
    public async Task Overlay_relocates_once_if_previous_bus_closes_after_provider_capture()
    {
        using var fixture = new DecoderFixture(continuous: false, outputFrames: 1);
        using var firstBus = new FinalPcmBus(128, 1);
        using var nextBus = new FinalPcmBus(128, 1);
        var captures = 0;
        await using var decoder = new WindowsFfmpegPcmDecoder();
        var result = await decoder.DecodeAsync(fixture.Plan, destination: null,
            finalPcmBus: firstBus, finalPcmOverlay: true,
            finalPcmBusProvider: () =>
            {
                if (++captures == 1)
                {
                    firstBus.Close();
                    return firstBus;
                }
                return nextBus;
            });
        Assert.IsTrue(result.IsSuccess, result.Error?.Message);
        Assert.AreEqual(0, firstBus.OutputOverlayBuffer.Snapshot.AvailableFrames);
        Assert.AreEqual(1, nextBus.OutputOverlayBuffer.Snapshot.AvailableFrames);
    }

    [TestMethod]
    public async Task Missing_current_overlay_bus_does_not_fall_back_to_retired_bus()
    {
        using var fixture = new DecoderFixture(continuous: false, outputFrames: 1);
        using var initialBus = new FinalPcmBus(128, 1);
        await using var decoder = new WindowsFfmpegPcmDecoder();
        var result = await decoder.DecodeAsync(fixture.Plan, destination: null,
            finalPcmBus: initialBus, finalPcmOverlay: true, finalPcmBusProvider: () => null);
        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(0, initialBus.OutputOverlayBuffer.Snapshot.AvailableFrames);
    }

    [TestMethod]
    public async Task Dispose_timeout_keeps_blocked_reader_owned_until_retry()
    {
        using var fixture = new DecoderFixture(continuous: true);
        var decoder = new WindowsFfmpegPcmDecoder();
        var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var decode = decoder.DecodeAsync(fixture.Plan, new AudioPcmRingBuffer(128, 1),
            pauseWaiter: async _ => { entered.TrySetResult(); await release.Task; });
        await entered.Task.WaitAsync(TimeSpan.FromSeconds(5));
        try
        {
            var error = await CaptureAsync(decoder.DisposeAsync().AsTask());
            Assert.IsInstanceOfType<TimeoutException>(error);
            Assert.IsFalse(decode.IsCompleted, "读取器未结束时 Decode 不得假装完成。");
        }
        finally
        {
            release.TrySetResult();
            _ = await CaptureAsync(decode.WaitAsync(TimeSpan.FromSeconds(5)));
        }

        await decoder.DisposeAsync();
        decoder.Stop();
        Assert.IsFalse(decoder.Snapshot.IsRunning);
        Assert.AreEqual(WindowsFfmpegPcmDecoderFailureCode.Closed,
            (await decoder.DecodeAsync(fixture.Plan, new AudioPcmRingBuffer(128, 1))).Error?.Code);
    }

    [TestMethod]
    public async Task Stop_is_cancelled_not_timeout_and_joins_process()
    {
        using var fixture = new DecoderFixture(continuous: true);
        await using var decoder = new WindowsFfmpegPcmDecoder();
        var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var decode = decoder.DecodeAsync(fixture.Plan, new AudioPcmRingBuffer(128, 1),
            pauseWaiter: token => { entered.TrySetResult(); return new(Task.Delay(Timeout.Infinite, token)); });
        await entered.Task.WaitAsync(TimeSpan.FromSeconds(5));
        var processId = decoder.Snapshot.ProcessId;
        decoder.Stop();
        var result = await decode.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.AreEqual(WindowsFfmpegPcmDecoderFailureCode.Cancelled, result.Error?.Code);
        Assert.IsFalse(decoder.Snapshot.IsRunning);
        AssertProcessExited(processId);
    }

    [TestMethod]
    public async Task Plan_timeout_keeps_timeout_classification_and_joins_process()
    {
        using var fixture = new DecoderFixture(continuous: true);
        await using var decoder = new WindowsFfmpegPcmDecoder();
        var result = await decoder.DecodeAsync(
            fixture.Plan with { Timeout = TimeSpan.FromMilliseconds(200) },
            new AudioPcmRingBuffer(128, 1));
        Assert.AreEqual(WindowsFfmpegPcmDecoderFailureCode.TimedOut, result.Error?.Code);
        Assert.IsFalse(decoder.Snapshot.IsRunning);
    }

    [TestMethod]
    public async Task Concurrent_and_repeated_dispose_are_idempotent()
    {
        var decoder = new WindowsFfmpegPcmDecoder();
        await Task.WhenAll(Enumerable.Range(0, 8).Select(_ => decoder.DisposeAsync().AsTask()));
        await decoder.DisposeAsync();
        decoder.Stop();
    }

    [TestMethod]
    public async Task Eof_and_stop_can_race_without_accessing_disposed_resources()
    {
        using var fixture = new DecoderFixture(continuous: false);
        await using var decoder = new WindowsFfmpegPcmDecoder();
        for (var iteration = 0; iteration < 48; iteration++)
        {
            var decode = decoder.DecodeAsync(fixture.Plan, new AudioPcmRingBuffer(128, 1));
            var stop = Task.Run(async () =>
            {
                while (!decode.IsCompleted)
                {
                    decoder.Stop();
                    await Task.Yield();
                }
                decoder.Stop();
            });
            await Task.WhenAll(decode, stop).WaitAsync(TimeSpan.FromSeconds(5));
            Assert.IsFalse(decoder.Snapshot.IsRunning);
        }
    }

    [TestMethod]
    public async Task Faulted_reader_terminates_process_without_waiting_for_decode_timeout()
    {
        using var fixture = new DecoderFixture(continuous: true);
        await using var decoder = new WindowsFfmpegPcmDecoder();
        int? processId = null;
        var decode = decoder.DecodeAsync(fixture.Plan, new AudioPcmRingBuffer(128, 1),
            pauseWaiter: _ =>
            {
                processId = decoder.Snapshot.ProcessId;
                return ValueTask.FromException(new InvalidOperationException("fixture failure"));
            });
        var result = await decode.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.AreEqual(WindowsFfmpegPcmDecoderFailureCode.OutputReadFailed, result.Error?.Code);
        AssertProcessExited(processId);
    }

    private static async Task<Exception?> CaptureAsync(Task task)
    {
        try { await task; return null; }
        catch (Exception exception) { return exception; }
    }

    private static void AssertProcessExited(int? processId)
    {
        Assert.IsNotNull(processId);
        try
        {
            using var process = Process.GetProcessById(processId.Value);
            Assert.IsTrue(process.HasExited);
        }
        catch (ArgumentException) { }
    }

    private sealed class DecoderFixture : IDisposable
    {
        private readonly string _directory = Path.Combine(Path.GetTempPath(), "gpautolive-decoder-lifecycle", Guid.NewGuid().ToString("N"));
        public DecoderFixture(bool continuous, int outputFrames = 0)
        {
            Directory.CreateDirectory(_directory);
            var script = Path.Combine(_directory, "decoder.cmd");
            var body = continuous
                ? "@echo off\r\n:loop\r\n@echo stderr-line 1>&2\r\n@goto loop\r\n"
                : "@exit /b 0\r\n";
            if (outputFrames > 0)
            {
                var pcmPath = Path.Combine(_directory, "pcm.bin");
                var samples = Enumerable.Repeat(0.125F, outputFrames).ToArray();
                File.WriteAllBytes(pcmPath, System.Runtime.InteropServices.MemoryMarshal.AsBytes(samples.AsSpan()).ToArray());
                body = $"@type \"{pcmPath}\"\r\n" + body;
            }
            File.WriteAllText(script, body);
            var command = Environment.GetEnvironmentVariable("ComSpec") ?? Path.Combine(Environment.SystemDirectory, "cmd.exe");
            Plan = new(command, ImmutableArray.Create("/d", "/q", "/c", script), script, 48_000, 1, TimeSpan.FromSeconds(30));
        }
        public FfmpegPcmDecodePlan Plan { get; }
        public void Dispose() => Directory.Delete(_directory, recursive: true);
    }
}
