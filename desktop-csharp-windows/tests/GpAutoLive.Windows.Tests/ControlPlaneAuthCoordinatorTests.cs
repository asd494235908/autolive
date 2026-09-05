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

        Assert.AreEqual(AuthSessionState.Unauthenticated, transition.Snapshot.State);
        Assert.IsFalse(transition.Snapshot.CanEnterWorkbench);
        Assert.AreEqual(AuthErrorCodes.AccountActivationRequired, transition.Snapshot.LastError?.Code);
        Assert.IsFalse(secrets.Contains(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));
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
        var handler = new SequenceHandler(
            request => request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => JsonResponse(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token", "refresh-token", Future(), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", Future()))),
                "/api/v1/client/activate" => JsonResponse(new ActivateDeviceResponseDto("activate-request", ActiveDevice("device-123", "user-1"))),
                "/api/v1/auth/logout" => new HttpResponseMessage(HttpStatusCode.ServiceUnavailable)
                {
                    Content = new StringContent(
                        "{\"code\":\"NETWORK_ERROR\",\"message\":\"暂时不可用\",\"request_id\":\"logout-request\"}",
                        Encoding.UTF8,
                        "application/json")
                },
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            });
        using var httpClient = handler.CreateHttpClient();
        using var coordinator = new ControlPlaneAuthCoordinator(CreateTransport(httpClient), secrets, Registration());
        await coordinator.LoginAsync("alice", "password123");

        var transition = await coordinator.LogoutAsync();

        Assert.AreEqual(AuthSessionState.Unauthenticated, transition.Snapshot.State);
        Assert.IsFalse(transition.RemoteLogoutConfirmed);
        Assert.IsFalse(secrets.Contains(ControlPlaneAuthCoordinator.RefreshTokenCredentialName));
    }

    [TestMethod]
    public void Device_identity_is_stable_and_rejects_corrupt_secret()
    {
        var secrets = new FakeSecretStore();
        var first = WindowsDeviceIdentity.GetOrCreate(secrets);
        var second = WindowsDeviceIdentity.GetOrCreate(secrets);

        Assert.AreEqual(first, second);
        Assert.IsTrue(AuthContractValidation.TryValidateDeviceId(first, out _));

        secrets.Set(WindowsDeviceIdentity.CredentialName, Encoding.UTF8.GetBytes("bad id"));
        Assert.ThrowsExactly<InvalidDataException>(() => WindowsDeviceIdentity.GetOrCreate(secrets));
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
        new(deviceId, userId, ControlPlaneContractValues.Product, "GPAL Desktop", "windows", "1.0.0", "active", null, null, null, null, null, null, null, null, null, false, Future(), null);

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
