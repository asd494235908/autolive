using System.Net;
using System.Text;
using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Security;
using GpAutoLive.Core.Configuration;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class ControlPlaneHeartbeatSchedulerTests
{
    [TestMethod]
    public async Task Start_is_idempotent_and_stop_joins_the_single_loop()
    {
        using var httpClient = CreateHttpClient();
        using var auth = CreateAuth(httpClient);
        var clock = new BlockingClock();
        await using var scheduler = CreateScheduler(auth, clock, static () =>
            new HeartbeatStatusDto(0, null, 0, 1, "Windows", "test", null, null, "stopped"));

        scheduler.Start();
        scheduler.Start();
        await clock.DelayEntered.Task.WaitAsync(TimeSpan.FromSeconds(1));

        Assert.IsTrue(scheduler.IsRunning);
        Assert.AreEqual(1, clock.DelayCount);

        await scheduler.StopAsync();

        Assert.IsFalse(scheduler.IsRunning);
        Assert.AreEqual(1, clock.DelayCount);
    }

    [TestMethod]
    public async Task Unauthenticated_session_does_not_collect_or_send_heartbeat()
    {
        using var httpClient = CreateHttpClient();
        using var auth = CreateAuth(httpClient);
        var clock = new BlockingClock();
        var providerCalls = 0;
        await using var scheduler = CreateScheduler(
            auth,
            clock,
            () =>
            {
                providerCalls++;
                return new HeartbeatStatusDto(0, null, 0, 1, "Windows", "test", null, null, "stopped");
            });

        scheduler.Start();
        await clock.DelayEntered.Task.WaitAsync(TimeSpan.FromSeconds(1));
        await scheduler.SendNowAsync();
        await scheduler.StopAsync();

        Assert.AreEqual(0, providerCalls);
    }

    [TestMethod]
    public async Task Disabled_heartbeat_is_observed_by_the_ui_projection_boundary()
    {
        using var httpClient = new HttpClient(new DisabledHeartbeatHandler());
        using var auth = CreateAuth(httpClient);
        var login = await auth.LoginAsync("alice", "password123");
        Assert.AreEqual(AuthSessionState.Activated, login.Snapshot.State);
        var observed = new TaskCompletionSource<AuthTransition>(TaskCreationOptions.RunContinuationsAsynchronously);
        var clock = new BlockingClock();
        var path = Path.Combine(Path.GetTempPath(), $"gpautolive-heartbeat-disabled-{Guid.NewGuid():N}.json");
        await using var scheduler = new ControlPlaneHeartbeatScheduler(
            auth,
            static () => new HeartbeatStatusDto(0, null, 0, 1, "Windows", "test", null, null, "idle"),
            new HeartbeatOutboxStore(path),
            clock,
            new ControlPlaneHeartbeatSchedulerOptions { Interval = TimeSpan.FromMinutes(1) },
            transition =>
            {
                if (transition.Error?.Code == AuthErrorCodes.DeviceDisabled)
                {
                    observed.TrySetResult(transition);
                }
            });

        scheduler.Start();
        var transition = await observed.Task.WaitAsync(TimeSpan.FromSeconds(2));
        await scheduler.StopAsync();

        Assert.AreEqual(AuthSessionState.Disabled, transition.Snapshot.State);
        Assert.AreEqual(AuthSessionState.Disabled, auth.Snapshot.State);
    }

    [TestMethod]
    public async Task Access_token_expiring_before_next_cycle_is_refreshed_before_heartbeat()
    {
        var now = DateTimeOffset.UtcNow;
        var clock = new BlockingClock { UtcNow = now };
        var handler = new ProactiveRefreshHandler(now);
        using var httpClient = new HttpClient(handler);
        using var auth = CreateAuth(httpClient, clock);
        var login = await auth.LoginAsync("alice", "password123");
        Assert.AreEqual(AuthSessionState.Activated, login.Snapshot.State);
        await using var scheduler = CreateScheduler(auth, clock, static () => new HeartbeatStatusDto(0));

        scheduler.Start();
        await handler.HeartbeatSent.Task.WaitAsync(TimeSpan.FromSeconds(2));
        await scheduler.StopAsync();

        Assert.AreEqual(1, handler.RefreshCount);
        Assert.AreEqual("access-token-2", handler.HeartbeatAccessToken);
        Assert.AreEqual(AuthSessionState.Activated, auth.Snapshot.State);
    }

    private static ControlPlaneHeartbeatScheduler CreateScheduler(
        ControlPlaneAuthCoordinator auth,
        IControlPlaneClock clock,
        HeartbeatStatusProvider statusProvider)
    {
        var path = Path.Combine(Path.GetTempPath(), $"gpautolive-heartbeat-test-{Guid.NewGuid():N}.json");
        return new ControlPlaneHeartbeatScheduler(
            auth,
            statusProvider,
            new HeartbeatOutboxStore(path),
            clock,
            new ControlPlaneHeartbeatSchedulerOptions { Interval = TimeSpan.FromMinutes(1) });
    }

    private static ControlPlaneAuthCoordinator CreateAuth(
        HttpClient httpClient,
        IControlPlaneClock? clock = null) =>
        new(
            new ControlPlaneHttpClient(
                httpClient,
                new ControlPlaneHttpClientOptions
                {
                    BaseUri = new Uri("http://127.0.0.1:18090"),
                    AllowLoopbackHttp = true,
                }),
            new InMemorySecretStore(),
            new DeviceRegistrationDto(
                ControlPlaneContractValues.Product,
                "device-123",
                "GPAL Desktop",
                "windows",
                "1.0.0",
                "Windows 11"),
            clock: clock);

    private static HttpClient CreateHttpClient() => new(new NeverCalledHandler());

    private sealed class BlockingClock : IControlPlaneClock
    {
        public TaskCompletionSource<object?> DelayEntered { get; } =
            new(TaskCreationOptions.RunContinuationsAsynchronously);

        public int DelayCount { get; private set; }

        public DateTimeOffset UtcNow { get; init; } = DateTimeOffset.UtcNow;

        public Task DelayAsync(TimeSpan delay, CancellationToken cancellationToken)
        {
            DelayCount++;
            DelayEntered.TrySetResult(null);
            return Task.Delay(Timeout.InfiniteTimeSpan, cancellationToken);
        }
    }

    private sealed class ProactiveRefreshHandler(DateTimeOffset now) : HttpMessageHandler
    {
        public TaskCompletionSource<object?> HeartbeatSent { get; } =
            new(TaskCreationOptions.RunContinuationsAsynchronously);

        public int RefreshCount { get; private set; }

        public string? HeartbeatAccessToken { get; private set; }

        protected override Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request,
            CancellationToken cancellationToken)
        {
            var response = request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => Json(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token-1", "refresh-token-1", now.AddSeconds(30).ToString("O"), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", now.ToString("O")))),
                "/api/v1/auth/refresh" => Refresh(),
                "/api/v1/client/activate" => Json(new ActivateDeviceResponseDto(
                    "activate-request",
                    new DeviceSummaryDto(
                        "device-123", "user-1", ControlPlaneContractValues.Product, "GPAL Desktop", "windows",
                        "1.0.0", "active", null, null, null, null, null, null, null, null, null, false,
                        now.AddMinutes(10).ToString("O"), now.AddMinutes(10).ToString("O")))),
                "/api/v1/client/heartbeat" => Heartbeat(request),
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            };
            return Task.FromResult(response);
        }

        private HttpResponseMessage Refresh()
        {
            RefreshCount++;
            return Json(new RefreshTokenResponseDto(
                "refresh-request",
                new SessionTokensDto("access-token-2", "refresh-token-2", now.AddMinutes(10).ToString("O"), "desktop")));
        }

        private HttpResponseMessage Heartbeat(HttpRequestMessage request)
        {
            HeartbeatAccessToken = request.Headers.Authorization?.Parameter;
            HeartbeatSent.TrySetResult(null);
            return Json(new HeartbeatResponseDto("heartbeat-request", now.AddMinutes(10).ToString("O"), "active"));
        }

        private static HttpResponseMessage Json<T>(T value) => new(HttpStatusCode.OK)
        {
            Content = new StringContent(JsonSerializer.Serialize(value, ContractJson.CreateOptions()), Encoding.UTF8, "application/json")
        };
    }

    private sealed class NeverCalledHandler : HttpMessageHandler
    {
        protected override Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request,
            CancellationToken cancellationToken) =>
            Task.FromException<HttpResponseMessage>(
                new AssertFailedException("未授权心跳测试不应访问控制面。"));
    }

    private sealed class DisabledHeartbeatHandler : HttpMessageHandler
    {
        protected override Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request,
            CancellationToken cancellationToken)
        {
            var response = request.RequestUri?.AbsolutePath switch
            {
                "/api/v1/client/auth/login" => Json(new DesktopLoginResponseDto(
                    "login-request",
                    new SessionTokensDto("access-token", "refresh-token", Future(), "desktop"),
                    new UserSummaryDto("user-1", "alice", "user", "active", Future()))),
                "/api/v1/client/activate" => Json(new ActivateDeviceResponseDto(
                    "activate-request",
                    new DeviceSummaryDto(
                        "device-123", "user-1", ControlPlaneContractValues.Product, "GPAL Desktop", "windows",
                        "1.0.0", "active", null, null, null, null, null, null, null, null, null, false, Future(), Future()))),
                "/api/v1/client/heartbeat" => Json(new HeartbeatResponseDto("heartbeat-request", Future(), "disabled")),
                _ => throw new AssertFailedException("访问了未预期的控制面路径。"),
            };
            return Task.FromResult(response);
        }

        private static HttpResponseMessage Json<T>(T value) => new(HttpStatusCode.OK)
        {
            Content = new StringContent(JsonSerializer.Serialize(value, ContractJson.CreateOptions()), Encoding.UTF8, "application/json")
        };

        private static string Future() => DateTimeOffset.UtcNow.AddMinutes(10).ToString("O");
    }

    private sealed class InMemorySecretStore : ISecretStore
    {
        public void Set(string name, ReadOnlySpan<byte> secret)
        {
        }

        public bool TryGet(string name, out SecretBuffer secret)
        {
            secret = null!;
            return false;
        }

        public bool Delete(string name) => false;
    }
}
