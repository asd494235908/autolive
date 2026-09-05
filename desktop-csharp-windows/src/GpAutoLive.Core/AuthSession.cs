using GpAutoLive.Contracts;

namespace GpAutoLive.Core;

#pragma warning disable CS1591

/// <summary>控制面会话状态；只有 Activated 才允许进入工作台。</summary>
public enum AuthSessionState
{
    Unauthenticated,
    Authenticating,
    Authenticated,
    Activated,
    Offline,
    Disabled,
    SigningOut
}

public enum AuthOperationKind
{
    Login,
    Restore,
    Refresh,
    Activation,
    Heartbeat,
    Logout
}

public enum CredentialActionKind
{
    None,
    StoreRefreshToken,
    DeleteRefreshToken
}

/// <summary>仅由控制面所有者短暂消费的 token 集合；快照不暴露 Refresh Token。</summary>
public sealed record AuthTokenSet(
    string AccessToken,
    string RefreshToken,
    DateTimeOffset AccessExpiresAt,
    DateTimeOffset? RefreshExpiresAt,
    string Audience)
{
    /// <summary>把服务端 wire DTO 转为内存模型；无效或过期响应不会进入凭据动作。</summary>
    public static bool TryCreate(SessionTokensDto? tokens, DateTimeOffset now, out AuthTokenSet? result, out ControlPlaneErrorDto? error)
    {
        if (!AuthContractValidation.TryValidateSessionTokens(tokens, now, out error))
        {
            result = null;
            return false;
        }

        if (!DateTimeOffset.TryParse(tokens!.ExpiresAt, System.Globalization.CultureInfo.InvariantCulture, System.Globalization.DateTimeStyles.RoundtripKind, out var accessExpiresAt))
        {
            result = null;
            error = new ControlPlaneErrorDto(AuthErrorCodes.ResponseInvalid, "控制面返回了无效的桌面会话", 502);
            return false;
        }

        result = new AuthTokenSet(tokens.AccessToken, tokens.RefreshToken, accessExpiresAt, null, tokens.Audience);
        error = null;
        return true;
    }

    public bool IsValid(DateTimeOffset now) =>
        string.Equals(Audience, ControlPlaneContractValues.DesktopAudience, StringComparison.Ordinal)
        && IsToken(AccessToken)
        && IsToken(RefreshToken)
        && AccessExpiresAt > now
        && (RefreshExpiresAt is null || RefreshExpiresAt > now);

    public bool CanPersistRefreshToken(DateTimeOffset now) =>
        IsToken(RefreshToken) && (RefreshExpiresAt is null || RefreshExpiresAt > now);

    private static bool IsToken(string? value) =>
        !string.IsNullOrWhiteSpace(value)
        && value.Length <= AuthInputLimits.TokenMaxLength
        && !value.Contains('\0');
}

/// <summary>系统凭据边界的最小动作；不会把 Refresh Token 写入普通状态快照。</summary>
public sealed record CredentialAction(
    CredentialActionKind Kind,
    string? RefreshToken = null,
    DateTimeOffset? ExpiresAt = null);

/// <summary>GUI 可消费的最小会话快照（不含 Refresh Token；Access Token 仅限当前进程内存）。</summary>
public sealed record AuthSessionSnapshot
{
    public static AuthSessionSnapshot Initial { get; } = new();

    public AuthSessionState State { get; init; } = AuthSessionState.Unauthenticated;
    public string? UserId { get; init; }
    public string? DeviceId { get; init; }
    public string? AccessToken { get; init; }
    public DateTimeOffset? AccessExpiresAt { get; init; }
    public DateTimeOffset? ActivationExpiresAt { get; init; }
    public int RetryAttempt { get; init; }
    public DateTimeOffset? NextRetryAt { get; init; }
    public ControlPlaneErrorDto? LastError { get; init; }

    public bool IsAuthenticated =>
        State is AuthSessionState.Authenticated or AuthSessionState.Activated or AuthSessionState.Offline;

    public bool CanEnterWorkbench => State == AuthSessionState.Activated;

    /// <summary>网络中断时允许已放行的本地播放继续，但不会把 Offline 当成新进入门禁。</summary>
    public bool CanContinueLocalPlayback => State is AuthSessionState.Activated or AuthSessionState.Offline;
}

