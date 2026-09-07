using System.Text.Json;
using GpAutoLive.Contracts;

namespace GpAutoLive.Windows;

/// <summary>受管抖音探针允许转发到桌面状态机的事件集合。</summary>
public enum WindowsDouyinProbeEventKind
{
    ProbeStarted,
    QrWaiting,
    QrIssued,
    AuthState,
    LoginConfirmed,
    SelfIdentityReady,
    RoomResolved,
    ReplySelected,
    WebsocketConnected,
    LiveState,
    LiveGap,
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

/// <summary>已脱敏的单行探针事件；弹幕事件只携带固定元数据，不携带正文、Cookie、Token 或异常正文。</summary>
public sealed record WindowsDouyinProbeEvent(
    WindowsDouyinProbeEventKind Kind,
    string Name,
    DouyinChatMessageMetadata? ChatMetadata = null,
    string? SessionId = null,
    ulong? Generation = null,
    string? State = null,
    byte[]? QrPngBytes = null,
    DateTimeOffset? QrExpiresAtUtc = null,
    string? GapReason = null,
    ulong? GapDroppedCount = null);

/// <summary>解析 sidecar stdout 的 JSON 行，并只返回固定事件白名单。</summary>
public static class WindowsDouyinProbeEventParser
{
    /// <summary>单行 UTF-8 上限，避免恶意输出触发无界字符串处理。</summary>
    public const int MaxLineBytes = WindowsDouyinSidecarProtocol.MaxLineBytes;

    /// <summary>解析一条事件行；未知事件或非对象会被拒绝且不暴露原文。</summary>
    public static bool TryParse(
        string? line,
        out WindowsDouyinProbeEvent? probeEvent,
        out string? error,
        string? expectedRoomId = null,
        string? expectedSessionId = null,
        ulong? expectedGeneration = null)
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

            DouyinChatMessageMetadata? chatMetadata = null;
            string? sessionId = null;
            ulong? generation = null;
            string? state = null;
            byte[]? qrPngBytes = null;
            DateTimeOffset? qrExpiresAtUtc = null;
            string? gapReason = null;
            ulong? gapDroppedCount = null;
            if (kind == WindowsDouyinProbeEventKind.ChatReceived
                && !TryParseChatMetadata(
                    document.RootElement,
                    name!,
                    expectedRoomId,
                    expectedSessionId,
                    expectedGeneration,
                    out chatMetadata,
                    out sessionId,
                    out generation,
                    out error))
            {
                return false;
            }

            if (kind == WindowsDouyinProbeEventKind.LiveState
                && !TryParseRustLiveState(
                    document.RootElement,
                    expectedSessionId,
                    expectedGeneration,
                    out sessionId,
                    out generation,
                    out state,
                    out error))
            {
                return false;
            }

            if (kind == WindowsDouyinProbeEventKind.LiveGap
                && !TryParseRustLiveGap(
                    document.RootElement,
                    expectedSessionId,
                    expectedGeneration,
                    out sessionId,
                    out generation,
                    out gapReason,
                    out gapDroppedCount,
                    out error))
            {
                return false;
            }

            if (kind == WindowsDouyinProbeEventKind.QrIssued
                && string.Equals(name, "auth.qr", StringComparison.Ordinal)
                && !TryParseRustAuthQr(
                    document.RootElement,
                    out qrPngBytes,
                    out qrExpiresAtUtc,
                    out error))
            {
                return false;
            }

            if (kind == WindowsDouyinProbeEventKind.AuthState
                && !TryParseRustAuthState(
                    document.RootElement,
                    out state,
                    out error))
            {
                return false;
            }

