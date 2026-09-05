using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsVirtualCameraSidecarStatusParserTests
{
    [TestMethod]
    public void Parses_bounded_client_count()
    {
        Assert.IsTrue(WindowsVirtualCameraSidecarStatusParser.TryParseClientCount("GPAKVC_CLIENTS 12"u8, out var count));
        Assert.AreEqual(12U, count);
        Assert.IsTrue(WindowsVirtualCameraSidecarStatusParser.TryParseClientCount("GPAKVC_CLIENTS 0\r"u8, out count));
        Assert.AreEqual(0U, count);
        Assert.IsTrue(WindowsVirtualCameraSidecarStatusParser.TryParseClientCount("GPAKVC_CLIENTS 1024"u8, out count));
        Assert.AreEqual(1024U, count);
    }

    [TestMethod]
    public void Rejects_malformed_or_unbounded_client_count()
    {
        Assert.IsFalse(WindowsVirtualCameraSidecarStatusParser.TryParseClientCount("GPAKVC_CLIENTS "u8, out _));
        Assert.IsFalse(WindowsVirtualCameraSidecarStatusParser.TryParseClientCount("GPAKVC_CLIENTS -1"u8, out _));
        Assert.IsFalse(WindowsVirtualCameraSidecarStatusParser.TryParseClientCount("GPAKVC_CLIENTS 1025"u8, out _));
        Assert.IsFalse(WindowsVirtualCameraSidecarStatusParser.TryParseClientCount("GPAKVC_CLIENTS 1 extra"u8, out _));
        Assert.IsFalse(WindowsVirtualCameraSidecarStatusParser.TryParseClientCount("GPAKVC_CLIENTS 1\n"u8, out _));
        Assert.IsFalse(WindowsVirtualCameraSidecarStatusParser.TryParseClientCount("OTHER 1"u8, out _));
    }
}