public enum AuthTransitionKind
{
    Accepted,
    Completed,
    Rejected,
    Duplicate,
    RetryNotDue,
    RetryScheduled
}

/// <summary>状态机结果；调用方据此决定是否访问 HTTP 或 Credential Manager。</summary>
public sealed record AuthTransition(
    AuthTransitionKind Kind,
    AuthOperationKind Operation,
    string OperationKey,
    AuthSessionSnapshot Snapshot,
    ControlPlaneErrorDto? Error = null,
    CredentialAction? CredentialAction = null)
{
    public bool IsSuccess => Error is null && Kind is AuthTransitionKind.Accepted or AuthTransitionKind.Completed or AuthTransitionKind.Duplicate;
    public bool ShouldRetry => Kind == AuthTransitionKind.RetryScheduled;
    public bool RemoteLogoutConfirmed { get; init; }
    public bool RequiresRemoteLogoutRetry =>
        Operation == AuthOperationKind.Logout
        && Kind is AuthTransitionKind.Completed or AuthTransitionKind.Duplicate
        && !RemoteLogoutConfirmed;
}

/// <summary>
/// 控制面纯逻辑所有者。它不创建 HttpClient，也不读写凭据；外层适配器只执行返回的边界动作。
/// </summary>
public sealed class AuthSessionMachine
{
    public const int MaxRetryAttempts = 5;
    public const int MaxCompletedOperations = 64;

    private static readonly TimeSpan[] RetryDelays =
    [
        TimeSpan.FromSeconds(1),
        TimeSpan.FromSeconds(2),
        TimeSpan.FromSeconds(4),
        TimeSpan.FromSeconds(8),
        TimeSpan.FromSeconds(15)
    ];

    private readonly Dictionary<string, CachedTransition> _completed = new(StringComparer.Ordinal);
    private readonly Queue<string> _completedOrder = new();
    private readonly int _completedCapacity;
    private PendingOperation? _pending;
    private AuthSessionSnapshot _snapshot = AuthSessionSnapshot.Initial;

    public AuthSessionMachine(int completedOperationCapacity = MaxCompletedOperations)
    {
        if (completedOperationCapacity is < 1 or > MaxCompletedOperations)
        {
            throw new ArgumentOutOfRangeException(nameof(completedOperationCapacity));
        }

        _completedCapacity = completedOperationCapacity;
    }

    public AuthSessionSnapshot Snapshot => _snapshot;

    public AuthTransition BeginLogin(
        string operationKey,
        string deviceId,
        DesktopLoginRequestDto request,
        DateTimeOffset now)
    {
        if (!AuthContractValidation.TryValidateLogin(request, out var error)
            || !AuthContractValidation.TryValidateDeviceId(deviceId, out error))
        {
            return Reject(AuthOperationKind.Login, operationKey, error!);
        }

        var start = Begin(AuthOperationKind.Login, operationKey, now);
        if (start is not null)
        {
            return start;
        }

        _snapshot = AuthSessionSnapshot.Initial with
        {
            State = AuthSessionState.Authenticating,
            DeviceId = deviceId
        };
        _pending = new PendingOperation(AuthOperationKind.Login, operationKey);
        return Accepted(AuthOperationKind.Login, operationKey);
    }

    public AuthTransition CompleteLogin(
        string operationKey,
        AuthTokenSet tokens,
        UserSummaryDto user,
        DateTimeOffset now)
    {
        var duplicate = CompleteStart(AuthOperationKind.Login, operationKey);
        if (duplicate is not null)
        {
            return duplicate;
        }

        if (tokens is null || !tokens.IsValid(now) || user is null)
        {
            var error = Error(AuthErrorCodes.ResponseInvalid, 502, "控制面返回了无效的桌面会话");
            _snapshot = AuthSessionSnapshot.Initial with { LastError = error };
            return CompleteFailure(
                AuthOperationKind.Login,
                operationKey,
                error,
                new CredentialAction(CredentialActionKind.DeleteRefreshToken));
        }

        if (!string.Equals(user.Role, "user", StringComparison.Ordinal)
            || !string.Equals(user.Status, "active", StringComparison.Ordinal))
        {
            var error = Error(AuthErrorCodes.Unauthenticated, 401, "账号或密码错误");
            _snapshot = AuthSessionSnapshot.Initial with { LastError = error };
            return CompleteFailure(
                AuthOperationKind.Login,
                operationKey,
                error,
                new CredentialAction(CredentialActionKind.DeleteRefreshToken));
        }

        _snapshot = _snapshot with
        {
            State = AuthSessionState.Authenticated,
            UserId = user.Id,
            AccessToken = tokens.AccessToken,
            AccessExpiresAt = tokens.AccessExpiresAt,
            ActivationExpiresAt = null,
            RetryAttempt = 0,
            NextRetryAt = null,
            LastError = null
        };
        return CompleteSuccess(
            AuthOperationKind.Login,
            operationKey,
            tokens.CanPersistRefreshToken(now)
                ? new CredentialAction(CredentialActionKind.StoreRefreshToken, tokens.RefreshToken, tokens.RefreshExpiresAt)
                : new CredentialAction(CredentialActionKind.DeleteRefreshToken));
    }

