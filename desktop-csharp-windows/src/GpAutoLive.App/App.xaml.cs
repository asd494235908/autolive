using System.Windows;
using System.Windows.Threading;
using GpAutoLive.App.Features.Runtime;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class App : Application
{
    private WindowsSingleInstanceLease? _singleInstance;
    private WindowsMediaOutputOwnershipLease? _mediaOutputOwnership;

    protected override void OnStartup(StartupEventArgs e)
    {
        ExternalWinRtRuntimeResolver.Configure();
        _ = WindowsAppIdentity.TryConfigure();

        var ownership = WindowsMediaOutputOwnershipLease.TryAcquire();
        if (!ownership.IsSuccess || ownership.Lease is null)
        {
            MessageBox.Show(
                FormatOwnershipFailure(ownership.Code),
                "GpAutoLive",
                MessageBoxButton.OK,
                MessageBoxImage.Warning);
            Shutdown();
            return;
        }

        _mediaOutputOwnership = ownership.Lease;

        var singleInstance = WindowsSingleInstanceLease.TryAcquire();
        if (!singleInstance.IsSuccess || singleInstance.Lease is null)
        {
            if (singleInstance.Code is not WindowsSingleInstanceCode.AlreadyOwned)
            {
                MessageBox.Show(
                    "无法建立 C# 桌面端单实例门禁，应用将退出。",
                    "GpAutoLive",
                    MessageBoxButton.OK,
                    MessageBoxImage.Warning);
            }

            _mediaOutputOwnership.Dispose();
            _mediaOutputOwnership = null;
            Shutdown();
            return;
        }

        _singleInstance = singleInstance.Lease;

        DispatcherUnhandledException += OnDispatcherUnhandledException;
        TaskScheduler.UnobservedTaskException += OnUnobservedTaskException;

        base.OnStartup(e);
        MainWindow = new MainWindow();
        MainWindow.Show();
    }

    protected override void OnExit(ExitEventArgs e)
    {
        DispatcherUnhandledException -= OnDispatcherUnhandledException;
        TaskScheduler.UnobservedTaskException -= OnUnobservedTaskException;

        _mediaOutputOwnership?.Dispose();
        _mediaOutputOwnership = null;

        if (_singleInstance is not null)
        {
            try
            {
                _singleInstance.Dispose();
            }
            finally
            {
                _singleInstance = null;
            }
        }

        base.OnExit(e);
    }

    private static string FormatOwnershipFailure(WindowsMediaOutputOwnershipCode code) => code switch
    {
        WindowsMediaOutputOwnershipCode.AlreadyOwned =>
            "C# 媒体或输出资源已被当前用户的另一个 C# 实例占用；请先关闭它再启动。",
        WindowsMediaOutputOwnershipCode.NotWindows =>
            "当前平台不是 Windows；此客户端只支持 Windows 10/11 x64。",
        _ => "无法建立媒体/输出资源所有权门禁；应用将退出。"
    };

    private static void OnDispatcherUnhandledException(object sender, DispatcherUnhandledExceptionEventArgs e)
    {
        e.Handled = true;
        MessageBox.Show(
            "界面发生未处理错误，当前操作已取消。请重试；媒体进程不会由壳层自动启动。",
            "GpAutoLive",
            MessageBoxButton.OK,
            MessageBoxImage.Warning);
    }

    private static void OnUnobservedTaskException(object? sender, UnobservedTaskExceptionEventArgs e) =>
        e.SetObserved();
}
