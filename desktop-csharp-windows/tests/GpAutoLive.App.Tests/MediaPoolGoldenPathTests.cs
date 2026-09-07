using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using GpAutoLive.App.Features.Auth;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Processes;
using GpAutoLive.Media;

namespace GpAutoLive.App.Tests;

[TestClass]
[DoNotParallelize]
public sealed class MediaPoolGoldenPathTests
{
    private readonly List<string> _temporaryDirectories = [];

    [TestCleanup]
    public void Cleanup()
    {
        foreach (var directory in _temporaryDirectories)
        {
            try
            {
                if (Directory.Exists(directory))
                {
                    Directory.Delete(directory, recursive: true);
                }
            }
            catch (IOException)
            {
                // 清理失败不覆盖主体断言。
            }
        }
    }

    [TestMethod]
    public void Import_busy_disables_every_media_pool_mutation_button()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                window.Show();
                GetPrivateField<LoginViewModel>(window, "_login").ApplyActivated("fixture-account");

                var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                var committed = mediaPool.ReplaceAll([
                    CreateMedia("busy-a.mp4"),
                    CreateMedia("busy-b.mp4"),
                ]);
                Assert.IsTrue(committed.IsSuccess, committed.Error?.Message);
                GetPrivateField<ShellState>(window, "_state").ApplyMediaSnapshot(committed.Snapshot);
                InvokeVoidPrivate(window, "UpdateMediaProjection");

                var list = GetPrivateField<ListBox>(window, "MediaListBox");
                list.SelectedIndex = 1;

                SetPrivateField(window, "_importBusy", true);
                InvokeVoidPrivate(window, "SetImportButtonsEnabled", false);

                Assert.IsFalse(GetPrivateField<Button>(window, "TopImportButton").IsEnabled);
                Assert.IsFalse(GetPrivateField<Button>(window, "TopImportPlaylistButton").IsEnabled);
                Assert.IsFalse(GetPrivateField<Button>(window, "MoveUpButton").IsEnabled);
                Assert.IsFalse(GetPrivateField<Button>(window, "MoveDownButton").IsEnabled);
                Assert.IsFalse(GetPrivateField<Button>(window, "RemoveMediaButton").IsEnabled);
                Assert.IsFalse(GetPrivateField<Button>(window, "ClearMediaButton").IsEnabled);

                list.SelectedIndex = 0;
                Assert.IsFalse(GetPrivateField<Button>(window, "MoveUpButton").IsEnabled);
                Assert.IsFalse(GetPrivateField<Button>(window, "MoveDownButton").IsEnabled);
                Assert.IsFalse(GetPrivateField<Button>(window, "RemoveMediaButton").IsEnabled);
                Assert.IsFalse(GetPrivateField<Button>(window, "ClearMediaButton").IsEnabled);

