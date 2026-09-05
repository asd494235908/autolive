using GpAutoLive.Contracts;
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

    private static ControlPlaneAuthCoordinator CreateAuth(HttpClient httpClient) =>
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
                "Windows 11"));

    private static HttpClient CreateHttpClient() => new(new NeverCalledHandler());

    private sealed class BlockingClock : IControlPlaneClock
    {
        public TaskCompletionSource<object?> DelayEntered { get; } =
            new(TaskCreationOptions.RunContinuationsAsynchronously);

        public int DelayCount { get; private set; }

        public DateTimeOffset UtcNow => DateTimeOffset.UtcNow;

        public Task DelayAsync(TimeSpan delay, CancellationToken cancellationToken)
        {
            DelayCount++;
            DelayEntered.TrySetResult(null);
            return Task.Delay(Timeout.InfiniteTimeSpan, cancellationToken);
        }
    }

    private sealed class NeverCalledHandler : HttpMessageHandler
    {
        protected override Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request,
            CancellationToken cancellationToken) =>
            Task.FromException<HttpResponseMessage>(
                new AssertFailedException("未授权心跳测试不应访问控制面。"));
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
