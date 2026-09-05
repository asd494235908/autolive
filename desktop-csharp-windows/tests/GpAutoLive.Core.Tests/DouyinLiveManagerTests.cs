using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class DouyinLiveManagerTests
{
    [TestMethod]
    public void Lifecycle_requires_qr_login_room_resolution_and_listening()
    {
        var manager = new DouyinLiveManager(new Random(1));
        Assert.IsTrue(manager.TryStart(Config()).IsSuccess);
        Assert.AreEqual(DouyinLiveState.WaitingQr, manager.Snapshot.State);
        Assert.IsTrue(manager.MarkQrIssued().IsSuccess);
        Assert.IsTrue(manager.MarkLoggedIn().IsSuccess);
        Assert.IsTrue(manager.MarkRoomResolved().IsSuccess);
        Assert.IsTrue(manager.BeginListening().IsSuccess);
        Assert.IsTrue(manager.Snapshot.Running);
        Assert.AreEqual("websocket_connected", manager.Snapshot.LastEvent);
    }

    [TestMethod]
    public void Self_replay_and_duplicate_messages_are_filtered_before_queueing()
    {
        var manager = ListeningManager();
        var now = DateTimeOffset.UnixEpoch;

        Assert.AreEqual(DouyinEnqueueDecision.IgnoredSelf, manager.ObserveChatMessage(Message("self", isSelf: true), now).Decision);
        Assert.AreEqual(DouyinEnqueueDecision.IgnoredReplay, manager.ObserveChatMessage(Message("replay", isReplay: true), now).Decision);
        Assert.AreEqual(DouyinEnqueueDecision.Enqueued, manager.ObserveChatMessage(Message("m1"), now).Decision);
        Assert.AreEqual(DouyinEnqueueDecision.IgnoredDuplicate, manager.ObserveChatMessage(Message("m1"), now).Decision);

        var status = manager.Snapshot;
        Assert.AreEqual(1, status.QueueCount);
        Assert.AreEqual(1UL, status.Metrics.IgnoredSelf);
        Assert.AreEqual(1UL, status.Metrics.IgnoredReplay);
        Assert.AreEqual(1UL, status.Metrics.IgnoredDuplicate);
        Assert.IsTrue(status.SelfEchoFiltered);
    }

    [TestMethod]
    public void Queue_is_bounded_and_drops_oldest_unread_task()
    {
        var manager = ListeningManager(Config() with { QueueCapacity = 10 });
        var now = DateTimeOffset.UnixEpoch;
        for (var index = 0; index < 11; index++)
        {
            var result = manager.ObserveChatMessage(Message($"m{index}"), now);
            Assert.IsTrue(result.IsAccepted, result.Error?.Message);
        }

        Assert.AreEqual(10, manager.Snapshot.QueueCount);
        Assert.AreEqual(1UL, manager.Snapshot.Metrics.DroppedOldest);
        Assert.IsTrue(manager.TryDequeue(now, out var first));
        Assert.AreEqual("m1", first!.MessageId);
    }

    [TestMethod]
    public void Tasks_older_than_sixty_seconds_are_expired_without_retry()
    {
        var manager = ListeningManager();
        var start = DateTimeOffset.UnixEpoch;
        Assert.IsTrue(manager.ObserveChatMessage(Message("m1"), start).IsAccepted);

        Assert.IsFalse(manager.TryDequeue(start.AddSeconds(60).AddMilliseconds(1), out _));
        Assert.AreEqual(1UL, manager.Snapshot.Metrics.DroppedExpired);
        Assert.AreEqual(0, manager.Snapshot.QueueCount);
    }

    [TestMethod]
    public void Outcome_unknown_is_terminal_and_not_reenqueued()
    {
        var manager = ListeningManager();
        var now = DateTimeOffset.UnixEpoch;
        Assert.IsTrue(manager.ObserveChatMessage(Message("m1"), now).IsAccepted);
        Assert.IsTrue(manager.TryDequeue(now, out _));
        Assert.IsTrue(manager.RecordSendOutcome(DouyinSendOutcome.OutcomeUnknown).IsSuccess);
        Assert.IsFalse(manager.TryDequeue(now, out _));
        Assert.AreEqual(1UL, manager.Snapshot.Metrics.OutcomeUnknown);
        Assert.IsTrue(manager.Snapshot.ReplyAttempted);
    }

    [TestMethod]
    public void Queue_capacity_is_locked_while_listening_and_can_shrink_when_paused()
    {
        var manager = ListeningManager();
        Assert.IsFalse(manager.SetQueueCapacity(10).IsSuccess);
        Assert.AreEqual("douyin_queue_capacity_locked", manager.SetQueueCapacity(10).Error!.Code);
        Assert.IsTrue(manager.Pause().IsSuccess);
        Assert.IsTrue(manager.SetQueueCapacity(10).IsSuccess);
        Assert.AreEqual(10, manager.Snapshot.QueueCapacity);
        Assert.IsTrue(manager.Resume().IsSuccess);
    }

    [TestMethod]
    public void Stop_clears_memory_queue_and_invalidates_generation()
    {
        var manager = ListeningManager();
        var before = manager.Snapshot.Generation;
        Assert.IsTrue(manager.ObserveChatMessage(Message("m1"), DateTimeOffset.UnixEpoch).IsAccepted);
        Assert.IsTrue(manager.Stop().IsSuccess);

        var status = manager.Snapshot;
        Assert.AreEqual(DouyinLiveState.Idle, status.State);
        Assert.IsFalse(status.Running);
        Assert.AreNotEqual(before, status.Generation);
        Assert.AreEqual(0, status.QueueCount);
        Assert.IsFalse(manager.TryDequeue(DateTimeOffset.UnixEpoch, out _));
    }

    [TestMethod]
    public void Sidecar_observations_project_without_exposing_chat_content()
    {
        var manager = ListeningManager();

        Assert.IsTrue(manager.MarkChatObserved().IsSuccess);
        Assert.IsTrue(manager.MarkReplyAttempted().IsSuccess);
        Assert.IsTrue(manager.MarkSelfEchoFiltered().IsSuccess);
        Assert.IsTrue(manager.MarkPassed().IsSuccess);

        var status = manager.Snapshot;
        Assert.AreEqual(DouyinLiveState.Passed, status.State);
        Assert.IsTrue(status.ChatReceived);
        Assert.IsTrue(status.ReplyAttempted);
        Assert.IsTrue(status.SelfEchoFiltered);
        Assert.AreEqual(0, status.QueueCount);
    }

    [TestMethod]
    public void Sidecar_inconclusive_is_terminal_and_not_success()
    {
        var manager = ListeningManager();

        var result = manager.MarkInconclusive("没有观察到自回显");

        Assert.IsTrue(result.IsSuccess);
        Assert.AreEqual(DouyinLiveState.Inconclusive, result.Snapshot.State);
        Assert.IsFalse(result.Snapshot.Running);
        Assert.AreEqual("没有观察到自回显", result.Snapshot.Error);
    }

    private static DouyinLiveManager ListeningManager(DouyinLiveConfig? config = null)
    {
        var manager = new DouyinLiveManager(new Random(2));
        Assert.IsTrue(manager.TryStart(config ?? Config()).IsSuccess);
        Assert.IsTrue(manager.MarkLoggedIn().IsSuccess);
        Assert.IsTrue(manager.MarkRoomResolved().IsSuccess);
        Assert.IsTrue(manager.BeginListening().IsSuccess);
        return manager;
    }

    private static DouyinLiveConfig Config() => new()
    {
        Enabled = true,
        RoomId = "12345",
        Replies = ["收到", "谢谢"],
        QueueCapacity = DouyinLiveRules.DefaultQueueCapacity
    };

    private static DouyinChatMessage Message(string id, bool isSelf = false, bool isReplay = false) =>
        new(id, "sender", "hello", isSelf, isReplay, DateTimeOffset.UnixEpoch);
}
