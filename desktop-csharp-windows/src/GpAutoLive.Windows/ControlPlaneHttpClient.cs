using System.Buffers;
using System.Net;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using GpAutoLive.Contracts;

namespace GpAutoLive.Windows;

/// <summary>控制面 HTTP 客户端的有限配置。</summary>
public sealed record ControlPlaneHttpClientOptions
{
    public required Uri BaseUri { get; init; }

    public TimeSpan RequestTimeout { get; init; } = TimeSpan.FromSeconds(15);

    public int MaxResponseBytes { get; init; } = 1024 * 1024;

    /// <summary>仅允许开发/测试使用的 loopback HTTP；正式地址必须 HTTPS。</summary>
    public bool AllowLoopbackHttp { get; init; }

    /// <summary>仅允许显式测试环境访问固定的远程 HTTP 控制面。</summary>
    public bool AllowDevelopmentTestHttp { get; init; }

    /// <summary>判断是否为唯一允许的远程开发测试控制面地址。</summary>
    public static bool IsFixedDevelopmentTestHttp(Uri? uri) =>
        uri is not null
        && uri.Scheme == Uri.UriSchemeHttp
        && string.Equals(uri.Host, "101.96.208.132", StringComparison.Ordinal)
        && uri.Port == 9090
        && uri.AbsolutePath == "/"
        && string.IsNullOrEmpty(uri.UserInfo)
        && string.IsNullOrEmpty(uri.Query)
        && string.IsNullOrEmpty(uri.Fragment);

    internal void Validate()
    {
        if (BaseUri is null
            || !BaseUri.IsAbsoluteUri
            || BaseUri.UserInfo.Length > 0
            || !string.IsNullOrEmpty(BaseUri.Query)
            || !string.IsNullOrEmpty(BaseUri.Fragment)
            || BaseUri.AbsolutePath.Contains("..", StringComparison.Ordinal))
        {
            throw new ArgumentException("控制面地址必须是无凭据、无查询参数的绝对地址。", nameof(BaseUri));
        }

        var loopbackHttp = BaseUri.Scheme == Uri.UriSchemeHttp && IPAddress.TryParse(BaseUri.Host, out var address)
            ? IPAddress.IsLoopback(address)
            : BaseUri.Scheme == Uri.UriSchemeHttp
                && (string.Equals(BaseUri.Host, "localhost", StringComparison.OrdinalIgnoreCase)
                    || string.Equals(BaseUri.Host, "[::1]", StringComparison.OrdinalIgnoreCase));
        var fixedDevelopmentTestHttp = AllowDevelopmentTestHttp && IsFixedDevelopmentTestHttp(BaseUri);
        if (BaseUri.Scheme != Uri.UriSchemeHttps
            && !(AllowLoopbackHttp && loopbackHttp)
            && !fixedDevelopmentTestHttp)
        {
            throw new ArgumentException(
                "正式控制面地址必须使用 HTTPS；HTTP 只允许显式开启的 loopback 或固定远程测试地址。",
                nameof(BaseUri));
        }

        if (RequestTimeout <= TimeSpan.Zero || RequestTimeout > TimeSpan.FromMinutes(2))
        {
            throw new ArgumentOutOfRangeException(nameof(RequestTimeout), "控制面请求超时必须在 0 秒到 120 秒之间。");
        }

        if (MaxResponseBytes is < 1 or > 4 * 1024 * 1024)
        {
            throw new ArgumentOutOfRangeException(nameof(MaxResponseBytes), "控制面响应上限必须在 1 字节到 4 MiB 之间。");
        }
    }
}

/// <summary>控制面调用的脱敏错误；不保留 URL、请求体或响应原文。</summary>
public sealed record ControlPlaneHttpError(
    string Code,
    string Message,
    int Status,
    string RequestId,
    bool Retryable = false);

/// <summary>控制面调用结果；失败时 Value 始终为空。</summary>
public sealed record ControlPlaneHttpResult<T>(
    bool IsSuccess,
    T? Value,
    ControlPlaneHttpError? Error)
{
    public static ControlPlaneHttpResult<T> Succeeded(T value) => new(true, value, null);

    public static ControlPlaneHttpResult<T> Failed(ControlPlaneHttpError error) => new(false, default, error);
}

/// <summary>
/// Windows 桌面端控制面 HTTP 传输边界。
/// 只发送版本化 Contracts DTO，响应按 1 MiB 默认上限读取并严格拒绝未知 JSON 字段；
/// 重试由 Core 的会话状态机决定，本类型不自行重复非幂等操作。
/// </summary>
public sealed class ControlPlaneHttpClient
{
    private const int MaxRequestIdLength = AuthInputLimits.RequestIdMaxLength;
    private const int MaxErrorMessageLength = 1_024;
    private static readonly UTF8Encoding StrictUtf8 = new(false, true);
    private readonly HttpClient _httpClient;
    private readonly ControlPlaneHttpClientOptions _options;

