namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsAppIdentityTests
{
    [TestMethod]
    public void AppUserModelId_IsStableAndBounded()
    {
        Assert.IsFalse(string.IsNullOrWhiteSpace(GpAutoLive.Windows.WindowsAppIdentity.AppUserModelId));
        Assert.IsTrue(GpAutoLive.Windows.WindowsAppIdentity.AppUserModelId.Length <= 128);
        Assert.IsFalse(GpAutoLive.Windows.WindowsAppIdentity.AppUserModelId.Any(char.IsControl));
        StringAssert.StartsWith(
            GpAutoLive.Windows.WindowsAppIdentity.AppUserModelId,
            "GpAutoLive.CSharp.");
    }

    [TestMethod]
    public void TryConfigure_IsClassifiedWithoutThrowing()
    {
        var result = GpAutoLive.Windows.WindowsAppIdentity.TryConfigure();

        Assert.IsTrue(
            result is GpAutoLive.Windows.WindowsAppIdentityCode.Applied
                or GpAutoLive.Windows.WindowsAppIdentityCode.NotWindows
                or GpAutoLive.Windows.WindowsAppIdentityCode.ApiUnavailable
                or GpAutoLive.Windows.WindowsAppIdentityCode.ApiFailed);
    }
}
