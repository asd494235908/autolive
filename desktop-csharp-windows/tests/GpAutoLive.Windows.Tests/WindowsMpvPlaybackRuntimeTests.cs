using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsMpvPlaybackRuntimeTests
{
    [TestMethod]
    public async Task StopBeforeStartIsIdempotent()
    {
        await using var runtime = new WindowsMpvPlaybackRuntime();

        var result = await runtime.StopAsync();

        Assert.IsTrue(result.IsSuccess);
        Assert.AreEqual(WindowsMpvPlaybackRuntimeState.Stopped, result.Snapshot.State);
        Assert.AreEqual(MpvIpcPipeState.Disconnected, result.Snapshot.IpcState);
    }

    [TestMethod]
    public async Task StartRejectsNullBindingWithoutStartingAProcess()
    {
        await using var runtime = new WindowsMpvPlaybackRuntime();

        var result = await runtime.StartAsync(null, null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsMpvPlaybackRuntimeFailureCode.InvalidBinding, result.Error?.Code);
        Assert.AreEqual(WindowsMpvPlaybackRuntimeState.Ready, result.Snapshot.State);
        Assert.IsNull(result.Snapshot.Host.ProcessId);
    }

    [TestMethod]
    public async Task DispatchBeforeStartFailsClosed()
    {
        await using var runtime = new WindowsMpvPlaybackRuntime();

        var result = await runtime.DispatchAsync(
            MpvIpcCommand.Quit(),
            new MediaPlaybackIdentity(1, 1, 0, 0));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MpvSessionFailureCode.SessionClosed, result.SessionError?.Code);
    }

    [TestMethod]
    public async Task PollBeforeStartFailsWithoutTouchingAnIpcGateway()
    {
        await using var runtime = new WindowsMpvPlaybackRuntime();

        var result = await runtime.PollPlaybackStateAsync(
            new MediaPlaybackIdentity(2, 3, 0, 0));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(
            MpvPlaybackStateMonitorFailureCode.RuntimeUnavailable,
            result.Error?.Code);
        Assert.IsNull(result.Snapshot);
        Assert.AreEqual(MpvIpcPipeState.Disconnected, runtime.Snapshot.IpcState);
    }

    [TestMethod]
    public async Task WatchAfterStopEndsAtRuntimeBoundaryWithoutUsingReleasedGateway()
    {
        await using var runtime = new WindowsMpvPlaybackRuntime();
        var stopped = await runtime.StopAsync();
        Assert.IsTrue(stopped.IsSuccess);

        var results = new List<MpvPlaybackStateMonitorResult>();
        await foreach (var result in runtime.WatchPlaybackStateAsync(
                           new MediaPlaybackIdentity(3, 4, 0, 0)))
        {
            results.Add(result);
        }

        Assert.AreEqual(1, results.Count);
        Assert.IsFalse(results[0].IsSuccess);
        Assert.AreEqual(
            MpvPlaybackStateMonitorFailureCode.RuntimeUnavailable,
            results[0].Error?.Code);
        Assert.AreEqual(MpvIpcPipeState.Disconnected, runtime.Snapshot.IpcState);
    }

    [TestMethod]
    public async Task WatchHonorsCancellationBeforeWaitingForRuntime()
    {
        await using var runtime = new WindowsMpvPlaybackRuntime();
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();

        var results = new List<MpvPlaybackStateMonitorResult>();
        await foreach (var result in runtime.WatchPlaybackStateAsync(
                           new MediaPlaybackIdentity(4, 5, 0, 0),
                           cancellation.Token))
        {
            results.Add(result);
        }

        Assert.AreEqual(1, results.Count);
        Assert.IsFalse(results[0].IsSuccess);
        Assert.AreEqual(
            MpvPlaybackStateMonitorFailureCode.Cancelled,
            results[0].Error?.Code);
    }

    [TestMethod]
    public async Task PollAfterDisposeReturnsClosedBoundaryWithoutUsingTheGateway()
    {
        var runtime = new WindowsMpvPlaybackRuntime();
        await runtime.DisposeAsync();

        var result = await runtime.PollPlaybackStateAsync(
            new MediaPlaybackIdentity(5, 6, 0, 0));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(
            MpvPlaybackStateMonitorFailureCode.RuntimeUnavailable,
            result.Error?.Code);
        Assert.IsFalse(result.Error?.Retryable);
        Assert.AreEqual(WindowsMpvPlaybackRuntimeState.Closed, runtime.Snapshot.State);
    }

    [TestMethod]
    public async Task ClosedRuntimeRejectsFurtherStarts()
    {
        var runtime = new WindowsMpvPlaybackRuntime();
        await runtime.DisposeAsync();

        var result = await runtime.StartAsync(null, null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsMpvPlaybackRuntimeFailureCode.InvalidBinding, result.Error?.Code);
        Assert.AreEqual(WindowsMpvPlaybackRuntimeState.Closed, result.Snapshot.State);
    }
}
