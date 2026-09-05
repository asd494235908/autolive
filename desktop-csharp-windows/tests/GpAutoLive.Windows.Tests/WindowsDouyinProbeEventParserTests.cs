using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsDouyinProbeEventParserTests
{
    [TestMethod]
    public void Known_event_is_parsed_without_returning_payload_fields()
    {
        Assert.IsTrue(
            WindowsDouyinProbeEventParser.TryParse(
                "{\"event\":\"chat_received\",\"content\":\"secret\",\"message_id\":\"42\"}",
                out var parsed,
                out var error),
            error);

        Assert.AreEqual(WindowsDouyinProbeEventKind.ChatReceived, parsed!.Kind);
        Assert.AreEqual("chat_received", parsed.Name);
        Assert.IsFalse(parsed.GetType().GetProperties().Any(property => property.Name.Contains("content", StringComparison.OrdinalIgnoreCase)));
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
