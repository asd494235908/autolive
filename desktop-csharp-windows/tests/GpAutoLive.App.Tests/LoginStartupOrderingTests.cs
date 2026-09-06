using System.IO;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class LoginStartupOrderingTests
{
    [TestMethod]
    public void Main_window_finishes_session_restore_before_enabling_login_gate()
    {
        var source = File.ReadAllText(FindMainWindowSource());

        StringAssert.Contains(source, "LoginGate.IsEnabled = _authCoordinator is null;");
        StringAssert.Contains(source, "await RestoreControlPlaneSessionAsync().ConfigureAwait(true);");
        StringAssert.Contains(source, "LoginGate.IsEnabled = true;");
        StringAssert.Contains(source, "控制面会话恢复失败，请重新登录。");
        Assert.IsFalse(
            source.Contains("_ = RestoreControlPlaneSessionAsync();", StringComparison.Ordinal),
            "冷启动恢复不得在登录按钮已可用时后台竞态执行。");
    }

    private static string FindMainWindowSource()
    {
        for (var directory = new DirectoryInfo(AppContext.BaseDirectory);
             directory is not null;
             directory = directory.Parent)
        {
            var candidate = Path.Combine(
                directory.FullName,
                "src",
                "GpAutoLive.App",
                "MainWindow.xaml.cs");
            if (File.Exists(candidate))
            {
                return candidate;
            }
        }

        Assert.Fail("未找到 MainWindow.xaml.cs。");
        return string.Empty;
    }
}
