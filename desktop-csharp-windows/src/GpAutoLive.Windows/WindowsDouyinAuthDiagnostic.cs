using System.Text.Json;

namespace GpAutoLive.Windows;

/// <summary>唯一允许落盘的 sidecar 登录诊断；所有文本均为固定类别。</summary>
public sealed record WindowsDouyinAuthDiagnostic(
    string Stage,
    string Code,
    string? ExceptionType = null,
    int? HttpStatus = null,
    int? PlatformCode = null)
{
    internal bool IsValid => Stage is "auth_start" or "qr_fetch" or "qr_poll" or "login_finalize" or "self_identity"
        && Code is "started" or "qr_issued" or "confirmed" or "cancelled" or "expired" or "http_error"
            or "platform_error" or "invalid_response" or "network_error" or "internal_error" or "scanned"
            or "verification_required" or "rate_limited" or "identity_missing"
        && ExceptionType is null or "timeout" or "connect" or "tls" or "http" or "protocol" or "cancelled" or "other"
        && HttpStatus is null or >= 100 and <= 599;

    internal static bool TryParse(JsonElement root, out WindowsDouyinAuthDiagnostic? diagnostic)
    {
        diagnostic = null;
        if (!HasOnlyUniqueFields(root, ["v", "type", "event", "payload"])
            || !root.TryGetProperty("v", out var version) || version.ValueKind != JsonValueKind.Number || !version.TryGetInt32(out var v) || v != 1
            || !ReadString(root, "type", out var type) || type != "event"
            || !ReadString(root, "event", out var name) || name != "auth.diagnostic"
            || !root.TryGetProperty("payload", out var payload)
            || !HasOnlyUniqueFields(payload, ["stage", "code", "exception_type", "http_status", "platform_code"])
            || !ReadString(payload, "stage", out var stage)
            || !ReadString(payload, "code", out var code)
            || !ReadOptionalString(payload, "exception_type", out var exceptionType)
            || !ReadOptionalNumber(payload, "http_status", out var httpStatus)
            || !ReadOptionalNumber(payload, "platform_code", out var platformCode))
        {
            return false;
        }
        var candidate = new WindowsDouyinAuthDiagnostic(stage!, code!, exceptionType, httpStatus, platformCode);
        if (!candidate.IsValid) return false;
        diagnostic = candidate;
        return true;
    }

    // 仅在普通解析拒绝后分类诊断事件；被拒绝的诊断不得累加聊天协议错误或保存原文。
    internal static bool IsDiagnosticEvent(string line)
    {
        try
        {
            using var document = JsonDocument.Parse(line, new JsonDocumentOptions { MaxDepth = 8 });
            return document.RootElement.ValueKind == JsonValueKind.Object
                && document.RootElement.EnumerateObject().Any(property => property.Name == "event"
                    && property.Value.ValueKind == JsonValueKind.String && property.Value.GetString() == "auth.diagnostic");
        }
        catch (JsonException) { return false; }
    }

    private static bool HasOnlyUniqueFields(JsonElement value, string[] allowed)
    {
        if (value.ValueKind != JsonValueKind.Object) return false;
        var seen = new HashSet<string>(StringComparer.Ordinal);
        return value.EnumerateObject().All(property => allowed.Contains(property.Name, StringComparer.Ordinal) && seen.Add(property.Name));
    }

    private static bool ReadString(JsonElement value, string name, out string? text)
    {
        text = null;
        if (!value.TryGetProperty(name, out var property) || property.ValueKind != JsonValueKind.String) return false;
        text = property.GetString();
        return text is { Length: > 0 and <= 32 };
    }

    private static bool ReadOptionalString(JsonElement value, string name, out string? text)
    {
        text = null;
        return !value.TryGetProperty(name, out _) || ReadString(value, name, out text);
    }

    private static bool ReadOptionalNumber(JsonElement value, string name, out int? number)
    {
        number = null;
        if (!value.TryGetProperty(name, out var property)) return true;
        if (property.ValueKind != JsonValueKind.Number || !property.TryGetInt32(out var result)) return false;
        number = result;
        return true;
    }
}
