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
                new WindowsDouyinProbeEvent(kind, kind.ToString()));
            Assert.IsTrue(result?.IsSuccess ?? true, $"event {kind} 应成功投影");
        }

        var status = manager.Snapshot;
        Assert.AreEqual(DouyinLiveState.Passed, status.State);
        Assert.IsTrue(status.ChatReceived);
        Assert.IsTrue(status.ReplyAttempted);
        Assert.IsTrue(status.SelfEchoFiltered);
    }
}
