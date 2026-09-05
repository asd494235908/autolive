using System.Collections.Immutable;
using System.Globalization;
using System.Text;
using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>桌面端控制面固定产品和会话 audience。</summary>
public static class ControlPlaneContractValues
{
    /// <summary>控制面产品标识。</summary>
    public const string Product = "autolive";
    /// <summary>桌面端会话 audience。</summary>
    public const string DesktopAudience = "desktop";
}

/// <summary>控制面边界的输入上限；客户端校验不能替代服务端校验。</summary>
public static class AuthInputLimits
{
    /// <summary>设备标识最小长度。</summary>
    public const int DeviceIdMinLength = 8;
    /// <summary>设备标识最大长度。</summary>
    public const int DeviceIdMaxLength = 64;
    /// <summary>用户名最小长度。</summary>
    public const int UsernameMinLength = 3;
    /// <summary>用户名最大长度。</summary>
    public const int UsernameMaxLength = 64;
    /// <summary>密码最小长度。</summary>
    public const int PasswordMinLength = 8;
    /// <summary>密码最大长度。</summary>
    public const int PasswordMaxLength = 256;
    /// <summary>设备名称最大长度。</summary>
    public const int DeviceNameMaxLength = 256;
    /// <summary>平台名称最大长度。</summary>
    public const int PlatformMaxLength = 64;
    /// <summary>应用版本最大长度。</summary>
    public const int VersionMaxLength = 64;
    /// <summary>操作系统版本最大长度。</summary>
    public const int OsVersionMaxLength = 128;
    /// <summary>请求 ID 最大长度。</summary>
    public const int RequestIdMaxLength = 128;
    /// <summary>Token 最大长度。</summary>
    public const int TokenMaxLength = 16 * 1024;
    /// <summary>心跳中媒体名称最大长度。</summary>
    public const int HeartbeatMediaNameMaxLength = 256;
    /// <summary>心跳中播放状态最大长度。</summary>
    public const int HeartbeatPlaybackStateMaxLength = 64;
}

/// <summary>跨客户端稳定错误码；显示文案可本地化，代码不可依赖文案。</summary>
public static class AuthErrorCodes
{
    /// <summary>请求参数无效。</summary>
    public const string InvalidRequest = "INVALID_ARGUMENT";
    /// <summary>当前请求未认证。</summary>
    public const string Unauthenticated = "UNAUTHENTICATED";
    /// <summary>账号或设备需要激活。</summary>
    public const string AccountActivationRequired = "ACCOUNT_ACTIVATION_REQUIRED";
    /// <summary>账号或设备激活已过期。</summary>
    public const string AccountActivationExpired = "ACCOUNT_ACTIVATION_EXPIRED";
    /// <summary>设备数量超过上限。</summary>
    public const string DeviceLimitExceeded = "DEVICE_LIMIT_EXCEEDED";
    /// <summary>设备绑定冲突。</summary>
    public const string DeviceBindingConflict = "DEVICE_BINDING_CONFLICT";
    /// <summary>设备已禁用。</summary>
    public const string DeviceDisabled = "DEVICE_DISABLED";
    /// <summary>设备已撤销。</summary>
    public const string DeviceRevoked = "DEVICE_REVOKED";
    /// <summary>需要设备绑定。</summary>
    public const string DeviceBindingRequired = "DEVICE_BINDING_REQUIRED";
    /// <summary>设备不存在。</summary>
    public const string DeviceNotFound = "DEVICE_NOT_FOUND";
    /// <summary>已有认证操作进行中。</summary>
    public const string OperationInProgress = "AUTH_OPERATION_IN_PROGRESS";
    /// <summary>幂等键冲突。</summary>
    public const string IdempotencyConflict = "AUTH_IDEMPOTENCY_CONFLICT";
    /// <summary>重试尚未到达允许时间。</summary>
    public const string RetryNotDue = "AUTH_RETRY_NOT_DUE";
    /// <summary>控制面重试次数已用尽。</summary>
    public const string RetryExhausted = "AUTH_RETRY_EXHAUSTED";
    /// <summary>控制面返回内容无效。</summary>
    public const string ResponseInvalid = "CONTROL_PLANE_RESPONSE_INVALID";
}

