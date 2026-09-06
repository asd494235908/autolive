using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Windows;

/// <summary>把脱敏 sidecar 事件映射到 M1 核心状态所有者；正文不离开 sidecar 内存。</summary>
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
            WindowsDouyinProbeEventKind.AuthState => ApplyAuthState(manager, probeEvent),
            WindowsDouyinProbeEventKind.LoginConfirmed => manager.MarkLoggedIn(),
            WindowsDouyinProbeEventKind.RoomResolved => manager.MarkRoomResolved(),
            WindowsDouyinProbeEventKind.WebsocketConnected => manager.BeginListening(),
            WindowsDouyinProbeEventKind.LiveState => ApplyLiveState(manager, probeEvent),
            WindowsDouyinProbeEventKind.LiveGap => ApplyLiveGap(manager, probeEvent),
            WindowsDouyinProbeEventKind.ChatReceived => ApplyChatReceived(manager, probeEvent),
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

    private static DouyinLiveOperationResult ApplyChatReceived(
        DouyinLiveManager manager,
        WindowsDouyinProbeEvent probeEvent)
    {
        if (probeEvent.ChatMetadata is null)
        {
            var failed = manager.Fail("sidecar chat_received 缺少脱敏消息元数据");
            return new(
                false,
                failed.Snapshot,
                new DouyinLiveOperationError(
                    "douyin_message_invalid",
                    "sidecar chat_received 缺少脱敏消息元数据"));
        }

        var result = manager.ObserveChatMetadata(probeEvent.ChatMetadata, DateTimeOffset.UtcNow);
        if (result.Error is null)
        {
            return new(true, result.Snapshot);
        }

        var terminalFailure = manager.Fail("sidecar chat_received 未通过本地元数据校验");
        return new(false, terminalFailure.Snapshot, result.Error);
    }

    private static DouyinLiveOperationResult ApplyAuthState(
        DouyinLiveManager manager,
        WindowsDouyinProbeEvent probeEvent)
    {
        return probeEvent.State switch
        {
            "waiting" or "scanned" => manager.MarkQrIssued(),
            "confirmed" => manager.MarkLoggedIn(),
            "cancelled" or "expired" => manager.Stop(),
            "failed" => manager.Fail("抖音扫码登录失败"),
            _ => manager.Fail("sidecar auth.state 状态无效")
        };
    }

    private static DouyinLiveOperationResult? ApplyLiveState(
        DouyinLiveManager manager,
        WindowsDouyinProbeEvent probeEvent)
    {
        if (string.IsNullOrWhiteSpace(probeEvent.SessionId)
            || probeEvent.Generation is not > 0)
        {
            var failed = manager.Fail("sidecar live.state 缺少会话代际");
            return new(
                false,
                failed.Snapshot,
                new DouyinLiveOperationError(
                    "douyin_protocol_invalid",
                    "sidecar live.state 缺少会话代际"));
        }

        return probeEvent.State switch
        {
            "connected" => manager.BeginListening(),
            "room_ended" => manager.MarkInconclusive("直播间已结束"),
            "auth_expired" => manager.Fail("抖音登录状态已失效"),
            "risk_controlled" => manager.BlockReplySending("抖音 sidecar 进入风控状态"),
            "failed" => manager.Fail("抖音 sidecar 报告直播状态失败"),
            "connecting" or "reconnecting" or "closed" => null,
            _ => manager.Fail("sidecar live.state 状态无效")
        };
    }

    private static DouyinLiveOperationResult ApplyLiveGap(
        DouyinLiveManager manager,
        WindowsDouyinProbeEvent probeEvent)
    {
        if (string.IsNullOrWhiteSpace(probeEvent.SessionId)
            || probeEvent.Generation is not > 0
            || string.IsNullOrWhiteSpace(probeEvent.GapReason)
            || probeEvent.GapDroppedCount is not > 0)
        {
            return manager.RecordLiveGap(null, 0);
        }

        return manager.RecordLiveGap(probeEvent.GapReason, probeEvent.GapDroppedCount.Value);
    }
}
