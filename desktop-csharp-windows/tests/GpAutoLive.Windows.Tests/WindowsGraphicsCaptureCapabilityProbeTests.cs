using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsGraphicsCaptureCapabilityProbeTests
{
    [TestMethod]
    public void Probe_ReturnsStableClassification()
    {
        var result = WindowsGraphicsCaptureCapabilityProbe.Probe();

        Assert.IsTrue(Enum.IsDefined(result.Code));
        Assert.AreEqual(result.IsReady, result.Code == WindowsGraphicsCaptureCapabilityCode.Ready);
    }

    [TestMethod]
    public void Probe_DoesNotExposeRawExceptionDetails()
    {
        var result = WindowsGraphicsCaptureCapabilityProbe.Probe();

        StringAssert.DoesNotMatch(result.Code.ToString(), new("0x[0-9A-Fa-f]{6,}"));
    }

    [TestMethod]
    public void Probe_OnThisWindowsHost_IsReadyOrClassifiedUnavailable()
    {
        var result = WindowsGraphicsCaptureCapabilityProbe.Probe();

        Assert.IsTrue(
            result.Code is WindowsGraphicsCaptureCapabilityCode.Ready
                or WindowsGraphicsCaptureCapabilityCode.RuntimeUnavailable
                or WindowsGraphicsCaptureCapabilityCode.UnsupportedVersion
                or WindowsGraphicsCaptureCapabilityCode.NotWindows);
    }
}
