using System.Windows;
using System.Windows.Threading;

namespace GpAutoLive.App.Tests;

internal static class WpfTestApplicationHost
{
    private static readonly object Gate = new();
    private static readonly object RunGate = new();
    private static readonly ManualResetEventSlim Ready = new(false);
    private static readonly TimeSpan DefaultActionTimeout = TimeSpan.FromSeconds(90);
    private static Dispatcher? _dispatcher;
    private static Exception? _startupException;
    private static Thread? _thread;

    public static void Run(Action action) => Run(action, DefaultActionTimeout);

    public static void Run(Action action, TimeSpan timeout)
    {
        ArgumentNullException.ThrowIfNull(action);
        if (timeout <= TimeSpan.Zero)
        {
            throw new ArgumentOutOfRangeException(nameof(timeout));
        }

        lock (RunGate)
        {
            EnsureStarted();
            var dispatcher = _dispatcher
                ?? throw new AssertFailedException("WPF 测试宿主未创建 Dispatcher。");
            if (dispatcher.HasShutdownStarted || dispatcher.HasShutdownFinished)
            {
                throw new AssertFailedException("WPF 测试宿主 Dispatcher 已关闭。");
            }

            var operation = dispatcher.InvokeAsync(action, DispatcherPriority.Send);
            if (!operation.Task.Wait(timeout))
            {
                throw new AssertFailedException(
                    $"WPF 测试宿主 action 在 {timeout.TotalSeconds:0.#} 秒内未完成；可能卡在窗口关闭或异步资源收尾。");
            }

            operation.Task.GetAwaiter().GetResult();
        }
    }

    private static void EnsureStarted()
    {
        lock (Gate)
        {
            if (_thread is not null)
            {
                if (!_thread.IsAlive || _dispatcher?.HasShutdownFinished == true)
                {
                    throw new AssertFailedException("WPF 测试宿主线程已退出。");
                }

                return;
            }

            _thread = new(ThreadMain)
            {
                IsBackground = true,
                Name = "GpAutoLive-wpf-test-host",
            };
            _thread.SetApartmentState(ApartmentState.STA);
            _thread.Start();
        }

        if (!Ready.Wait(TimeSpan.FromSeconds(5)))
        {
            throw new AssertFailedException("WPF 测试宿主启动超时。");
        }

        if (_startupException is not null)
        {
            throw new AssertFailedException("WPF 测试宿主启动失败。", _startupException);
        }
    }

    private static void ThreadMain()
    {
        try
        {
            var dispatcher = Dispatcher.CurrentDispatcher;
            SynchronizationContext.SetSynchronizationContext(
                new DispatcherSynchronizationContext(dispatcher));
            var application = new App();
            application.InitializeComponent();
            application.ShutdownMode = ShutdownMode.OnExplicitShutdown;

            lock (Gate)
            {
                _dispatcher = dispatcher;
            }

            Ready.Set();
            Dispatcher.Run();
        }
        catch (Exception exception)
        {
            _startupException = exception;
            Ready.Set();
        }
    }
}