    public AuthTransition FailLogin(string operationKey, ControlPlaneErrorDto error, DateTimeOffset now) =>
        Fail(AuthOperationKind.Login, operationKey, error, now, AuthSessionState.Unauthenticated);

    public AuthTransition BeginRefresh(string operationKey, DateTimeOffset now) =>
        BeginAuthenticatedOperation(AuthOperationKind.Refresh, operationKey, now);

    /// <summary>
    /// 从系统凭据存储恢复 Refresh Token 的临时状态。
    /// 该入口只建立“正在恢复”状态；只有服务端返回有效令牌后才能进入已认证状态。
    /// </summary>
    public AuthTransition BeginRestore(string operationKey, string deviceId, DateTimeOffset now)
    {
        if (!AuthContractValidation.TryValidateDeviceId(deviceId, out var error))
        {
            return Reject(AuthOperationKind.Restore, operationKey, error!);
        }

        var start = Begin(AuthOperationKind.Restore, operationKey, now);
        if (start is not null)
        {
            return start;
        }

        _snapshot = AuthSessionSnapshot.Initial with
        {
            State = AuthSessionState.Authenticating,
            DeviceId = deviceId
        };
        _pending = new PendingOperation(AuthOperationKind.Restore, operationKey);
        return Accepted(AuthOperationKind.Restore, operationKey);
    }

    public AuthTransition CompleteRefresh(string operationKey, AuthTokenSet tokens, DateTimeOffset now)
    {
        var duplicate = CompleteStart(AuthOperationKind.Refresh, operationKey);
        if (duplicate is not null)
        {
            return duplicate;
        }

        if (!tokens.IsValid(now))
        {
            _snapshot = AuthSessionSnapshot.Initial with { LastError = Error(AuthErrorCodes.ResponseInvalid, 502, "控制面返回了无效的桌面会话") };
            return CompleteFailure(
                AuthOperationKind.Refresh,
                operationKey,
                _snapshot.LastError,
                new CredentialAction(CredentialActionKind.DeleteRefreshToken));
        }

        _snapshot = _snapshot with
        {
            State = AuthSessionState.Authenticated,
            AccessToken = tokens.AccessToken,
            AccessExpiresAt = tokens.AccessExpiresAt,
            ActivationExpiresAt = null,
            RetryAttempt = 0,
            NextRetryAt = null,
            LastError = null
        };
        return CompleteSuccess(
            AuthOperationKind.Refresh,
            operationKey,
            tokens.CanPersistRefreshToken(now)
                ? new CredentialAction(CredentialActionKind.StoreRefreshToken, tokens.RefreshToken, tokens.RefreshExpiresAt)
                : new CredentialAction(CredentialActionKind.DeleteRefreshToken));
    }

    public AuthTransition FailRefresh(string operationKey, ControlPlaneErrorDto error, DateTimeOffset now) =>
        Fail(AuthOperationKind.Refresh, operationKey, error, now, _snapshot.IsAuthenticated ? AuthSessionState.Offline : AuthSessionState.Unauthenticated);