    public ControlPlaneHttpClient(
        HttpClient httpClient,
        ControlPlaneHttpClientOptions options)
    {
        _httpClient = httpClient ?? throw new ArgumentNullException(nameof(httpClient));
        _options = options ?? throw new ArgumentNullException(nameof(options));
        _options.Validate();
    }

    public async Task<ControlPlaneHttpResult<DesktopLoginResponseDto>> LoginAsync(
        DesktopLoginRequestDto? request,
        CancellationToken cancellationToken = default)
    {
        if (!AuthContractValidation.TryValidateLogin(request, out var error))
        {
            return ControlPlaneHttpResult<DesktopLoginResponseDto>.Failed(ToHttpError(error!));
        }

        return await SendAsync<DesktopLoginRequestDto, DesktopLoginResponseDto>(
            HttpMethod.Post,
            "/api/v1/client/auth/login",
            request,
            accessToken: null,
            idempotencyKey: null,
            ValidateLoginResponse,
            cancellationToken).ConfigureAwait(false);
    }

    public async Task<ControlPlaneHttpResult<RefreshTokenResponseDto>> RefreshAsync(
        RefreshTokenRequestDto? request,
        CancellationToken cancellationToken = default)
    {
        if (!AuthContractValidation.TryValidateRefreshToken(request?.RefreshToken, out var error))
        {
            return ControlPlaneHttpResult<RefreshTokenResponseDto>.Failed(ToHttpError(error!));
        }

        return await SendAsync<RefreshTokenRequestDto, RefreshTokenResponseDto>(
            HttpMethod.Post,
            "/api/v1/auth/refresh",
            request,
            accessToken: null,
            idempotencyKey: null,
            ValidateRefreshResponse,
            cancellationToken).ConfigureAwait(false);
    }

    public async Task<ControlPlaneHttpResult<LogoutResponseDto>> LogoutAsync(
        LogoutRequestDto? request,
        CancellationToken cancellationToken = default)
    {
        if (!AuthContractValidation.TryValidateLogout(request, out var error))
        {
            return ControlPlaneHttpResult<LogoutResponseDto>.Failed(ToHttpError(error!));
        }

        return await SendAsync<LogoutRequestDto, LogoutResponseDto>(
            HttpMethod.Post,
            "/api/v1/auth/logout",
            request,
            accessToken: null,
            idempotencyKey: null,
            ValidateLogoutResponse,
            cancellationToken).ConfigureAwait(false);
    }

    public async Task<ControlPlaneHttpResult<ActivateDeviceResponseDto>> ActivateAsync(
        string? accessToken,
        ActivateDeviceRequestDto? request,
        string? idempotencyKey = null,
        CancellationToken cancellationToken = default)
    {
        if (!TryValidateActivationInputs(accessToken, request, out var error))
        {
            return ControlPlaneHttpResult<ActivateDeviceResponseDto>.Failed(ToHttpError(error!));
        }

        return await SendAsync<ActivateDeviceRequestDto, ActivateDeviceResponseDto>(
            HttpMethod.Post,
            "/api/v1/client/activate",
            request,
            accessToken,
            idempotencyKey ?? CreateRequestId(),
            ValidateActivationResponse,
            cancellationToken).ConfigureAwait(false);
    }

    public async Task<ControlPlaneHttpResult<HeartbeatResponseDto>> HeartbeatAsync(
        string? accessToken,
        HeartbeatRequestDto? request,
        string? idempotencyKey = null,
        CancellationToken cancellationToken = default)
    {
        if (!TryValidateHeartbeatInputs(accessToken, request, out var error))
        {
            return ControlPlaneHttpResult<HeartbeatResponseDto>.Failed(ToHttpError(error!));
        }

        return await SendAsync<HeartbeatRequestDto, HeartbeatResponseDto>(
            HttpMethod.Post,
            "/api/v1/client/heartbeat",
            request,
            accessToken,
            idempotencyKey ?? CreateRequestId(),
            ValidateHeartbeatResponse,
            cancellationToken).ConfigureAwait(false);
    }

