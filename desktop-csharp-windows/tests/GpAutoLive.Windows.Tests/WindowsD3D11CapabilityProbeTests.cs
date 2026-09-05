using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsD3D11CapabilityProbeTests
{
    [TestMethod]
    public void Probe_returns_a_bounded_classification_without_raw_hresult()
    {
        var result = WindowsD3D11CapabilityProbe.Probe();

        Assert.IsTrue(Enum.IsDefined(result.Code));
        Assert.IsTrue(result.FeatureLevel is null or >= 0x9000);
        if (result.IsReady)
        {
            Assert.IsTrue(result.FeatureLevel >= WindowsD3D11CapabilityProbe.MinimumFeatureLevel);
        }
    }
}
