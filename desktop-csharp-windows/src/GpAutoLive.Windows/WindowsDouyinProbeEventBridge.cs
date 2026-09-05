using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Windows;

/// <summary>把脱敏 sidecar 事件映射到 M1 核心状态所有者；不接触事件附加字段。</summary>
public static class WindowsDouyinProbeEventBridge
{
    /// <summary>应用一个固定事件；无需改变核心状态的事件返回 null。</summary>
    public static DouyinLiveOperationResult? Apply(
        DouyinLiveManager manager,
        WindowsDouyinProbeEvent probeEvent)
    {
        ArgumentNullException.ThrowIfNull(manager);
        ArgumentNullException.ThrowIfNull(probeEvent);

        return probeEvent.Kind switch
        {
            WindowsDouyinProbeEventKind.QrIssued => manager.MarkQrIssued(),
            WindowsDouyinProbeEventKind.LoginConfirmed => manager.MarkLoggedIn(),
            WindowsDouyinProbeEventKind.RoomResolved => manager.MarkRoomResolved(),
            WindowsDouyinProbeEventKind.WebsocketConnected => manager.BeginListening(),
            WindowsDouyinProbeEventKind.ChatReceived => manager.MarkChatObserved(),
            WindowsDouyinProbeEventKind.ReplyAttempted => manager.MarkReplyAttempted(),
            WindowsDouyinProbeEventKind.SelfEchoFiltered => manager.MarkSelfEchoFiltered(),
            WindowsDouyinProbeEventKind.ProbePassed => manager.MarkPassed(),
            WindowsDouyinProbeEventKind.ProbeInconclusive => manager.MarkInconclusive(
                "真实弹幕或自回显未在时间预算内观察到"),
            WindowsDouyinProbeEventKind.ProbeFailed
                or WindowsDouyinProbeEventKind.ReplyFailed
                or WindowsDouyinProbeEventKind.ProbeCancelled
                or WindowsDouyinProbeEventKind.WebsocketError => manager.Fail("sidecar 报告失败"),
            _ => null
        };
    }
}
