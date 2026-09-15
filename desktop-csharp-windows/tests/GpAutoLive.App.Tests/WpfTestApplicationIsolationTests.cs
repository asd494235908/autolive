using System.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class WpfTestApplicationIsolationTests
{
    [TestMethod]
    public void Resource_host_does_not_acquire_production_application_ownership()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var application = Application.Current;
            Assert.IsNotNull(application);
            Assert.IsNotNull(application.TryFindResource("CardStyle"));
            Assert.IsInstanceOfType<System.Windows.Media.Brush>(application.TryFindResource("TextBrush"));
            Assert.AreEqual(typeof(Application), application.GetType(),
                "资源测试宿主不得实例化具有生产启动回调的 App。");
        });
    }
}