    private async Task<ControlPlaneHttpResult<TResponse>> SendAsync<TRequest, TResponse>(
        HttpMethod method,
        string path,
        TRequest? body,
        string? accessToken,
        string? idempotencyKey,
        Func<TResponse?, ControlPlaneHttpError?> validateResponse,
        CancellationToken cancellationToken)
    {
        var requestId = CreateRequestId();
        if (!TryValidateToken(accessToken, out var tokenError))
        {
            return ControlPlaneHttpResult<TResponse>.Failed(tokenError! with { RequestId = requestId });
        }

        if (!TryValidateIdempotencyKey(idempotencyKey, out var idempotencyError))
        {
            return ControlPlaneHttpResult<TResponse>.Failed(idempotencyError! with { RequestId = requestId });
        }

        string? requestJson = null;
        if (body is not null)
        {
            try
            {
                requestJson = JsonSerializer.Serialize(body, ContractJson.CreateOptions());
            }
            catch (JsonException)
            {
                return ControlPlaneHttpResult<TResponse>.Failed(Error(
                    AuthErrorCodes.InvalidRequest,
                    "控制面请求格式无效。",
                    400,
                    requestId,
                    retryable: false));
            }
        }

        using var request = new HttpRequestMessage(method, BuildUri(path));
        request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        request.Headers.TryAddWithoutValidation("X-Request-Id", requestId);
        if (accessToken is not null)
        {
            request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", accessToken);
        }

        if (idempotencyKey is not null)
        {
            request.Headers.TryAddWithoutValidation("Idempotency-Key", idempotencyKey);
        }

        if (requestJson is not null)
        {
            request.Content = new StringContent(requestJson, StrictUtf8, "application/json");
        }

        using var timeoutCancellation = new CancellationTokenSource(_options.RequestTimeout);
        using var linkedCancellation = CancellationTokenSource.CreateLinkedTokenSource(
            cancellationToken,
            timeoutCancellation.Token);

        HttpResponseMessage response;
        try
        {
            response = await _httpClient.SendAsync(
                request,
                HttpCompletionOption.ResponseHeadersRead,
                linkedCancellation.Token).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return ControlPlaneHttpResult<TResponse>.Failed(
                Error(
                    cancellationToken.IsCancellationRequested
                        ? "CONTROL_PLANE_CANCELLED"
                        : "CONTROL_PLANE_TIMEOUT",
                    cancellationToken.IsCancellationRequested
                        ? "控制面请求已取消。"
                        : "控制面请求超时。",
                    cancellationToken.IsCancellationRequested ? 0 : 408,
                    requestId,
                    retryable: true));
        }
        catch (HttpRequestException)
        {
            return ControlPlaneHttpResult<TResponse>.Failed(
                Error("NETWORK_ERROR", "网络异常，暂时无法连接到控制面。", 0, requestId, retryable: true));
        }

        using (response)
        {
            var bodyResult = await ReadBoundedBodyAsync(
                response,
                _options.MaxResponseBytes,
                linkedCancellation.Token).ConfigureAwait(false);
            if (!bodyResult.IsSuccess)
            {
                var bodyError = bodyResult.Error! with { RequestId = requestId };
                if (bodyError.Code == "CONTROL_PLANE_CANCELLED"
                    && timeoutCancellation.IsCancellationRequested
                    && !cancellationToken.IsCancellationRequested)
                {
                    bodyError = bodyError with
                    {
                        Code = "CONTROL_PLANE_TIMEOUT",
                        Message = "控制面请求超时。",
                        Status = 408,
                    };
                }

                return ControlPlaneHttpResult<TResponse>.Failed(bodyError);
            }

            if (!response.IsSuccessStatusCode)
            {
                return ControlPlaneHttpResult<TResponse>.Failed(ParseRemoteError(
                    bodyResult.Text,
                    (int)response.StatusCode,
                    requestId));
            }

            TResponse? parsed;
            try
            {
                parsed = string.IsNullOrWhiteSpace(bodyResult.Text)
                    ? default
                    : JsonSerializer.Deserialize<TResponse>(bodyResult.Text, ContractJson.CreateOptions());
            }
            catch (JsonException)
            {
                return ControlPlaneHttpResult<TResponse>.Failed(
                    Error(AuthErrorCodes.ResponseInvalid, "控制面返回了无效响应。", 502, requestId, retryable: false));
            }

            var responseError = validateResponse(parsed);
            return responseError is null
                ? ControlPlaneHttpResult<TResponse>.Succeeded(parsed!)
                : ControlPlaneHttpResult<TResponse>.Failed(responseError with { RequestId = requestId });
        }
    }

    private Uri BuildUri(string path)
    {
        var baseUri = _options.BaseUri.AbsoluteUri.TrimEnd('/');
        return new Uri(baseUri + path, UriKind.Absolute);
    }

