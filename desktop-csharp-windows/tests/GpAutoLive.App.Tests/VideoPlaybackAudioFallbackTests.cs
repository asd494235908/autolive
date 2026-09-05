using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using GpAutoLive.App.Features.Auth;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
[DoNotParallelize]
public sealed class VideoPlaybackAudioFallbackTests
{
    [TestMethod]
    public void Unauthorized_import_reports_login_gate_without_invoking_request_or_runtime()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                window.Show();

                var requestInvoked = false;
                var requestFactory = new Func<Task<MediaImportRequest?>>(() =>
                {
                    requestInvoked = true;
                    return Task.FromResult<MediaImportRequest?>(
                        new(MediaImportOperation.ReplaceAll, [Path.Combine(Path.GetTempPath(), "not-used.mp4")]));
                });
                var importTask = InvokePrivate(window, "RunImportAsync", requestFactory) as Task
                    ?? throw new InvalidOperationException("主窗口导入入口未返回异步任务。");
                PumpUntilCompleted(importTask);
                importTask.GetAwaiter().GetResult();

                var state = GetPrivateField<ShellState>(window, "_state");
                var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                if (requestInvoked
                    || !mediaPool.Snapshot.SourceMediaPool.IsEmpty
                    || !state.StatusMessage.Contains("请先完成登录与设备授权", StringComparison.Ordinal))
                {
                    throw new InvalidOperationException(
                        $"未授权导入门禁不完整：requestInvoked={requestInvoked}，媒体数={mediaPool.Snapshot.SourceMediaPool.Length}，状态={state.StatusMessage}");
                }
            }
            finally
            {
                window?.Close();
            }
        });
    }

    [TestMethod]
    public void Authorized_workbench_exposes_the_primary_import_button()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                window.Show();
                GetPrivateField<LoginViewModel>(window, "_login")
                    .ApplyActivated("fixture-account");

                var importButton = GetPrivateField<Button>(window, "ImportButton");
                var topImportButton = GetPrivateField<Button>(window, "TopImportButton");
                var workbench = GetPrivateField<FrameworkElement>(window, "WorkbenchSurface");
                var loginGate = GetPrivateField<FrameworkElement>(window, "LoginGate");

                Assert.AreEqual(Visibility.Visible, importButton.Visibility);
                Assert.IsTrue(importButton.IsEnabled);
                Assert.AreEqual(Visibility.Visible, topImportButton.Visibility);
                Assert.IsTrue(topImportButton.IsEnabled);
                Assert.IsTrue(workbench.IsEnabled);
                Assert.AreEqual(Visibility.Collapsed, loginGate.Visibility);
            }
            finally
            {
                window?.Close();
            }
        });
    }

    [TestMethod]
    public void Missing_runtime_keeps_existing_pool_and_reports_repair_action()
    {
        var missingRuntime = Path.Combine(
            Path.GetTempPath(),
            $"gpautolive-missing-runtime-{Guid.NewGuid():N}");
        Directory.CreateDirectory(missingRuntime);
        var existingPath = Path.Combine(missingRuntime, "existing.mp4");
        File.WriteAllBytes(existingPath, [0x01]);
        var previousRuntime = Environment.GetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT");
        Environment.SetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT", missingRuntime);
        try
        {
            WpfTestApplicationHost.Run(() =>
            {
                MainWindow? window = null;
                try
                {
                    window = new MainWindow();
                    window.Show();
                    GetPrivateField<LoginViewModel>(window, "_login")
                        .ApplyActivated("fixture-account");

                    var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                    var state = GetPrivateField<ShellState>(window, "_state");
                    var committed = mediaPool.ReplaceAll([CreateSyntheticVideoSource(existingPath)]);
                    if (!committed.IsSuccess)
                    {
                        throw new InvalidOperationException(committed.Error?.Message ?? "无法准备旧媒体池夹具。");
                    }

                    state.ApplyMediaSnapshot(committed.Snapshot);
                    InvokePrivate(window, "UpdateMediaProjection");
                    var importPath = Path.Combine(missingRuntime, "new.mp4");
                    var requestFactory = new Func<Task<MediaImportRequest?>>(
                        () => Task.FromResult<MediaImportRequest?>(
                            new MediaImportRequest(MediaImportOperation.ReplaceAll, [importPath])));
                    var importTask = InvokePrivate(window, "RunImportAsync", requestFactory) as Task
                        ?? throw new InvalidOperationException("主窗口导入入口未返回异步任务。");
                    PumpUntilCompleted(importTask);
                    importTask.GetAwaiter().GetResult();

                    var snapshot = mediaPool.Snapshot;
                    var mediaStatus = GetPrivateField<TextBlock>(window, "MediaStatusText");
                    if (snapshot.SourceMediaPool.Length != 1
                        || !string.Equals(snapshot.SourceMediaPool[0].SourcePath, Path.GetFullPath(existingPath), StringComparison.OrdinalIgnoreCase)
                        || !state.StatusMessage.Contains("媒体运行资源清单不存在", StringComparison.Ordinal)
                        || !state.StatusMessage.Contains("安装或修复", StringComparison.Ordinal)
                        || !mediaStatus.Text.Contains("安装或修复", StringComparison.Ordinal))
                    {
                        throw new InvalidOperationException(
                            $"运行包缺失未保留旧池或未给出修复动作：媒体数={snapshot.SourceMediaPool.Length}，状态={state.StatusMessage}，运行时状态={mediaStatus.Text}");
                    }
                }
                finally
                {
                    window?.Close();
                }
            });
        }
        finally
        {
            Environment.SetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT", previousRuntime);
            try
            {
                File.Delete(existingPath);
                Directory.Delete(missingRuntime, recursive: true);
            }
            catch (IOException)
            {
                // 测试宿主异常退出时保留临时夹具，避免清理错误覆盖真实断言。
            }
        }
    }

    [TestMethod]
    public void Video_remains_playing_when_portaudio_device_cannot_start()
    {
        var fixture = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA")
            ?? @"E:\下载\csharp-golden-av.mp4";
        var runtime = Environment.GetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT")
            ?? FindRuntimeFromRepository();
        fixture = Path.GetFullPath(fixture);
        runtime = Path.GetFullPath(runtime);
        if (!OperatingSystem.IsWindows() || !File.Exists(fixture) || !File.Exists(Path.Combine(runtime, "runtime", "media", "1.0.0", "manifest.json")))
        {
            Assert.Inconclusive("需要 Windows、E:\\下载\\csharp-golden-av.mp4 和已校验的 C# 媒体运行时夹具。");
        }

        var previousRuntime = Environment.GetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT");
        Environment.SetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT", runtime);
        try
        {
            WpfTestApplicationHost.Run(() =>
            {
                MainWindow? window = null;
                try
                {
                    window = new MainWindow();
                    window.Show();

                    var login = GetPrivateField<LoginViewModel>(window, "_login");
                    login.ApplyActivated("fixture-account");

                    var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                    var state = GetPrivateField<ShellState>(window, "_state");
                    var source = new SourceMediaDto(
                        fixture,
                        fixture,
                        MediaKind.Video,
                        MediaCompatibilityMode.Direct,
                        Path.GetFileName(fixture),
                        (ulong)new FileInfo(fixture).Length,
                        2_500,
                        null,
                        null,
                        320,
                        180,
                        30,
                        48_000,
                        2,
                        "h264",
                        "aac",
                        null,
                        "disabled");
                    var committed = mediaPool.ReplaceAll([source]);
                    if (!committed.IsSuccess)
                    {
                        throw new InvalidOperationException(committed.Error?.Message ?? "无法准备媒体池夹具。");
                    }

                    state.ApplyMediaSnapshot(committed.Snapshot);
                    InvokePrivate(window, "UpdateMediaProjection");
                    var outputDevice = GetPrivateField<ComboBox>(window, "AudioOutputDeviceComboBox");
                    outputDevice.SelectedValue = int.MaxValue;

                    var startTask = InvokePrivate(window, "TogglePlaybackCoreAsync") as Task
                        ?? throw new InvalidOperationException("视频播放入口未返回异步任务。");
                    PumpUntilCompleted(startTask);
                    startTask.GetAwaiter().GetResult();

                    if (mediaPool.Snapshot.PlaybackState is not PlaybackState.Playing)
                    {
                        throw new InvalidOperationException(
                            $"PortAudio 失败时视频没有保持播放，实际状态为 {mediaPool.Snapshot.PlaybackState}。");
                    }

                    var mpv = GetPrivateField<WindowsMpvPlaybackController>(window, "_mpvController");
                    if (mpv.Snapshot.ActiveIdentity is null)
                    {
                        throw new InvalidOperationException("PortAudio 失败时 mpv 会话被错误关闭。");
                    }

                    var pauseTask = InvokePrivate(window, "TogglePlaybackCoreAsync") as Task
                        ?? throw new InvalidOperationException("视频暂停入口未返回异步任务。");
                    PumpUntilCompleted(pauseTask);
                    pauseTask.GetAwaiter().GetResult();
                    if (mediaPool.Snapshot.PlaybackState is not PlaybackState.Paused)
                    {
                        throw new InvalidOperationException(
                            $"声音失败后视频无法暂停，实际状态为 {mediaPool.Snapshot.PlaybackState}。");
                    }

                    var resumeTask = InvokePrivate(window, "TogglePlaybackCoreAsync") as Task
                        ?? throw new InvalidOperationException("视频恢复入口未返回异步任务。");
                    PumpUntilCompleted(resumeTask);
                    resumeTask.GetAwaiter().GetResult();
                    var audioStatus = GetPrivateField<TextBlock>(window, "AudioDeviceStatusText");
                    if (mediaPool.Snapshot.PlaybackState is not PlaybackState.Playing
                        || !audioStatus.Text.Contains("视频画面继续运行", StringComparison.Ordinal))
                    {
                        throw new InvalidOperationException(
                            $"声音恢复失败后视频未保持播放或状态不明确：池状态={mediaPool.Snapshot.PlaybackState}，声音状态={audioStatus.Text}");
                    }
                }
                finally
                {
                    try
                    {
                        StopPlaybackForTest(window);
                    }
                    finally
                    {
                        window?.Close();
                    }
                }
            });
        }
        finally
        {
            Environment.SetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT", previousRuntime);
        }
    }

    [TestMethod]
    public void Main_window_imports_fixture_and_starts_video()
    {
        var fixture = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA")
            ?? @"E:\下载\csharp-golden-av.mp4";
        var runtime = Environment.GetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT")
            ?? FindRuntimeFromRepository();
        fixture = Path.GetFullPath(fixture);
        runtime = Path.GetFullPath(runtime);
        if (!OperatingSystem.IsWindows()
            || !File.Exists(fixture)
            || !File.Exists(Path.Combine(runtime, "runtime", "media", "1.0.0", "manifest.json")))
        {
            Assert.Inconclusive("需要 Windows、E:\\下载\\csharp-golden-av.mp4 和已校验的 C# 媒体运行时夹具。");
        }

        var previousRuntime = Environment.GetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT");
        Environment.SetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT", runtime);
        try
        {
            WpfTestApplicationHost.Run(() =>
            {
                MainWindow? window = null;
                try
                {
                    window = new MainWindow();
                    window.Show();
                    GetPrivateField<LoginViewModel>(window, "_login")
                        .ApplyActivated("fixture-account");

                    var importMethod = window.GetType()
                        .GetMethod("RunImportAsync", BindingFlags.Instance | BindingFlags.NonPublic)
                        ?? throw new MissingMethodException(window.GetType().FullName, "RunImportAsync");
                    var requestFactory = new Func<Task<MediaImportRequest?>>(
                        () => Task.FromResult<MediaImportRequest?>(
                            new(MediaImportOperation.ReplaceAll, [fixture])));
                    var importTask = importMethod.Invoke(window, [requestFactory]) as Task
                        ?? throw new InvalidOperationException("主窗口导入入口未返回异步任务。");
                    PumpUntilCompleted(importTask);
                    importTask.GetAwaiter().GetResult();

                    var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                    if (mediaPool.Snapshot.SourceMediaPool.Length != 1
                        || mediaPool.Snapshot.PlaybackState is not PlaybackState.Ready)
                    {
                        throw new InvalidOperationException(
                            $"主窗口导入后播放池状态不正确：{mediaPool.Snapshot.SourceMediaPool.Length} 项，状态 {mediaPool.Snapshot.PlaybackState}。");
                    }

                    var mediaList = GetPrivateField<ListBox>(window, "MediaListBox");
                    if (mediaList.Items.Count != 1 || mediaList.SelectedIndex != 0)
                    {
                        throw new InvalidOperationException("主窗口导入后媒体列表没有选中首项。");
                    }

                    GetPrivateField<ComboBox>(window, "AudioOutputDeviceComboBox").SelectedValue = int.MaxValue;
                    var startTask = InvokePrivate(window, "TogglePlaybackCoreAsync") as Task
                        ?? throw new InvalidOperationException("主窗口播放入口未返回异步任务。");
                    PumpUntilCompleted(startTask);
                    startTask.GetAwaiter().GetResult();

                    var controller = GetPrivateField<WindowsMpvPlaybackController>(window, "_mpvController");
                    var shellState = GetPrivateField<ShellState>(window, "_state");
                    var audioStatus = GetPrivateField<TextBlock>(window, "AudioDeviceStatusText");
                    if (mediaPool.Snapshot.PlaybackState is not PlaybackState.Playing
                        || controller.Snapshot.ActiveIdentity is null)
                    {
                        throw new InvalidOperationException(
                            $"主窗口导入后的首项没有保持视频播放：池={mediaPool.Snapshot.PlaybackState}，"
                            + $"mpv={controller.Snapshot.State}/{controller.Snapshot.Runtime.State}/{controller.Snapshot.Runtime.IpcState}，"
                            + $"身份={controller.Snapshot.ActiveIdentity}，状态={shellState.StatusMessage}，声音={audioStatus.Text}");
                    }
                }
                finally
                {
                    try
                    {
                        StopPlaybackForTest(window);
                    }
                    finally
                    {
                        window?.Close();
                    }
                }
            });
        }
        finally
        {
            Environment.SetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT", previousRuntime);
        }
    }

    [TestMethod]
    public void Main_window_advances_media_pool_at_video_eof()
    {
        var fixture = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA")
            ?? @"E:\下载\csharp-golden-av.mp4";
        var runtime = Environment.GetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT")
            ?? FindRuntimeFromRepository();
        fixture = Path.GetFullPath(fixture);
        runtime = Path.GetFullPath(runtime);
        if (!OperatingSystem.IsWindows()
            || !File.Exists(fixture)
            || !File.Exists(Path.Combine(runtime, "runtime", "media", "1.0.0", "manifest.json")))
        {
            Assert.Inconclusive("需要 Windows、E:\\下载\\csharp-golden-av.mp4 和已校验的 C# 媒体运行时夹具。");
        }

        var secondFixture = Path.Combine(
            Path.GetTempPath(),
            $"gpautolive-eof-{Guid.NewGuid():N}{Path.GetExtension(fixture)}");
        File.Copy(fixture, secondFixture);
        var previousRuntime = Environment.GetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT");
        Environment.SetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT", runtime);
        try
        {
            WpfTestApplicationHost.Run(() =>
            {
                MainWindow? window = null;
                try
                {
                    window = new MainWindow();
                    window.Show();
                    GetPrivateField<LoginViewModel>(window, "_login")
                        .ApplyActivated("fixture-account");

                    var importMethod = window.GetType()
                        .GetMethod("RunImportAsync", BindingFlags.Instance | BindingFlags.NonPublic)
                        ?? throw new MissingMethodException(window.GetType().FullName, "RunImportAsync");
                    var requestFactory = new Func<Task<MediaImportRequest?>>(
                        () => Task.FromResult<MediaImportRequest?>(
                            new(MediaImportOperation.ReplaceAll, [fixture, secondFixture])));
                    var importTask = importMethod.Invoke(window, [requestFactory]) as Task
                        ?? throw new InvalidOperationException("主窗口导入入口未返回异步任务。");
                    PumpUntilCompleted(importTask);
                    importTask.GetAwaiter().GetResult();

                    var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                    if (mediaPool.Snapshot.SourceMediaPool.Length != 2
                        || mediaPool.Snapshot.PlaybackState is not PlaybackState.Ready)
                    {
                        throw new InvalidOperationException(
                            $"主窗口导入后播放池状态不正确：{mediaPool.Snapshot.SourceMediaPool.Length} 项，状态 {mediaPool.Snapshot.PlaybackState}。");
                    }

                    var startTask = InvokePrivate(window, "TogglePlaybackCoreAsync") as Task
                        ?? throw new InvalidOperationException("主窗口播放入口未返回异步任务。");
                    PumpUntilCompleted(startTask);
                    startTask.GetAwaiter().GetResult();

                    var mpv = GetPrivateField<WindowsMpvPlaybackController>(window, "_mpvController");
                    var state = GetPrivateField<ShellState>(window, "_state");
                    if (!PumpUntil(
                            () =>
                            {
                                var poolSnapshot = mediaPool.Snapshot;
                                return poolSnapshot.SourceMediaIndex == 1
                                    && poolSnapshot.PlaybackState is PlaybackState.Playing
                                    && state.StatusMessage.Contains("已自动切换到第 2 项", StringComparison.Ordinal);
                            },
                            TimeSpan.FromSeconds(20)))
                    {
                        throw new InvalidOperationException(
                            $"第一项 EOF 后未完成真实换源，当前索引 {mediaPool.Snapshot.SourceMediaIndex}，"
                            + $"池状态 {mediaPool.Snapshot.PlaybackState}，主窗口状态 {state.StatusMessage}，"
                            + $"mpv={mpv.Snapshot.State}/{mpv.Snapshot.Runtime.State}/{mpv.Snapshot.Runtime.IpcState}，"
                            + $"身份={mpv.Snapshot.ActiveIdentity}，声音={GetPrivateField<TextBlock>(window, "AudioDeviceStatusText").Text}。");
                    }

                    var controllerSnapshot = mpv.Snapshot;
                    if (controllerSnapshot.ActiveIdentity != mediaPool.CurrentIdentity
                        || controllerSnapshot.State is not WindowsMpvPlaybackControllerState.Playing
                        || controllerSnapshot.Runtime.State is not WindowsMpvPlaybackRuntimeState.Running)
                    {
                        throw new InvalidOperationException(
                            $"主窗口虽投影换源完成，但 mpv 活动会话未同步到第二项：身份 {controllerSnapshot.ActiveIdentity}，状态 {controllerSnapshot.State}/{controllerSnapshot.Runtime.State}。");
                    }
                }
                finally
                {
                    try
                    {
                        StopPlaybackForTest(window);
                    }
                    finally
                    {
                        window?.Close();
                    }
                }
            });
        }
        finally
        {
            Environment.SetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT", previousRuntime);
            try
            {
                File.Delete(secondFixture);
            }
            catch (IOException)
            {
                // 测试宿主尚未释放句柄时，保留临时文件供系统回收，不影响业务结果。
            }
        }
    }

    private static object? InvokePrivate(object instance, string methodName, params object?[] arguments)
    {
        var method = instance.GetType()
            .GetMethod(methodName, BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new MissingMethodException(instance.GetType().FullName, methodName);
        return method.Invoke(instance, arguments);
    }

    private static T GetPrivateField<T>(object instance, string fieldName) =>
        instance.GetType()
            .GetField(fieldName, BindingFlags.Instance | BindingFlags.NonPublic)
            ?.GetValue(instance) is T value
            ? value
            : throw new MissingFieldException(instance.GetType().FullName, fieldName);

    private static void StopPlaybackForTest(MainWindow? window)
    {
        if (window is null)
        {
            return;
        }

        try
        {
            var stopTask = InvokePrivate(window, "StopPlaybackAfterFinalEffectWindowClosedAsync") as Task;
            if (stopTask is null)
            {
                return;
            }

            PumpUntilCompleted(stopTask, "停止媒体播放");
            stopTask.GetAwaiter().GetResult();
        }
        catch (InvalidOperationException)
        {
            // 断言失败时只尽力释放测试启动的媒体会话，不覆盖原始测试结果。
        }
        catch (IOException)
        {
            // 命名管道已断开时由窗口关闭路径继续做最终清理。
        }
    }

    private static void PumpUntilCompleted(Task task, string description = "WPF 异步任务")
    {
        if (task.IsCompleted)
        {
            return;
        }

        if (!PumpUntil(() => task.IsCompleted, TimeSpan.FromSeconds(20)))
        {
            throw new TimeoutException($"{description}在 20 秒内未完成。");
        }
    }

    private static bool PumpUntil(Func<bool> predicate, TimeSpan timeout)
    {
        var deadline = Stopwatch.GetTimestamp()
            + (long)(timeout.TotalSeconds * Stopwatch.Frequency);
        while (!predicate())
        {
            if (Stopwatch.GetTimestamp() >= deadline)
            {
                return false;
            }

            Dispatcher.CurrentDispatcher.Invoke(
                DispatcherPriority.Background,
                new Action(static () => { }));
            Thread.Sleep(10);
        }

        return true;
    }

    private static string FindRuntimeFromRepository()
    {
        for (var directory = new DirectoryInfo(AppContext.BaseDirectory);
             directory is not null;
             directory = directory.Parent)
        {
            var candidate = Path.Combine(
                directory.FullName,
                "artifacts",
                "csharp-windows-controller-20260903-v90");
            if (Directory.Exists(candidate))
            {
                return candidate;
            }
        }

        return Path.Combine(AppContext.BaseDirectory, "missing-csharp-media-runtime");
    }

    private static SourceMediaDto CreateSyntheticVideoSource(string path) => new(
        path,
        path,
        MediaKind.Video,
        MediaCompatibilityMode.Direct,
        Path.GetFileName(path),
        (ulong)new FileInfo(path).Length,
        2_500,
        null,
        null,
        320,
        180,
        30,
        48_000,
        2,
        "h264",
        "aac",
        null,
        "disabled");
}
