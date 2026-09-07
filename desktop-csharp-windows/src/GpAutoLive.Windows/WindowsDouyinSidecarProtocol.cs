using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Serialization;
using GpAutoLive.Contracts;

namespace GpAutoLive.Windows;

/// <summary>发给抖音 sidecar 的单条内部发送请求；正文只在当前进程内存中存在。</summary>
public sealed record WindowsDouyinChatSendRequest(
    string SessionId,
    ulong Generation,
    string ClientActionId,
    string Content);

/// <summary>sidecar 对 live.open 的脱敏响应；不返回 canonical room_id 或凭据。</summary>
public sealed record WindowsDouyinLiveOpenResponse(
    string RequestId,
    bool IsSuccess,
    string? SessionId = null,
    string? Title = null,
    string? LiveStatus = null,
    string? ErrorCode = null,
    bool IsRetryable = false,
    DouyinSendOutcome Outcome = DouyinSendOutcome.NotSent);

/// <summary>sidecar 对无业务载荷命令的脱敏响应。</summary>
public sealed record WindowsDouyinCommandResponse(
    string RequestId,
    bool IsSuccess,
    string? ErrorCode = null,
    bool IsRetryable = false,
    DouyinSendOutcome Outcome = DouyinSendOutcome.NotSent);

/// <summary>sidecar 对单条发送请求的脱敏响应；不保留平台错误正文。</summary>
public sealed record WindowsDouyinSidecarResponse(
    string RequestId,
    bool IsSuccess,
    DouyinSendOutcome Outcome,
    string? ClientActionId = null,
    int? PlatformStatusCode = null,
    string? ErrorCode = null,
    bool IsRetryable = false);

/// <summary>Rust 冻结的 NDJSON v1 发送请求/响应边界。</summary>
public static class WindowsDouyinSidecarProtocol
{
    /// <summary>Rust sidecar 对单行 NDJSON 的固定上限。</summary>
    public const int MaxLineBytes = 64 * 1024;
    private static readonly UTF8Encoding Utf8 = new(false, true);
    private static readonly JsonSerializerOptions SerializerOptions = new()
    {
        Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
        WriteIndented = false
    };

    /// <summary>序列化 Rust 冻结的二维码登录启动命令；超时固定为方案约定的 300 秒。</summary>
    public static bool TrySerializeAuthQrStart(
        string? requestId,
        out string line,
        out string? error)
    {
        line = string.Empty;
        error = null;
        if (!TryValidateAsciiId(requestId, out error))
        {
            error ??= "auth.qr.start 请求无效。";
            return false;
        }

        line = JsonSerializer.Serialize(
            new AuthQrStartEnvelope(
                1,
                requestId!,
                "auth.qr.start",
                new AuthQrStartPayload(300_000)),
            SerializerOptions);
        if (!TryGetLineBytes(line, out var bytes) || bytes > MaxLineBytes)
        {
            line = string.Empty;
            error = "auth.qr.start 请求超出 NDJSON 行上限。";
            return false;
        }

        return true;
    }

    /// <summary>序列化取消扫码命令；不携带凭据或登录态。</summary>
    public static bool TrySerializeAuthCancel(
        string? requestId,
        out string line,
        out string? error) => TrySerializeEmptyCommand(requestId, "auth.cancel", out line, out error);

    /// <summary>序列化清除 sidecar 内存登录身份命令；不携带凭据。</summary>
    public static bool TrySerializeAuthLogout(
        string? requestId,
        out string line,
        out string? error) => TrySerializeEmptyCommand(requestId, "auth.logout", out line, out error);

    /// <summary>序列化 sidecar 受管关闭命令。</summary>
    public static bool TrySerializeShutdown(
        string? requestId,
        out string line,
        out string? error) => TrySerializeEmptyCommand(requestId, "shutdown", out line, out error);

    /// <summary>序列化关闭当前直播会话命令。</summary>
    public static bool TrySerializeLiveClose(
        string? requestId,
        string? sessionId,
        ulong generation,
        out string line,
        out string? error)
    {
        line = string.Empty;
        error = null;
        if (!TryValidateAsciiId(requestId, out error)
            || !TryValidateAsciiId(sessionId, out error)
            || generation == 0)
        {
            error ??= "live.close 请求无效。";
            return false;
        }

        line = JsonSerializer.Serialize(
            new LiveCloseEnvelope(
                1,
                requestId!,
                "live.close",
                new LiveClosePayload(sessionId!, generation)),
            SerializerOptions);
        if (!TryGetLineBytes(line, out var bytes) || bytes > MaxLineBytes)
        {
            line = string.Empty;
            error = "live.close 请求超出 NDJSON 行上限。";
            return false;
        }

        return true;
    }