    /// <summary>完成冷启动 Refresh Token 恢复；响应仍必须通过完整令牌校验。</summary>
    public AuthTransition CompleteRestore(string operationKey, AuthTokenSet tokens, DateTimeOffset now)
    {
        var duplicate = CompleteStart(AuthOperationKind.Restore, operationKey);
        if (duplicate is not null)
        {
            return duplicate;
        }

        if (tokens is null || !tokens.IsValid(now))
        {
            var error = Error(AuthErrorCodes.ResponseInvalid, 502, "控制面返回了无效的桌面会话");
            _snapshot = AuthSessionSnapshot.Initial with { LastError = error };
            return CompleteFailure(
                AuthOperationKind.Restore,
                operationKey,
                error,
                new CredentialAction(CredentialActionKind.DeleteRefreshToken));
        }

        _snapshot = _snapshot with
        {
            State = AuthSessionState.Authenticated,
            AccessToken = tokens.AccessToken,
            AccessExpiresAt = tokens.AccessExpiresAt,
            ActivationExpiresAt = null,
            RetryAttempt = 0,
            NextRetryAt = null,
            LastError = null
        };
        return CompleteSuccess(
            AuthOperationKind.Restore,
            operationKey,
            tokens.CanPersistRefreshToken(now)
                ? new CredentialAction(CredentialActionKind.StoreRefreshToken, tokens.RefreshToken, tokens.RefreshExpiresAt)
                : new CredentialAction(CredentialActionKind.DeleteRefreshToken));
    }

    /// <summary>恢复网络失败时保持未授权门禁；不把冷启动凭据伪装成 Offline。</summary>
    public AuthTransition FailRestore(string operationKey, ControlPlaneErrorDto error, DateTimeOffset now) =>
        Fail(AuthOperationKind.Restore, operationKey, error, now, AuthSessionState.Unauthenticated);

    /// <summary>取消仍在途的认证操作并清除 pending；取消不删除仍可用的 Refresh Token。</summary>
    public AuthTransition CancelOperation(
        AuthOperationKind operation,
        string operationKey)
    {
        var duplicate = CompleteStart(operation, operationKey);
        if (duplicate is not null)
        {
            return duplicate;
        }

        var error = Error("CONTROL_PLANE_CANCELLED", 499, "控制面操作已取消");
        var state = operation switch
        {
            AuthOperationKind.Login or AuthOperationKind.Restore => AuthSessionState.Unauthenticated,
            AuthOperationKind.Logout => _snapshot.IsAuthenticated
                ? (_snapshot.State == AuthSessionState.Offline ? AuthSessionState.Offline : AuthSessionState.Activated)
                : AuthSessionState.Unauthenticated,
            _ => _snapshot.State
        };
        _snapshot = _snapshot with
        {
            State = state,
            NextRetryAt = null,
            LastError = error
        };
        return CompleteFailure(operation, operationKey, error);
    }

    /// <summary>取消当前仍在途的操作；用于外层把嵌套的激活请求一并收敛。</summary>
    public AuthTransition? CancelPendingOperation()
    {
        return _pending is { } pending
            ? CancelOperation(pending.Kind, pending.Key)
            : null;
    }

    public AuthTransition BeginActivation(string operationKey, DateTimeOffset now) =>
        BeginAuthenticatedOperation(AuthOperationKind.Activation, operationKey, now, requireActivated: false);

