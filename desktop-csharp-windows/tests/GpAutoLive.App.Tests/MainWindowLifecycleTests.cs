using System.Reflection;
using System.Windows.Threading;

namespace GpAutoLive.App.Tests;

[TestClass]
[DoNotParallelize]
public sealed class MainWindowLifecycleTests
{
    [TestMethod]
    public void Completed_background_failure_does_not_prevent_resource_shutdown()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            var closed = false;
            window.Closed += (_, _) => closed = true;
            typeof(MainWindow).GetField("_loadedTask", BindingFlags.Instance | BindingFlags.NonPublic)!
                .SetValue(window, Task.FromException(new InvalidOperationException("fixture failure")));
            window.Close();
            PumpUntil(window.ShutdownCompletion!);
            Assert.IsTrue(closed, "已结束的后台失败必须被观察，但不能阻断资源退出。");
        });
    }

    [TestMethod]
    public void Close_waits_for_inflight_command_and_rejects_new_commands()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            window.Show();
            var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var closed = false;
            window.Closed += (_, _) => closed = true;
            var command = RunCommand(window, () => release.Task);
            try
            {
                typeof(MainWindow).GetMethod("EnsureFinalEffectWindowVisible", BindingFlags.Instance | BindingFlags.NonPublic)!
                    .Invoke(window, null);
                var effect = (System.Windows.Window)typeof(MainWindow)
                    .GetField("_finalEffectWindow", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(window)!;
                effect.Close();
                window.Close();
                Assert.IsFalse(closed, "关闭必须等待在途命令，同时保留 Dispatcher。");
                window.Close();
                var ran = false;
                var rejected = RunCommand(window, () => { ran = true; return Task.CompletedTask; });
                PumpUntil(rejected);
                Assert.IsFalse(ran);
            }
            finally
            {
                release.TrySetResult();
                if (window.ShutdownCompletion is null) window.Close();
                PumpUntil(command);
                var shutdown = typeof(MainWindow).GetField("_shutdownTask", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(window) as Task;
                if (shutdown is not null) PumpUntil(shutdown);
            }
            Assert.IsTrue(closed);
        });
    }

    private static Task RunCommand(MainWindow window, Func<Task> command) =>
        (Task)(typeof(MainWindow).GetMethod("RunPlaybackCommandAsync", BindingFlags.Instance | BindingFlags.NonPublic,
            null, [typeof(Func<Task>)], null)?.Invoke(window, [command])
            ?? throw new AssertFailedException("播放命令入口缺失"));

    private static void PumpUntil(Task task)
    {
        var frame = new DispatcherFrame();
        var timer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(10) };
        timer.Tick += (_, _) => frame.Continue = false;
        _ = task.ContinueWith(_ => Dispatcher.CurrentDispatcher.BeginInvoke(() => frame.Continue = false),
            CancellationToken.None, TaskContinuationOptions.ExecuteSynchronously,
            TaskScheduler.FromCurrentSynchronizationContext());
        timer.Start();
        Dispatcher.PushFrame(frame);
        timer.Stop();
        Assert.IsTrue(task.IsCompleted, "关闭任务应完成，不能阻塞 Dispatcher。");
        task.GetAwaiter().GetResult();
    }
}