            probeEvent = new(
                kind,
                name!,
                chatMetadata,
                sessionId,
                generation,
                state,
                qrPngBytes,
                qrExpiresAtUtc,
                gapReason,
                gapDroppedCount);
            return true;
        }
        catch (JsonException)
        {
            error = "sidecar 事件不是有效 JSON。";
            return false;
        }
    }

    private static bool TryParseChatMetadata(
        JsonElement root,
        string eventName,
        string? expectedRoomId,
        string? expectedSessionId,
        ulong? expectedGeneration,
        out DouyinChatMessageMetadata? metadata,
        out string? sessionId,
        out ulong? generation,
        out string? error)
    {
        metadata = null;
        sessionId = null;
        generation = null;
        error = null;
        if (string.Equals(eventName, "live.chat", StringComparison.Ordinal))
        {
            return TryParseRustLiveChatMetadata(
                root,
                expectedRoomId,
                expectedSessionId,
                expectedGeneration,
                out metadata,
                out sessionId,
                out generation,
                out error);
        }

        if (root.TryGetProperty("text", out _)
            || root.TryGetProperty("content", out _)
            || root.TryGetProperty("body", out _))
        {
            error = "chat_received 只允许脱敏元数据，不接受正文。";
            return false;
        }

        if (!TryGetString(root, "message_type", out var messageType)
            || !string.Equals(messageType, "WebcastChatMessage", StringComparison.Ordinal)
            || !TryGetString(root, "room_id", out var roomId)
            || !TryGetString(root, "message_id", out var messageId)
            || !TryGetString(root, "sender_id", out var senderId))
        {
            error = "chat_received 缺少合法的脱敏消息元数据。";
            return false;
        }

        if (!root.TryGetProperty("text_length", out var textLengthProperty)
            || !textLengthProperty.TryGetInt32(out var textLength)
            || textLength is < 0 or > DouyinLiveRules.MaxChatTextLength
            || !TryGetBoolean(root, "is_self", out var isSelf)
            || !TryGetBoolean(root, "is_replay", out var isReplay))
        {
            error = "chat_received 缺少合法的脱敏消息元数据。";
            return false;
        }

        if (!DouyinLiveRules.TryNormalizeRoomId(roomId, out roomId)
            || !DouyinLiveRules.IsValidMessageId(messageId)
            || string.IsNullOrWhiteSpace(senderId)
            || System.Text.Encoding.UTF8.GetByteCount(senderId.Trim()) > DouyinLiveRules.MaxMessageIdBytes)
        {
            error = "chat_received 的房间、消息或发送者标识无效。";
            return false;
        }

        metadata = new(
            roomId,
            messageId.Trim(),
            senderId.Trim(),
            textLength,
            isSelf,
            isReplay);
        return true;
    }

    private static bool TryParseRustLiveChatMetadata(
        JsonElement root,
        string? expectedRoomId,
        string? expectedSessionId,
        ulong? expectedGeneration,
        out DouyinChatMessageMetadata? metadata,
        out string? sessionId,
        out ulong? generation,
        out string? error)
    {
        metadata = null;
        sessionId = null;
        generation = null;
        error = null;
        if (root.TryGetProperty("text", out _)
            || root.TryGetProperty("content", out _)
            || root.TryGetProperty("body", out _)
            || !TryGetInt32(root, "v", out var version)
            || version != 1
            || !TryGetString(root, "type", out var type)
            || !string.Equals(type, "event", StringComparison.Ordinal)
            || !HasOnlyProperties(root, "v", "type", "event", "session_id", "generation", "payload")
            || !TryGetAsciiId(root, "session_id", out var parsedSessionId)
            || !root.TryGetProperty("generation", out var generationProperty)
            || !generationProperty.TryGetUInt64(out var parsedGeneration)
            || parsedGeneration == 0
            || !root.TryGetProperty("payload", out var payload)
            || payload.ValueKind != JsonValueKind.Object
            || !HasOnlyProperties(
                payload,
                "msg_id",
                "author_id",
                "nickname",
                "content",
                "received_at_unix_ms",
                "room_id",
                "is_self",
                "is_replay"))
        {
            error = "live.chat 缺少合法的 NDJSON v1 事件包络。";
            return false;
        }

        if ((expectedSessionId is not null
                && !string.Equals(parsedSessionId, expectedSessionId, StringComparison.Ordinal))
            || (expectedGeneration is not null && parsedGeneration != expectedGeneration.Value))
        {
            error = "live.chat 会话代际与当前宿主不一致。";
            return false;
        }

        sessionId = parsedSessionId;
        generation = parsedGeneration;

        if (!TryGetString(payload, "msg_id", out var messageId)
            || !TryGetString(payload, "author_id", out var senderId)
            || !TryGetString(payload, "nickname", out var nickname)
            || !TryGetString(payload, "content", out var content)
            || !TryGetInt64(payload, "received_at_unix_ms", out var receivedAtUnixMs))
        {
            error = "live.chat 缺少合法的消息元数据。";
            return false;
        }

        if (!DouyinLiveRules.TryNormalizeRoomId(expectedRoomId, out var roomId)
            || !DouyinLiveRules.IsValidMessageId(messageId)
            || string.IsNullOrWhiteSpace(senderId)
            || System.Text.Encoding.UTF8.GetByteCount(senderId.Trim()) > DouyinLiveRules.MaxMessageIdBytes
            || nickname.EnumerateRunes().Count() > 64
            || System.Text.Encoding.UTF8.GetByteCount(nickname) > 256
            || nickname.Any(char.IsControl)
            || content.EnumerateRunes().Count() > DouyinLiveRules.MaxChatTextLength
            || content.Any(char.IsControl))
        {
            error = "live.chat 的房间、消息、昵称或正文长度无效。";
            return false;
        }

        if (payload.TryGetProperty("room_id", out var roomProperty))
        {
            if (roomProperty.ValueKind != JsonValueKind.String
                || !DouyinLiveRules.TryNormalizeRoomId(roomProperty.GetString(), out var payloadRoomId)
                || !string.Equals(payloadRoomId, roomId, StringComparison.Ordinal))
            {
                error = "live.chat 房间与当前会话不一致。";
                return false;
            }
        }

        if (!TryGetOptionalBoolean(payload, "is_self", out var isSelf)
            || !TryGetOptionalBoolean(payload, "is_replay", out var isReplay))
        {
            error = "live.chat 的自回显或重放标志无效。";
            return false;
        }

        try
        {
            _ = DateTimeOffset.FromUnixTimeMilliseconds(receivedAtUnixMs);
        }
        catch (ArgumentOutOfRangeException)
        {
            error = "live.chat 的接收时间无效。";
            return false;
        }

        metadata = new(
            roomId,
            messageId.Trim(),
            senderId.Trim(),
            content.EnumerateRunes().Count(),
            isSelf,
            isReplay);
        return true;
    }

    private static bool TryParseRustLiveState(
        JsonElement root,
        string? expectedSessionId,
        ulong? expectedGeneration,
        out string? sessionId,
        out ulong? generation,
        out string? state,
        out string? error)
    {
        sessionId = null;
        generation = null;
        state = null;
        error = null;
        if (root.TryGetProperty("text", out _)
            || root.TryGetProperty("content", out _)
            || root.TryGetProperty("body", out _)
            || !TryGetInt32(root, "v", out var version)
            || version != 1
            || !TryGetString(root, "type", out var type)
            || !string.Equals(type, "event", StringComparison.Ordinal)
            || !HasOnlyProperties(root, "v", "type", "event", "session_id", "generation", "payload")
            || !TryGetAsciiId(root, "session_id", out var parsedSessionId)
            || !root.TryGetProperty("generation", out var generationProperty)
            || !generationProperty.TryGetUInt64(out var parsedGeneration)
            || parsedGeneration == 0
            || !root.TryGetProperty("payload", out var payload)
            || payload.ValueKind != JsonValueKind.Object
            || !HasOnlyProperties(payload, "state")
            || !TryGetString(payload, "state", out var parsedState)
            || parsedState is not (
                "connecting"
                or "connected"
                or "reconnecting"
                or "room_ended"
                or "auth_expired"
                or "risk_controlled"
                or "closed"
                or "failed"))
        {
            error = "live.state 缺少合法的 NDJSON v1 状态包络。";
            return false;
        }

        if ((expectedSessionId is not null
                && !string.Equals(parsedSessionId, expectedSessionId, StringComparison.Ordinal))
            || (expectedGeneration is not null && parsedGeneration != expectedGeneration.Value))
        {
            error = "live.state 会话代际与当前宿主不一致。";
            return false;
        }

        sessionId = parsedSessionId;
        generation = parsedGeneration;
        state = parsedState;
        return true;
    }

    private static bool TryParseRustLiveGap(
        JsonElement root,
        string? expectedSessionId,
        ulong? expectedGeneration,
        out string? sessionId,
        out ulong? generation,
        out string? reason,
        out ulong? droppedCount,
        out string? error)
    {
        sessionId = null;
        generation = null;
        reason = null;
        droppedCount = null;
        error = null;
        if (!HasOnlyProperties(root, "v", "type", "event", "session_id", "generation", "payload")
            || !TryGetInt32(root, "v", out var version)
            || version != 1
            || !TryGetString(root, "type", out var type)
            || !string.Equals(type, "event", StringComparison.Ordinal)
            || !TryGetString(root, "event", out var eventName)
            || !string.Equals(eventName, "live.gap", StringComparison.Ordinal)
            || !TryGetAsciiId(root, "session_id", out var parsedSessionId)
            || !root.TryGetProperty("generation", out var generationProperty)
            || !generationProperty.TryGetUInt64(out var parsedGeneration)
            || parsedGeneration == 0
            || !root.TryGetProperty("payload", out var payload)
            || payload.ValueKind != JsonValueKind.Object
            || !HasOnlyProperties(payload, "reason", "dropped_count")
            || !TryGetString(payload, "reason", out var parsedReason)
            || parsedReason is not ("sidecar_backpressure" or "reconnect" or "no_replay")
            || !payload.TryGetProperty("dropped_count", out var droppedCountProperty)
            || !droppedCountProperty.TryGetUInt64(out var parsedDroppedCount)
            || parsedDroppedCount == 0)
        {
            error = "live.gap 缺少合法的 NDJSON v1 缺口载荷。";
            return false;
        }

        if ((expectedSessionId is not null
                && !string.Equals(parsedSessionId, expectedSessionId, StringComparison.Ordinal))
            || (expectedGeneration is not null && parsedGeneration != expectedGeneration.Value))
        {
            error = "live.gap 会话代际与当前宿主不一致。";
            return false;
        }

        sessionId = parsedSessionId;
        generation = parsedGeneration;
        reason = parsedReason;
        droppedCount = parsedDroppedCount;
        return true;
    }

    private static bool TryParseRustAuthQr(
        JsonElement root,
        out byte[]? pngBytes,
        out DateTimeOffset? expiresAtUtc,
        out string? error)
    {
        pngBytes = null;
        expiresAtUtc = null;
        error = null;
        if (!HasOnlyProperties(root, "v", "type", "event", "payload")
            || !TryGetInt32(root, "v", out var version)
            || version != 1
            || !TryGetString(root, "type", out var type)
            || !string.Equals(type, "event", StringComparison.Ordinal)
            || !TryGetString(root, "event", out var eventName)
            || !string.Equals(eventName, "auth.qr", StringComparison.Ordinal)
            || !root.TryGetProperty("payload", out var payload)
            || payload.ValueKind != JsonValueKind.Object
            || !HasOnlyProperties(payload, "png_base64", "expires_at_unix_ms")
            || !TryGetString(payload, "png_base64", out var encoded)
            || !encoded.All(IsBase64Character)
            || !TryGetInt64(payload, "expires_at_unix_ms", out var expiresAtUnixMs))
        {
            error = "auth.qr 缺少合法的二维码载荷。";
            return false;
        }

        try
        {
            var decoded = Convert.FromBase64String(encoded);
            if (decoded.Length is 0 or > 256 * 1024 || !IsPng(decoded))
            {
                error = "auth.qr 二维码必须是 256 KiB 以内的 PNG。";
                return false;
            }

            expiresAtUtc = DateTimeOffset.FromUnixTimeMilliseconds(expiresAtUnixMs);
            pngBytes = decoded;
            return true;
        }
        catch (FormatException)
        {
            error = "auth.qr 二维码编码无效。";
            return false;
        }
        catch (ArgumentOutOfRangeException)
        {
            error = "auth.qr 二维码过期时间无效。";
            return false;
        }
    }

    private static bool TryParseRustAuthState(
        JsonElement root,
        out string? state,
        out string? error)
    {
        state = null;
        error = null;
        if (!HasOnlyProperties(root, "v", "type", "event", "payload")
            || !TryGetInt32(root, "v", out var version)
            || version != 1
            || !TryGetString(root, "type", out var type)
            || !string.Equals(type, "event", StringComparison.Ordinal)
            || !TryGetString(root, "event", out var eventName)
            || !string.Equals(eventName, "auth.state", StringComparison.Ordinal)
            || !root.TryGetProperty("payload", out var payload)
            || payload.ValueKind != JsonValueKind.Object
            || !HasOnlyProperties(payload, "state")
            || !TryGetString(payload, "state", out var parsedState)
            || parsedState is not ("waiting" or "scanned" or "confirmed" or "expired" or "cancelled" or "failed"))
        {
            error = "auth.state 缺少合法的登录状态。";
            return false;
        }

        state = parsedState;
        return true;
    }

    private static bool HasOnlyProperties(JsonElement root, params string[] allowed) =>
        root.EnumerateObject().All(property => allowed.Contains(property.Name, StringComparer.Ordinal));

    private static bool IsBase64Character(char value) =>
        value is >= 'A' and <= 'Z'
            or >= 'a' and <= 'z'
            or >= '0' and <= '9'
            or '+' or '/' or '=';

    private static bool IsPng(byte[] bytes) =>
        bytes.Length >= 8
        && bytes[0] == 0x89
        && bytes[1] == 0x50
        && bytes[2] == 0x4E
        && bytes[3] == 0x47
        && bytes[4] == 0x0D
        && bytes[5] == 0x0A
        && bytes[6] == 0x1A
        && bytes[7] == 0x0A;

    private static bool TryGetString(JsonElement root, string name, out string value)
    {
        value = string.Empty;
        if (!root.TryGetProperty(name, out var property)
            || property.ValueKind != JsonValueKind.String)
        {
            return false;
        }

        var parsed = property.GetString();
        if (string.IsNullOrWhiteSpace(parsed))
        {
            return false;
        }

        value = parsed;
        return true;
    }

    private static bool TryGetBoolean(JsonElement root, string name, out bool value)
    {
        value = false;
        if (!root.TryGetProperty(name, out var property))
        {
            return false;
        }

        if (property.ValueKind == JsonValueKind.True)
        {
            value = true;
            return true;
        }

        return property.ValueKind == JsonValueKind.False;
    }

    private static bool TryGetOptionalBoolean(JsonElement root, string name, out bool value)
    {
        value = false;
        return !root.TryGetProperty(name, out _)
            || TryGetBoolean(root, name, out value);
    }

    private static bool TryGetInt32(JsonElement root, string name, out int value)
    {
        value = 0;
        return root.TryGetProperty(name, out var property) && property.TryGetInt32(out value);
    }

    private static bool TryGetInt64(JsonElement root, string name, out long value)
    {
        value = 0;
        return root.TryGetProperty(name, out var property) && property.TryGetInt64(out value);
    }

    private static bool TryGetAsciiId(JsonElement root, string name, out string value)
    {
        if (!TryGetString(root, name, out value))
        {
            return false;
        }

        return value.Length <= 64
            && value.All(static character => character is >= (char)0x21 and <= (char)0x7E);
    }

    private static bool TryMap(string? name, out WindowsDouyinProbeEventKind kind)
    {
        kind = name switch
        {
            "probe_started" => WindowsDouyinProbeEventKind.ProbeStarted,
            "qr_waiting" => WindowsDouyinProbeEventKind.QrWaiting,
            "qr_issued" => WindowsDouyinProbeEventKind.QrIssued,
            "auth.qr" => WindowsDouyinProbeEventKind.QrIssued,
            "auth.state" => WindowsDouyinProbeEventKind.AuthState,
            "login_confirmed" => WindowsDouyinProbeEventKind.LoginConfirmed,
            "self_identity_ready" => WindowsDouyinProbeEventKind.SelfIdentityReady,
            "room_resolved" => WindowsDouyinProbeEventKind.RoomResolved,
            "reply_selected" => WindowsDouyinProbeEventKind.ReplySelected,
            "websocket_connected" => WindowsDouyinProbeEventKind.WebsocketConnected,
            "live.state" => WindowsDouyinProbeEventKind.LiveState,
            "live.gap" => WindowsDouyinProbeEventKind.LiveGap,
            "chat_received" => WindowsDouyinProbeEventKind.ChatReceived,
            "live.chat" => WindowsDouyinProbeEventKind.ChatReceived,
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
            or "auth.qr"
            or "auth.state"
            or "login_confirmed"
            or "self_identity_ready"
            or "room_resolved"
            or "reply_selected"
            or "websocket_connected"
            or "live.state"
            or "live.gap"
            or "chat_received"
            or "live.chat"
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