    public AuthTransition CompleteActivation(string operationKey, DeviceSummaryDto device, DateTimeOffset now)
    {
        var duplicate = CompleteStart(AuthOperationKind.Activation, operationKey);
        if (duplicate is not null)
        {
            return duplicate;
        }

        if (device is null)
        {
            var error = Error(AuthErrorCodes.ResponseInvalid, 502, "控制面返回了无效的设备摘要");
            _snapshot = _snapshot with { State = AuthSessionState.Unauthenticated, LastError = error };
            return CompleteFailure(AuthOperationKind.Activation, operationKey, error);
        }

        if (!string.Equals(device.Id, _snapshot.DeviceId, StringComparison.Ordinal)
            || !string.Equals(device.Product, ControlPlaneContractValues.Product, StringComparison.Ordinal)
            || string.IsNullOrWhiteSpace(device.UserId)
            || (_snapshot.UserId is not null
                && !string.Equals(device.UserId, _snapshot.UserId, StringComparison.Ordinal)))
        {
            var error = Error(AuthErrorCodes.DeviceBindingConflict, 409, "设备绑定身份与当前会话不一致");
            _snapshot = _snapshot with { State = AuthSessionState.Unauthenticated, LastError = error };
            return CompleteFailure(AuthOperationKind.Activation, operationKey, error);
        }

        if (!string.Equals(device.Status, "active", StringComparison.Ordinal))
        {
            var disabled = string.Equals(device.Status, "disabled", StringComparison.Ordinal)
                || string.Equals(device.Status, "revoked", StringComparison.Ordinal);
            var error = Error(
                disabled
                    ? (string.Equals(device.Status, "revoked", StringComparison.Ordinal)
                        ? AuthErrorCodes.DeviceRevoked
                        : AuthErrorCodes.DeviceDisabled)
                    : AuthErrorCodes.AccountActivationRequired,
                403,
                disabled ? "设备已被禁用" : "当前设备尚未完成激活");
            _snapshot = _snapshot with
            {
                State = disabled ? AuthSessionState.Disabled : AuthSessionState.Unauthenticated,
                LastError = error,
                NextRetryAt = null
            };
            return CompleteFailure(AuthOperationKind.Activation, operationKey, error);
        }

        DateTimeOffset? activationExpiresAt = null;
        if (device.ActivationExpiresAt is not null)
        {
            if (!DateTimeOffset.TryParse(device.ActivationExpiresAt, System.Globalization.CultureInfo.InvariantCulture, System.Globalization.DateTimeStyles.RoundtripKind, out var parsed) || parsed <= now)
            {
                var error = Error(AuthErrorCodes.AccountActivationExpired, 403, "设备激活已过期");
                _snapshot = _snapshot with { State = AuthSessionState.Unauthenticated, LastError = error };
                return CompleteFailure(AuthOperationKind.Activation, operationKey, error);
            }

            activationExpiresAt = parsed;
        }

        _snapshot = _snapshot with
        {
            State = AuthSessionState.Activated,
            // 冷启动 Refresh 响应按服务端契约不携带 UserSummary；首次受保护的
            // 激活响应补齐 user_id，后续心跳仍会继续做设备/账号绑定校验。
            UserId = _snapshot.UserId ?? device.UserId,
            ActivationExpiresAt = activationExpiresAt,
            RetryAttempt = 0,
            NextRetryAt = null,
            LastError = null
        };
        return CompleteSuccess(AuthOperationKind.Activation, operationKey);
    }

    public AuthTransition FailActivation(string operationKey, ControlPlaneErrorDto error, DateTimeOffset now) =>
        Fail(AuthOperationKind.Activation, operationKey, error, now, _snapshot.State == AuthSessionState.Activated ? AuthSessionState.Offline : AuthSessionState.Authenticated);

    public AuthTransition BeginHeartbeat(string operationKey, HeartbeatRequestDto request, DateTimeOffset now)
    {
        if (!AuthContractValidation.TryValidateHeartbeat(request, out var error))
        {
            return Reject(AuthOperationKind.Heartbeat, operationKey, error!);
        }

        if (!string.Equals(request.DeviceId, _snapshot.DeviceId, StringComparison.Ordinal))
        {
            return Reject(AuthOperationKind.Heartbeat, operationKey, Error(AuthErrorCodes.DeviceBindingConflict, 409, "心跳设备与当前会话不一致"));
        }

        return BeginAuthenticatedOperation(AuthOperationKind.Heartbeat, operationKey, now, requireActivated: true);
    }

    public AuthTransition CompleteHeartbeat(string operationKey, string deviceStatus, DateTimeOffset now)
    {
        var duplicate = CompleteStart(AuthOperationKind.Heartbeat, operationKey);
        if (duplicate is not null)
        {
            return duplicate;
        }

        if (!string.Equals(deviceStatus, "active", StringComparison.Ordinal))
        {
            var disabled = string.Equals(deviceStatus, "disabled", StringComparison.Ordinal)
                || string.Equals(deviceStatus, "revoked", StringComparison.Ordinal);
            var error = Error(
                disabled
                    ? (string.Equals(deviceStatus, "revoked", StringComparison.Ordinal)
                        ? AuthErrorCodes.DeviceRevoked
                        : AuthErrorCodes.DeviceDisabled)
                    : AuthErrorCodes.AccountActivationRequired,
                403,
                disabled ? "设备已被禁用" : "当前设备激活状态无效");
            _snapshot = _snapshot with
            {
                State = disabled ? AuthSessionState.Disabled : AuthSessionState.Unauthenticated,
                LastError = error,
                NextRetryAt = null
            };
            return CompleteFailure(AuthOperationKind.Heartbeat, operationKey, error);
        }

        _snapshot = _snapshot with
        {
            State = AuthSessionState.Activated,
            RetryAttempt = 0,
            NextRetryAt = null,
            LastError = null
        };
        return CompleteSuccess(AuthOperationKind.Heartbeat, operationKey);
    }

