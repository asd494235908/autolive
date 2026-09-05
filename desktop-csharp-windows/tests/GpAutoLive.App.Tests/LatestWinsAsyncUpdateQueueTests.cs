using GpAutoLive.App.Features.Performance;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class LatestWinsAsyncUpdateQueueTests
{
    [TestMethod]
    public void Pending_updates_are_replaced_before_dispatch()
    {
        var scheduled = new Queue<Action>();
        var applied = new List<int>();
        using var updates = new LatestWinsAsyncUpdateQueue(scheduled.Enqueue);

        updates.Post(() =>
        {
            applied.Add(1);
            return Task.CompletedTask;
        });
        updates.Post(() =>
        {
            applied.Add(2);
            return Task.CompletedTask;
        });

        Assert.AreEqual(1, scheduled.Count);
        scheduled.Dequeue()();

        CollectionAssert.AreEqual(new[] { 2 }, applied);
        Assert.AreEqual(0, scheduled.Count);
    }

    [TestMethod]
    public async Task Pending_updates_wait_for_inflight_update_and_keep_only_latest()
    {
        var scheduled = new System.Collections.Concurrent.ConcurrentQueue<Action>();
        var firstStarted = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var releaseFirst = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var secondScheduled = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var applied = new List<int>();
        var scheduleCount = 0;
        using var updates = new LatestWinsAsyncUpdateQueue(action =>
        {
            scheduled.Enqueue(action);
            if (Interlocked.Increment(ref scheduleCount) == 2)
            {
                secondScheduled.TrySetResult(true);
            }
        });

        updates.Post(async () =>
        {
            firstStarted.SetResult(true);
            await releaseFirst.Task.ConfigureAwait(false);
            applied.Add(1);
        });

        Assert.IsTrue(scheduled.TryDequeue(out var first));
        first();
        await firstStarted.Task.ConfigureAwait(false);

        updates.Post(() =>
        {
            applied.Add(2);
            return Task.CompletedTask;
        });
        updates.Post(() =>
        {
            applied.Add(3);
            return Task.CompletedTask;
        });
        releaseFirst.SetResult(true);

        await secondScheduled.Task.ConfigureAwait(false);
        Assert.IsTrue(scheduled.TryDequeue(out var latest));
        latest();

        CollectionAssert.AreEqual(new[] { 1, 3 }, applied);
        Assert.IsFalse(scheduled.TryDequeue(out _));
    }

    [TestMethod]
    public void Dispose_drops_pending_update_and_rejects_new_update()
    {
        var scheduled = new Queue<Action>();
        var applied = false;
        using var updates = new LatestWinsAsyncUpdateQueue(scheduled.Enqueue);

        updates.Post(() =>
        {
            applied = true;
            return Task.CompletedTask;
        });
        updates.Dispose();
        updates.Post(() =>
        {
            applied = true;
            return Task.CompletedTask;
        });

        scheduled.Dequeue()();

        Assert.IsFalse(applied);
        Assert.AreEqual(0, scheduled.Count);
    }
}
