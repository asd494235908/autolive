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

            Shutdown();
            return;
        }

        _singleInstance = singleInstance.Lease;

        var ownership = WindowsMediaOutputOwnershipLease.TryAcquire();
        if (!ownership.IsSuccess || ownership.Lease is null)
        {
            MessageBox.Show(
                FormatOwnershipFailure(ownership.Code),
                "GpAutoLive",
                MessageBoxButton.OK,
                MessageBoxImage.Warning);
            _singleInstance.Dispose();
            _singleInstance = null;
            Shutdown();
            return;
        }

        _mediaOutputOwnership = ownership.Lease;

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

        _mediaOutputOwnership?.Dispose();
        _mediaOutputOwnership = null;

        base.OnExit(e);
    }

    private static string FormatOwnershipFailure(WindowsMediaOutputOwnershipCode code) => code switch
    {
        WindowsMediaOutputOwnershipCode.ReferenceClientRunning =>
            "检测到现有 Rust/Tauri 桌面端正在运行；为避免媒体或输出资源争抢，C# 客户端未启动。",
        WindowsMediaOutputOwnershipCode.ReferenceClientProbeFailed =>
            "无法确认现有桌面端资源状态；为安全起见，C# 客户端未启动。",
        WindowsMediaOutputOwnershipCode.AlreadyOwned =>
            "媒体或输出资源已被另一桌面客户端占用；请先关闭它再启动 C# 客户端。",
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