/// <summary>控制面稳定错误结构；HTTP 状态由适配器填充，业务错误码不依赖文案。</summary>
public sealed record ControlPlaneErrorDto(
    [property: JsonPropertyName("code")] string Code,
    [property: JsonPropertyName("message")] string Message,
    [property: JsonPropertyName("status")] int Status,
    [property: JsonPropertyName("request_id")] string? RequestId = null,
    [property: JsonPropertyName("retry_after_seconds")] int? RetryAfterSeconds = null)
{
    /// <summary>服务端输入错误的字段级详情；客户端只展示稳定摘要，不记录完整请求体。</summary>
    [JsonPropertyName("details")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingDefault)]
    public ImmutableArray<ControlPlaneErrorDetailDto> Details { get; init; }

    /// <summary>只按稳定传输状态判断是否允许有限重试。</summary>
    [JsonIgnore]
    public bool IsTransient => Status is 0 or 408 or 429 or >= 500;
}

/// <summary>控制面错误的字段级详情。</summary>
public sealed record ControlPlaneErrorDetailDto(
    [property: JsonPropertyName("field")] string Field,
    [property: JsonPropertyName("reason")] string Reason);

/// <summary>桌面账号密码登录请求。密码只应存在于当前调用生命周期。</summary>
public sealed record DesktopLoginRequestDto(
    [property: JsonPropertyName("username")] string Username,
    [property: JsonPropertyName("password")] string Password,
    [property: JsonPropertyName("product")] string Product = ControlPlaneContractValues.Product);

/// <summary>Refresh 请求只携带 Refresh Token，不允许覆盖会话绑定字段。</summary>
public sealed record RefreshTokenRequestDto(
    [property: JsonPropertyName("refresh_token")] string RefreshToken);

/// <summary>Logout 请求。服务端以 Refresh Token 哈希定位会话并幂等撤销。</summary>
public sealed record LogoutRequestDto(
    [property: JsonPropertyName("refresh_token")] string RefreshToken);

/// <summary>控制面返回的 token 集合；Refresh Token 不得投影到 WebView。</summary>
public sealed record SessionTokensDto(
    [property: JsonPropertyName("access_token")] string AccessToken,
    [property: JsonPropertyName("refresh_token")] string RefreshToken,
    [property: JsonPropertyName("expires_at")] string ExpiresAt,
    [property: JsonPropertyName("audience")] string Audience);

/// <summary>服务端返回的最小用户摘要。</summary>
public sealed record UserSummaryDto(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("username")] string Username,
    [property: JsonPropertyName("role")] string Role,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("created_at")] string CreatedAt);

/// <summary>桌面登录响应。</summary>
public sealed record DesktopLoginResponseDto(
    [property: JsonPropertyName("request_id")] string RequestId,
    [property: JsonPropertyName("tokens")] SessionTokensDto Tokens,
    [property: JsonPropertyName("user")] UserSummaryDto User);

/// <summary>Refresh 响应。</summary>
public sealed record RefreshTokenResponseDto(
    [property: JsonPropertyName("request_id")] string RequestId,
    [property: JsonPropertyName("tokens")] SessionTokensDto Tokens);

/// <summary>Logout 响应。无论远端是否已经存在目标会话都可返回成功。</summary>
public sealed record LogoutResponseDto(
    [property: JsonPropertyName("request_id")] string RequestId,
    [property: JsonPropertyName("success")] bool Success);

/// <summary>设备注册/自动激活的请求内容；桌面端不接收激活码明文。</summary>
public sealed record DeviceRegistrationDto(
    [property: JsonPropertyName("product")] string Product,
    [property: JsonPropertyName("device_id")] string DeviceId,
    [property: JsonPropertyName("device_name")] string DeviceName,
    [property: JsonPropertyName("platform")] string Platform,
    [property: JsonPropertyName("app_version")] string AppVersion,
    [property: JsonPropertyName("os_version")] string? OsVersion = null);

/// <summary>设备激活请求。</summary>
public sealed record ActivateDeviceRequestDto(
    [property: JsonPropertyName("device")] DeviceRegistrationDto Device);

