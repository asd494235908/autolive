using System.Windows;
using System.Windows.Markup;
using System.Windows.Threading;
using System.Xml.Linq;

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

            var operation = dispatcher.InvokeAsync(() =>
            {
                try { action(); }
                finally
                {
                    foreach (var window in Application.Current.Windows.OfType<MainWindow>().ToArray())
                    {
                        if (window.ShutdownCompletion is { } shutdown)
                        {
                            DrainShutdown(shutdown);
                            Assert.IsFalse(Application.Current.Windows.Cast<Window>().Contains(window),
                                "测试窗口资源退出失败，窗口仍等待重试。");
                        }
                    }
                }
            }, DispatcherPriority.Send);
            if (!operation.Task.Wait(timeout))
            {
                throw new AssertFailedException(
                    $"WPF 测试宿主 action 在 {timeout.TotalSeconds:0.#} 秒内未完成；可能卡在窗口关闭或异步资源收尾。");
            }

            operation.Task.GetAwaiter().GetResult();
        }
    }

    private static void DrainShutdown(Task shutdown)
    {
        if (shutdown.IsCompleted) return;
        var dispatcher = Dispatcher.CurrentDispatcher;
        var frame = new DispatcherFrame();
        var timer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(20) };
        timer.Tick += (_, _) => frame.Continue = false;
        _ = shutdown.ContinueWith(_ => dispatcher.BeginInvoke(new Action(() => frame.Continue = false)),
            CancellationToken.None, TaskContinuationOptions.ExecuteSynchronously, TaskScheduler.Default);
        timer.Start();
        Dispatcher.PushFrame(frame);
        timer.Stop();
        Assert.IsTrue(shutdown.IsCompleted, "测试窗口异步关闭未能完成。");
        shutdown.GetAwaiter().GetResult();
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
            throw new AssertFailedException($"WPF 测试宿主启动失败：{_startupException}", _startupException);
        }
    }

    private static void ThreadMain()
    {
        try
        {
            var dispatcher = Dispatcher.CurrentDispatcher;
            SynchronizationContext.SetSynchronizationContext(
                new DispatcherSynchronizationContext(dispatcher));
            // 复用同一份 App.xaml 资源，不实例化会获取生产锁的 App。
            var source = XDocument.Load(System.IO.Path.Combine(AppContext.BaseDirectory, "Fixtures", "App.resources-source.xaml"));
            XNamespace presentation = "http://schemas.microsoft.com/winfx/2006/xaml/presentation";
            var resources = source.Root!.Element(presentation + "Application.Resources")!.Elements().Single();
            var application = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
            // 先发布主题字典，保证后续模板的 StaticResource 能在解析时找到颜色。
            var merged = resources.Element(presentation + "ResourceDictionary.MergedDictionaries");
            if (merged is not null)
            {
                foreach (var dictionary in merged.Elements())
                    application.Resources.MergedDictionaries.Add((ResourceDictionary)XamlReader.Parse(dictionary.ToString()));
                merged.Remove();
            }
            application.Resources.MergedDictionaries.Add((ResourceDictionary)XamlReader.Parse(resources.ToString()));

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