    private static async Task<BoundedBodyResult> ReadBoundedBodyAsync(
        HttpResponseMessage response,
        int limit,
        CancellationToken cancellationToken)
    {
        if (response.Content.Headers.ContentLength is long contentLength && contentLength > limit)
        {
            return BoundedBodyResult.Failed(Error(
                "CONTROL_PLANE_RESPONSE_TOO_LARGE",
                "控制面响应超过配置上限。",
                502,
                string.Empty,
                retryable: false));
        }

        try
        {
            await using var stream = await response.Content.ReadAsStreamAsync(cancellationToken).ConfigureAwait(false);
            var rented = ArrayPool<byte>.Shared.Rent(64 * 1024);
            using var output = new MemoryStream();
            try
            {
                while (true)
                {
                    var read = await stream.ReadAsync(rented.AsMemory(), cancellationToken).ConfigureAwait(false);
                    if (read == 0)
                    {
                        break;
                    }

                    if (output.Length + read > limit)
                    {
                        return BoundedBodyResult.Failed(Error(
                            "CONTROL_PLANE_RESPONSE_TOO_LARGE",
                            "控制面响应超过配置上限。",
                            502,
                            string.Empty,
                            retryable: false));
                    }

                    output.Write(rented, 0, read);
                }

                return BoundedBodyResult.Succeeded(StrictUtf8.GetString(output.GetBuffer(), 0, checked((int)output.Length)));
            }
            finally
            {
                ArrayPool<byte>.Shared.Return(rented);
            }
        }
        catch (OperationCanceledException)
        {
            return BoundedBodyResult.Failed(Error(
                "CONTROL_PLANE_CANCELLED",
                "控制面响应读取已取消。",
                0,
                string.Empty,
                retryable: true));
        }
        catch (DecoderFallbackException)
        {
            return BoundedBodyResult.Failed(Error(
                AuthErrorCodes.ResponseInvalid,
                "控制面响应编码无效。",
                502,
                string.Empty,
                retryable: false));
        }
        catch (HttpRequestException)
        {
            return BoundedBodyResult.Failed(Error(
                "NETWORK_ERROR",
                "控制面响应读取失败。",
                0,
                string.Empty,
                retryable: true));
        }
        catch (IOException)
        {
            return BoundedBodyResult.Failed(Error(
                "NETWORK_ERROR",
                "控制面响应读取失败。",
                0,
                string.Empty,
                retryable: true));
        }
    }

    private static ControlPlaneHttpError ParseRemoteError(string body, int status, string requestId)
    {
        try
        {
            var error = JsonSerializer.Deserialize<ControlPlaneErrorDto>(body, ContractJson.CreateOptions());
            if (error is not null
                && !string.IsNullOrWhiteSpace(error.Code)
                && !string.IsNullOrWhiteSpace(error.Message)
                && error.Code.Length <= 128)
            {
                var message = error.Message.Length <= MaxErrorMessageLength
                    ? error.Message
                    : "控制面返回了无效错误。";
                return Error(error.Code, message, status, requestId, status is 408 or 429 or >= 500);
            }
        }
        catch (JsonException)
        {
        }

        return Error("HTTP_ERROR", "控制面返回了无法识别的错误。", status, requestId, status is 408 or 429 or >= 500);
    }

    private static ControlPlaneHttpError? ValidateLoginResponse(DesktopLoginResponseDto? response)
    {
        if (response is null
            || !ValidRequestId(response.RequestId)
            || response.Tokens is null
            || !AuthContractValidation.TryValidateSessionTokens(response.Tokens, DateTimeOffset.UtcNow, out _)
            || response.User is null
            || string.IsNullOrWhiteSpace(response.User.Id)
            || string.IsNullOrWhiteSpace(response.User.Username))
        {
            return Error(AuthErrorCodes.ResponseInvalid, "控制面返回了无效登录响应。", 502, string.Empty, false);
        }

        return null;
    }

    private static ControlPlaneHttpError? ValidateRefreshResponse(RefreshTokenResponseDto? response) =>
        response is not null
        && ValidRequestId(response.RequestId)
        && response.Tokens is not null
        && AuthContractValidation.TryValidateSessionTokens(response.Tokens, DateTimeOffset.UtcNow, out _)
            ? null
            : Error(AuthErrorCodes.ResponseInvalid, "控制面返回了无效刷新响应。", 502, string.Empty, false);

    private static ControlPlaneHttpError? ValidateLogoutResponse(LogoutResponseDto? response) =>
        response is not null && ValidRequestId(response.RequestId) && response.Success
            ? null
            : Error(AuthErrorCodes.ResponseInvalid, "控制面返回了无效退出响应。", 502, string.Empty, false);