    public AuthTransition FailHeartbeat(string operationKey, ControlPlaneErrorDto error, DateTimeOffset now) =>
        Fail(AuthOperationKind.Heartbeat, operationKey, error, now, AuthSessionState.Offline);

    public AuthTransition BeginLogout(string operationKey, DateTimeOffset now)
    {
        var start = Begin(AuthOperationKind.Logout, operationKey, now);
        if (start is not null)
        {
            return start;
        }

        _snapshot = _snapshot with { State = AuthSessionState.SigningOut, NextRetryAt = null };
        _pending = new PendingOperation(AuthOperationKind.Logout, operationKey);
        return Accepted(AuthOperationKind.Logout, operationKey);
    }

    public AuthTransition CompleteLogout(string operationKey, bool remoteConfirmed, DateTimeOffset now)
    {
        var duplicate = CompleteStart(AuthOperationKind.Logout, operationKey);
        if (duplicate is not null)
        {
            return duplicate;
        }

        _snapshot = AuthSessionSnapshot.Initial;
        var action = new CredentialAction(CredentialActionKind.DeleteRefreshToken);
        return CompleteSuccess(AuthOperationKind.Logout, operationKey, action, remoteConfirmed);
    }

    public static TimeSpan RetryDelayForAttempt(int attempt)
    {
        if (attempt is < 1 or > MaxRetryAttempts)
        {
            throw new ArgumentOutOfRangeException(nameof(attempt));
        }

        return RetryDelays[attempt - 1];
    }

    private AuthTransition BeginAuthenticatedOperation(
        AuthOperationKind operation,
        string operationKey,
        DateTimeOffset now,
        bool requireActivated = false)
    {
        if (!_snapshot.IsAuthenticated || (requireActivated && !_snapshot.CanEnterWorkbench && _snapshot.State != AuthSessionState.Offline))
        {
            return Reject(operation, operationKey, Error(AuthErrorCodes.Unauthenticated, 401, "当前会话未认证"));
        }

        if (_snapshot.NextRetryAt is { } nextRetryAt && now < nextRetryAt)
        {
            return new AuthTransition(
                AuthTransitionKind.RetryNotDue,
                operation,
                operationKey,
                _snapshot,
                Error(AuthErrorCodes.RetryNotDue, 409, "重试尚未到达允许时间"));
        }

        var start = Begin(operation, operationKey, now);
        if (start is not null)
        {
            return start;
        }

        _pending = new PendingOperation(operation, operationKey);
        return Accepted(operation, operationKey);
    }

    private AuthTransition? Begin(AuthOperationKind operation, string operationKey, DateTimeOffset now)
    {
        if (!IsOperationKeyValid(operationKey))
        {
            return Reject(operation, operationKey, Error(AuthErrorCodes.InvalidRequest, 400, "操作幂等键无效"));
        }

        if (_completed.TryGetValue(operationKey, out var cached))
        {
            return cached.Kind == operation
                ? cached.Transition with { Kind = AuthTransitionKind.Duplicate }
                : Reject(operation, operationKey, Error(AuthErrorCodes.IdempotencyConflict, 409, "操作幂等键已用于其他操作"));
        }

        if (_pending is not null)
        {
            return _pending.Value.Kind == operation && _pending.Value.Key == operationKey
                ? Accepted(operation, operationKey) with { Kind = AuthTransitionKind.Duplicate }
                : Reject(operation, operationKey, Error(AuthErrorCodes.OperationInProgress, 409, "已有控制面操作正在进行"));
        }

        return null;
    }

    private AuthTransition? CompleteStart(AuthOperationKind operation, string operationKey)
    {
        if (_completed.TryGetValue(operationKey, out var cached))
        {
            return cached.Kind == operation
                ? cached.Transition with { Kind = AuthTransitionKind.Duplicate }
                : Reject(operation, operationKey, Error(AuthErrorCodes.IdempotencyConflict, 409, "操作幂等键已用于其他操作"));
        }

        if (_pending is null || _pending.Value.Kind != operation || _pending.Value.Key != operationKey)
        {
            return Reject(operation, operationKey, Error("auth_operation_stale", 409, "控制面操作已过期或顺序无效"));
        }

        _pending = null;
        return null;
    }

