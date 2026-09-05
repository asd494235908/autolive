using System.Text.Json;

namespace GpAutoLive.Windows;

/// <summary>受管抖音探针允许转发到桌面状态机的事件集合。</summary>
public enum WindowsDouyinProbeEventKind
{
    ProbeStarted,
    QrWaiting,
    QrIssued,
    LoginConfirmed,
    SelfIdentityReady,
    RoomResolved,
    ReplySelected,
    WebsocketConnected,
    ChatReceived,
    ReplyAttempted,
    SelfEchoFiltered,
    ProbePassed,
    ProbeInconclusive,
    ProbeFailed,
    ReplyFailed,
    ProbeCancelled,
    WebsocketError,
    WebsocketClosed
}

/// <summary>已脱敏的单行探针事件；不携带弹幕正文、Cookie、Token 或异常正文。</summary>
public sealed record WindowsDouyinProbeEvent(
    WindowsDouyinProbeEventKind Kind,
    string Name);

/// <summary>解析 sidecar stdout 的 JSON 行，并只返回固定事件白名单。</summary>
public static class WindowsDouyinProbeEventParser
{
    /// <summary>单行 UTF-8 上限，避免恶意输出触发无界字符串处理。</summary>
    public const int MaxLineBytes = 16 * 1024;

    /// <summary>解析一条事件行；未知事件或非对象会被拒绝且不暴露原文。</summary>
    public static bool TryParse(
        string? line,
        out WindowsDouyinProbeEvent? probeEvent,
        out string? error)
    {
        probeEvent = null;
        error = null;
        if (string.IsNullOrWhiteSpace(line)
            || line.Length > MaxLineBytes
            || System.Text.Encoding.UTF8.GetByteCount(line) > MaxLineBytes)
        {
            error = "sidecar 事件行超出边界。";
            return false;
        }

        try
        {
            using var document = JsonDocument.Parse(line, new JsonDocumentOptions
            {
                MaxDepth = 8,
                AllowTrailingCommas = false,
                CommentHandling = JsonCommentHandling.Disallow
            });
            if (document.RootElement.ValueKind != JsonValueKind.Object
                || !document.RootElement.TryGetProperty("event", out var eventProperty)
                || eventProperty.ValueKind != JsonValueKind.String)
            {
                error = "sidecar 事件缺少固定 event 字段。";
                return false;
            }

            var name = eventProperty.GetString();
            if (!TryMap(name, out var kind))
            {
                error = "sidecar 事件不在允许白名单内。";
                return false;
            }

            probeEvent = new(kind, name!);
            return true;
        }
        catch (JsonException)
        {
            error = "sidecar 事件不是有效 JSON。";
            return false;
        }
    }

    private static bool TryMap(string? name, out WindowsDouyinProbeEventKind kind)
    {
        kind = name switch
        {
            "probe_started" => WindowsDouyinProbeEventKind.ProbeStarted,
            "qr_waiting" => WindowsDouyinProbeEventKind.QrWaiting,
            "qr_issued" => WindowsDouyinProbeEventKind.QrIssued,
            "login_confirmed" => WindowsDouyinProbeEventKind.LoginConfirmed,
            "self_identity_ready" => WindowsDouyinProbeEventKind.SelfIdentityReady,
            "room_resolved" => WindowsDouyinProbeEventKind.RoomResolved,
            "reply_selected" => WindowsDouyinProbeEventKind.ReplySelected,
            "websocket_connected" => WindowsDouyinProbeEventKind.WebsocketConnected,
            "chat_received" => WindowsDouyinProbeEventKind.ChatReceived,
            "reply_attempted" => WindowsDouyinProbeEventKind.ReplyAttempted,
            "self_echo_filtered" => WindowsDouyinProbeEventKind.SelfEchoFiltered,
            "probe_passed" => WindowsDouyinProbeEventKind.ProbePassed,
            "probe_inconclusive" => WindowsDouyinProbeEventKind.ProbeInconclusive,
            "probe_failed" => WindowsDouyinProbeEventKind.ProbeFailed,
            "reply_failed" => WindowsDouyinProbeEventKind.ReplyFailed,
            "probe_cancelled" => WindowsDouyinProbeEventKind.ProbeCancelled,
            "websocket_error" => WindowsDouyinProbeEventKind.WebsocketError,
            "websocket_closed" => WindowsDouyinProbeEventKind.WebsocketClosed,
            _ => default
        };

        return name is "probe_started"
            or "qr_waiting"
            or "qr_issued"
            or "login_confirmed"
            or "self_identity_ready"
            or "room_resolved"
            or "reply_selected"
            or "websocket_connected"
            or "chat_received"
            or "reply_attempted"
            or "self_echo_filtered"
            or "probe_passed"
            or "probe_inconclusive"
            or "probe_failed"
            or "reply_failed"
            or "probe_cancelled"
            or "websocket_error"
            or "websocket_closed";
    }
}
