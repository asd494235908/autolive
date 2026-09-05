using System.IO;
using System.Reflection;
using System.Security.Cryptography;
using System.Windows.Controls;
using System.Windows.Threading;
using GpAutoLive.App.Features.Auth;
using GpAutoLive.App.Features.Playback;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
[DoNotParallelize]
public sealed class VideoEffectPixelFixtureTests
{
    private sealed class FrameCaptureState
    {
        public TaskCompletionSource<byte[]>? RequestedFrame;

        public byte[]? BaselineHash;

        public ulong Sequence;
    }

    [TestMethod]
    public void Explicit_wpf_fixture_confirms_cpu4_changes_final_video_frame()
    {
        RunExplicitFixture(MpvVideoProcessingMode.Cpu4);
    }

    [TestMethod]
    public void Explicit_wpf_fixture_confirms_gpu83_changes_final_video_frame()
    {
        RunExplicitFixture(MpvVideoProcessingMode.Gpu83);
    }

    private static void RunExplicitFixture(MpvVideoProcessingMode mode)
    {
        if (!string.Equals(
                Environment.GetEnvironmentVariable("AUTOLIVE_TEST_PIXEL_EFFECTS"),
                "1",
                StringComparison.Ordinal))
        {
            // 像素夹具需要真实 Windows 桌面、D3D11 和可见的最终效果 HWND；普通套件保持隔离。
            return;
        }

        var fixture = Path.GetFullPath(
            Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA")
            ?? @"E:\下载\csharp-golden-av.mp4");
        var runtime = Path.GetFullPath(
            Environment.GetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT")
            ?? FindRuntimeFromRepository());
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
            WpfTestApplicationHost.Run(() => RunFixture(fixture, mode));
        }
        finally
        {
            Environment.SetEnvironmentVariable("AUTOLIVE_MEDIA_RUNTIME_ROOT", previousRuntime);
        }
    }

    private static void RunFixture(string fixture, MpvVideoProcessingMode mode)
    {
        MainWindow? window = null;
        WindowsVirtualCameraSurfaceBinding? binding = null;
        WindowsGraphicsCaptureWindowSession? capture = null;
        WindowsGraphicsCaptureGpuYuy2Converter? converter = null;
        try
        {
            window = new MainWindow();
            window.Show();
            GetPrivateField<ShellState>(window, "_state").VideoProcessing = mode is MpvVideoProcessingMode.Gpu83;
            GetPrivateField<LoginViewModel>(window, "_login").ApplyActivated("fixture-account");

            var importMethod = window.GetType()
                .GetMethod("RunImportAsync", BindingFlags.Instance | BindingFlags.NonPublic)
                ?? throw new MissingMethodException(window.GetType().FullName, "RunImportAsync");
            var requestFactory = new Func<Task<MediaImportRequest?>>(
                () => Task.FromResult<MediaImportRequest?>(
                    new(MediaImportOperation.ReplaceAll, [fixture])));
            var importTask = importMethod.Invoke(window, [requestFactory]) as Task
                ?? throw new InvalidOperationException("主窗口导入入口未返回异步任务。");
            PumpUntilCompleted(importTask, "导入媒体");
            importTask.GetAwaiter().GetResult();

            var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
            if (mediaPool.Snapshot.SourceMediaPool.Length != 1
                || mediaPool.Snapshot.PlaybackState is not PlaybackState.Ready)
            {
                throw new InvalidOperationException("像素夹具导入后媒体池未处于单项 Ready 状态。");
            }
            GetPrivateField<ComboBox>(window, "AudioOutputDeviceComboBox").SelectedValue = int.MaxValue;
            var startTask = InvokePrivate(window, "TogglePlaybackCoreAsync") as Task
                ?? throw new InvalidOperationException("主窗口播放入口未返回异步任务。");
            PumpUntilCompleted(startTask, "启动视频");
            startTask.GetAwaiter().GetResult();

            if (mediaPool.Snapshot.PlaybackState is not PlaybackState.Playing)
            {
                throw new InvalidOperationException("像素夹具原始视频没有进入 Playing。");
            }

            var finalEffectWindow = GetPrivateField<FinalEffectWindow>(window, "_finalEffectWindow");
            if (!finalEffectWindow.TryGetCaptureWindowHandle(out var surfaceHandle))
            {
                throw new InvalidOperationException("像素夹具未取得最终效果窗口 HWND。");
            }

            binding = new WindowsVirtualCameraSurfaceBinding();
            var bound = binding.Bind(surfaceHandle);
            if (!bound.IsSuccess)
            {
                throw new InvalidOperationException("像素夹具绑定最终效果视频表面失败。");
            }

            converter = new WindowsGraphicsCaptureGpuYuy2Converter();
            var frameState = new FrameCaptureState
            {
                RequestedFrame = NewFrameRequest(),
            };
            capture = new WindowsGraphicsCaptureWindowSession();
            var captureResult = capture.StartAsync(
                    binding,
                    TimeSpan.FromSeconds(10),
                    frameConsumer: (frame, context) =>
                    {
                        var converted = converter.TryConvert(
                            frame,
                            context,
                            binding.Snapshot.Generation,
                            unchecked(++frameState.Sequence));
                        if (!converted.IsSuccess || converted.Frame is null)
                        {
                            return;
                        }

                        var hash = SHA256.HashData(converted.Frame.Payload);
                        var baseline = Volatile.Read(ref frameState.BaselineHash);
                        if (baseline is null || !hash.AsSpan().SequenceEqual(baseline))
                        {
                            Volatile.Read(ref frameState.RequestedFrame)?.TrySetResult(hash);
                        }
                    })
                .GetAwaiter()
                .GetResult();
            if (!captureResult.IsSuccess)
            {
                Assert.Inconclusive($"WGC 最终效果表面夹具不可用：{captureResult.Code}。");
            }
            var originalHash = WaitForFrame(frameState, "原始视频帧");
            Volatile.Write(ref frameState.BaselineHash, originalHash);

            var identity = mediaPool.CurrentIdentity
                ?? throw new InvalidOperationException("像素夹具没有当前媒体播放身份。");
            var controller = GetPrivateField<WindowsMpvPlaybackController>(window, "_mpvController");
            MpvVideoEffectSnapshot? processedSnapshot;
            MpvVideoParameterError? processedError;
            bool created;
            if (mode is MpvVideoProcessingMode.Gpu83)
            {
                var source = mediaPool.Snapshot.SourceMediaPool[mediaPool.Snapshot.SourceMediaIndex];
                created = MpvVideoEffectSnapshot.TryCreateGpu83(
                    VideoEffectParams.Default with
                    {
                        BrightnessPercent = 100,
                        ContrastPercent = 200,
                        SaturationPercent = 0,
                        HueRotationDegrees = 120,
                    },
                    AdvancedEffectParams.Default,
                    source.FrameRateFps ?? 30,
                    epochStartSeconds: 0,
                    randomSeed: 2,
                    out processedSnapshot,
                    out _,
                    out processedError);
            }
            else
            {
                created = MpvVideoEffectSnapshot.TryCreate(
                    MpvVideoProcessingMode.Cpu4,
                    brightnessPercent: 100,
                    contrastPercent: 200,
                    saturationPercent: 0,
                    hueRotationDegrees: 120,
                    MpvShaderOptionsSnapshot.Empty,
                    out processedSnapshot,
                    out processedError);
            }

            if (!created || processedSnapshot is null)
            {
                throw new InvalidOperationException(processedError?.Message ?? $"像素夹具 {mode} 快照创建失败。");
            }

            var updated = controller.UpdateEffectsAsync(identity, processedSnapshot)
                .GetAwaiter()
                .GetResult();
            if (!updated.IsSuccess)
            {
                throw new InvalidOperationException(updated.Error?.Message ?? $"像素夹具 {mode} 更新失败。");
            }

            Volatile.Write(ref frameState.RequestedFrame, NewFrameRequest());
            var processedHash = WaitForFrame(frameState, $"{mode} 处理视频帧");
            if (processedHash.AsSpan().SequenceEqual(originalHash))
            {
                throw new InvalidOperationException($"{mode} 参数已回读，但最终效果表面帧哈希没有变化。");
            }
        }
        finally
        {
            try
            {
                if (capture is not null)
                {
                    capture.StopAsync().GetAwaiter().GetResult();
                    capture.DisposeAsync().AsTask().GetAwaiter().GetResult();
                }
            }
            finally
            {
                converter?.Dispose();
                binding?.Dispose();
                try
                {
                    StopPlaybackForTest(window);
                }
                finally
                {
                    window?.Close();
                }
            }
        }
    }

    private static byte[] WaitForFrame(FrameCaptureState state, string description)
    {
        var request = Volatile.Read(ref state.RequestedFrame)
            ?? throw new InvalidOperationException($"{description}请求未创建。");
        var deadline = DateTime.UtcNow + TimeSpan.FromSeconds(10);
        while (!request.Task.IsCompleted && DateTime.UtcNow < deadline)
        {
            Thread.Sleep(10);
        }

        if (!request.Task.IsCompleted)
        {
            throw new InvalidOperationException($"等待{description}超时。");
        }

        return request.Task.GetAwaiter().GetResult();
    }

    private static TaskCompletionSource<byte[]> NewFrameRequest() =>
        new(TaskCreationOptions.RunContinuationsAsynchronously);

    private static object? InvokePrivate(object instance, string methodName) =>
        instance.GetType()
            .GetMethod(methodName, BindingFlags.Instance | BindingFlags.NonPublic)
            ?.Invoke(instance, null);

    private static void StopPlaybackForTest(MainWindow? window)
    {
        if (window is null)
        {
            return;
        }

        var stopTask = InvokePrivate(window, "StopPlaybackCoreAsync") as Task;
        if (stopTask is null)
        {
            return;
        }

        PumpUntilCompleted(stopTask, "停止媒体播放");
        stopTask.GetAwaiter().GetResult();
    }

    private static T GetPrivateField<T>(object instance, string fieldName) =>
        instance.GetType()
            .GetField(fieldName, BindingFlags.Instance | BindingFlags.NonPublic)
            ?.GetValue(instance) is T value
            ? value
            : throw new MissingFieldException(instance.GetType().FullName, fieldName);

    private static void PumpUntilCompleted(Task task, string description)
    {
        if (task.IsCompleted)
        {
            return;
        }

        var dispatcher = Dispatcher.CurrentDispatcher;
        var frame = new DispatcherFrame();
        var deadline = DateTime.UtcNow + TimeSpan.FromSeconds(15);
        var timer = new DispatcherTimer(
            TimeSpan.FromMilliseconds(25),
            DispatcherPriority.Background,
            (_, _) =>
            {
                if (task.IsCompleted || DateTime.UtcNow >= deadline)
                {
                    frame.Continue = false;
                }
            },
            dispatcher);
        _ = task.ContinueWith(
            _ => dispatcher.BeginInvoke(
                DispatcherPriority.Send,
                new Action(() => frame.Continue = false)),
            CancellationToken.None,
            TaskContinuationOptions.ExecuteSynchronously,
            TaskScheduler.Default);
        timer.Start();
        Dispatcher.PushFrame(frame);
        timer.Stop();

        if (!task.IsCompleted)
        {
            throw new InvalidOperationException($"{description}任务在 15 秒内未完成。");
        }
    }

    private static string FindRuntimeFromRepository()
    {
        for (var directory = new DirectoryInfo(AppContext.BaseDirectory);
             directory is not null;
             directory = directory.Parent)
        {
            foreach (var candidateName in new[] { "csharp-gpu83-real-v2", "csharp-windows-controller-20260903-v90" })
            {
                var candidate = Path.Combine(directory.FullName, "artifacts", candidateName);
                if (Directory.Exists(candidate))
                {
                    return candidate;
                }
            }
        }

        return Path.Combine(AppContext.BaseDirectory, "missing-csharp-media-runtime");
    }
}
