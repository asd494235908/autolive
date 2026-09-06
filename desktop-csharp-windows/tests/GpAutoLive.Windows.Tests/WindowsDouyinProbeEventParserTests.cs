using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsDouyinProbeEventParserTests
{
    private static readonly byte[] OnePixelPng =
    [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
        0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
        0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41,
        0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
        0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99,
        0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
        0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82
    ];

    [TestMethod]
    public void Rust_ndjson_auth_qr_decodes_bounded_png_and_expiry()
    {
        var expiresAt = DateTimeOffset.UtcNow.AddMinutes(5).ToUnixTimeMilliseconds();
        var encoded = Convert.ToBase64String(OnePixelPng);
        var line = $"{{\"v\":1,\"type\":\"event\",\"event\":\"auth.qr\",\"payload\":{{\"png_base64\":\"{encoded}\",\"expires_at_unix_ms\":{expiresAt}}}}}";

        var ok = WindowsDouyinProbeEventParser.TryParse(line, out var parsed, out var error);

        Assert.IsTrue(ok, error);
        Assert.AreEqual(WindowsDouyinProbeEventKind.QrIssued, parsed!.Kind);
        Assert.AreEqual("auth.qr", parsed.Name);
        CollectionAssert.AreEqual(OnePixelPng, parsed.QrPngBytes);
        Assert.AreEqual(DateTimeOffset.FromUnixTimeMilliseconds(expiresAt), parsed.QrExpiresAtUtc);
    }

    [TestMethod]
    public void Rust_ndjson_auth_qr_rejects_unknown_fields_invalid_base64_and_non_png()
    {
        var expiresAt = DateTimeOffset.UtcNow.AddMinutes(5).ToUnixTimeMilliseconds();
        var encoded = Convert.ToBase64String(OnePixelPng);
        var prefix = $"{{\"v\":1,\"type\":\"event\",\"event\":\"auth.qr\",\"payload\":{{\"png_base64\":\"{encoded}\",\"expires_at_unix_ms\":{expiresAt}";

        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            prefix + ",\"unexpected\":true}}}",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            prefix + ",\"png_base64\":\"bad\\nbase64\"}}}",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            $"{{\"v\":1,\"type\":\"event\",\"event\":\"auth.qr\",\"payload\":{{\"png_base64\":\"{Convert.ToBase64String([0x01, 0x02])}\",\"expires_at_unix_ms\":{expiresAt}}}}}",
            out _,
            out _));
    }

    [TestMethod]
    public void Rust_ndjson_auth_state_accepts_waiting_and_confirmed_states()
    {
        Assert.IsTrue(WindowsDouyinProbeEventParser.TryParse(
            "{\"v\":1,\"type\":\"event\",\"event\":\"auth.state\",\"payload\":{\"state\":\"waiting\"}}",
            out var waiting,
            out var waitingError),
            waitingError);
        Assert.AreEqual(WindowsDouyinProbeEventKind.AuthState, waiting!.Kind);
        Assert.AreEqual("waiting", waiting.State);

        Assert.IsTrue(WindowsDouyinProbeEventParser.TryParse(
            "{\"v\":1,\"type\":\"event\",\"event\":\"auth.state\",\"payload\":{\"state\":\"confirmed\"}}",
            out var confirmed,
            out var confirmedError),
            confirmedError);
        Assert.AreEqual("confirmed", confirmed!.State);
    }

    [TestMethod]
    public void Rust_ndjson_auth_state_rejects_unknown_state_and_extra_fields()
    {
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            "{\"v\":1,\"type\":\"event\",\"event\":\"auth.state\",\"payload\":{\"state\":\"logged_in\"}}",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            "{\"v\":1,\"type\":\"event\",\"event\":\"auth.state\",\"payload\":{\"state\":\"confirmed\",\"user_id\":\"secret\"}}",
            out _,
            out _));
    }

    [TestMethod]
    public void Chat_event_parses_only_bounded_redacted_metadata()
    {
        Assert.IsTrue(
            WindowsDouyinProbeEventParser.TryParse(
                "{\"event\":\"chat_received\",\"message_type\":\"WebcastChatMessage\",\"room_id\":\"12345\",\"message_id\":\"42\",\"sender_id\":\"user-1\",\"text_length\":12,\"is_self\":false,\"is_replay\":false}",
                out var parsed,
                out var error),
            error);

        Assert.AreEqual(WindowsDouyinProbeEventKind.ChatReceived, parsed!.Kind);
        Assert.AreEqual("chat_received", parsed.Name);
        Assert.AreEqual("42", parsed.ChatMetadata!.MessageId);
        Assert.AreEqual("12345", parsed.ChatMetadata.RoomId);
        Assert.AreEqual(12, parsed.ChatMetadata.TextLength);
        Assert.IsFalse(parsed.GetType().GetProperties().Any(property => property.Name.Contains("content", StringComparison.OrdinalIgnoreCase)));
    }

    [TestMethod]
    public void Chat_event_rejects_body_fields_and_missing_fixed_metadata()
    {
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            "{\"event\":\"chat_received\",\"message_type\":\"WebcastChatMessage\",\"room_id\":\"12345\",\"message_id\":\"42\",\"sender_id\":\"user-1\",\"text_length\":12,\"text\":\"secret\",\"is_self\":false,\"is_replay\":false}",
            out _,
            out var bodyError));
        StringAssert.Contains(bodyError!, "不接受正文");

        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            "{\"event\":\"chat_received\",\"message_type\":\"WebcastChatMessage\"}",
            out _,
            out _));
    }

    [TestMethod]
    public void Rust_ndjson_live_chat_event_is_projected_to_redacted_metadata()
    {
        Assert.IsTrue(
            WindowsDouyinProbeEventParser.TryParse(
                "{\"v\":1,\"type\":\"event\",\"event\":\"live.chat\",\"session_id\":\"ls-1\",\"generation\":1,\"payload\":{\"msg_id\":\"msg-1\",\"received_at_unix_ms\":0,\"author_id\":\"author-1\",\"nickname\":\"观众\",\"content\":\"hello\"}}",
                out var parsed,
                out var error,
                "12345"),
            error);

        Assert.AreEqual(WindowsDouyinProbeEventKind.ChatReceived, parsed!.Kind);
        Assert.AreEqual("live.chat", parsed.Name);
        Assert.AreEqual("msg-1", parsed.ChatMetadata!.MessageId);
        Assert.AreEqual("author-1", parsed.ChatMetadata.SenderId);
        Assert.AreEqual(5, parsed.ChatMetadata.TextLength);
        Assert.AreEqual("12345", parsed.ChatMetadata.RoomId);
        Assert.AreEqual("ls-1", parsed.SessionId);
        Assert.AreEqual(1UL, parsed.Generation);
    }

    [TestMethod]
    public void Rust_ndjson_live_chat_requires_the_current_room_and_rejects_raw_top_level_body()
    {
        const string eventWithoutExpectedRoom = "{\"v\":1,\"type\":\"event\",\"event\":\"live.chat\",\"session_id\":\"ls-1\",\"generation\":1,\"payload\":{\"msg_id\":\"msg-1\",\"received_at_unix_ms\":0,\"author_id\":\"author-1\",\"nickname\":\"观众\",\"content\":\"hello\"}}";
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(eventWithoutExpectedRoom, out _, out _));

        const string eventWithTopLevelBody = "{\"v\":1,\"type\":\"event\",\"event\":\"live.chat\",\"session_id\":\"ls-1\",\"generation\":1,\"content\":\"secret\",\"payload\":{\"msg_id\":\"msg-1\",\"received_at_unix_ms\":0,\"author_id\":\"author-1\",\"nickname\":\"观众\",\"content\":\"hello\"}}";
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(eventWithTopLevelBody, out _, out _),
            "正文只能存在于受协议约束的 payload.content 中");
    }

    [TestMethod]
    public void Rust_ndjson_live_chat_rejects_unknown_envelope_or_payload_fields()
    {
        const string unknownEnvelope = "{\"v\":1,\"type\":\"event\",\"event\":\"live.chat\",\"session_id\":\"ls-1\",\"generation\":1,\"trace_id\":\"secret\",\"payload\":{\"msg_id\":\"msg-1\",\"received_at_unix_ms\":0,\"author_id\":\"author-1\",\"nickname\":\"观众\",\"content\":\"hello\"}}";
        const string unknownPayload = "{\"v\":1,\"type\":\"event\",\"event\":\"live.chat\",\"session_id\":\"ls-1\",\"generation\":1,\"payload\":{\"msg_id\":\"msg-1\",\"received_at_unix_ms\":0,\"author_id\":\"author-1\",\"nickname\":\"观众\",\"content\":\"hello\",\"raw_message\":\"secret\"}}";

        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            unknownEnvelope,
            out _,
            out _,
            expectedRoomId: "12345"));
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            unknownPayload,
            out _,
            out _,
            expectedRoomId: "12345"));
    }

    [TestMethod]
    public void Rust_ndjson_events_reject_a_stale_session_identity()
    {
        const string line = "{\"v\":1,\"type\":\"event\",\"event\":\"live.state\",\"session_id\":\"ls-2\",\"generation\":2,\"payload\":{\"state\":\"connected\"}}";

        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            line,
            out _,
            out _,
            expectedRoomId: null,
            expectedSessionId: "ls-1",
            expectedGeneration: 1));
    }

    [TestMethod]
    public void Rust_ndjson_live_state_validates_envelope_and_preserves_state_identity()
    {
        const string line = "{\"v\":1,\"type\":\"event\",\"event\":\"live.state\",\"session_id\":\"ls-1\",\"generation\":7,\"payload\":{\"state\":\"reconnecting\"}}";

        var ok = WindowsDouyinProbeEventParser.TryParse(line, out var parsed, out var error);

        Assert.IsTrue(ok, error);
        Assert.IsNotNull(parsed);
        Assert.AreEqual(WindowsDouyinProbeEventKind.LiveState, parsed!.Kind);
        Assert.AreEqual("live.state", parsed.Name);
        Assert.AreEqual("ls-1", parsed.SessionId);
        Assert.AreEqual(7UL, parsed.Generation);
        Assert.AreEqual("reconnecting", parsed.State);
    }

    [TestMethod]
    public void Rust_ndjson_live_state_rejects_unknown_state_or_missing_generation()
    {
        const string unknown = "{\"v\":1,\"type\":\"event\",\"event\":\"live.state\",\"session_id\":\"ls-1\",\"generation\":7,\"payload\":{\"state\":\"connected-but-not-contract\"}}";
        const string missingGeneration = "{\"v\":1,\"type\":\"event\",\"event\":\"live.state\",\"session_id\":\"ls-1\",\"payload\":{\"state\":\"connected\"}}";

        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(unknown, out _, out _));
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(missingGeneration, out _, out _));
    }

    [TestMethod]
    public void Rust_ndjson_live_state_rejects_unknown_envelope_or_payload_fields()
    {
        const string unknownEnvelope = "{\"v\":1,\"type\":\"event\",\"event\":\"live.state\",\"session_id\":\"ls-1\",\"generation\":7,\"trace_id\":\"secret\",\"payload\":{\"state\":\"connected\"}}";
        const string unknownPayload = "{\"v\":1,\"type\":\"event\",\"event\":\"live.state\",\"session_id\":\"ls-1\",\"generation\":7,\"payload\":{\"state\":\"connected\",\"room_title\":\"secret\"}}";

        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(unknownEnvelope, out _, out _));
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(unknownPayload, out _, out _));
    }

    [TestMethod]
    public void Rust_ndjson_live_gap_accepts_bounded_reason_and_count()
    {
        const string line = "{\"v\":1,\"type\":\"event\",\"event\":\"live.gap\",\"session_id\":\"ls-1\",\"generation\":7,\"payload\":{\"reason\":\"sidecar_backpressure\",\"dropped_count\":3}}";

        var ok = WindowsDouyinProbeEventParser.TryParse(
            line,
            out var parsed,
            out var error,
            expectedSessionId: "ls-1",
            expectedGeneration: 7);

        Assert.IsTrue(ok, error);
        Assert.AreEqual(WindowsDouyinProbeEventKind.LiveGap, parsed!.Kind);
        Assert.AreEqual("live.gap", parsed.Name);
        Assert.AreEqual("ls-1", parsed.SessionId);
        Assert.AreEqual(7UL, parsed.Generation);
        Assert.AreEqual("sidecar_backpressure", parsed.GapReason);
        Assert.AreEqual(3UL, parsed.GapDroppedCount);
    }

    [TestMethod]
    public void Rust_ndjson_live_gap_rejects_unknown_reason_zero_count_and_extra_fields()
    {
        const string prefix = "{\"v\":1,\"type\":\"event\",\"event\":\"live.gap\",\"session_id\":\"ls-1\",\"generation\":7,\"payload\":{";

        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            prefix + "\"reason\":\"unknown\",\"dropped_count\":1}}",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            prefix + "\"reason\":\"reconnect\",\"dropped_count\":0}}",
            out _,
            out _));
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(
            prefix + "\"reason\":\"reconnect\",\"dropped_count\":1,\"extra\":true}}",
            out _,
            out _));
    }

    [TestMethod]
    public void Unknown_or_malformed_event_is_rejected()
    {
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse("{\"event\":\"unknown\"}", out _, out _));
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse("not-json", out _, out _));
        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse("{\"state\":\"waiting_qr\"}", out _, out _));
    }

    [TestMethod]
    public void Oversized_event_line_is_rejected_before_json_work()
    {
        var line = "{\"event\":\"qr_waiting\",\"padding\":\"" + new string('x', WindowsDouyinProbeEventParser.MaxLineBytes) + "\"}";

        Assert.IsFalse(WindowsDouyinProbeEventParser.TryParse(line, out _, out var error));
        StringAssert.Contains(error!, "超出边界");
    }
}