                SetPrivateField(window, "_importBusy", false);
                InvokeVoidPrivate(window, "SetImportButtonsEnabled", true);
                Assert.IsTrue(GetPrivateField<Button>(window, "TopImportButton").IsEnabled);
                Assert.IsTrue(GetPrivateField<Button>(window, "TopImportPlaylistButton").IsEnabled);
                Assert.IsFalse(GetPrivateField<Button>(window, "MoveUpButton").IsEnabled);
                Assert.IsTrue(
                    GetPrivateField<Button>(window, "MoveDownButton").IsEnabled,
                    $"忙碌态结束后下移按钮未按当前选择恢复：selected={list.SelectedIndex}，"
                    + $"items={list.Items.Count}，stateItems={GetPrivateField<ShellState>(window, "_state").MediaItems.Count}。");
                Assert.IsTrue(GetPrivateField<Button>(window, "RemoveMediaButton").IsEnabled);
                Assert.IsTrue(GetPrivateField<Button>(window, "ClearMediaButton").IsEnabled);
            }
            finally
            {
                window?.Close();
            }
        });
    }

    [TestMethod]
    public void Unauthorized_move_handler_cannot_mutate_media_pool()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                window.Show();

                var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                var committed = mediaPool.ReplaceAll([
                    CreateMedia("unauthorized-a.mp4"),
                    CreateMedia("unauthorized-b.mp4"),
                ]);
                Assert.IsTrue(committed.IsSuccess, committed.Error?.Message);
                GetPrivateField<ShellState>(window, "_state").ApplyMediaSnapshot(committed.Snapshot);
                InvokeVoidPrivate(window, "UpdateMediaProjection");
                GetPrivateField<ListBox>(window, "MediaListBox").SelectedIndex = 1;

                InvokeVoidPrivate(window, "MoveUpButton_Click", null, new RoutedEventArgs());

                Assert.AreEqual("unauthorized-a.mp4", mediaPool.Snapshot.SourceMediaPool[0].FileName);
                Assert.AreEqual("unauthorized-b.mp4", mediaPool.Snapshot.SourceMediaPool[1].FileName);
            }
            finally
            {
                window?.Close();
            }
        });
    }

    [TestMethod]
    public void Failed_import_after_stop_projects_the_actual_pool_snapshot()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                window.Show();
                GetPrivateField<LoginViewModel>(window, "_login").ApplyActivated("fixture-account");

                var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                var existing = CreateMedia("failed-commit.mp4");
                var committed = mediaPool.ReplaceAll([existing]);
                Assert.IsTrue(committed.IsSuccess, committed.Error?.Message);
                var playing = mediaPool.StartPlayback();
                Assert.IsTrue(playing.IsSuccess, playing.Error?.Message);
                var state = GetPrivateField<ShellState>(window, "_state");
                state.ApplyMediaSnapshot(playing.Snapshot);
                InvokeVoidPrivate(window, "UpdateMediaProjection");

                var importer = new MediaImportCoordinator(
                    mediaPool,
                    new FfprobeMediaProbe(
                        Path.Combine(CreateDirectory(), "ffprobe.exe"),
                        new SuccessfulFfprobeRunner()));
                SetPrivateField(window, "_mediaImporter", importer);

                var requestFactory = new Func<Task<MediaImportRequest?>>(() =>
                    Task.FromResult<MediaImportRequest?>(
                        new(MediaImportOperation.Append, [existing.SourcePath])));
                var importTask = InvokePrivate(window, "RunImportAsync", requestFactory) as Task
                    ?? throw new InvalidOperationException("主窗口导入入口未返回异步任务。");
                PumpUntilCompleted(importTask);
                importTask.GetAwaiter().GetResult();

                Assert.AreEqual(PlaybackState.Stopped, mediaPool.Snapshot.PlaybackState);
                Assert.AreEqual(mediaPool.Snapshot.PlaybackState, state.PlaybackState);
                Assert.AreEqual(1, mediaPool.Snapshot.SourceMediaPool.Length);
                Assert.IsTrue(state.StatusMessage.Contains("保留原播放池", StringComparison.Ordinal));
            }
            finally
            {
                window?.Close();
            }
        });
    }

    [TestMethod]
    public async Task File_picker_import_appends_and_keeps_existing_source_file()
    {
        var mediaPool = new MediaPoolService();
        var existing = CreateMedia("existing-source.mp4");
        var added = CreateMedia("added-source.mp4");
        Assert.IsTrue(mediaPool.ReplaceAll([existing]).IsSuccess);
        using var importer = new MediaImportCoordinator(
            mediaPool,
            new FfprobeMediaProbe(
                Path.Combine(CreateDirectory(), "ffprobe.exe"),
                new SuccessfulFfprobeRunner()));

        var result = await importer.ImportAsync(
            MainWindow.CreateFileImportRequest([added.SourcePath]));

        Assert.IsTrue(result.IsSuccess, result.Error?.Message);
        CollectionAssert.AreEqual(
            new[] { existing.SourcePath, added.SourcePath },
            result.Snapshot.SourceMediaPool.Select(static item => item.SourcePath).ToArray());
        Assert.IsTrue(File.Exists(existing.SourcePath), "追加导入不得删除原播放池的用户源文件。");
        Assert.IsTrue(File.Exists(added.SourcePath), "追加导入不得删除新选择的用户源文件。");
    }

    [TestMethod]
    public void Import_waits_for_the_shared_playback_command_gate()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            var gateHeld = false;
            try
            {
                window = new MainWindow();
                window.Show();
                GetPrivateField<LoginViewModel>(window, "_login").ApplyActivated("fixture-account");
                var importer = new MediaImportCoordinator(
                    GetPrivateField<MediaPoolService>(window, "_mediaPool"),
                    new FfprobeMediaProbe(
                        Path.Combine(CreateDirectory(), "ffprobe.exe"),
                        new SuccessfulFfprobeRunner()));
                SetPrivateField(window, "_mediaImporter", importer);

                var gate = GetPrivateField<SemaphoreSlim>(window, "_playbackCommandSerial");
                gate.Wait();
                gateHeld = true;
                var path = Path.Combine(CreateDirectory(), "gate.mp4");
                File.WriteAllBytes(path, [1, 2, 3]);
                var requestFactory = new Func<Task<MediaImportRequest?>>(() =>
                    Task.FromResult<MediaImportRequest?>(
                        new(MediaImportOperation.ReplaceAll, [path])));
                var importTask = InvokePrivate(window, "RunImportAsync", requestFactory) as Task
                    ?? throw new InvalidOperationException("主窗口导入入口未返回异步任务。");

                Dispatcher.CurrentDispatcher.Invoke(
                    DispatcherPriority.Background,
                    new Action(static () => { }));
                Assert.IsFalse(
                    importTask.IsCompleted,
                    $"导入不应绕过共享播放命令串行门；login={GetPrivateField<LoginViewModel>(window, "_login").Status}，"
                    + $"gate={gate.CurrentCount}，busy={GetPrivateField<bool>(window, "_importBusy")}，"
                    + $"closing={GetPrivateField<bool>(window, "_isClosing")}，"
                    + $"cancelled={GetPrivateField<CancellationTokenSource>(window, "_windowCancellation").IsCancellationRequested}，"
                    + $"status={GetPrivateField<ShellState>(window, "_state").StatusMessage}。");

                gate.Release();
                gateHeld = false;
                PumpUntilCompleted(importTask);
                importTask.GetAwaiter().GetResult();
                Assert.AreEqual(1, GetPrivateField<MediaPoolService>(window, "_mediaPool").Snapshot.SourceMediaPool.Length);
            }
            finally
            {
                if (gateHeld)
                {
                    GetPrivateField<SemaphoreSlim>(window!, "_playbackCommandSerial").Release();
                }

                window?.Close();
            }
        });
    }

    [TestMethod]
    public void Closing_final_effect_window_stops_media_pool_playback()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                window.Show();
                GetPrivateField<LoginViewModel>(window, "_login").ApplyActivated("fixture-account");

                var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                var committed = mediaPool.ReplaceAll([CreateMedia("window-close.mp4")]);
                Assert.IsTrue(committed.IsSuccess, committed.Error?.Message);
                var playing = mediaPool.StartPlayback();
                Assert.IsTrue(playing.IsSuccess, playing.Error?.Message);
                var state = GetPrivateField<ShellState>(window, "_state");
                state.ApplyMediaSnapshot(playing.Snapshot);
                InvokeVoidPrivate(window, "UpdateMediaProjection");

                InvokeVoidPrivate(window, "FinalEffectWindow_Closed", null, EventArgs.Empty);

                PumpUntil(() => mediaPool.Snapshot.PlaybackState is PlaybackState.Stopped);
                Assert.AreEqual(PlaybackState.Stopped, mediaPool.Snapshot.PlaybackState);
                Assert.AreEqual(mediaPool.Snapshot.PlaybackState, state.PlaybackState);
                Assert.IsTrue(
                    state.StatusMessage.Contains("播放与视频运行时已释放", StringComparison.Ordinal));
            }
            finally
            {
                window?.Close();
            }
        });
    }

    private string CreateDirectory()
    {
        var directory = Path.Combine(Path.GetTempPath(), "GpAutoLive.CSharp.App.MediaPool", Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        _temporaryDirectories.Add(directory);
        return directory;
    }

    private SourceMediaDto CreateMedia(string fileName)
    {
        var path = Path.Combine(CreateDirectory(), fileName);
        File.WriteAllBytes(path, [1, 2, 3]);
        return new(
            path,
            path,
            MediaKind.Video,
            MediaCompatibilityMode.Direct,
            fileName,
            3,
            1_000,
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

    private static object? InvokePrivate(object instance, string methodName, params object?[] arguments) =>
        instance.GetType()
            .GetMethod(methodName, BindingFlags.Instance | BindingFlags.NonPublic)
            ?.Invoke(instance, arguments)
        ?? throw new MissingMethodException(instance.GetType().FullName, methodName);

    private static T GetPrivateField<T>(object instance, string fieldName) =>
        instance.GetType()
            .GetField(fieldName, BindingFlags.Instance | BindingFlags.NonPublic)
            ?.GetValue(instance) is T value
            ? value
            : throw new MissingFieldException(instance.GetType().FullName, fieldName);

    private static void SetPrivateField<T>(object instance, string fieldName, T value) =>
        instance.GetType()
            .GetField(fieldName, BindingFlags.Instance | BindingFlags.NonPublic)
            ?.SetValue(instance, value);

    private static void PumpUntilCompleted(Task task)
    {
        while (!task.IsCompleted)
        {
            Dispatcher.CurrentDispatcher.Invoke(DispatcherPriority.Background, new Action(static () => { }));
        }
    }

    private static void PumpUntil(Func<bool> condition)
    {
        var deadline = DateTime.UtcNow + TimeSpan.FromSeconds(2);
        while (!condition() && DateTime.UtcNow < deadline)
        {
            Dispatcher.CurrentDispatcher.Invoke(DispatcherPriority.Background, new Action(static () => { }));
        }

        Assert.IsTrue(condition(), "等待 UI 播放状态变更超时。");
    }

    private static void InvokeVoidPrivate(object instance, string methodName, params object?[] arguments)
    {
        var method = instance.GetType()
            .GetMethod(methodName, BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new MissingMethodException(instance.GetType().FullName, methodName);
        method.Invoke(instance, arguments);
    }

    private sealed class SuccessfulFfprobeRunner : IExternalProcessRunner
    {
        public Task<ExternalProcessResult> RunAsync(
            ExternalProcessPlan plan,
            CancellationToken cancellationToken) =>
            Task.FromResult(new ExternalProcessResult(
                ExternalProcessRunStatus.Completed,
                0,
                "{\"format\":{\"duration\":\"1\"},\"streams\":[{\"codec_type\":\"video\",\"width\":320,\"height\":180,\"avg_frame_rate\":\"30/1\",\"codec_name\":\"h264\"},{\"codec_type\":\"audio\",\"sample_rate\":48000,\"channels\":2,\"codec_name\":\"aac\"}]}",
                string.Empty));
    }
}
