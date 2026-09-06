using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsDouyinProbeEventBridgeTests
{
    [TestMethod]
    public void Redacted_probe_events_follow_the_m1_lifecycle()
    {
        var manager = new DouyinLiveManager();
        Assert.IsTrue(manager.TryStart(new DouyinLiveConfig
        {
            Enabled = true,
            RoomId = "12345",
            Replies = ["收到"],
            QueueCapacity = DouyinLiveRules.DefaultQueueCapacity
        }).IsSuccess);

        foreach (var kind in new[]
        {
            WindowsDouyinProbeEventKind.QrIssued,
            WindowsDouyinProbeEventKind.LoginConfirmed,
            WindowsDouyinProbeEventKind.RoomResolved,
            WindowsDouyinProbeEventKind.WebsocketConnected,
            WindowsDouyinProbeEventKind.ChatReceived,
            WindowsDouyinProbeEventKind.ReplyAttempted,
            WindowsDouyinProbeEventKind.SelfEchoFiltered,
            WindowsDouyinProbeEventKind.ProbePassed
        })
        {
            var result = WindowsDouyinProbeEventBridge.Apply(
                manager,
                kind == WindowsDouyinProbeEventKind.ChatReceived
                    ? new WindowsDouyinProbeEvent(
                        kind,
                        kind.ToString(),
                        new DouyinChatMessageMetadata(
                            "12345",
                            "message-1",
                            "sender-1",
                            8,
                            false,
                            false))
                    : new WindowsDouyinProbeEvent(kind, kind.ToString()));
            Assert.IsTrue(result?.IsSuccess ?? true, $"event {kind} 应成功投影");
        }

        var status = manager.Snapshot;
        Assert.AreEqual(DouyinLiveState.Passed, status.State);
        Assert.IsTrue(status.ChatReceived);
        Assert.IsTrue(status.ReplyAttempted);
        Assert.IsTrue(status.SelfEchoFiltered);
        Assert.AreEqual(1UL, status.Metrics.Enqueued);
    }

    [TestMethod]
    public void Chat_event_without_metadata_fails_closed()
    {
        var manager = new DouyinLiveManager();
        Assert.IsTrue(manager.TryStart(new DouyinLiveConfig
        {
            Enabled = true,
            RoomId = "12345",
            Replies = ["收到"],
            QueueCapacity = DouyinLiveRules.DefaultQueueCapacity
        }).IsSuccess);

        var result = WindowsDouyinProbeEventBridge.Apply(
            manager,
            new WindowsDouyinProbeEvent(
                WindowsDouyinProbeEventKind.ChatReceived,
                "chat_received"));

        Assert.IsFalse(result!.IsSuccess);
        Assert.AreEqual(DouyinLiveState.Failed, manager.Snapshot.State);
    }

    [TestMethod]
    public void Canonical_live_state_projects_connected_and_terminal_states()
    {
        var manager = new DouyinLiveManager();
        Assert.IsTrue(manager.TryStart(new DouyinLiveConfig
        {
            Enabled = true,
            RoomId = "12345",
            Replies = ["收到"],
            QueueCapacity = DouyinLiveRules.DefaultQueueCapacity
        }).IsSuccess);
        Assert.IsTrue(manager.MarkLoggedIn().IsSuccess);
        Assert.IsTrue(manager.MarkRoomResolved().IsSuccess);

        var connecting = WindowsDouyinProbeEventBridge.Apply(
            manager,
                new WindowsDouyinProbeEvent(
                    WindowsDouyinProbeEventKind.LiveState,
                    "live.state",
                    SessionId: "ls-1",
                    Generation: 1,
                    State: "connecting"));
        Assert.IsNull(connecting);

        var connected = WindowsDouyinProbeEventBridge.Apply(
            manager,
                new WindowsDouyinProbeEvent(
                    WindowsDouyinProbeEventKind.LiveState,
                    "live.state",
                    SessionId: "ls-1",
                    Generation: 1,
                    State: "connected"));
        Assert.IsTrue(connected?.IsSuccess);
        Assert.AreEqual(DouyinLiveState.Listening, manager.Snapshot.State);

        var ended = WindowsDouyinProbeEventBridge.Apply(
            manager,
                new WindowsDouyinProbeEvent(
                    WindowsDouyinProbeEventKind.LiveState,
                    "live.state",
                    SessionId: "ls-1",
                    Generation: 1,
                    State: "room_ended"));
        Assert.IsTrue(ended?.IsSuccess);
        Assert.AreEqual(DouyinLiveState.Inconclusive, manager.Snapshot.State);
    }

    [TestMethod]
    public void Canonical_live_state_without_identity_fails_closed()
    {
        var manager = new DouyinLiveManager();
        Assert.IsTrue(manager.TryStart(new DouyinLiveConfig
        {
            Enabled = true,
            RoomId = "12345",
            Replies = ["收到"],
            QueueCapacity = DouyinLiveRules.DefaultQueueCapacity
        }).IsSuccess);

        var result = WindowsDouyinProbeEventBridge.Apply(
            manager,
            new WindowsDouyinProbeEvent(
                WindowsDouyinProbeEventKind.LiveState,
                "live.state",
                State: "connecting"));

        Assert.IsFalse(result!.IsSuccess);
        Assert.AreEqual(DouyinLiveState.Failed, manager.Snapshot.State);
    }

    [TestMethod]
    public void Canonical_live_gap_is_projected_as_visible_loss_without_terminal_failure()
    {
        var manager = new DouyinLiveManager();
        Assert.IsTrue(manager.TryStart(new DouyinLiveConfig
        {
            Enabled = true,
            RoomId = "12345",
            Replies = ["收到"],
            QueueCapacity = DouyinLiveRules.DefaultQueueCapacity
        }).IsSuccess);
        Assert.IsTrue(manager.MarkLoggedIn().IsSuccess);
        Assert.IsTrue(manager.MarkRoomResolved().IsSuccess);
        Assert.IsTrue(manager.BeginListening().IsSuccess);

        var result = WindowsDouyinProbeEventBridge.Apply(
            manager,
            new WindowsDouyinProbeEvent(
                WindowsDouyinProbeEventKind.LiveGap,
                "live.gap",
                SessionId: "ls-1",
                Generation: 1,
                GapReason: "sidecar_backpressure",
                GapDroppedCount: 2));

        Assert.IsTrue(result?.IsSuccess, result?.Error?.Message);
        Assert.AreEqual(DouyinLiveState.Listening, result!.Snapshot.State);
        Assert.AreEqual(1UL, result.Snapshot.Metrics.GapEvents);
        Assert.AreEqual(2UL, result.Snapshot.Metrics.GapDroppedCount);
        Assert.AreEqual("sidecar_backpressure", result.Snapshot.LastGapReason);
    }

    [TestMethod]
    public void Canonical_auth_state_projects_waiting_and_confirmed_lifecycle()
    {
        var manager = new DouyinLiveManager();
        Assert.IsTrue(manager.TryStart(new DouyinLiveConfig
        {
            Enabled = true,
            RoomId = "12345",
            Replies = ["收到"],
            QueueCapacity = DouyinLiveRules.DefaultQueueCapacity
        }).IsSuccess);

        Assert.IsTrue(WindowsDouyinProbeEventBridge.Apply(
            manager,
            new WindowsDouyinProbeEvent(
                WindowsDouyinProbeEventKind.AuthState,
                "auth.state",
                State: "waiting"))?.IsSuccess);
        Assert.IsTrue(WindowsDouyinProbeEventBridge.Apply(
            manager,
            new WindowsDouyinProbeEvent(
                WindowsDouyinProbeEventKind.AuthState,
                "auth.state",
                State: "confirmed"))?.IsSuccess);
        Assert.AreEqual(DouyinLiveState.LoggedIn, manager.Snapshot.State);
    }

    [TestMethod]
    public void Canonical_risk_state_blocks_new_replies_but_keeps_reading_session()
    {
        var manager = new DouyinLiveManager();
        Assert.IsTrue(manager.TryStart(new DouyinLiveConfig
        {
            Enabled = true,
            RoomId = "12345",
            Replies = ["收到"],
            QueueCapacity = DouyinLiveRules.DefaultQueueCapacity
        }).IsSuccess);
        Assert.IsTrue(manager.MarkLoggedIn().IsSuccess);
        Assert.IsTrue(manager.MarkRoomResolved().IsSuccess);
        Assert.IsTrue(manager.BeginListening().IsSuccess);

        var result = WindowsDouyinProbeEventBridge.Apply(
            manager,
            new WindowsDouyinProbeEvent(
                WindowsDouyinProbeEventKind.LiveState,
                "live.state",
                SessionId: "ls-1",
                Generation: 1,
                State: "risk_controlled"));

        Assert.IsTrue(result?.IsSuccess);
        Assert.AreEqual(DouyinLiveState.Listening, manager.Snapshot.State);
        Assert.IsTrue(manager.Snapshot.ReplySendingBlocked);
        Assert.IsTrue(manager.ObserveChatMetadata(new DouyinChatMessageMetadata(
            "12345",
            "after-risk",
            "sender-1",
            4,
            false,
            false),
            DateTimeOffset.UtcNow).IsAccepted);
    }
}