/// <summary>设备摘要；只保留管理端和工作台需要的非敏感事实。</summary>
public sealed record DeviceSummaryDto(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("user_id")] string UserId,
    [property: JsonPropertyName("product")] string Product,
    [property: JsonPropertyName("device_name")] string DeviceName,
    [property: JsonPropertyName("platform")] string Platform,
    [property: JsonPropertyName("app_version")] string AppVersion,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("disk_free_bytes")] long? DiskFreeBytes,
    [property: JsonPropertyName("memory_total_bytes")] long? MemoryTotalBytes,
    [property: JsonPropertyName("memory_available_bytes")] long? MemoryAvailableBytes,
    [property: JsonPropertyName("cpu_logical_cores")] int? CpuLogicalCores,
    [property: JsonPropertyName("runtime_os_name")] string? RuntimeOsName,
    [property: JsonPropertyName("runtime_os_version")] string? RuntimeOsVersion,
    [property: JsonPropertyName("kernel_version")] string? KernelVersion,
    [property: JsonPropertyName("current_media_name")] string? CurrentMediaName,
    [property: JsonPropertyName("playback_state")] string? PlaybackState,
    [property: JsonPropertyName("online")] bool Online,
    [property: JsonPropertyName("last_seen_at")] string LastSeenAt,
    [property: JsonPropertyName("activation_expires_at")] string? ActivationExpiresAt);

/// <summary>设备激活响应。</summary>
public sealed record ActivateDeviceResponseDto(
    [property: JsonPropertyName("request_id")] string RequestId,
    [property: JsonPropertyName("device")] DeviceSummaryDto Device);

/// <summary>心跳中允许上报的设备运行摘要。</summary>
public sealed record HeartbeatStatusDto(
    [property: JsonPropertyName("disk_free_bytes")] long DiskFreeBytes,
    [property: JsonPropertyName("memory_total_bytes")] long? MemoryTotalBytes = null,
    [property: JsonPropertyName("memory_available_bytes")] long? MemoryAvailableBytes = null,
    [property: JsonPropertyName("cpu_logical_cores")] int? CpuLogicalCores = null,
    [property: JsonPropertyName("os_name")] string? OsName = null,
    [property: JsonPropertyName("os_version")] string? OsVersion = null,
    [property: JsonPropertyName("kernel_version")] string? KernelVersion = null,
    [property: JsonPropertyName("current_media_name")] string? CurrentMediaName = null,
    [property: JsonPropertyName("playback_state")] string? PlaybackState = null);

/// <summary>心跳请求；通常由唯一控制面所有者串行发出。</summary>
public sealed record HeartbeatRequestDto(
    [property: JsonPropertyName("product")] string Product,
    [property: JsonPropertyName("device_id")] string DeviceId,
    [property: JsonPropertyName("sent_at")] DateTimeOffset SentAt,
    [property: JsonPropertyName("status")] HeartbeatStatusDto Status);

/// <summary>心跳响应。</summary>
public sealed record HeartbeatResponseDto(
    [property: JsonPropertyName("request_id")] string RequestId,
    [property: JsonPropertyName("accepted_at")] string AcceptedAt,
    [property: JsonPropertyName("device_status")] string DeviceStatus);

/// <summary>控制面 DTO 的本地边界校验；服务端仍是最终信任边界。</summary>
public static class AuthContractValidation
{
    /// <summary>验证账号密码登录请求。</summary>
    public static bool TryValidateLogin(DesktopLoginRequestDto? request, out ControlPlaneErrorDto? error)
    {
        if (request is null
            || !LengthBetween(request.Username, AuthInputLimits.UsernameMinLength, AuthInputLimits.UsernameMaxLength)
            || request.Username.Any(char.IsWhiteSpace)
            || request.Username.Contains('\0')
            || !LengthBetween(request.Password, AuthInputLimits.PasswordMinLength, AuthInputLimits.PasswordMaxLength)
            || request.Password.Contains('\0')
            || Encoding.UTF8.GetByteCount(request.Password) > AuthInputLimits.PasswordMaxLength
            || !string.Equals(request.Product, ControlPlaneContractValues.Product, StringComparison.Ordinal))
        {
            error = Invalid("登录请求格式无效");
            return false;
        }

        error = null;
        return true;
    }

    /// <summary>验证设备标识。</summary>
    public static bool TryValidateDeviceId(string? deviceId, out ControlPlaneErrorDto? error)
    {
        if (!LengthBetween(deviceId, AuthInputLimits.DeviceIdMinLength, AuthInputLimits.DeviceIdMaxLength)
            || !char.IsAsciiLetterOrDigit(deviceId![0])
            || deviceId.Any(static c => !char.IsAsciiLetterOrDigit(c) && c is not ('_' or '-')))
        {
            error = Invalid("设备标识格式无效");
            return false;
        }

        error = null;
        return true;
    }

    /// <summary>验证 Refresh Token。</summary>
    public static bool TryValidateRefreshToken(string? token, out ControlPlaneErrorDto? error) =>
        TryValidateToken(token, "刷新凭据格式无效", out error);