    private static ControlPlaneHttpError? ValidateActivationResponse(ActivateDeviceResponseDto? response) =>
        response is not null
        && ValidRequestId(response.RequestId)
        && response.Device is not null
        && string.Equals(response.Device.Product, ControlPlaneContractValues.Product, StringComparison.Ordinal)
        && ValidText(response.Device.Id, AuthInputLimits.DeviceIdMaxLength)
        && ValidText(response.Device.UserId, AuthInputLimits.DeviceIdMaxLength)
        && ValidText(response.Device.DeviceName, AuthInputLimits.DeviceNameMaxLength)
        && ValidText(response.Device.Platform, AuthInputLimits.PlatformMaxLength)
        && ValidText(response.Device.AppVersion, AuthInputLimits.VersionMaxLength)
        && ValidText(response.Device.Status, 64)
            ? null
            : Error(AuthErrorCodes.ResponseInvalid, "控制面返回了无效设备响应。", 502, string.Empty, false);

    private static ControlPlaneHttpError? ValidateHeartbeatResponse(HeartbeatResponseDto? response) =>
        response is not null
        && ValidRequestId(response.RequestId)
        && ValidText(response.DeviceStatus, AuthInputLimits.HeartbeatPlaybackStateMaxLength)
            ? null
            : Error(AuthErrorCodes.ResponseInvalid, "控制面返回了无效心跳响应。", 502, string.Empty, false);

    private static bool ValidText(string? value, int maximumLength) =>
        !string.IsNullOrWhiteSpace(value)
        && value.Length <= maximumLength
        && !value.Contains('\0')
        && !value.Any(char.IsControl);

    private static bool ValidRequestId(string? requestId) =>
        !string.IsNullOrWhiteSpace(requestId)
        && requestId.Length <= MaxRequestIdLength
        && !requestId.Any(char.IsControl);

    private static bool TryValidateToken(string? token, out ControlPlaneHttpError? error)
    {
        if (token is not null
            && (string.IsNullOrWhiteSpace(token)
                || token.Length > AuthInputLimits.TokenMaxLength
                || token.Contains('\0')))
        {
            error = Error(AuthErrorCodes.InvalidRequest, "控制面访问凭据格式无效。", 400, string.Empty, false);
            return false;
        }

        error = null;
        return true;
    }

    private static bool TryValidateRequiredToken(string? token, out ControlPlaneErrorDto? error)
    {
        if (string.IsNullOrWhiteSpace(token)
            || token.Length > AuthInputLimits.TokenMaxLength
            || token.Contains('\0'))
        {
            error = new ControlPlaneErrorDto(
                AuthErrorCodes.InvalidRequest,
                "控制面访问凭据格式无效。",
                400);
            return false;
        }

        error = null;
        return true;
    }

    private static bool TryValidateActivationInputs(
        string? accessToken,
        ActivateDeviceRequestDto? request,
        out ControlPlaneErrorDto? error)
    {
        if (!AuthContractValidation.TryValidateDeviceRegistration(request?.Device, out error))
        {
            return false;
        }

        return TryValidateRequiredToken(accessToken, out error);
    }

    private static bool TryValidateHeartbeatInputs(
        string? accessToken,
        HeartbeatRequestDto? request,
        out ControlPlaneErrorDto? error)
    {
        if (!AuthContractValidation.TryValidateHeartbeat(request, out error))
        {
            return false;
        }

        return TryValidateRequiredToken(accessToken, out error);
    }

    private static bool TryValidateIdempotencyKey(string? key, out ControlPlaneHttpError? error)
    {
        if (key is not null
            && (string.IsNullOrWhiteSpace(key)
                || key.Length > MaxRequestIdLength
                || key.Any(char.IsControl)))
        {
            error = Error(AuthErrorCodes.InvalidRequest, "控制面幂等键格式无效。", 400, string.Empty, false);
            return false;
        }

        error = null;
        return true;
    }

    private static string CreateRequestId() => Guid.NewGuid().ToString("N");

    private static ControlPlaneHttpError ToHttpError(ControlPlaneErrorDto error) =>
        Error(error.Code, error.Message, error.Status, string.Empty, error.IsTransient);

    private static ControlPlaneHttpError Error(
        string code,
        string message,
        int status,
        string requestId,
        bool retryable) =>
        new(code, message, status, requestId, retryable);

    private readonly record struct BoundedBodyResult(bool IsSuccess, string Text, ControlPlaneHttpError? Error)
    {
        public static BoundedBodyResult Succeeded(string text) => new(true, text, null);

        public static BoundedBodyResult Failed(ControlPlaneHttpError error) => new(false, string.Empty, error);
    }
}