    /// <summary>解析无业务载荷命令响应，并按 request_id 做严格关联。</summary>
    public static bool TryParseCommandResponse(
        string? line,
        string? expectedRequestId,
        out WindowsDouyinCommandResponse? response,
        out string? error)
    {
        response = null;
        error = null;
        if (!TryValidateAsciiId(expectedRequestId, out error)
            || string.IsNullOrWhiteSpace(line)
            || !TryGetLineBytes(line, out var bytes)
            || bytes > MaxLineBytes)
        {
            error ??= "sidecar 命令响应行超出边界。";
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
            var root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !HasOnlyProperties(root, "v", "type", "request_id", "ok", "result", "error")
                || !TryGetInt32(root, "v", out var version)
                || version != 1
                || !TryGetString(root, "type", out var type)
                || !string.Equals(type, "response", StringComparison.Ordinal)
                || !TryGetAsciiId(root, "request_id", out var requestId)
                || !string.Equals(requestId, expectedRequestId, StringComparison.Ordinal)
                || !TryGetBoolean(root, "ok", out var ok))
            {
                error = "sidecar 命令响应缺少合法的 NDJSON v1 包络或 request_id 不匹配。";
                return false;
            }

            if (!ok)
            {
                if (!TryParseFailure(root, requestId, out var failure, out error)
                    || failure is null)
                {
                    return false;
                }

                response = new(
                    requestId,
                    false,
                    failure.ErrorCode,
                    failure.IsRetryable,
                    failure.Outcome);
                return true;
            }

            if (!HasOnlyProperties(root, "v", "type", "request_id", "ok", "result")
                || !root.TryGetProperty("result", out var result)
                || result.ValueKind != JsonValueKind.Object
                || result.EnumerateObject().Any())
            {
                error = "sidecar 命令成功响应必须只包含空 result。";
                return false;
            }

            response = new(requestId, true);
            return true;
        }
        catch (JsonException)
        {
            error = "sidecar 命令响应不是有效 JSON。";
            return false;
        }
    }

    /// <summary>校验并序列化唯一的 chat.send 请求。</summary>
    public static bool TrySerializeChatSend(
        WindowsDouyinChatSendRequest? request,
        out string line,
        out string? error)
    {
        line = string.Empty;
        error = null;
        if (request is null
            || !TryValidateAsciiId(request.SessionId, out error)
            || request.Generation == 0
            || !TryValidateAsciiId(request.ClientActionId, out error)
            || !TryNormalizeContent(request.Content, out var content, out error))
        {
            error ??= "chat.send 请求无效。";
            return false;
        }

        line = JsonSerializer.Serialize(
            new ChatSendEnvelope(
                1,
                request.ClientActionId,
                "chat.send",
                new ChatSendPayload(
                    request.SessionId,
                    request.Generation,
                    request.ClientActionId,
                    content)),
            SerializerOptions);
        if (!TryGetLineBytes(line, out var bytes) || bytes > MaxLineBytes)
        {
            line = string.Empty;
            error = "chat.send 请求超出 NDJSON 行上限。";
            return false;
        }

        return true;
    }

    /// <summary>校验并序列化 Rust 冻结的 live.open 请求。</summary>
    public static bool TrySerializeLiveOpen(
        string? requestId,
        string? webRid,
        ulong generation,
        out string line,
        out string? error)
    {
        line = string.Empty;
        error = null;
        if (!TryValidateAsciiId(requestId, out error)
            || !TryValidateWebRid(webRid, out error)
            || generation == 0)
        {
            error ??= "live.open 请求的 generation 必须为正数。";
            return false;
        }

        line = JsonSerializer.Serialize(
            new LiveOpenEnvelope(
                1,
                requestId!,
                "live.open",
                new LiveOpenPayload(webRid!, generation)),
            SerializerOptions);
        if (!TryGetLineBytes(line, out var bytes) || bytes > MaxLineBytes)
        {
            line = string.Empty;
            error = "live.open 请求超出 NDJSON 行上限。";
            return false;
        }

        return true;
    }

    /// <summary>解析并按 request_id 绑定 Rust 冻结的 live.open 响应。</summary>
    public static bool TryParseLiveOpenResponse(
        string? line,
        string? expectedRequestId,
        out WindowsDouyinLiveOpenResponse? response,
        out string? error)
    {
        response = null;
        error = null;
        if (!TryValidateAsciiId(expectedRequestId, out error)
            || string.IsNullOrWhiteSpace(line)
            || !TryGetLineBytes(line, out var bytes)
            || bytes > MaxLineBytes)
        {
            error ??= "live.open 响应行超出边界。";
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
            var root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !HasOnlyProperties(root, "v", "type", "request_id", "ok", "result", "error")
                || !TryGetInt32(root, "v", out var version)
                || version != 1
                || !TryGetString(root, "type", out var type)
                || !string.Equals(type, "response", StringComparison.Ordinal)
                || !TryGetAsciiId(root, "request_id", out var requestId)
                || !string.Equals(requestId, expectedRequestId, StringComparison.Ordinal)
                || !TryGetBoolean(root, "ok", out var ok))
            {
                error = "live.open 响应缺少合法的 NDJSON v1 包络或 request_id 不匹配。";
                return false;
            }

            if (!ok)
            {
                if (!TryParseFailure(root, requestId, out var failure, out error)
                    || failure is null)
                {
                    return false;
                }

                response = new(
                    requestId,
                    false,
                    ErrorCode: failure.ErrorCode,
                    IsRetryable: failure.IsRetryable,
                    Outcome: failure.Outcome);
                return true;
            }

            if (!HasOnlyProperties(root, "v", "type", "request_id", "ok", "result")
                || !root.TryGetProperty("result", out var result)
                || result.ValueKind != JsonValueKind.Object
                || !HasOnlyProperties(result, "session_id", "title", "live_status")
                || !TryGetAsciiId(result, "session_id", out var sessionId)
                || !TryGetString(result, "title", out var title)
                || title.Any(char.IsControl)
                || title.EnumerateRunes().Count() > 128
                || !TryGetString(result, "live_status", out var liveStatus)
                || !IsLiveOpenStatus(liveStatus))
            {
                error = "live.open 成功响应缺少合法的脱敏房间状态。";
                return false;
            }

            response = new(requestId, true, sessionId, title, liveStatus);
            return true;
        }
        catch (JsonException)
        {
            error = "live.open 响应不是有效 JSON。";
            return false;
        }
    }

    /// <summary>解析并脱敏 chat.send response；未知终态、错误码和正文结构均拒绝。</summary>
    public static bool TryParseResponse(
        string? line,
        out WindowsDouyinSidecarResponse? response,
        out string? error)
    {
        response = null;
        error = null;
        if (string.IsNullOrWhiteSpace(line)
            || !TryGetLineBytes(line, out var bytes)
            || bytes > MaxLineBytes)
        {
            error = "sidecar 响应行超出边界。";
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
            var root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !TryGetInt32(root, "v", out var version)
                || version != 1
                || !TryGetString(root, "type", out var type)
                || !string.Equals(type, "response", StringComparison.Ordinal)
                || !TryGetAsciiId(root, "request_id", out var requestId)
                || !TryGetBoolean(root, "ok", out var ok))
            {
                error = "sidecar 响应缺少合法的 NDJSON v1 包络。";
                return false;
            }

            return ok
                ? TryParseSuccess(root, requestId, out response, out error)
                : TryParseFailure(root, requestId, out response, out error);
        }
        catch (JsonException)
        {
            error = "sidecar 响应不是有效 JSON。";
            return false;
        }
    }

    private static bool TryParseSuccess(
        JsonElement root,
        string requestId,
        out WindowsDouyinSidecarResponse? response,
        out string? error)
    {
        response = null;
        error = null;
        if (!HasOnlyProperties(root, "v", "type", "request_id", "ok", "result")
            || !root.TryGetProperty("result", out var result)
            || result.ValueKind != JsonValueKind.Object
            || !HasOnlyProperties(result, "state", "client_action_id", "platform_status_code")
            || !TryGetOutcome(result, "state", out var outcome)
            || !TryGetAsciiId(result, "client_action_id", out var clientActionId))
        {
            error = "sidecar 成功响应缺少合法发送终态。";
            return false;
        }

        if (!TryGetOptionalStatusCode(result, out var platformStatusCode))
        {
            error = "sidecar 平台状态码无效。";
            return false;
        }

        response = new(requestId, true, outcome, clientActionId, platformStatusCode);
        return true;
    }

    private static bool TryParseFailure(
        JsonElement root,
        string requestId,
        out WindowsDouyinSidecarResponse? response,
        out string? error)
    {
        response = null;
        error = null;
        if (!HasOnlyProperties(root, "v", "type", "request_id", "ok", "error")
            || !root.TryGetProperty("error", out var errorObject)
            || errorObject.ValueKind != JsonValueKind.Object
            || !HasOnlyProperties(errorObject, "code", "message", "retryable", "outcome", "client_action_id")
            || !TryGetAsciiId(errorObject, "code", out var errorCode)
            || !IsStableErrorCode(errorCode)
            || !TryGetString(errorObject, "message", out var message)
            || message.Any(char.IsControl)
            || !TryGetBoolean(errorObject, "retryable", out var retryable)
            || !TryGetOutcome(errorObject, "outcome", out var outcome)
            || outcome == DouyinSendOutcome.Accepted)
        {
            error = "sidecar 失败响应缺少合法的脱敏错误字段。";
            return false;
        }

        string? clientActionId = null;
        if (errorObject.TryGetProperty("client_action_id", out _)
            && !TryGetAsciiId(errorObject, "client_action_id", out clientActionId))
        {
            error = "sidecar 失败响应的 client_action_id 无效。";
            return false;
        }

        response = new(requestId, false, outcome, clientActionId, ErrorCode: errorCode, IsRetryable: retryable);
        return true;
    }

    private static bool HasOnlyProperties(JsonElement root, params string[] allowed)
    {
        foreach (var property in root.EnumerateObject())
        {
            if (!allowed.Contains(property.Name, StringComparer.Ordinal))
            {
                return false;
            }
        }

        return true;
    }

    private static bool TryGetOutcome(
        JsonElement root,
        string name,
        out DouyinSendOutcome outcome)
    {
        outcome = default;
        if (!TryGetString(root, name, out var value))
        {
            return false;
        }

        outcome = value switch
        {
            "accepted" => DouyinSendOutcome.Accepted,
            "not_sent" => DouyinSendOutcome.NotSent,
            "rejected" => DouyinSendOutcome.Rejected,
            "unknown" => DouyinSendOutcome.OutcomeUnknown,
            _ => default
        };
        return value is "accepted" or "not_sent" or "rejected" or "unknown";
    }

    private static bool TryNormalizeContent(
        string? value,
        out string content,
        out string? error)
    {
        content = value?.Trim() ?? string.Empty;
        error = null;
        try
        {
            if (content.Length is 0
                || content.EnumerateRunes().Count() > 100
                || Utf8.GetByteCount(content) > 400
                || content.Any(char.IsControl))
            {
                error = "chat.send 正文必须是 1～100 个可打印 Unicode 字符且不超过 400 UTF-8 字节。";
                return false;
            }
        }
        catch (EncoderFallbackException)
        {
            error = "chat.send 正文必须是 1～100 个可打印 Unicode 字符且不超过 400 UTF-8 字节。";
            return false;
        }

        return true;
    }

    private static bool TryValidateAsciiId(string? value, out string? error)
    {
        error = null;
        if (string.IsNullOrWhiteSpace(value)
            || value.Length > 64
            || !value.All(static character => character is >= (char)0x21 and <= (char)0x7E))
        {
            error = "sidecar ID 必须是 1～64 个可打印 ASCII 字符。";
            return false;
        }

        return true;
    }

    private static bool TryValidateWebRid(string? value, out string? error)
    {
        error = null;
        if (string.IsNullOrWhiteSpace(value)
            || value.Length is > 20
            || value[0] is < '1' or > '9'
            || !value.All(static character => character is >= '0' and <= '9'))
        {
            error = "live.open 的 web_rid 必须是 1～20 位数字。";
            return false;
        }

        return true;
    }

    private static bool IsLiveOpenStatus(string value) => value is
        "connecting"
        or "connected"
        or "reconnecting"
        or "room_ended"
        or "failed";

    private static bool TryGetAsciiId(JsonElement root, string name, out string value)
    {
        value = string.Empty;
        if (!TryGetString(root, name, out value))
        {
            return false;
        }

        return value.Length <= 64
            && value.All(static character => character is >= (char)0x21 and <= (char)0x7E);
    }

    private static bool TryGetString(JsonElement root, string name, out string value)
    {
        value = string.Empty;
        if (!root.TryGetProperty(name, out var property)
            || property.ValueKind != JsonValueKind.String)
        {
            return false;
        }

        value = property.GetString() ?? string.Empty;
        return !string.IsNullOrWhiteSpace(value);
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

        if (property.ValueKind == JsonValueKind.False)
        {
            return true;
        }

        return false;
    }

    private static bool TryGetInt32(JsonElement root, string name, out int value)
    {
        value = 0;
        return root.TryGetProperty(name, out var property) && property.TryGetInt32(out value);
    }

    private static bool TryGetOptionalStatusCode(JsonElement root, out int? value)
    {
        value = null;
        if (!root.TryGetProperty("platform_status_code", out var property))
        {
            return true;
        }

        if (!property.TryGetInt32(out var parsed) || parsed is < 0 or > 9_999)
        {
            return false;
        }

        value = parsed;
        return true;
    }

    private static bool IsStableErrorCode(string value) => value is
        "protocol_invalid"
        or "auth_timeout"
        or "auth_expired"
        or "room_input_invalid"
        or "room_not_live"
        or "room_resolve_failed"
        or "ws_handshake_failed"
        or "ws_protocol_invalid"
        or "risk_controlled"
        or "rate_limited"
        or "send_rejected"
        or "transport_before_dispatch"
        or "transport_after_dispatch"
        or "sidecar_exited";

    private static bool TryGetLineBytes(string value, out int bytes)
    {
        try
        {
            bytes = Utf8.GetByteCount(value);
            return true;
        }
        catch (EncoderFallbackException)
        {
            bytes = 0;
            return false;
        }
    }

    private static bool TrySerializeEmptyCommand(
        string? requestId,
        string operation,
        out string line,
        out string? error)
    {
        line = string.Empty;
        error = null;
        if (!TryValidateAsciiId(requestId, out error))
        {
            error ??= "sidecar 命令请求无效。";
            return false;
        }

        line = JsonSerializer.Serialize(
            new EmptyCommandEnvelope(1, requestId!, operation, new EmptyPayload()),
            SerializerOptions);
        if (!TryGetLineBytes(line, out var bytes) || bytes > MaxLineBytes)
        {
            line = string.Empty;
            error = "sidecar 命令请求超出 NDJSON 行上限。";
            return false;
        }

        return true;
    }

    private sealed record ChatSendEnvelope(
        [property: JsonPropertyName("v")] int Version,
        [property: JsonPropertyName("id")] string Id,
        [property: JsonPropertyName("op")] string Operation,
        [property: JsonPropertyName("payload")] ChatSendPayload Payload);

    private sealed record ChatSendPayload(
        [property: JsonPropertyName("session_id")] string SessionId,
        [property: JsonPropertyName("generation")] ulong Generation,
        [property: JsonPropertyName("client_action_id")] string ClientActionId,
        [property: JsonPropertyName("content")] string Content);

    private sealed record LiveOpenEnvelope(
        [property: JsonPropertyName("v")] int Version,
        [property: JsonPropertyName("id")] string Id,
        [property: JsonPropertyName("op")] string Operation,
        [property: JsonPropertyName("payload")] LiveOpenPayload Payload);

    private sealed record LiveOpenPayload(
        [property: JsonPropertyName("web_rid")] string WebRid,
        [property: JsonPropertyName("generation")] ulong Generation);

    private sealed record AuthQrStartEnvelope(
        [property: JsonPropertyName("v")] int Version,
        [property: JsonPropertyName("id")] string Id,
        [property: JsonPropertyName("op")] string Operation,
        [property: JsonPropertyName("payload")] AuthQrStartPayload Payload);

    private sealed record AuthQrStartPayload(
        [property: JsonPropertyName("timeout_ms")] int TimeoutMilliseconds);

    private sealed record EmptyCommandEnvelope(
        [property: JsonPropertyName("v")] int Version,
        [property: JsonPropertyName("id")] string Id,
        [property: JsonPropertyName("op")] string Operation,
        [property: JsonPropertyName("payload")] EmptyPayload Payload);

    private sealed record EmptyPayload;

    private sealed record LiveCloseEnvelope(
        [property: JsonPropertyName("v")] int Version,
        [property: JsonPropertyName("id")] string Id,
        [property: JsonPropertyName("op")] string Operation,
        [property: JsonPropertyName("payload")] LiveClosePayload Payload);

    private sealed record LiveClosePayload(
        [property: JsonPropertyName("session_id")] string SessionId,
        [property: JsonPropertyName("generation")] ulong Generation);
}
