using System.Security.Cryptography;
using System.Text;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Security;

namespace GpAutoLive.Windows;

/// <summary>
/// 控制面认证的唯一桌面所有者：串行驱动 Core 会话状态机、HTTP 传输和 Windows 凭据动作。
/// 不把 Refresh Token 放入快照、日志或普通配置。
/// </summary>
public sealed class ControlPlaneAuthCoordinator : IDisposable
{
    public const string RefreshTokenCredentialName = "control-plane-refresh-token";

    private readonly ControlPlaneHttpClient _client;
    private readonly ISecretStore _secretStore;
    private readonly DeviceRegistrationDto _deviceRegistration;
    private readonly AuthSessionMachine _session = new();
    private readonly SemaphoreSlim _serial = new(1, 1);
    private readonly IDisposable? _ownedClient;
    private readonly IControlPlaneClock _clock;
    private AuthTokenSet? _tokens;
    private bool _disposed;

    public ControlPlaneAuthCoordinator(
        ControlPlaneHttpClient client,
        ISecretStore secretStore,
        DeviceRegistrationDto deviceRegistration,
        IDisposable? ownedClient = null,
        IControlPlaneClock? clock = null)
    {
        _client = client ?? throw new ArgumentNullException(nameof(client));
        _secretStore = secretStore ?? throw new ArgumentNullException(nameof(secretStore));
        _deviceRegistration = deviceRegistration ?? throw new ArgumentNullException(nameof(deviceRegistration));
        if (!AuthContractValidation.TryValidateDeviceRegistration(_deviceRegistration, out var error))
        {
            throw new ArgumentException(error?.Message ?? "设备注册信息无效。", nameof(deviceRegistration));
        }

        _ownedClient = ownedClient;
        _clock = clock ?? SystemControlPlaneClock.Instance;
    }

    public AuthSessionSnapshot Snapshot => _session.Snapshot;

    public DeviceRegistrationDto DeviceRegistration => _deviceRegistration;

