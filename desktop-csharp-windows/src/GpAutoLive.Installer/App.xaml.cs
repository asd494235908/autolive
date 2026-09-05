using System.Windows;
using System.Windows.Threading;

namespace GpAutoLive.Installer;

public partial class App : Application
{
    protected override void OnStartup(StartupEventArgs e)
    {
        if (!OperatingSystem.IsWindowsVersionAtLeast(10, 0, 19041))
        {
            MessageBox.Show(
                "此安装维护助手只支持 Windows 10 2004 或更高版本的 Windows 10/11 x64。",
                "GpAutoLive 安装维护助手",
                MessageBoxButton.OK,
                MessageBoxImage.Warning);
            Shutdown();
            return;
        }

        DispatcherUnhandledException += OnDispatcherUnhandledException;
        base.OnStartup(e);
    }

    protected override void OnExit(ExitEventArgs e)
    {
        DispatcherUnhandledException -= OnDispatcherUnhandledException;
        base.OnExit(e);
    }

    private static void OnDispatcherUnhandledException(
        object sender,
        DispatcherUnhandledExceptionEventArgs e)
    {
        e.Handled = true;
        MessageBox.Show(
            "安装维护助手发生未处理错误，当前操作已取消。请刷新安装状态后重试。",
            "GpAutoLive 安装维护助手",
            MessageBoxButton.OK,
            MessageBoxImage.Warning);
    }
}