    /// <summary>验证退出请求。</summary>
    public static bool TryValidateLogout(LogoutRequestDto? request, out ControlPlaneErrorDto? error) =>
        request is not null && TryValidateRefreshToken(request.RefreshToken, out error)
            ? true
            : SetInvalid(out error, "退出请求格式无效");

    /// <summary>验证设备注册请求。</summary>
    public static bool TryValidateDeviceRegistration(DeviceRegistrationDto? device, out ControlPlaneErrorDto? error)
    {
        if (device is null
            || !string.Equals(device.Product, ControlPlaneContractValues.Product, StringComparison.Ordinal)
            || !TryValidateDeviceId(device.DeviceId, out _)
            || !LengthBetween(device.DeviceName, 1, AuthInputLimits.DeviceNameMaxLength)
            || device.DeviceName.Contains('\0')
            || !LengthBetween(device.Platform, 1, AuthInputLimits.PlatformMaxLength)
            || !LengthBetween(device.AppVersion, 1, AuthInputLimits.VersionMaxLength)
            || (device.OsVersion is not null && !LengthBetween(device.OsVersion, 1, AuthInputLimits.OsVersionMaxLength)))
        {
            error = Invalid("设备注册信息无效");
            return false;
        }

        error = null;
        return true;
    }

    /// <summary>验证心跳请求。</summary>
    public static bool TryValidateHeartbeat(HeartbeatRequestDto? request, out ControlPlaneErrorDto? error)
    {
        if (request is null
            || !string.Equals(request.Product, ControlPlaneContractValues.Product, StringComparison.Ordinal)
            || !TryValidateDeviceId(request.DeviceId, out _)
            || request.SentAt == default
            || request.Status is null
            || request.Status.DiskFreeBytes < 0
            || (request.Status.MemoryTotalBytes is < 0)
            || (request.Status.MemoryAvailableBytes is < 0)
            || (request.Status.CpuLogicalCores is < 0)
            || !OptionalLengthWithin(request.Status.OsName, AuthInputLimits.OsVersionMaxLength)
            || !OptionalLengthWithin(request.Status.OsVersion, AuthInputLimits.OsVersionMaxLength)
            || !OptionalLengthWithin(request.Status.KernelVersion, AuthInputLimits.OsVersionMaxLength)
            || !OptionalLengthWithin(request.Status.CurrentMediaName, AuthInputLimits.HeartbeatMediaNameMaxLength)
            || !OptionalLengthWithin(request.Status.PlaybackState, AuthInputLimits.HeartbeatPlaybackStateMaxLength))
        {
            error = Invalid("心跳请求格式无效");
            return false;
        }

        error = null;
        return true;
    }

    /// <summary>验证控制面返回的会话令牌。</summary>
    public static bool TryValidateSessionTokens(SessionTokensDto? tokens, DateTimeOffset now, out ControlPlaneErrorDto? error)
    {
        if (tokens is null
            || !string.Equals(tokens.Audience, ControlPlaneContractValues.DesktopAudience, StringComparison.Ordinal)
            || !TryValidateToken(tokens.AccessToken, "访问凭据格式无效", out _)
            || !TryValidateToken(tokens.RefreshToken, "刷新凭据格式无效", out _)
            || !DateTimeOffset.TryParse(tokens.ExpiresAt, CultureInfo.InvariantCulture, DateTimeStyles.RoundtripKind, out var accessExpiresAt)
            || accessExpiresAt <= now)
        {
            error = new ControlPlaneErrorDto(AuthErrorCodes.ResponseInvalid, "控制面返回了无效的桌面会话", 502);
            return false;
        }

        error = null;
        return true;
    }

    private static bool TryValidateToken(string? token, string message, out ControlPlaneErrorDto? error)
    {
        if (string.IsNullOrWhiteSpace(token) || token.Length > AuthInputLimits.TokenMaxLength || token.Contains('\0'))
        {
            error = Invalid(message);
            return false;
        }

        error = null;
        return true;
    }

    private static bool LengthBetween(string? value, int minimum, int maximum) =>
        value is not null
        && value.Length >= minimum
        && value.Length <= maximum;

    private static bool OptionalLengthWithin(string? value, int maximum) =>
        value is null || (value.Length <= maximum && !value.Contains('\0'));

    private static ControlPlaneErrorDto Invalid(string message) =>
        new(AuthErrorCodes.InvalidRequest, message, 400);

    private static bool SetInvalid(out ControlPlaneErrorDto? error, string message)
    {
        error = Invalid(message);
        return false;
    }
}
