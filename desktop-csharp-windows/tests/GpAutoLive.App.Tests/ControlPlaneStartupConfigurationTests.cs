using GpAutoLive.App.Features.Auth;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class ControlPlaneStartupConfigurationTests
{
    [TestMethod]
    public void Cloud_test_package_uses_fixed_endpoint_and_test_credentials()
    {
        var configuration = ControlPlaneStartupConfiguration.Resolve(
            ControlPlaneStartupConfiguration.CloudTestProfile,
            null,
            null);

        Assert.AreEqual("http://101.96.208.132:9090", configuration.BaseUriText);
        Assert.IsTrue(configuration.AllowsDevelopmentHttp);
        Assert.IsTrue(configuration.UsesTestCredentialStore);
    }

    [TestMethod]
    public void Cloud_test_package_allows_offline_to_force_no_network()
    {
        var configuration = ControlPlaneStartupConfiguration.Resolve(
            ControlPlaneStartupConfiguration.CloudTestProfile,
            "offline",
            "https://ignored.example.com");

        Assert.IsNull(configuration.BaseUriText);
        Assert.IsFalse(configuration.AllowsDevelopmentHttp);
        Assert.IsTrue(configuration.UsesTestCredentialStore);
    }

    [TestMethod]
    public void Production_package_rejects_inherited_test_environment()
    {
        var configuration = ControlPlaneStartupConfiguration.Resolve(
            ControlPlaneStartupConfiguration.ProductionProfile,
            "test",
            "http://101.96.208.132:9090");

        Assert.IsNull(configuration.BaseUriText);
        Assert.IsFalse(configuration.AllowsDevelopmentHttp);
        Assert.IsFalse(configuration.UsesTestCredentialStore);
    }

    [TestMethod]
    public void Source_development_without_package_profile_keeps_existing_test_fallback()
    {
        var configuration = ControlPlaneStartupConfiguration.Resolve(null, "development", null);

        Assert.AreEqual("http://101.96.208.132:9090", configuration.BaseUriText);
        Assert.IsTrue(configuration.AllowsDevelopmentHttp);
        Assert.IsTrue(configuration.UsesTestCredentialStore);
    }
}
