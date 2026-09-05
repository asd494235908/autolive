using System.Net;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class ControlPlaneHttpClientTests
{
    [TestMethod]
    public async Task Login_sends_fixed_route_headers_and_strict_success()
    {
        var handler = new StubHandler(request =>
        {
            Assert.AreEqual(HttpMethod.Post, request.Method);
            Assert.AreEqual(
                "/api/v1/client/auth/login",
                request.RequestUri?.AbsolutePath);
            Assert.IsTrue(request.Headers.TryGetValues("X-Request-Id", out var requestIds));
            Assert.IsTrue(requestIds!.Single().Length is > 0 and <= AuthInputLimits.RequestIdMaxLength);
            Assert.IsFalse(request.Headers.Authorization is not null);
            Assert.AreEqual("application/json", request.Content?.Headers.ContentType?.MediaType);

            return JsonResponse(new DesktopLoginResponseDto(
                "server-request",
                new SessionTokensDto(
                    "access-token",
                    "refresh-token",
                    DateTimeOffset.UtcNow.AddMinutes(10).ToString("O"),
                    "desktop"),
                new UserSummaryDto("user-1", "alice", "user", "active", DateTimeOffset.UtcNow.ToString("O"))));
        });
        var client = CreateClient(handler);

        var result = await client.LoginAsync(new DesktopLoginRequestDto("alice", "password123"));

        Assert.IsTrue(result.IsSuccess, result.Error?.Message);
        Assert.AreEqual("user-1", result.Value?.User.Id);
        Assert.IsNull(result.Error);
    }

    [TestMethod]
    public async Task Activation_sends_bearer_and_idempotency_headers()
    {
        var handler = new StubHandler(request =>
        {
            Assert.AreEqual("Bearer access-token", request.Headers.Authorization?.ToString());
            Assert.IsTrue(request.Headers.TryGetValues("Idempotency-Key", out var keys));
            Assert.AreEqual("idem-1", keys!.Single());
            return JsonResponse(new ActivateDeviceResponseDto(
                "server-request",
                new DeviceSummaryDto(
                    "device-1",
                    "user-1",
                    "autolive",
                    "Windows desktop",
                    "windows",
                    "1.0.0",
                    "active",
                    null,
                    null,
                    null,
                    null,
                    null,
                    null,
                    null,
                    null,
                    null,
                    false,
                    DateTimeOffset.UtcNow.ToString("O"),
                    null)));
        });
        var client = CreateClient(handler);
        var request = new ActivateDeviceRequestDto(
            new DeviceRegistrationDto("autolive", "device-123", "Windows desktop", "windows", "1.0.0"));

        var result = await client.ActivateAsync("access-token", request, "idem-1");

        Assert.IsTrue(result.IsSuccess, result.Error?.Message);
        Assert.AreEqual("active", result.Value?.Device.Status);
    }

    [TestMethod]
    public async Task Unknown_success_fields_fail_closed()
    {
        var handler = new StubHandler(_ => new HttpResponseMessage(HttpStatusCode.OK)
        {
            Content = new StringContent(
                "{\"request_id\":\"server-request\",\"tokens\":{\"access_token\":\"access\",\"refresh_token\":\"refresh\",\"expires_at\":\"2099-01-01T00:00:00Z\",\"audience\":\"desktop\"},\"user\":{\"id\":\"user-1\",\"username\":\"alice\",\"role\":\"user\",\"status\":\"active\",\"created_at\":\"2099-01-01T00:00:00Z\"},\"unexpected\":true}",
                Encoding.UTF8,
                "application/json")
        });
        var result = await CreateClient(handler).LoginAsync(new DesktopLoginRequestDto("alice", "password123"));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(AuthErrorCodes.ResponseInvalid, result.Error?.Code);
        Assert.IsNull(result.Value);
    }

    [TestMethod]
    public async Task Remote_error_keeps_stable_code_and_http_retry_semantics()
    {
        var handler = new StubHandler(_ => new HttpResponseMessage(HttpStatusCode.ServiceUnavailable)
        {
            Content = new StringContent(
                "{\"code\":\"DEVICE_DISABLED\",\"message\":\"设备已被禁用\",\"request_id\":\"server-request\",\"details\":[]}",
                Encoding.UTF8,
                "application/json")
        });
        var result = await CreateClient(handler).LoginAsync(new DesktopLoginRequestDto("alice", "password123"));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual("DEVICE_DISABLED", result.Error?.Code);
        Assert.AreEqual(503, result.Error?.Status);
        Assert.IsTrue(result.Error?.Retryable);
    }

    [TestMethod]
    public async Task Oversized_response_is_rejected_before_json_parse()
    {
        var handler = new StubHandler(_ => new HttpResponseMessage(HttpStatusCode.OK)
        {
            Content = new StringContent(new string('x', 2_048), Encoding.UTF8, "application/json")
        });
        var client = new ControlPlaneHttpClient(
            handler.CreateHttpClient(),
            new ControlPlaneHttpClientOptions
            {
                BaseUri = new Uri("http://127.0.0.1:18090"),
                AllowLoopbackHttp = true,
                MaxResponseBytes = 1_024,
            });

        var result = await client.LoginAsync(new DesktopLoginRequestDto("alice", "password123"));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual("CONTROL_PLANE_RESPONSE_TOO_LARGE", result.Error?.Code);
        Assert.IsNull(result.Value);
    }

    [TestMethod]
    public async Task Invalid_input_does_not_hit_handler()
    {
        var handler = new StubHandler(_ => throw new AssertFailedException("无效输入不应访问网络"));
        var result = await CreateClient(handler).LoginAsync(null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(AuthErrorCodes.InvalidRequest, result.Error?.Code);
        Assert.AreEqual(0, handler.CallCount);
    }

    [TestMethod]
    public void Production_http_requires_https()
    {
        var handler = new StubHandler(_ => JsonResponse(new { }));

        Assert.ThrowsExactly<ArgumentException>(() => new ControlPlaneHttpClient(
            handler.CreateHttpClient(),
            new ControlPlaneHttpClientOptions { BaseUri = new Uri("http://example.invalid") }));
    }

    [TestMethod]
    public void Fixed_development_test_http_requires_explicit_opt_in()
    {
        var handler = new StubHandler(_ => JsonResponse(new { }));

        Assert.ThrowsExactly<ArgumentException>(() => new ControlPlaneHttpClient(
            handler.CreateHttpClient(),
            new ControlPlaneHttpClientOptions
            {
                BaseUri = new Uri("http://101.96.208.132:9090"),
            }));
    }

    [TestMethod]
    public void Fixed_development_test_http_is_allowed_only_when_explicitly_enabled()
    {
        var handler = new StubHandler(_ => JsonResponse(new { }));

        _ = new ControlPlaneHttpClient(
            handler.CreateHttpClient(),
            new ControlPlaneHttpClientOptions
            {
                BaseUri = new Uri("http://101.96.208.132:9090"),
                AllowDevelopmentTestHttp = true,
            });
    }

    [TestMethod]
    public void Development_test_http_opt_in_does_not_allow_other_remote_origins()
    {
        var handler = new StubHandler(_ => JsonResponse(new { }));

        Assert.ThrowsExactly<ArgumentException>(() => new ControlPlaneHttpClient(
            handler.CreateHttpClient(),
            new ControlPlaneHttpClientOptions
            {
                BaseUri = new Uri("http://101.96.208.131:9090"),
                AllowDevelopmentTestHttp = true,
            }));
    }

    private static ControlPlaneHttpClient CreateClient(StubHandler handler) =>
        new(
            handler.CreateHttpClient(),
            new ControlPlaneHttpClientOptions
            {
                BaseUri = new Uri("http://127.0.0.1:18090"),
                AllowLoopbackHttp = true,
            });

    private static HttpResponseMessage JsonResponse<T>(T value) => new(HttpStatusCode.OK)
    {
        Content = new StringContent(
            JsonSerializer.Serialize(value, ContractJson.CreateOptions()),
            Encoding.UTF8,
            "application/json")
    };

    private sealed class StubHandler(Func<HttpRequestMessage, HttpResponseMessage> responder) : HttpMessageHandler
    {
        public int CallCount { get; private set; }

        public HttpClient CreateHttpClient() => new(this);

        protected override Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request,
            CancellationToken cancellationToken)
        {
            CallCount++;
            return Task.FromResult(responder(request));
        }
    }
}
