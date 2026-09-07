using System.Net;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Security;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class ControlPlaneAuthCoordinatorTests
{
    [TestMethod]
    public async Task Login_activates_device_and_persists_only_refresh_token()
    {
        var secrets = new FakeSecretStore();
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => JsonResponse(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token", "refresh-token", Future(), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", Future()))),
                "/api/v1/client/activate" => JsonResponse(new ActivateDeviceResponseDto(
                    "activate-request",
                    ActiveDevice("device-123", "user-1"))),
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(
            CreateTransport(httpClient),
            secrets,
            Registration(),
            ownedClient: null);

        var transition = await coordinator.LoginAsync("alice", "password123");

        Assert.AreEqual(AuthTransitionKind.Completed, transition.Kind);
        Assert.AreEqual(AuthSessionState.Activated, transition.Snapshot.State);
        Assert.IsTrue(transition.Snapshot.CanEnterWorkbench);
        Assert.AreEqual(2, handler.CallCount);
        Assert.AreEqual("refresh-token", secrets.ReadString(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));
        Assert.IsFalse(secrets.Contains("access-token"));
    }

    [TestMethod]
    public async Task Activation_error_keeps_authenticated_session_out_of_workbench()
    {
        var secrets = new FakeSecretStore();
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath == "/api/v1/client/auth/login"
                ? JsonResponse(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token", "refresh-token", Future(), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", Future())))
                : new HttpResponseMessage(HttpStatusCode.Forbidden)
                {
                    Content = new StringContent(
                        "{\"code\":\"ACCOUNT_ACTIVATION_REQUIRED\",\"message\":\"设备尚未激活\",\"request_id\":\"activation-request\"}",
                        Encoding.UTF8,
                        "application/json")
                });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());

        var transition = await coordinator.LoginAsync("alice", "password123");

        Assert.AreEqual(AuthSessionState.Authenticated, transition.Snapshot.State);
        Assert.IsFalse(transition.Snapshot.CanEnterWorkbench);
        Assert.AreEqual(AuthErrorCodes.AccountActivationRequired, transition.Snapshot.LastError?.Code);
        Assert.AreEqual("refresh-token", secrets.ReadString(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));
    }

    [TestMethod]
    public async Task Local_activation_expiry_keeps_login_and_refresh_credential()
    {
        var now = DateTimeOffset.UtcNow;
        var clock = new MutableClock(now);
        var secrets = new FakeSecretStore();
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => JsonResponse(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token", "refresh-token", now.AddMinutes(10).ToString("O"), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", now.ToString("O")))),
                "/api/v1/client/activate" => JsonResponse(new ActivateDeviceResponseDto(
                    "activate-request",
                    ActiveDevice("device-123", "user-1") with { ActivationExpiresAt = now.AddMinutes(1).ToString("O") })),
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(
            CreateTransport(httpClient),
            secrets,
            Registration(),
            clock: clock);
        await coordinator.LoginAsync("alice", "password123");
        clock.UtcNow = now.AddMinutes(2);

        var transition = await coordinator.ExpireActivationIfNeededAsync();

        Assert.IsNotNull(transition);
        Assert.AreEqual(AuthSessionState.Authenticated, transition!.Snapshot.State);
        Assert.AreEqual("refresh-token", secrets.ReadString(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));
    }

    [TestMethod]
    public async Task Unexpected_activation_failure_keeps_tokens_for_refresh_retry()
    {
        var activationAttempts = 0;
        var secrets = new FakeSecretStore();
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => JsonResponse(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token", "refresh-token", Future(), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", Future()))),
                "/api/v1/auth/refresh" => JsonResponse(new RefreshTokenResponseDto(
                    "refresh-request",
                    new SessionTokensDto("access-token-2", "refresh-token-2", Future(), "desktop"))),
                "/api/v1/client/activate" when ++activationAttempts == 1 =>
                    throw new InvalidOperationException("simulated activation failure"),
                "/api/v1/client/activate" => JsonResponse(new ActivateDeviceResponseDto(
                    "activate-request",
                    ActiveDevice("device-123", "user-1"))),
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());

        var failedActivation = await coordinator.LoginAsync("alice", "password123");
        var recovered = await coordinator.RefreshAsync();

        Assert.AreEqual(AuthSessionState.Authenticated, failedActivation.Snapshot.State);
        Assert.AreEqual(AuthSessionState.Activated, recovered.Snapshot.State);
        Assert.AreEqual("refresh-token-2", secrets.ReadString(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));
    }

    [TestMethod]
    public async Task Login_failure_deletes_any_stale_persisted_refresh_token()
    {
        var secrets = new FakeSecretStore();
        secrets.Set(ControlPlaneAuthCoordinator.RefreshTokenCredentialName, Encoding.UTF8.GetBytes("stale-refresh"));
        var handler = new SequenceHandler(
            _ => new HttpResponseMessage(HttpStatusCode.Unauthorized)
            {
                Content = new StringContent(
                    "{\"code\":\"UNAUTHENTICATED\",\"message\":\"账号或密码错误\",\"request_id\":\"login-request\"}",
                    Encoding.UTF8,
                    "application/json")
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());

        var transition = await coordinator.LoginAsync("alice", "password123");

        Assert.AreEqual(AuthSessionState.Unauthenticated, transition.Snapshot.State);
        Assert.IsFalse(secrets.Contains(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));
    }

    [TestMethod]
    public async Task Refresh_failure_deletes_rotated_refresh_token_when_session_is_rejected()
    {
        var secrets = new FakeSecretStore();
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => JsonResponse(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token", "refresh-token", Future(), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", Future()))),
                "/api/v1/client/activate" => JsonResponse(new ActivateDeviceResponseDto(
                    "activate-request",
                    ActiveDevice("device-123", "user-1"))),
                "/api/v1/auth/refresh" => new HttpResponseMessage(HttpStatusCode.Unauthorized)
                {
                    Content = new StringContent(
                        "{\"code\":\"UNAUTHENTICATED\",\"message\":\"Refresh Token 已失效\",\"request_id\":\"refresh-request\"}",
                        Encoding.UTF8,
                        "application/json")
                },
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());
        await coordinator.LoginAsync("alice", "password123");
        Assert.IsTrue(secrets.Contains(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));

        var transition = await coordinator.RefreshAsync();

        Assert.AreEqual(AuthSessionState.Unauthenticated, transition.Snapshot.State);
        Assert.IsFalse(secrets.Contains(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));
    }

    [TestMethod]
    public async Task Pre_cancelled_login_returns_a_cancelled_transition_without_throwing()
    {
        var secrets = new FakeSecretStore();
        var handler = new SequenceHandler(
            _ => throw new AssertFailedException("取消发生在串行门之前时不应发出请求。"));
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();

        var transition = await coordinator.LoginAsync("alice", "password123", cancellation.Token);

        Assert.AreEqual(AuthTransitionKind.Rejected, transition.Kind);
        Assert.AreEqual("CONTROL_PLANE_CANCELLED", transition.Error?.Code);
        Assert.AreEqual(0, handler.CallCount);
    }

    [TestMethod]
    public async Task Restore_refreshes_then_activates_without_exposing_access_token()
    {
        var secrets = new FakeSecretStore();
        secrets.Set(ControlPlaneAuthCoordinator.RefreshTokenCredentialName, Encoding.UTF8.GetBytes("refresh-old"));
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/auth/refresh" => JsonResponse(new RefreshTokenResponseDto(
                    "restore-refresh",
                    new SessionTokensDto("access-restored", "refresh-new", Future(), "desktop"))),
                "/api/v1/client/activate" => JsonResponse(new ActivateDeviceResponseDto(
                    "restore-activate",
                    ActiveDevice("device-123", "user-1"))),
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());

        var transition = await coordinator.RestoreAsync();

        Assert.IsNotNull(transition);
        Assert.AreEqual(AuthOperationKind.Activation, transition!.Operation);
        Assert.AreEqual(AuthSessionState.Activated, transition.Snapshot.State);
        Assert.AreEqual("user-1", transition.Snapshot.UserId);
        Assert.AreEqual(2, handler.CallCount);
        Assert.AreEqual("refresh-new", secrets.ReadString(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));
        Assert.IsFalse(secrets.Contains("access-restored"));
    }

    [TestMethod]
    public async Task Restore_missing_credential_is_a_noop_and_does_not_call_control_plane()
    {
        var secrets = new FakeSecretStore();
        var handler = new SequenceHandler(
            _ => throw new AssertFailedException("没有凭据时不应访问控制面。"));
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());

        var transition = await coordinator.RestoreAsync();

        Assert.IsNull(transition);
        Assert.AreEqual(AuthSessionState.Unauthenticated, coordinator.Snapshot.State);
        Assert.AreEqual(0, handler.CallCount);
    }

    [TestMethod]
    public async Task Restore_invalid_persisted_bytes_are_deleted_fail_closed_and_never_sent()
    {
        var secrets = new FakeSecretStore();
        secrets.Set(ControlPlaneAuthCoordinator.RefreshTokenCredentialName, [0xff, 0xfe]);
        var handler = new SequenceHandler(
            _ => throw new AssertFailedException("无效的本机凭据不应发送到控制面。"));
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());

        var transition = await coordinator.RestoreAsync();

        Assert.IsNotNull(transition);
        Assert.AreEqual(AuthTransitionKind.Rejected, transition!.Kind);
        Assert.AreEqual(AuthErrorCodes.ResponseInvalid, transition.Error!.Code);
        Assert.AreEqual(AuthSessionState.Unauthenticated, transition.Snapshot.State);
        Assert.IsFalse(secrets.Contains(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));
        Assert.AreEqual(0, handler.CallCount);
    }

    [TestMethod]
    public async Task Logout_clears_local_session_when_remote_revoke_is_unavailable()
    {
        var secrets = new FakeSecretStore();
        var logoutAttempts = 0;
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => JsonResponse(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token", "refresh-token", Future(), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", Future()))),
                "/api/v1/client/activate" => JsonResponse(new ActivateDeviceResponseDto("activate-request", ActiveDevice("device-123", "user-1"))),
                "/api/v1/auth/logout" when ++logoutAttempts == 1 => new HttpResponseMessage(HttpStatusCode.ServiceUnavailable)
                {
                    Content = new StringContent(
                        "{\"code\":\"NETWORK_ERROR\",\"message\":\"暂时不可用\",\"request_id\":\"logout-request\"}",
                        Encoding.UTF8,
                        "application/json")
                },
                "/api/v1/auth/logout" => JsonResponse(new LogoutResponseDto("logout-retry", true)),
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());
        await coordinator.LoginAsync("alice", "password123");

        var transition = await coordinator.LogoutAsync();

        Assert.AreEqual(AuthSessionState.Unauthenticated, transition.Snapshot.State);
        Assert.IsFalse(transition.RemoteLogoutConfirmed);
        Assert.IsNotNull(transition.Warning);
        Assert.IsFalse(secrets.Contains(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));
        Assert.IsTrue(secrets.Contains(ControlPlaneAuthCoordinator.PendingLogoutCredentialName));

        var retryWarning = await coordinator.RetryPendingLogoutAsync();

        Assert.IsNull(retryWarning);
        Assert.AreEqual(2, logoutAttempts);
        Assert.IsFalse(secrets.Contains(ControlPlaneAuthCoordinator.PendingLogoutCredentialName));
    }

    [TestMethod]
    public async Task Disabled_heartbeat_clears_in_memory_and_persisted_session_credentials()
    {
        var secrets = new FakeSecretStore();
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => JsonResponse(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token", "refresh-token", Future(), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", Future()))),
                "/api/v1/client/activate" => JsonResponse(new ActivateDeviceResponseDto(
                    "activate-request",
                    ActiveDevice("device-123", "user-1"))),
                "/api/v1/client/heartbeat" => JsonResponse(new HeartbeatResponseDto(
                    "heartbeat-request",
                    Future(),
                    "disabled")),
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());
        await coordinator.LoginAsync("alice", "password123");

        var transition = await coordinator.HeartbeatAsync(new HeartbeatStatusDto(0));

        Assert.AreEqual(AuthSessionState.Disabled, transition.Snapshot.State);
        Assert.IsFalse(secrets.Contains(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));
        var logout = await coordinator.LogoutAsync();
        Assert.IsTrue(logout.RemoteLogoutConfirmed);
        Assert.AreEqual(3, handler.CallCount);
    }

    [TestMethod]
    public async Task Unauthorized_heartbeat_refreshes_and_retries_once_with_rotated_access_token()
    {
        var heartbeatAttempts = 0;
        var refreshAttempts = 0;
        var secrets = new FakeSecretStore();
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => JsonResponse(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token-1", "refresh-token-1", Future(), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", Future()))),
                "/api/v1/client/activate" => JsonResponse(new ActivateDeviceResponseDto(
                    "activate-request",
                    ActiveDevice("device-123", "user-1"))),
                "/api/v1/auth/refresh" => Refresh(),
                "/api/v1/client/heartbeat" when ++heartbeatAttempts == 1 => Unauthorized(),
                "/api/v1/client/heartbeat" => AuthorizedHeartbeat(request),
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());
        await coordinator.LoginAsync("alice", "password123");

        var transition = await coordinator.HeartbeatAsync(new HeartbeatStatusDto(0));

        Assert.IsTrue(transition.IsSuccess);
        Assert.AreEqual(AuthSessionState.Activated, transition.Snapshot.State);
        Assert.AreEqual(2, heartbeatAttempts);
        Assert.AreEqual(1, refreshAttempts);
        Assert.AreEqual("refresh-token-2", secrets.ReadString(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));

        HttpResponseMessage Refresh()
        {
            refreshAttempts++;
            return JsonResponse(new RefreshTokenResponseDto(
                "refresh-request",
                new SessionTokensDto("access-token-2", "refresh-token-2", Future(), "desktop")));
        }

        static HttpResponseMessage Unauthorized() => new(HttpStatusCode.Forbidden)
        {
            Content = new StringContent(
                "{\"code\":\"UNAUTHENTICATED\",\"message\":\"Access Token 已失效\",\"request_id\":\"heartbeat-request\"}",
                Encoding.UTF8,
                "application/json")
        };

        static HttpResponseMessage AuthorizedHeartbeat(HttpRequestMessage request)
        {
            Assert.AreEqual("access-token-2", request.Headers.Authorization?.Parameter);
            return JsonResponse(new HeartbeatResponseDto("heartbeat-retry", Future(), "active"));
        }
    }

    [TestMethod]
    public async Task Unauthorized_heartbeat_clears_session_only_when_refresh_is_rejected()
    {
        var heartbeatAttempts = 0;
        var secrets = new FakeSecretStore();
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => JsonResponse(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token-1", "refresh-token-1", Future(), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", Future()))),
                "/api/v1/client/activate" => JsonResponse(new ActivateDeviceResponseDto(
                    "activate-request",
                    ActiveDevice("device-123", "user-1"))),
                "/api/v1/client/heartbeat" => RejectHeartbeat(),
                "/api/v1/auth/refresh" => new HttpResponseMessage(HttpStatusCode.Unauthorized)
                {
                    Content = new StringContent(
                        "{\"code\":\"UNAUTHENTICATED\",\"message\":\"Refresh Token 已失效\",\"request_id\":\"refresh-request\"}",
                        Encoding.UTF8,
                        "application/json")
                },
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());
        await coordinator.LoginAsync("alice", "password123");

        var transition = await coordinator.HeartbeatAsync(new HeartbeatStatusDto(0));

        Assert.AreEqual(AuthOperationKind.Refresh, transition.Operation);
        Assert.AreEqual(AuthSessionState.Unauthenticated, transition.Snapshot.State);
        Assert.AreEqual(1, heartbeatAttempts);
        Assert.IsFalse(secrets.Contains(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));

        HttpResponseMessage RejectHeartbeat()
        {
            heartbeatAttempts++;
            return new HttpResponseMessage(HttpStatusCode.Unauthorized)
            {
                Content = new StringContent(
                    "{\"code\":\"UNAUTHENTICATED\",\"message\":\"Access Token 已失效\",\"request_id\":\"heartbeat-request\"}",
                    Encoding.UTF8,
                    "application/json")
            };
        }
    }

    [TestMethod]
    public async Task Heartbeat_rejected_after_successful_refresh_does_not_delete_refresh_credential()
    {
        var heartbeatAttempts = 0;
        var secrets = new FakeSecretStore();
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => JsonResponse(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token-1", "refresh-token-1", Future(), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", Future()))),
                "/api/v1/client/activate" => JsonResponse(new ActivateDeviceResponseDto(
                    "activate-request",
                    ActiveDevice("device-123", "user-1"))),
                "/api/v1/auth/refresh" => JsonResponse(new RefreshTokenResponseDto(
                    "refresh-request",
                    new SessionTokensDto("access-token-2", "refresh-token-2", Future(), "desktop"))),
                "/api/v1/client/heartbeat" => RejectHeartbeat(),
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());
        await coordinator.LoginAsync("alice", "password123");

        var transition = await coordinator.HeartbeatAsync(new HeartbeatStatusDto(0));

        Assert.AreEqual(AuthOperationKind.Heartbeat, transition.Operation);
        Assert.AreEqual(AuthErrorCodes.Unauthenticated, transition.Error?.Code);
        Assert.AreEqual(AuthSessionState.Offline, coordinator.Snapshot.State);
        Assert.AreEqual(2, heartbeatAttempts);
        Assert.AreEqual("refresh-token-2", secrets.ReadString(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));

        HttpResponseMessage RejectHeartbeat()
        {
            heartbeatAttempts++;
            return new HttpResponseMessage(HttpStatusCode.Unauthorized)
            {
                Content = new StringContent(
                    "{\"code\":\"UNAUTHENTICATED\",\"message\":\"Access Token 已失效\",\"request_id\":\"heartbeat-request\"}",
                    Encoding.UTF8,
                    "application/json")
            };
        }
    }

    [TestMethod]
    public async Task Transient_refresh_failure_after_unauthorized_heartbeat_keeps_session_for_retry()
    {
        var heartbeatAttempts = 0;
        var secrets = new FakeSecretStore();
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => JsonResponse(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token-1", "refresh-token-1", Future(), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", Future()))),
                "/api/v1/client/activate" => JsonResponse(new ActivateDeviceResponseDto(
                    "activate-request",
                    ActiveDevice("device-123", "user-1"))),
                "/api/v1/client/heartbeat" => RejectHeartbeat(),
                "/api/v1/auth/refresh" => new HttpResponseMessage(HttpStatusCode.ServiceUnavailable)
                {
                    Content = new StringContent(
                        "{\"code\":\"CONTROL_PLANE_UNAVAILABLE\",\"message\":\"暂时不可用\",\"request_id\":\"refresh-request\"}",
                        Encoding.UTF8,
                        "application/json")
                },
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());
        await coordinator.LoginAsync("alice", "password123");

        var transition = await coordinator.HeartbeatAsync(new HeartbeatStatusDto(0));

        Assert.AreEqual(AuthOperationKind.Refresh, transition.Operation);
        Assert.IsTrue(transition.ShouldRetry);
        Assert.AreEqual(AuthSessionState.Offline, transition.Snapshot.State);
        Assert.AreEqual(1, heartbeatAttempts);
        Assert.AreEqual("refresh-token-1", secrets.ReadString(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));

        HttpResponseMessage RejectHeartbeat()
        {
            heartbeatAttempts++;
            return new HttpResponseMessage(HttpStatusCode.Unauthorized)
            {
                Content = new StringContent(
                    "{\"code\":\"UNAUTHENTICATED\",\"message\":\"Access Token 已失效\",\"request_id\":\"heartbeat-request\"}",
                    Encoding.UTF8,
                    "application/json")
            };
        }
    }

    [TestMethod]
    public void Device_identity_is_stable_and_rejects_corrupt_secret()
    {
        var secrets = new FakeSecretStore();
        var legacy = new FakeSecretStore();
        var first = WindowsDeviceIdentity.GetOrCreate(secrets, legacy);
        var second = WindowsDeviceIdentity.GetOrCreate(secrets, legacy);

        Assert.AreEqual(first, second);
        Assert.IsTrue(AuthContractValidation.TryValidateDeviceId(first, out _));

        secrets.Set(WindowsDeviceIdentity.CredentialName, Encoding.UTF8.GetBytes("bad id"));
        Assert.ThrowsExactly<InvalidDataException>(() => WindowsDeviceIdentity.GetOrCreate(secrets, legacy));
    }

    [TestMethod]
    public void Device_identity_migrates_legacy_rust_id_only_when_csharp_id_is_missing()
    {
        var primary = new FakeSecretStore();
        var legacy = new FakeSecretStore();
        legacy.Set(WindowsDeviceIdentity.LegacyRustCredentialName, Encoding.Unicode.GetBytes("desktop-rust-device-01"));

        var migrated = WindowsDeviceIdentity.GetOrCreate(primary, legacy);

        Assert.AreEqual("desktop-rust-device-01", migrated);
        Assert.AreEqual(migrated, primary.ReadString(WindowsDeviceIdentity.CredentialName));
        Assert.IsTrue(legacy.Contains(WindowsDeviceIdentity.LegacyRustCredentialName));

        primary.Set(WindowsDeviceIdentity.CredentialName, Encoding.UTF8.GetBytes("desktop-csharp-device-01"));
        Assert.AreEqual("desktop-csharp-device-01", WindowsDeviceIdentity.GetOrCreate(primary, legacy));
    }

    [TestMethod]
    public void Invalid_legacy_rust_device_id_fails_closed_without_creating_a_second_identity()
    {
        var primary = new FakeSecretStore();
        var legacy = new FakeSecretStore();
        legacy.Set(WindowsDeviceIdentity.LegacyRustCredentialName, Encoding.Unicode.GetBytes("bad id"));

        Assert.ThrowsExactly<InvalidDataException>(() => WindowsDeviceIdentity.GetOrCreate(primary, legacy));
        Assert.IsFalse(primary.Contains(WindowsDeviceIdentity.CredentialName));
    }

    [TestMethod]
    public void Pending_logout_store_deduplicates_and_enforces_its_fixed_capacity()
    {
        var secrets = new FakeSecretStore();
        var store = new PendingLogoutTokenStore(secrets);

        Assert.IsTrue(store.Enqueue("refresh-token-duplicate"));
        Assert.IsTrue(store.Enqueue("refresh-token-duplicate"));
        for (var index = 1; index < PendingLogoutTokenStore.MaxPendingTokens; index++)
        {
            Assert.IsTrue(store.Enqueue($"refresh-token-{index:D2}"));
        }

        Assert.IsFalse(store.Enqueue("refresh-token-overflow"));
    }

    private static ControlPlaneHttpClient CreateTransport(HttpClient httpClient) =>
        new(httpClient, new ControlPlaneHttpClientOptions
        {
            BaseUri = new Uri("http://127.0.0.1:18090"),
            AllowLoopbackHttp = true,
        });

    private static DeviceRegistrationDto Registration() =>
        new(ControlPlaneContractValues.Product, "device-123", "GPAL Desktop", "windows", "1.0.0", "Windows 11");

    private static DeviceSummaryDto ActiveDevice(string deviceId, string userId) =>
        new(deviceId, userId, ControlPlaneContractValues.Product, "GPAL Desktop", "windows", "1.0.0", "active", null, null, null, null, null, null, null, null, null, false, Future(), Future());

    private static string Future() => DateTimeOffset.UtcNow.AddMinutes(10).ToString("O");

    private static HttpResponseMessage JsonResponse<T>(T value) => new(HttpStatusCode.OK)
    {
        Content = new StringContent(JsonSerializer.Serialize(value, ContractJson.CreateOptions()), Encoding.UTF8, "application/json")
    };

    private sealed class SequenceHandler(Func<HttpRequestMessage, HttpResponseMessage> responder) : HttpMessageHandler
    {
        public int CallCount { get; private set; }

        public HttpClient CreateHttpClient() => new(this);

        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
        {
            CallCount++;
            return Task.FromResult(responder(request));
        }
    }

    private sealed class MutableClock(DateTimeOffset utcNow) : IControlPlaneClock
    {
        public DateTimeOffset UtcNow { get; set; } = utcNow;

        public Task DelayAsync(TimeSpan delay, CancellationToken cancellationToken) =>
            Task.Delay(delay, cancellationToken);
    }

    private sealed class FakeSecretStore : ISecretStore
    {
        private readonly Dictionary<string, byte[]> _values = new(StringComparer.Ordinal);

        public void Set(string name, ReadOnlySpan<byte> secret)
        {
            if (_values.Remove(name, out var previous))
            {
                CryptographicOperations.ZeroMemory(previous);
            }

            _values[name] = secret.ToArray();
        }

        public bool TryGet(string name, out SecretBuffer secret)
        {
            if (!_values.TryGetValue(name, out var value))
            {
                secret = null!;
                return false;
            }

            secret = SecretBuffer.FromBytes(value);
            return true;
        }

        public bool Delete(string name)
        {
            if (!_values.Remove(name, out var value))
            {
                return false;
            }

            CryptographicOperations.ZeroMemory(value);
            return true;
        }

        public bool Contains(string name) => _values.ContainsKey(name);

        public string ReadString(string name) => Encoding.UTF8.GetString(_values[name]);
    }
}
