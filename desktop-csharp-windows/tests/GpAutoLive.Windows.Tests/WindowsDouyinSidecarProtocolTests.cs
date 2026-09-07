using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsDouyinSidecarProtocolTests
{
    [TestMethod]
    public void Live_open_serializes_the_frozen_request_and_binds_response_identity()
    {
        var ok = WindowsDouyinSidecarProtocol.TrySerializeLiveOpen(
            "open-1",
            "1234567890",
            3,
            out var line,
            out var error);

        Assert.IsTrue(ok, error);
        Assert.AreEqual(
            "{\"v\":1,\"id\":\"open-1\",\"op\":\"live.open\",\"payload\":{\"web_rid\":\"1234567890\",\"generation\":3}}",
            line);

        Assert.IsTrue(WindowsDouyinSidecarProtocol.TryParseLiveOpenResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"open-1\",\"ok\":true,\"result\":{\"session_id\":\"ls-3\",\"title\":\"测试直播\",\"live_status\":\"connected\"}}",
            "open-1",
            out var response,
            out var responseError),
            responseError);
        Assert.IsTrue(response!.IsSuccess);
        Assert.AreEqual("ls-3", response.SessionId);
        Assert.AreEqual("测试直播", response.Title);
        Assert.AreEqual("connected", response.LiveStatus);
    }

    [TestMethod]
    public void Live_open_rejects_wrong_identity_invalid_status_and_unknown_fields()
    {
        const string response = "{\"v\":1,\"type\":\"response\",\"request_id\":\"open-1\",\"ok\":true,\"result\":{\"session_id\":\"ls-3\",\"title\":\"测试直播\",\"live_status\":\"connected\"}}";
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TryParseLiveOpenResponse(response, "open-2", out _, out _));
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TryParseLiveOpenResponse(
            response.Replace("connected", "unknown", StringComparison.Ordinal),
            "open-1",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TryParseLiveOpenResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"open-1\",\"ok\":true,\"result\":{\"session_id\":\"ls-3\",\"title\":\"测试直播\",\"live_status\":\"connected\",\"unexpected\":true}}",
            "open-1",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TryParseLiveOpenResponse(
            response.Replace(",\"result\":", ",\"error\":{\"code\":\"room_not_live\",\"message\":\"脱敏\",\"retryable\":false,\"outcome\":\"not_sent\"},\"result\":", StringComparison.Ordinal),
            "open-1",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TrySerializeLiveOpen("open-1", "1abc", 1, out _, out _));
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TrySerializeLiveOpen("open-1", "1", 0, out _, out _));
    }

    [TestMethod]
    public void Live_open_failure_reuses_redacted_stable_error_contract()
    {
        Assert.IsTrue(WindowsDouyinSidecarProtocol.TryParseLiveOpenResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"open-1\",\"ok\":false,\"error\":{\"code\":\"room_not_live\",\"message\":\"脱敏\",\"retryable\":false,\"outcome\":\"not_sent\"}}",
            "open-1",
            out var response,
            out var error),
            error);
        Assert.IsFalse(response!.IsSuccess);
        Assert.AreEqual("room_not_live", response.ErrorCode);
        Assert.AreEqual(DouyinSendOutcome.NotSent, response.Outcome);
        Assert.IsNull(response.SessionId);
    }

    [TestMethod]
    public void Auth_qr_start_serializes_fixed_timeout_and_parses_empty_success_result()
    {
        Assert.IsTrue(WindowsDouyinSidecarProtocol.TrySerializeAuthQrStart(
            "qr-1",
            out var line,
            out var serializeError),
            serializeError);
        Assert.AreEqual(
            "{\"v\":1,\"id\":\"qr-1\",\"op\":\"auth.qr.start\",\"payload\":{\"timeout_ms\":300000}}",
            line);

        Assert.IsTrue(WindowsDouyinSidecarProtocol.TryParseCommandResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"qr-1\",\"ok\":true,\"result\":{}}",
            "qr-1",
            out var response,
            out var responseError),
            responseError);
        Assert.IsTrue(response!.IsSuccess);
    }

    [TestMethod]
    public void Auth_qr_start_rejects_mismatched_request_and_nonempty_result()
    {
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TryParseCommandResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"qr-2\",\"ok\":true,\"result\":{}}",
            "qr-1",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TryParseCommandResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"qr-1\",\"ok\":true,\"result\":{\"unexpected\":true}}",
            "qr-1",
            out _,
            out _));
    }

    [TestMethod]
    public void Canonical_stop_commands_serialize_bounded_payloads()
    {
        Assert.IsTrue(WindowsDouyinSidecarProtocol.TrySerializeAuthCancel(
            "cancel-1",
            out var cancel,
            out var cancelError), cancelError);
        Assert.AreEqual(
            "{\"v\":1,\"id\":\"cancel-1\",\"op\":\"auth.cancel\",\"payload\":{}}",
            cancel);

        Assert.IsTrue(WindowsDouyinSidecarProtocol.TrySerializeLiveClose(
            "close-1",
            "ls-3",
            3,
            out var close,
            out var closeError), closeError);
        Assert.AreEqual(
            "{\"v\":1,\"id\":\"close-1\",\"op\":\"live.close\",\"payload\":{\"session_id\":\"ls-3\",\"generation\":3}}",
            close);

        Assert.IsTrue(WindowsDouyinSidecarProtocol.TrySerializeShutdown(
            "shutdown-1",
            out var shutdown,
            out var shutdownError), shutdownError);
        Assert.AreEqual(
            "{\"v\":1,\"id\":\"shutdown-1\",\"op\":\"shutdown\",\"payload\":{}}",
            shutdown);
        Assert.IsTrue(WindowsDouyinSidecarProtocol.TrySerializeAuthLogout(
            "logout-1",
            out var logout,
            out var logoutError), logoutError);
        Assert.AreEqual(
            "{\"v\":1,\"id\":\"logout-1\",\"op\":\"auth.logout\",\"payload\":{}}",
            logout);
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TrySerializeLiveClose(
            "close-1",
            "ls-3",
            0,
            out _,
            out _));
    }

    [TestMethod]
    public void Chat_send_serializes_the_frozen_ndjson_request()
    {
        var request = new WindowsDouyinChatSendRequest(
            "ls-1",
            7,
            "chat-1",
            " 收到啦 ");

        var ok = WindowsDouyinSidecarProtocol.TrySerializeChatSend(request, out var line, out var error);

        Assert.IsTrue(ok, error);
        Assert.AreEqual(
            "{\"v\":1,\"id\":\"chat-1\",\"op\":\"chat.send\",\"payload\":{\"session_id\":\"ls-1\",\"generation\":7,\"client_action_id\":\"chat-1\",\"content\":\"收到啦\"}}",
            line);
    }

    [TestMethod]
    public void Chat_send_rejects_protocol_boundaries_before_serializing()
    {
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TrySerializeChatSend(
            new WindowsDouyinChatSendRequest("ls-1", 7, "chat-1", new string('x', 101)),
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TrySerializeChatSend(
            new WindowsDouyinChatSendRequest("ls-1", 7, "chat-1", "line\nfeed"),
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TrySerializeChatSend(
            new WindowsDouyinChatSendRequest("ls-1", 0, "chat-1", "收到"),
            out _,
            out _));
    }

    [TestMethod]
    public void Chat_send_response_maps_success_and_unknown_outcomes_without_message_body()
    {
        Assert.IsTrue(WindowsDouyinSidecarProtocol.TryParseResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"chat-1\",\"ok\":true,\"result\":{\"state\":\"accepted\",\"client_action_id\":\"chat-1\",\"platform_status_code\":200}}",
            out var accepted,
            out var acceptedError),
            acceptedError);
        Assert.AreEqual(DouyinSendOutcome.Accepted, accepted!.Outcome);
        Assert.AreEqual("chat-1", accepted.ClientActionId);
        Assert.AreEqual(200, accepted.PlatformStatusCode);

        Assert.IsTrue(WindowsDouyinSidecarProtocol.TryParseResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"chat-2\",\"ok\":false,\"error\":{\"code\":\"transport_after_dispatch\",\"message\":\"脱敏\",\"retryable\":false,\"outcome\":\"unknown\"}}",
            out var unknown,
            out var unknownError),
            unknownError);
        Assert.AreEqual(DouyinSendOutcome.OutcomeUnknown, unknown!.Outcome);
        Assert.AreEqual("transport_after_dispatch", unknown.ErrorCode);
        Assert.IsNull(unknown.ClientActionId);

        Assert.IsTrue(WindowsDouyinSidecarProtocol.TryParseResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"chat-3\",\"ok\":false,\"error\":{\"code\":\"rate_limited\",\"message\":\"脱敏\",\"retryable\":true,\"outcome\":\"rejected\"}}",
            out var rateLimited,
            out var rateLimitedError),
            rateLimitedError);
        Assert.IsTrue(rateLimited!.IsRetryable);
    }

    [TestMethod]
    public void Chat_send_response_rejects_unknown_outcome_or_malformed_envelope()
    {
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TryParseResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"chat-1\",\"ok\":true,\"result\":{\"state\":\"maybe\",\"client_action_id\":\"chat-1\"}}",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TryParseResponse(
            "{\"v\":1,\"type\":\"event\",\"event\":\"live.chat\"}",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TryParseResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"chat-1\",\"ok\":false,\"error\":{\"code\":\"send_rejected\"}}",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TryParseResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"chat-1\",\"ok\":true,\"result\":{\"state\":\"accepted\",\"client_action_id\":\"chat-1\",\"unexpected\":true}}",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinSidecarProtocol.TryParseResponse(
            "{\"v\":1,\"type\":\"response\",\"request_id\":\"chat-1\",\"ok\":false,\"error\":{\"code\":\"send_rejected\",\"message\":\"脱敏\",\"retryable\":false,\"outcome\":\"rejected\",\"unexpected\":true}}",
            out _,
            out _));
    }
}