    private AuthTransition Fail(
        AuthOperationKind operation,
        string operationKey,
        ControlPlaneErrorDto error,
        DateTimeOffset now,
        AuthSessionState transientState)
    {
        if (error is null)
        {
            error = Error(AuthErrorCodes.ResponseInvalid, 502, "控制面返回了空错误");
        }

        var duplicate = CompleteStart(operation, operationKey);
        if (duplicate is not null)
        {
            return duplicate;
        }

        if (error.IsTransient)
        {
            if (_snapshot.RetryAttempt >= MaxRetryAttempts)
            {
                var exhausted = Error(AuthErrorCodes.RetryExhausted, 503, "控制面重试次数已用尽");
                _snapshot = _snapshot with
                {
                    RetryAttempt = MaxRetryAttempts,
                    NextRetryAt = null,
                    LastError = exhausted
                };
                return CompleteFailure(operation, operationKey, exhausted);
            }

            var attempt = Math.Min(_snapshot.RetryAttempt + 1, MaxRetryAttempts);
            _snapshot = _snapshot with
            {
                State = transientState,
                RetryAttempt = attempt,
                NextRetryAt = now + RetryDelayForAttempt(attempt),
                LastError = error
            };
            return new AuthTransition(AuthTransitionKind.RetryScheduled, operation, operationKey, _snapshot, error);
        }

        var nextState = error.Code is AuthErrorCodes.DeviceDisabled or AuthErrorCodes.DeviceRevoked
            ? AuthSessionState.Disabled
            : AuthSessionState.Unauthenticated;
        _snapshot = _snapshot with { State = nextState, LastError = error, NextRetryAt = null };
        return CompleteFailure(operation, operationKey, error, nextState == AuthSessionState.Unauthenticated
            ? new CredentialAction(CredentialActionKind.DeleteRefreshToken)
            : null);
    }

    private AuthTransition CompleteSuccess(
        AuthOperationKind operation,
        string operationKey,
        CredentialAction? action = null,
        bool remoteLogoutConfirmed = false)
    {
        var transition = new AuthTransition(AuthTransitionKind.Completed, operation, operationKey, _snapshot, CredentialAction: action)
        {
            RemoteLogoutConfirmed = remoteLogoutConfirmed
        };
        Remember(operationKey, operation, transition);
        return transition;
    }

    private AuthTransition CompleteFailure(AuthOperationKind operation, string operationKey, ControlPlaneErrorDto error, CredentialAction? action = null)
    {
        var transition = new AuthTransition(AuthTransitionKind.Rejected, operation, operationKey, _snapshot, error, action);
        if (!error.IsTransient)
        {
            Remember(operationKey, operation, transition);
        }

        return transition;
    }

    private AuthTransition Accepted(AuthOperationKind operation, string operationKey) =>
        new(AuthTransitionKind.Accepted, operation, operationKey, _snapshot);

    private AuthTransition Reject(AuthOperationKind operation, string operationKey, ControlPlaneErrorDto error) =>
        new(AuthTransitionKind.Rejected, operation, operationKey, _snapshot, error);

    private void Remember(string operationKey, AuthOperationKind operation, AuthTransition transition)
    {
        _completed[operationKey] = new CachedTransition(operation, transition);
        _completedOrder.Enqueue(operationKey);
        while (_completed.Count > _completedCapacity && _completedOrder.TryDequeue(out var oldest))
        {
            _completed.Remove(oldest);
        }
    }

    private static bool IsOperationKeyValid(string? value) =>
        !string.IsNullOrWhiteSpace(value)
        && value.Length <= AuthInputLimits.RequestIdMaxLength
        && !value.Contains('\0');

    private static ControlPlaneErrorDto Error(string code, int status, string message) =>
        new(code, message, status);

    private readonly record struct PendingOperation(AuthOperationKind Kind, string Key);
    private sealed record CachedTransition(AuthOperationKind Kind, AuthTransition Transition);
}

#pragma warning restore CS1591
