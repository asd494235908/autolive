using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsRtmpReconnectCoordinatorTests
{
    [TestMethod]
    public async Task Successful_attempt_enters_publishing_and_keeps_bounded_count()
    {
        var observedStates = new List<WindowsRtmpReconnectSnapshot>();
        var coordinator = CreateCoordinator();

        var result = await coordinator.ReconnectAsync((attempt, _) =>
        {
            Assert.AreEqual(1, attempt);
            observedStates.Add(coordinator.Snapshot);
            return Task.FromResult(WindowsRtmpReconnectAttempt.Succeeded());
        });

        Assert.IsTrue(result.IsSuccess);
        Assert.AreEqual(RtmpOutputState.Reconnecting, observedStates[0].State);
        Assert.AreEqual(RtmpOutputState.Publishing, result.Snapshot.State);
        Assert.AreEqual(1, result.Snapshot.RetryCount);
        Assert.IsNull(result.Error);
    }

    [TestMethod]
    public async Task Non_retryable_failure_stops_without_another_attempt()
    {
        var attempts = 0;
        var coordinator = CreateCoordinator();

        var result = await coordinator.ReconnectAsync((_, _) =>
        {
            attempts++;
            return Task.FromResult(
                WindowsRtmpReconnectAttempt.Failed(
                    WindowsRtmpFailureCode.InvalidPlan,
                    retryable: false));
        });

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(1, attempts);
        Assert.AreEqual(RtmpOutputState.Failed, result.Snapshot.State);
        Assert.AreEqual(WindowsRtmpFailureCode.InvalidPlan.ToString(), result.Snapshot.ErrorCode);
        Assert.AreEqual(WindowsRtmpFailureCode.InvalidPlan, result.Error?.Code);
        Assert.IsFalse(result.Error?.Retryable);
    }

    [TestMethod]
    public async Task Retryable_failures_are_bounded_and_end_as_exhausted()
    {
        var attempts = 0;
        var delays = new List<TimeSpan>();
        var coordinator = new WindowsRtmpReconnectCoordinator(
            new WindowsRtmpReconnectPolicy(maxAttempts: 3),
            (delay, _) =>
            {
                delays.Add(delay);
                return Task.CompletedTask;
            });

        var result = await coordinator.ReconnectAsync((_, _) =>
        {
            attempts++;
            return Task.FromResult(
                WindowsRtmpReconnectAttempt.Failed(
                    WindowsRtmpFailureCode.ProcessExited,
                    retryable: true));
        });

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(3, attempts);
        CollectionAssert.AreEqual(
            new[] { TimeSpan.FromMilliseconds(250), TimeSpan.FromMilliseconds(500) },
            delays);
        Assert.AreEqual(RtmpOutputState.Failed, result.Snapshot.State);
        Assert.AreEqual(WindowsRtmpFailureCode.ReconnectExhausted.ToString(), result.Snapshot.ErrorCode);
        Assert.AreEqual(WindowsRtmpFailureCode.ReconnectExhausted, result.Error?.Code);
        Assert.IsFalse(result.Error?.Retryable);
        Assert.IsFalse(result.Snapshot.Error?.Contains("rtmp://", StringComparison.OrdinalIgnoreCase));
    }

    [TestMethod]
    public async Task Cancellation_during_backoff_returns_idle_without_next_attempt()
    {
        var attempts = 0;
        var delayEntered = new TaskCompletionSource<object?>(TaskCreationOptions.RunContinuationsAsynchronously);
        var coordinator = new WindowsRtmpReconnectCoordinator(
            new WindowsRtmpReconnectPolicy(maxAttempts: 3),
            async (_, cancellationToken) =>
            {
                delayEntered.SetResult(null);
                await Task.Delay(Timeout.InfiniteTimeSpan, cancellationToken);
            });
        using var cancellation = new CancellationTokenSource();

        var reconnect = coordinator.ReconnectAsync((_, _) =>
        {
            attempts++;
            return Task.FromResult(
                WindowsRtmpReconnectAttempt.Failed(
                    WindowsRtmpFailureCode.ProcessExited,
                    retryable: true));
        }, cancellation.Token);

        await delayEntered.Task.WaitAsync(TimeSpan.FromSeconds(1));
        cancellation.Cancel();
        var result = await reconnect.WaitAsync(TimeSpan.FromSeconds(1));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(1, attempts);
        Assert.AreEqual(RtmpOutputState.Idle, result.Snapshot.State);
        Assert.AreEqual(WindowsRtmpFailureCode.Cancelled, result.Error?.Code);
        Assert.IsTrue(result.Error?.Retryable);
    }

    [TestMethod]
    public async Task Cancellation_during_attempt_is_not_retried()
    {
        var attempts = 0;
        var attemptEntered = new TaskCompletionSource<object?>(TaskCreationOptions.RunContinuationsAsynchronously);
        using var cancellation = new CancellationTokenSource();
        var coordinator = CreateCoordinator();

        var reconnect = coordinator.ReconnectAsync(async (_, cancellationToken) =>
        {
            attempts++;
            attemptEntered.SetResult(null);
            await Task.Delay(Timeout.InfiniteTimeSpan, cancellationToken);
            return WindowsRtmpReconnectAttempt.Succeeded();
        }, cancellation.Token);

        await attemptEntered.Task.WaitAsync(TimeSpan.FromSeconds(1));
        cancellation.Cancel();
        var result = await reconnect.WaitAsync(TimeSpan.FromSeconds(1));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(1, attempts);
        Assert.AreEqual(RtmpOutputState.Idle, result.Snapshot.State);
        Assert.AreEqual(WindowsRtmpFailureCode.Cancelled, result.Error?.Code);
    }

    [TestMethod]
    public async Task Concurrent_reconnect_is_rejected_without_corrupting_active_state()
    {
        var attemptEntered = new TaskCompletionSource<object?>(TaskCreationOptions.RunContinuationsAsynchronously);
        var releaseAttempt = new TaskCompletionSource<object?>(TaskCreationOptions.RunContinuationsAsynchronously);
        var coordinator = CreateCoordinator();

        var first = coordinator.ReconnectAsync(async (_, _) =>
        {
            attemptEntered.SetResult(null);
            await releaseAttempt.Task;
            return WindowsRtmpReconnectAttempt.Succeeded();
        });
        await attemptEntered.Task.WaitAsync(TimeSpan.FromSeconds(1));

        var second = await coordinator.ReconnectAsync((_, _) =>
            Task.FromResult(WindowsRtmpReconnectAttempt.Succeeded()));

        Assert.IsFalse(second.IsSuccess);
        Assert.AreEqual(WindowsRtmpFailureCode.AlreadyRunning, second.Error?.Code);
        Assert.AreEqual(RtmpOutputState.Reconnecting, coordinator.Snapshot.State);

        releaseAttempt.SetResult(null);
        var firstResult = await first.WaitAsync(TimeSpan.FromSeconds(1));
        Assert.IsTrue(firstResult.IsSuccess);
        Assert.AreEqual(RtmpOutputState.Publishing, coordinator.Snapshot.State);
    }

    [TestMethod]
    public async Task Unexpected_attempt_exception_is_redacted_and_eventually_exhausted()
    {
        var coordinator = CreateCoordinator();

        var result = await coordinator.ReconnectAsync((_, _) =>
            throw new InvalidOperationException("rtmp://secret/path?stream_key=secret"));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsRtmpFailureCode.ReconnectExhausted, result.Error?.Code);
        Assert.IsFalse(result.Snapshot.Error?.Contains("secret", StringComparison.Ordinal));
        Assert.IsFalse(result.Snapshot.Error?.Contains("rtmp://", StringComparison.OrdinalIgnoreCase));
    }

    [TestMethod]
    public async Task Null_attempt_result_fails_closed_and_is_bounded()
    {
        var attempts = 0;
        var coordinator = CreateCoordinator();

        var result = await coordinator.ReconnectAsync((_, _) =>
        {
            attempts++;
            return Task.FromResult<WindowsRtmpReconnectAttempt>(null!);
        });

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(3, attempts);
        Assert.AreEqual(RtmpOutputState.Failed, result.Snapshot.State);
        Assert.AreEqual(WindowsRtmpFailureCode.ReconnectExhausted, result.Error?.Code);
        Assert.IsFalse(result.Snapshot.Error?.Contains("null", StringComparison.OrdinalIgnoreCase));
    }

    [TestMethod]
    public async Task Cancellation_before_first_attempt_does_not_invoke_transport_callback()
    {
        var attempts = 0;
        var coordinator = CreateCoordinator();
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();

        var result = await coordinator.ReconnectAsync((_, _) =>
        {
            attempts++;
            return Task.FromResult(WindowsRtmpReconnectAttempt.Succeeded());
        }, cancellation.Token);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(0, attempts);
        Assert.AreEqual(RtmpOutputState.Idle, result.Snapshot.State);
        Assert.AreEqual(WindowsRtmpFailureCode.Cancelled, result.Error?.Code);
        Assert.IsTrue(result.Error?.Retryable);
    }

    [TestMethod]
    public async Task Cancellation_after_attempt_completion_does_not_publish_success()
    {
        var attempts = 0;
        var coordinator = CreateCoordinator();
        using var cancellation = new CancellationTokenSource();

        var result = await coordinator.ReconnectAsync((_, _) =>
        {
            attempts++;
            cancellation.Cancel();
            return Task.FromResult(WindowsRtmpReconnectAttempt.Succeeded());
        }, cancellation.Token);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(1, attempts);
        Assert.AreEqual(RtmpOutputState.Idle, result.Snapshot.State);
        Assert.AreEqual(WindowsRtmpFailureCode.Cancelled, result.Error?.Code);
    }

    private static WindowsRtmpReconnectCoordinator CreateCoordinator() =>
        new(
            new WindowsRtmpReconnectPolicy(maxAttempts: 3),
            (_, _) => Task.CompletedTask);
}