    public async Task<AuthTransition> LoginAsync(
        string username,
        string password,
        CancellationToken cancellationToken = default)
    {
        var entered = false;
        string? operationKey = null;
        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
            entered = true;
            ThrowIfDisposed();
            var now = _clock.UtcNow;
            operationKey = CreateOperationKey();
            var request = new DesktopLoginRequestDto(username, password);
            var begin = _session.BeginLogin(operationKey, _deviceRegistration.DeviceId, request, now);
            if (begin.Kind != AuthTransitionKind.Accepted)
            {
                return begin;
            }

            _tokens = null;
            var result = await _client.LoginAsync(request, cancellationToken).ConfigureAwait(false);
            if (!result.IsSuccess || result.Value is null)
            {
                return FinalizeFailure(_session.FailLogin(
                    operationKey,
                    ToContractError(result.Error, "登录请求失败"),
                    _clock.UtcNow));
            }

            if (!AuthTokenSet.TryCreate(result.Value.Tokens, _clock.UtcNow, out var tokens, out var tokenError))
            {
                return FinalizeFailure(_session.FailLogin(operationKey, tokenError!, _clock.UtcNow));
            }

            var completed = _session.CompleteLogin(operationKey, tokens!, result.Value.User, _clock.UtcNow);
            if (completed.Kind != AuthTransitionKind.Completed)
            {
                return FinalizeFailure(completed);
            }

            _tokens = tokens;
            ApplyCredentialAction(completed.CredentialAction);
            return await ActivateCoreAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return operationKey is null
                ? CreateRejected(AuthOperationKind.Login, "登录已取消", 499, "CONTROL_PLANE_CANCELLED")
                : _session.CancelPendingOperation()
                    ?? CreateRejected(AuthOperationKind.Login, "登录已取消", 499, "CONTROL_PLANE_CANCELLED");
        }
        catch (Exception exception) when (IsCredentialStoreFailure(exception))
        {
            return HandleUnexpectedFailure(
                AuthOperationKind.Login,
                operationKey,
                "登录未完成，安全凭据存储不可用。",
                "AUTH_CREDENTIAL_STORAGE_UNAVAILABLE",
                clearPersistedCredential: true);
        }
        catch (Exception)
        {
            return HandleUnexpectedFailure(
                AuthOperationKind.Login,
                operationKey,
                "控制面登录未完成，请重试。",
                "CONTROL_PLANE_CLIENT_FAILURE");
        }
        finally
        {
            if (entered)
            {
                _serial.Release();
            }
        }
    }

    public async Task<AuthTransition> RefreshAsync(CancellationToken cancellationToken = default)
    {
        var entered = false;
        string? operationKey = null;
        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
            entered = true;
            ThrowIfDisposed();
            operationKey = CreateOperationKey();
            var begin = _session.BeginRefresh(operationKey, _clock.UtcNow);
            if (begin.Kind != AuthTransitionKind.Accepted)
            {
                return begin;
            }

            if (_tokens is null)
            {
                return FinalizeFailure(_session.FailRefresh(
                    operationKey,
                    ToContractError(null, "当前进程没有可用的刷新凭据"),
                    _clock.UtcNow));
            }

            var result = await _client.RefreshAsync(
                new RefreshTokenRequestDto(_tokens.RefreshToken),
                cancellationToken).ConfigureAwait(false);
            if (!result.IsSuccess || result.Value is null)
            {
                return FinalizeFailure(_session.FailRefresh(
                    operationKey,
                    ToContractError(result.Error, "刷新会话失败"),
                    _clock.UtcNow));
            }

            if (!AuthTokenSet.TryCreate(result.Value.Tokens, _clock.UtcNow, out var tokens, out var tokenError))
            {
                return FinalizeFailure(_session.FailRefresh(operationKey, tokenError!, _clock.UtcNow));
            }

            var completed = _session.CompleteRefresh(operationKey, tokens!, _clock.UtcNow);
            if (completed.Kind != AuthTransitionKind.Completed)
            {
                return FinalizeFailure(completed);
            }

            _tokens = tokens;
            ApplyCredentialAction(completed.CredentialAction);
            return await ActivateCoreAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return operationKey is null
                ? CreateRejected(AuthOperationKind.Refresh, "刷新已取消", 499, "CONTROL_PLANE_CANCELLED")
                : _session.CancelPendingOperation()
                    ?? CreateRejected(AuthOperationKind.Refresh, "刷新已取消", 499, "CONTROL_PLANE_CANCELLED");
        }
        catch (Exception exception) when (IsCredentialStoreFailure(exception))
        {
            return HandleUnexpectedFailure(
                AuthOperationKind.Refresh,
                operationKey,
                "刷新未完成，安全凭据存储不可用。",
                "AUTH_CREDENTIAL_STORAGE_UNAVAILABLE",
                clearPersistedCredential: true);
        }
        catch (Exception)
        {
            return HandleUnexpectedFailure(
                AuthOperationKind.Refresh,
                operationKey,
                "控制面刷新未完成，请重试。",
                "CONTROL_PLANE_CLIENT_FAILURE");
        }
        finally
        {
            if (entered)
            {
                _serial.Release();
            }
        }
    }

    /// <summary>
    /// 从 Windows Credential Manager 恢复会话。缺少凭据返回 null；无效或被拒绝的凭据
    /// 只会 fail-closed 并触发删除动作，不会把 Token 投影到 UI、日志或普通配置。
    /// </summary>
    public async Task<AuthTransition?> RestoreAsync(CancellationToken cancellationToken = default)
    {
        var entered = false;
        string? operationKey = null;
        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
            entered = true;
            ThrowIfDisposed();
            if (!_secretStore.TryGet(RefreshTokenCredentialName, out var secret))
            {
                return null;
            }

            using (secret)
            {
                if (!TryReadSecretToken(secret, out var refreshToken))
                {
                    TryDeletePersistedRefreshToken();
                    return CreateRejected(
                        AuthOperationKind.Restore,
                        "本机保存的会话凭据无效，已清理。",
                        401,
                        AuthErrorCodes.ResponseInvalid);
                }

                operationKey = CreateOperationKey();
                var begin = _session.BeginRestore(operationKey, _deviceRegistration.DeviceId, _clock.UtcNow);
                if (begin.Kind != AuthTransitionKind.Accepted)
                {
                    return begin;
                }

                var result = await _client.RefreshAsync(
                    new RefreshTokenRequestDto(refreshToken),
                    cancellationToken).ConfigureAwait(false);
                if (!result.IsSuccess || result.Value is null)
                {
                    var failed = FinalizeFailure(_session.FailRestore(
                        operationKey,
                        ToContractError(result.Error, "恢复登录会话失败"),
                        _clock.UtcNow));
                    return failed;
                }

                if (!AuthTokenSet.TryCreate(result.Value.Tokens, _clock.UtcNow, out var tokens, out var tokenError))
                {
                    var failed = FinalizeFailure(_session.FailRestore(operationKey, tokenError!, _clock.UtcNow));
                    return failed;
                }

                var completed = _session.CompleteRestore(operationKey, tokens!, _clock.UtcNow);
                if (completed.Kind != AuthTransitionKind.Completed)
                {
                    return FinalizeFailure(completed);
                }

                _tokens = tokens;
                ApplyCredentialAction(completed.CredentialAction);
                return await ActivateCoreAsync(cancellationToken).ConfigureAwait(false);
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return operationKey is null
                ? CreateRejected(AuthOperationKind.Restore, "恢复登录已取消", 499, "CONTROL_PLANE_CANCELLED")
                : _session.CancelPendingOperation()
                    ?? CreateRejected(AuthOperationKind.Restore, "恢复登录已取消", 499, "CONTROL_PLANE_CANCELLED");
        }
        catch (Exception exception) when (IsCredentialStoreFailure(exception))
        {
            return HandleUnexpectedFailure(
                AuthOperationKind.Restore,
                operationKey,
                "恢复登录未完成，安全凭据存储不可用。",
                "AUTH_CREDENTIAL_STORAGE_UNAVAILABLE",
                clearPersistedCredential: true);
        }
        catch (Exception)
        {
            return HandleUnexpectedFailure(
                AuthOperationKind.Restore,
                operationKey,
                "控制面恢复登录未完成，请重试。",
                "CONTROL_PLANE_CLIENT_FAILURE");
        }
        finally
        {
            if (entered)
            {
                _serial.Release();
            }
        }
    }

    public async Task<AuthTransition> HeartbeatAsync(
        HeartbeatStatusDto status,
        CancellationToken cancellationToken = default,
        string? operationKey = null)
    {
        ArgumentNullException.ThrowIfNull(status);
        return await HeartbeatAsync(
            new HeartbeatRequestDto(
                _deviceRegistration.Product,
                _deviceRegistration.DeviceId,
                _clock.UtcNow,
                status),
            cancellationToken,
            operationKey).ConfigureAwait(false);
    }

    /// <summary>使用既有请求和幂等键补发心跳；请求中不含任何访问凭据。</summary>
    public async Task<AuthTransition> HeartbeatAsync(
        HeartbeatRequestDto request,
        CancellationToken cancellationToken = default,
        string? operationKey = null)
    {
        ArgumentNullException.ThrowIfNull(request);
        var entered = false;
        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
            entered = true;
            ThrowIfDisposed();
            operationKey ??= CreateOperationKey();
            var begin = _session.BeginHeartbeat(operationKey, request, _clock.UtcNow);
            if (begin.Kind != AuthTransitionKind.Accepted)
            {
                return begin;
            }

            if (_tokens is null)
            {
                return _session.FailHeartbeat(
                    operationKey,
                    ToContractError(null, "当前进程没有可用的访问凭据"),
                    _clock.UtcNow);
            }

            var result = await _client.HeartbeatAsync(
                _tokens.AccessToken,
                request,
                operationKey,
                cancellationToken).ConfigureAwait(false);
            return result.IsSuccess && result.Value is not null
                ? _session.CompleteHeartbeat(operationKey, result.Value.DeviceStatus, _clock.UtcNow)
                : _session.FailHeartbeat(operationKey, ToContractError(result.Error, "心跳失败"), _clock.UtcNow);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return operationKey is null
                ? CreateRejected(AuthOperationKind.Heartbeat, "心跳已取消", 499, "CONTROL_PLANE_CANCELLED")
                : _session.CancelPendingOperation()
                    ?? CreateRejected(AuthOperationKind.Heartbeat, "心跳已取消", 499, "CONTROL_PLANE_CANCELLED");
        }
        catch (Exception)
        {
            return CreateRejected(AuthOperationKind.Heartbeat, "心跳未完成。", 500, "CONTROL_PLANE_CLIENT_FAILURE");
        }
        finally
        {
            if (entered)
            {
                _serial.Release();
            }
        }
    }

    public async Task<AuthTransition> LogoutAsync(CancellationToken cancellationToken = default)
    {
        var entered = false;
        string? operationKey = null;
        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
            entered = true;
            ThrowIfDisposed();
            operationKey = CreateOperationKey();
            var begin = _session.BeginLogout(operationKey, _clock.UtcNow);
            if (begin.Kind != AuthTransitionKind.Accepted)
            {
                return begin;
            }

            var remoteConfirmed = false;
            if (_tokens is not null)
            {
                var result = await _client.LogoutAsync(
                    new LogoutRequestDto(_tokens.RefreshToken),
                    cancellationToken).ConfigureAwait(false);
                remoteConfirmed = result.IsSuccess && result.Value?.Success == true;
            }

            var completed = _session.CompleteLogout(operationKey, remoteConfirmed, _clock.UtcNow);
            _tokens = null;
            ApplyCredentialAction(completed.CredentialAction);
            return completed;
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return operationKey is null
                ? CreateRejected(AuthOperationKind.Logout, "退出已取消", 499, "CONTROL_PLANE_CANCELLED")
                : _session.CancelPendingOperation()
                    ?? CreateRejected(AuthOperationKind.Logout, "退出已取消", 499, "CONTROL_PLANE_CANCELLED");
        }
        catch (Exception)
        {
            var localLogout = operationKey is null
                ? null
                : _session.CompleteLogout(operationKey, remoteConfirmed: false, now: _clock.UtcNow);
            _tokens = null;
            TryDeletePersistedRefreshToken();
            var error = new ControlPlaneErrorDto(
                "CONTROL_PLANE_CLIENT_FAILURE",
                "远端退出未确认，本地会话已安全清理。",
                500);
            return new AuthTransition(
                AuthTransitionKind.Rejected,
                AuthOperationKind.Logout,
                operationKey ?? CreateOperationKey(),
                localLogout?.Snapshot ?? _session.Snapshot,
                error)
            {
                RemoteLogoutConfirmed = false
            };
        }
        finally
        {
            if (entered)
            {
                _serial.Release();
            }
        }
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }

        _disposed = true;
        _tokens = null;
        _ownedClient?.Dispose();
        // 不立即 Dispose 串行门：窗口关闭时可能仍有一个已取消的 UI 调用在 finally 中 Release。
        // 进程结束会回收该小型同步对象，避免把关闭竞态升级为未观察异常。
    }

    private async Task<AuthTransition> ActivateCoreAsync(CancellationToken cancellationToken)
    {
        var operationKey = CreateOperationKey();
        var begin = _session.BeginActivation(operationKey, _clock.UtcNow);
        if (begin.Kind != AuthTransitionKind.Accepted)
        {
            return begin;
        }

        if (_tokens is null)
        {
            return _session.FailActivation(operationKey, ToContractError(null, "当前进程没有可用的访问凭据"), _clock.UtcNow);
        }

        var result = await _client.ActivateAsync(
            _tokens.AccessToken,
            new ActivateDeviceRequestDto(_deviceRegistration),
            operationKey,
            cancellationToken).ConfigureAwait(false);
        var transition = result.IsSuccess && result.Value is not null
            ? _session.CompleteActivation(operationKey, result.Value.Device, _clock.UtcNow)
            : _session.FailActivation(operationKey, ToContractError(result.Error, "设备激活失败"), _clock.UtcNow);
        if (!transition.Snapshot.IsAuthenticated)
        {
            ClearLocalTokensAfterAuthorizationFailure();
        }

        return transition;
    }

    private void ApplyCredentialAction(CredentialAction? action)
    {
        if (action is null)
        {
            return;
        }

        if (action.Kind == CredentialActionKind.DeleteRefreshToken)
        {
            _secretStore.Delete(RefreshTokenCredentialName);
            return;
        }

        if (action.Kind != CredentialActionKind.StoreRefreshToken || string.IsNullOrEmpty(action.RefreshToken))
        {
            return;
        }

        var bytes = Encoding.UTF8.GetBytes(action.RefreshToken);
        try
        {
            _secretStore.Set(RefreshTokenCredentialName, bytes);
        }
        finally
        {
            CryptographicOperations.ZeroMemory(bytes);
        }
    }

    private AuthTransition FinalizeFailure(AuthTransition transition)
    {
        ApplyCredentialAction(transition.CredentialAction);
        if (!transition.Snapshot.IsAuthenticated)
        {
            _tokens = null;
        }

        return transition;
    }

    private AuthTransition HandleUnexpectedFailure(
        AuthOperationKind operation,
        string? operationKey,
        string message,
        string code,
        bool clearPersistedCredential = false)
    {
        var cancelled = _session.CancelPendingOperation();
        _tokens = null;
        if (clearPersistedCredential)
        {
            TryDeletePersistedRefreshToken();
        }

        var error = new ControlPlaneErrorDto(code, message, 500);
        return new AuthTransition(
            AuthTransitionKind.Rejected,
            operation,
            operationKey ?? CreateOperationKey(),
            cancelled?.Snapshot ?? _session.Snapshot,
            error);
    }

    private static bool IsCredentialStoreFailure(Exception exception) =>
        exception is UnauthorizedAccessException
            or System.Security.SecurityException
            or CryptographicException
            or IOException
            or System.ComponentModel.Win32Exception;

    private void ClearLocalTokensAfterAuthorizationFailure()
    {
        _tokens = null;
        try
        {
            _secretStore.Delete(RefreshTokenCredentialName);
        }
        catch (Exception)
        {
            // 授权已经 fail-closed；凭据删除失败不应把令牌重新带回会话或日志。
        }
    }

    private static bool TryReadSecretToken(SecretBuffer secret, out string token)
    {
        token = string.Empty;
        if (secret.Length is <= 0 or > AuthInputLimits.TokenMaxLength)
        {
            return false;
        }

        var bytes = new byte[secret.Length];
        try
        {
            secret.CopyTo(bytes);
            token = new UTF8Encoding(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true)
                .GetString(bytes);
            return AuthContractValidation.TryValidateRefreshToken(token, out _);
        }
        catch (DecoderFallbackException)
        {
            token = string.Empty;
            return false;
        }
        finally
        {
            CryptographicOperations.ZeroMemory(bytes);
        }
    }

    private void TryDeletePersistedRefreshToken()
    {
        try
        {
            _secretStore.Delete(RefreshTokenCredentialName);
        }
        catch (Exception)
        {
            // 读取到的凭据已经 fail-closed；删除失败不把秘密带入错误正文或日志。
        }
    }

    private AuthTransition CreateRejected(
        AuthOperationKind operation,
        string message,
        int status,
        string code = AuthErrorCodes.InvalidRequest)
    {
        var error = new ControlPlaneErrorDto(code, message, status);
        return new AuthTransition(AuthTransitionKind.Rejected, operation, CreateOperationKey(), _session.Snapshot, error);
    }

    private static ControlPlaneErrorDto ToContractError(ControlPlaneHttpError? error, string fallback) =>
        error is null
            ? new ControlPlaneErrorDto(AuthErrorCodes.ResponseInvalid, fallback, 502)
            : new ControlPlaneErrorDto(error.Code, error.Message, error.Status, error.RequestId);

    private static string CreateOperationKey() => $"desktop-{Guid.NewGuid():N}";

    private void ThrowIfDisposed()
    {
        if (_disposed)
        {
            throw new ObjectDisposedException(nameof(ControlPlaneAuthCoordinator));
        }
    }
}
