using System.Runtime.InteropServices;
using System.Reflection;
using System.Security.Cryptography;
using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsMpvRealFixtureTests
{
    [TestMethod]
    public async Task Real_ffprobe_imports_the_explicit_download_fixture_into_the_media_pool()
    {
        var installationDirectory = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_INSTALL_ROOT");
        var mediaPath = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA");
        if (string.IsNullOrWhiteSpace(installationDirectory)
            || string.IsNullOrWhiteSpace(mediaPath))
        {
            // Real media fixtures are opt-in; the regular suite remains hermetic.
            return;
        }

        Assert.IsTrue(OperatingSystem.IsWindows(), "真实 FFprobe 夹具只支持 Windows。");
        Assert.IsTrue(File.Exists(mediaPath), "显式夹具媒体文件不存在。");

        var runtimeResult = await RuntimeMediaManifestBoundary.LoadAndVerifyAsync(
            installationDirectory,
            MediaRuntimeBoundary.DefaultRuntimeVersion);
        Assert.IsTrue(runtimeResult.IsSuccess, runtimeResult.Error?.Message);
        Assert.IsNotNull(runtimeResult.Runtime);
        Assert.IsTrue(runtimeResult.Runtime.TryCreateFfprobeProbe(
            new WindowsExternalProcessRunner(),
            out var probe,
            out var probeError), probeError?.Message);
        Assert.IsNotNull(probe);

        var pool = new MediaPoolService();
        using var importer = new MediaImportCoordinator(pool, probe);
        var result = await importer.ImportAsync(new(
            MediaImportOperation.ReplaceAll,
            [mediaPath]));

        Assert.IsTrue(result.IsSuccess, result.Error?.Message);
        Assert.AreEqual(1, result.ProbedCount);
        Assert.AreEqual(1, result.Snapshot.SourceMediaPool.Length);
        Assert.AreEqual(MediaKind.Video, result.Snapshot.SourceMediaPool[0].MediaKind);
        Assert.IsFalse(string.IsNullOrWhiteSpace(result.Snapshot.SourceMediaPool[0].VideoCodecName));
        Assert.IsFalse(string.IsNullOrWhiteSpace(result.Snapshot.SourceMediaPool[0].AudioCodecName));
    }

    [TestMethod]
    public async Task Real_mpv_reads_playback_time_and_eof_when_explicitly_enabled()
    {
        var installationDirectory = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_INSTALL_ROOT");
        var mediaPath = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA");
        var modeText = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MODE");
        if (string.IsNullOrWhiteSpace(installationDirectory)
            || string.IsNullOrWhiteSpace(mediaPath))
        {
            // Real media/GUI fixtures are opt-in; the regular suite remains hermetic.
            return;
        }

        Assert.IsTrue(OperatingSystem.IsWindows(), "真实 mpv 夹具只支持 Windows。");
        Assert.IsTrue(File.Exists(mediaPath), "显式夹具媒体文件不存在。");
        var mode = string.IsNullOrWhiteSpace(modeText)
            ? MpvLaunchMode.Original
            : Enum.TryParse<MpvLaunchMode>(modeText, ignoreCase: true, out var parsedMode)
                ? parsedMode
                : throw new AssertFailedException("AUTOLIVE_TEST_MPV_MODE 不是受支持的 mpv 模式。");
        var useFullGpu83 = mode is MpvLaunchMode.Gpu83
            && string.Equals(
                Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_GPU83_FULL"),
                "1",
                StringComparison.Ordinal);

        var runtimeResult = await RuntimeMediaManifestBoundary.LoadAndVerifyAsync(
            installationDirectory,
            MediaRuntimeBoundary.DefaultRuntimeVersion);
        Assert.IsTrue(runtimeResult.IsSuccess, runtimeResult.Error?.Message);
        Assert.IsNotNull(runtimeResult.Runtime);

        using var hostWindow = NativeHostWindow.Create();
        var fileInfo = new FileInfo(mediaPath);
        var source = new SourceMediaDto(
            mediaPath,
            mediaPath,
            MediaKind.Video,
            MediaCompatibilityMode.Direct,
            fileInfo.Name,
            checked((ulong)fileInfo.Length),
            2_000,
            null,
            null,
            320,
            180,
            30,
            null,
            null,
            "mpeg4",
            null,
            null,
            "not_calculated");
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        MpvVideoEffectSnapshot? initialEffectSnapshot = null;
        if (mode is MpvLaunchMode.Cpu4)
        {
            Assert.IsTrue(MpvVideoEffectSnapshot.TryCreate(
                MpvVideoProcessingMode.Cpu4,
                brightnessPercent: 25,
                contrastPercent: 150,
                saturationPercent: 80,
                hueRotationDegrees: -10,
                MpvShaderOptionsSnapshot.Empty,
                out initialEffectSnapshot,
                out var effectError), effectError?.Message);
        }
        else if (mode is MpvLaunchMode.Gpu83)
        {
            if (useFullGpu83)
            {
                Assert.IsTrue(MpvVideoEffectSnapshot.TryCreateGpu83(
                    FullGpu83Video(25, 150, 80, -10),
                    FullGpu83Advanced(),
                    sourceFps: source.FrameRateFps ?? 30,
                    epochStartSeconds: 0,
                    randomSeed: 42,
                    out initialEffectSnapshot,
                    out _,
                    out var effectError), effectError?.Message);
            }
            else
            {
                Assert.IsTrue(MpvVideoEffectSnapshot.TryCreateGpu83BaselineColor(
                    brightnessPercent: 25,
                    contrastPercent: 150,
                    saturationPercent: 80,
                    hueRotationDegrees: -10,
                    out initialEffectSnapshot,
                    out var effectError), effectError?.Message);
            }
        }

        await using var controller = new WindowsMpvPlaybackController();
        var started = await controller.StartAsync(
            runtimeResult.Runtime,
            source,
            identity,
            checked((uint)hostWindow.Handle.ToInt64()),
            mode,
            initialEffectSnapshot: initialEffectSnapshot,
            waitForFirstFrame: true);
        Assert.IsTrue(started.IsSuccess, started.Error?.Message);

        var seek = await controller.SeekAsync(identity, 500);
        Assert.IsTrue(seek.IsSuccess, seek.Error?.Message);

        var switchPath = Path.Combine(
            Path.GetTempPath(),
            $"gpautolive-mpv-switch-{Guid.NewGuid():N}.mp4");
        File.Copy(mediaPath, switchPath);
        try
        {
            var switchedSource = source with
            {
                SourcePath = switchPath,
                PlaybackReference = switchPath,
                FileName = Path.GetFileName(switchPath),
            };
            var switchedIdentity = new MediaPlaybackIdentity(1, 1, 1, 0);
            var switched = await controller.SwitchSourceAsync(switchedSource, switchedIdentity);
            Assert.IsTrue(switched.IsSuccess, switched.Error?.Message);
            identity = switchedIdentity;

            var pathReadback = await controller.ReadPropertyAsync(identity, MpvIpcProperty.MediaPath);
            Assert.IsTrue(pathReadback.IsSuccess, pathReadback.IpcError?.Message ?? pathReadback.SessionError?.Message);
            Assert.IsTrue(MpvIpcValueReader.TryReadString(pathReadback.Frame!, out var activePath, out var pathError), pathError?.Message);
            Assert.AreEqual(Path.GetFullPath(switchPath), Path.GetFullPath(activePath!));

            var eofReadback = await controller.ReadPropertyAsync(identity, MpvIpcProperty.EofReached);
            Assert.IsTrue(eofReadback.IsSuccess, eofReadback.IpcError?.Message ?? eofReadback.SessionError?.Message);
            Assert.IsTrue(MpvIpcValueReader.TryReadBoolean(eofReadback.Frame!, out var eofReached, out var eofError), eofError?.Message);
            Assert.IsFalse(eofReached, "换源成功返回时新源不能仍停留在 EOF。");

            var frameReadback = await controller.ReadPropertyAsync(identity, MpvIpcProperty.EstimatedFrameNumber);
            Assert.IsTrue(frameReadback.IsSuccess, frameReadback.IpcError?.Message ?? frameReadback.SessionError?.Message);
            Assert.IsTrue(MpvIpcValueReader.TryReadFiniteDouble(frameReadback.Frame!, out var frameNumber, out var frameError), frameError?.Message);
            Assert.IsTrue(frameNumber is > 0, "换源成功返回时必须已经观察到新源首帧。");
        }
        finally
        {
            File.Delete(switchPath);
        }

        if (mode is MpvLaunchMode.Cpu4)
        {
            var filterReadback = await controller.ReadPropertyAsync(
                identity,
                MpvIpcProperty.VideoFilterChain);
            Assert.IsTrue(filterReadback.IsSuccess, filterReadback.IpcError?.Message ?? filterReadback.SessionError?.Message);
            Assert.IsTrue(filterReadback.Frame?.Data is JsonElement,
                "真实 mpv 未返回视频滤镜链属性。");
            StringAssert.Contains(
                filterReadback.Frame!.Data!.Value.GetRawText(),
                "autolive_cpu4",
                "真实 mpv 回读的滤镜链中没有 CPU4 固定标签。");

            Assert.IsTrue(MpvVideoEffectSnapshot.TryCreate(
                MpvVideoProcessingMode.Cpu4,
                brightnessPercent: 10,
                contrastPercent: 110,
                saturationPercent: 90,
                hueRotationDegrees: 5,
                MpvShaderOptionsSnapshot.Empty,
                out var updatedEffectSnapshot,
                out var updatedEffectError), updatedEffectError?.Message);
            var updated = await controller.UpdateEffectsAsync(
                identity,
                updatedEffectSnapshot,
                waitForNextFrame: true);
            Assert.IsTrue(updated.IsSuccess, updated.Error?.Message);

            var updatedFilterReadback = await controller.ReadPropertyAsync(
                identity,
                MpvIpcProperty.VideoFilterChain);
            Assert.IsTrue(
                updatedFilterReadback.IsSuccess,
                updatedFilterReadback.IpcError?.Message ?? updatedFilterReadback.SessionError?.Message);
            var updatedFilter = updatedFilterReadback.Frame?.Data;
            Assert.IsTrue(updatedFilter is JsonElement, "真实 mpv 未返回更新后的滤镜链属性。");
            StringAssert.Contains(updatedFilter!.Value.GetRawText(), "brightness=0.1");
            StringAssert.Contains(updatedFilter.Value.GetRawText(), "contrast=1.1");
            StringAssert.Contains(updatedFilter.Value.GetRawText(), "saturation=0.9");
        }
        else if (mode is MpvLaunchMode.Gpu83)
        {
            var shaderReadback = await controller.ReadPropertyAsync(
                identity,
                MpvIpcProperty.ShaderOptions);
            Assert.IsTrue(
                shaderReadback.IsSuccess,
                shaderReadback.IpcError?.Message ?? shaderReadback.SessionError?.Message);
            Assert.IsTrue(shaderReadback.Frame?.Data is JsonElement,
                "真实 mpv 未返回 GPU83 shader 参数属性。");
            var shaderOptions = shaderReadback.Frame!.Data!.Value;
            Assert.AreEqual("25", shaderOptions.GetProperty("al_brightness_percent").GetString());
            Assert.AreEqual("150", shaderOptions.GetProperty("al_contrast_percent").GetString());
            Assert.AreEqual("-10", shaderOptions.GetProperty("al_hue_degrees").GetString());
            Assert.AreEqual("80", shaderOptions.GetProperty("al_saturation_percent").GetString());

            MpvVideoEffectSnapshot? updatedEffectSnapshot;
            if (useFullGpu83)
            {
                Assert.IsTrue(MpvVideoEffectSnapshot.TryCreateGpu83(
                    FullGpu83Video(-15, 120, 95, 8),
                    FullGpu83Advanced(),
                    sourceFps: source.FrameRateFps ?? 30,
                    epochStartSeconds: 0,
                    randomSeed: 43,
                    out updatedEffectSnapshot,
                    out _,
                    out var updatedEffectError), updatedEffectError?.Message);
            }
            else
            {
                Assert.IsTrue(MpvVideoEffectSnapshot.TryCreateGpu83BaselineColor(
                    brightnessPercent: -15,
                    contrastPercent: 120,
                    saturationPercent: 95,
                    hueRotationDegrees: 8,
                    out updatedEffectSnapshot,
                    out var updatedEffectError), updatedEffectError?.Message);
            }
            var updated = await controller.UpdateEffectsAsync(
                identity,
                updatedEffectSnapshot,
                waitForNextFrame: true);
            Assert.IsTrue(updated.IsSuccess, updated.Error?.Message);

            var updatedShaderReadback = await controller.ReadPropertyAsync(
                identity,
                MpvIpcProperty.ShaderOptions);
            Assert.IsTrue(
                updatedShaderReadback.IsSuccess,
                updatedShaderReadback.IpcError?.Message ?? updatedShaderReadback.SessionError?.Message);
            Assert.IsTrue(updatedShaderReadback.Frame?.Data is JsonElement,
                "真实 mpv 未返回更新后的 GPU83 shader 参数属性。");
            var updatedShaderOptions = updatedShaderReadback.Frame!.Data!.Value;
            Assert.AreEqual("-15", updatedShaderOptions.GetProperty("al_brightness_percent").GetString());
            Assert.AreEqual("120", updatedShaderOptions.GetProperty("al_contrast_percent").GetString());
            Assert.AreEqual("8", updatedShaderOptions.GetProperty("al_hue_degrees").GetString());
            Assert.AreEqual("95", updatedShaderOptions.GetProperty("al_saturation_percent").GetString());
            if (useFullGpu83)
            {
                Assert.AreEqual("2", updatedShaderOptions.GetProperty("al_blur_radius_px").GetString());
                Assert.AreEqual("1.25", updatedShaderOptions.GetProperty("al_band_1110").GetString());
                Assert.AreEqual("30", updatedShaderOptions.GetProperty("al_runtime_source_fps").GetString());
                Assert.AreEqual("43", updatedShaderOptions.GetProperty("al_runtime_random_seed").GetString());
                Assert.AreEqual("5", updatedShaderOptions.GetProperty("al_runtime_frame_probability_percent").GetString());
            }
        }

        var originalUpdate = await controller.UpdateEffectsAsync(
            identity,
            MpvVideoEffectSnapshot.Default,
            waitForNextFrame: true);
        Assert.IsTrue(originalUpdate.IsSuccess, originalUpdate.Error?.Message);

        var sawPositivePlaybackTime = false;
        var sawEof = false;
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
        try
        {
            await foreach (var state in controller.WatchPlaybackStateAsync(identity, timeout.Token))
            {
                Assert.IsTrue(state.IsSuccess, state.Error?.Message);
                Assert.IsNotNull(state.Snapshot);
                sawPositivePlaybackTime |= state.Snapshot!.PlaybackTimeMs is > 0;
                if (state.Snapshot.EofReached)
                {
                    sawEof = true;
                    break;
                }
            }
        }
        finally
        {
            var stopped = await controller.ShutdownAsync();
            Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
        }

        Assert.IsTrue(sawPositivePlaybackTime, "真实 mpv 未观察到首个有效播放时间。");
        Assert.IsTrue(sawEof, "真实 mpv 未观察到 EOF。");
    }

    [TestMethod]
    public async Task Real_controller_faults_when_ipc_closes_while_mpv_remains_running()
    {
        var installationDirectory = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_INSTALL_ROOT");
        var mediaPath = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA");
        if (string.IsNullOrWhiteSpace(installationDirectory)
            || string.IsNullOrWhiteSpace(mediaPath))
        {
            return;
        }

        Assert.IsTrue(OperatingSystem.IsWindows());
        Assert.IsTrue(File.Exists(mediaPath));
        var runtimeResult = await RuntimeMediaManifestBoundary.LoadAndVerifyAsync(
            installationDirectory,
            MediaRuntimeBoundary.DefaultRuntimeVersion);
        Assert.IsTrue(runtimeResult.IsSuccess, runtimeResult.Error?.Message);
        Assert.IsNotNull(runtimeResult.Runtime);

        using var hostWindow = NativeHostWindow.Create();
        var fileInfo = new FileInfo(mediaPath);
        var source = new SourceMediaDto(
            mediaPath,
            mediaPath,
            MediaKind.Video,
            MediaCompatibilityMode.Direct,
            fileInfo.Name,
            checked((ulong)fileInfo.Length),
            2_000,
            null,
            null,
            320,
            180,
            30,
            null,
            null,
            "mpeg4",
            null,
            null,
            "not_calculated");
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);

        await using var controller = new WindowsMpvPlaybackController();
        var started = await controller.StartAsync(
            runtimeResult.Runtime,
            source,
            identity,
            checked((uint)hostWindow.Handle.ToInt64()),
            MpvLaunchMode.Original,
            waitForFirstFrame: true);
        Assert.IsTrue(started.IsSuccess, started.Error?.Message);
        Assert.AreEqual(WindowsMpvPlaybackRuntimeState.Running, controller.Snapshot.Runtime.State);
        Assert.AreEqual(MpvIpcPipeState.Connected, controller.Snapshot.Runtime.IpcState);

        var runtimeField = typeof(WindowsMpvPlaybackController).GetField(
            "_runtime",
            BindingFlags.Instance | BindingFlags.NonPublic);
        Assert.IsNotNull(runtimeField);
        var runtime = runtimeField!.GetValue(controller) as WindowsMpvPlaybackRuntime;
        Assert.IsNotNull(runtime);
        var gatewayField = typeof(WindowsMpvPlaybackRuntime).GetField(
            "_gateway",
            BindingFlags.Instance | BindingFlags.NonPublic);
        Assert.IsNotNull(gatewayField);
        var gateway = gatewayField!.GetValue(runtime) as MpvPlaybackIpcGateway;
        Assert.IsNotNull(gateway);

        // 只断开 IPC；受管 mpv 进程仍应保持 Running，模拟控制链路失效。
        await gateway!.DisposeAsync();
        var disconnected = controller.Snapshot;
        Assert.AreEqual(WindowsMpvPlaybackRuntimeState.Running, disconnected.Runtime.State);
        Assert.AreEqual(MpvIpcPipeState.Closed, disconnected.Runtime.IpcState);
        Assert.AreEqual(
            WindowsMpvPlaybackControllerState.Faulted,
            disconnected.State,
            "IPC 已关闭但 mpv 仍活着时，控制器不能继续报告 Playing/Paused。");

        var stopped = await controller.ShutdownAsync();
        Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
        Assert.AreEqual(WindowsMpvPlaybackControllerState.Ready, controller.Snapshot.State);
    }

    [TestMethod]
    public async Task Real_switch_source_preserves_paused_intent_and_stop_releases_session()
    {
        var installationDirectory = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_INSTALL_ROOT");
        var mediaPath = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA");
        if (string.IsNullOrWhiteSpace(installationDirectory)
            || string.IsNullOrWhiteSpace(mediaPath))
        {
            return;
        }

        Assert.IsTrue(OperatingSystem.IsWindows());
        Assert.IsTrue(File.Exists(mediaPath));
        var runtimeResult = await RuntimeMediaManifestBoundary.LoadAndVerifyAsync(
            installationDirectory,
            MediaRuntimeBoundary.DefaultRuntimeVersion);
        Assert.IsTrue(runtimeResult.IsSuccess, runtimeResult.Error?.Message);
        Assert.IsNotNull(runtimeResult.Runtime);

        using var hostWindow = NativeHostWindow.Create();
        var fileInfo = new FileInfo(mediaPath);
        var source = new SourceMediaDto(
            mediaPath,
            mediaPath,
            MediaKind.Video,
            MediaCompatibilityMode.Direct,
            fileInfo.Name,
            checked((ulong)fileInfo.Length),
            2_000,
            null,
            null,
            320,
            180,
            30,
            null,
            null,
            "mpeg4",
            null,
            null,
            "not_calculated");
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        var switchedIdentity = new MediaPlaybackIdentity(1, 1, 1, 0);
        var switchPath = Path.Combine(
            Path.GetTempPath(),
            $"gpautolive-mpv-paused-switch-{Guid.NewGuid():N}.mp4");
        File.Copy(mediaPath, switchPath);

        var controller = new WindowsMpvPlaybackController();
        try
        {
            var started = await controller.StartAsync(
                runtimeResult.Runtime,
                source,
                identity,
                checked((uint)hostWindow.Handle.ToInt64()),
                MpvLaunchMode.Original,
                waitForFirstFrame: true);
            Assert.IsTrue(started.IsSuccess, started.Error?.Message);

            var paused = await controller.TogglePauseAsync(identity);
            Assert.IsTrue(paused.IsSuccess, paused.Error?.Message);
            Assert.AreEqual(WindowsMpvPlaybackControllerState.Paused, paused.Snapshot.State);

            var switched = await controller.SwitchSourceAsync(
                source with
                {
                    SourcePath = switchPath,
                    PlaybackReference = switchPath,
                    FileName = Path.GetFileName(switchPath),
                },
                switchedIdentity);
            Assert.IsTrue(switched.IsSuccess, switched.Error?.Message);
            Assert.AreEqual(switchedIdentity, switched.Snapshot.ActiveIdentity);
            Assert.AreEqual(WindowsMpvPlaybackControllerState.Paused, switched.Snapshot.State);

            var pausedReadback = await controller.ReadPropertyAsync(
                switchedIdentity,
                MpvIpcProperty.Paused);
            Assert.IsTrue(
                pausedReadback.IsSuccess,
                pausedReadback.IpcError?.Message ?? pausedReadback.SessionError?.Message);
            Assert.IsTrue(MpvIpcValueReader.TryReadBoolean(
                pausedReadback.Frame!,
                out var isPaused,
                out var pausedError), pausedError?.Message);
            Assert.IsTrue(isPaused, "暂停换源后 mpv 的物理暂停事实没有保持。");

            var resumed = await controller.TogglePauseAsync(switchedIdentity);
            Assert.IsTrue(resumed.IsSuccess, resumed.Error?.Message);
            Assert.AreEqual(WindowsMpvPlaybackControllerState.Playing, resumed.Snapshot.State);

            var stopped = await controller.StopPlaybackAsync(switchedIdentity);
            Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
            Assert.AreEqual(WindowsMpvPlaybackControllerState.Ready, stopped.Snapshot.State);
        }
        finally
        {
            var shutdown = await controller.ShutdownAsync();
            Assert.IsTrue(shutdown.IsSuccess, shutdown.Error?.Message);
            await controller.DisposeAsync();
            File.Delete(switchPath);
        }
    }

    [TestMethod]
    public async Task Real_mpv_continuous_pool_switches_only_after_observed_eof_and_new_source_playback()
    {
        var installationDirectory = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_INSTALL_ROOT");
        var mediaPath = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA");
        if (string.IsNullOrWhiteSpace(installationDirectory)
            || string.IsNullOrWhiteSpace(mediaPath))
        {
            return;
        }

        Assert.IsTrue(OperatingSystem.IsWindows(), "真实 mpv 播放池夹具只支持 Windows。");
        Assert.IsTrue(File.Exists(mediaPath), "显式夹具媒体文件不存在。");
        var runtimeResult = await RuntimeMediaManifestBoundary.LoadAndVerifyAsync(
            installationDirectory,
            MediaRuntimeBoundary.DefaultRuntimeVersion);
        Assert.IsTrue(runtimeResult.IsSuccess, runtimeResult.Error?.Message);
        Assert.IsNotNull(runtimeResult.Runtime);

        var firstPath = CreateFixtureCopy(mediaPath, "gpautolive-mpv-pool-first");
        var secondPath = CreateFixtureCopy(mediaPath, "gpautolive-mpv-pool-second");
        var thirdPath = CreateFixtureCopy(mediaPath, "gpautolive-mpv-pool-third");
        using var hostWindow = NativeHostWindow.Create();
        await using var controller = new WindowsMpvPlaybackController();
        try
        {
            var first = CreateFixtureSource(firstPath);
            var second = CreateFixtureSource(secondPath);
            var third = CreateFixtureSource(thirdPath);
            var firstIdentity = new MediaPlaybackIdentity(1, 1, 0, 0);
            var secondIdentity = new MediaPlaybackIdentity(1, 1, 1, 0);
            var thirdIdentity = new MediaPlaybackIdentity(1, 1, 2, 0);

            var started = await controller.StartAsync(
                runtimeResult.Runtime,
                first,
                firstIdentity,
                checked((uint)hostWindow.Handle.ToInt64()),
                MpvLaunchMode.Original,
                waitForFirstFrame: true);
            Assert.IsTrue(started.IsSuccess, started.Error?.Message);

            var firstEof = await WaitForObservedEofAsync(controller, firstIdentity, "播放池第 1 项");
            Assert.AreEqual(firstIdentity, firstEof.Identity);

            var switchedToSecond = await controller.SwitchSourceAsync(second, secondIdentity);
            Assert.IsTrue(switchedToSecond.IsSuccess, switchedToSecond.Error?.Message);
            await AssertSourceIsActuallyPlayingAsync(controller, secondIdentity, secondPath, "播放池第 2 项");

            var secondEof = await WaitForObservedEofAsync(controller, secondIdentity, "播放池第 2 项");
            Assert.AreEqual(secondIdentity, secondEof.Identity);

            var switchedToThird = await controller.SwitchSourceAsync(third, thirdIdentity);
            Assert.IsTrue(switchedToThird.IsSuccess, switchedToThird.Error?.Message);
            await AssertSourceIsActuallyPlayingAsync(controller, thirdIdentity, thirdPath, "播放池第 3 项");

            Assert.AreEqual(thirdIdentity, controller.Snapshot.ActiveIdentity);
            Assert.AreEqual(WindowsMpvPlaybackControllerState.Playing, controller.Snapshot.State);
        }
        finally
        {
            var stopped = await controller.ShutdownAsync();
            Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
            DeleteFixtureCopy(firstPath);
            DeleteFixtureCopy(secondPath);
            DeleteFixtureCopy(thirdPath);
        }
    }

    [TestMethod]
    public async Task Real_mpv_single_item_loops_by_reloading_same_path_with_new_loop_identity()
    {
        var installationDirectory = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_INSTALL_ROOT");
        var mediaPath = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA");
        if (string.IsNullOrWhiteSpace(installationDirectory)
            || string.IsNullOrWhiteSpace(mediaPath))
        {
            return;
        }

        Assert.IsTrue(OperatingSystem.IsWindows(), "真实 mpv 单项回环夹具只支持 Windows。");
        Assert.IsTrue(File.Exists(mediaPath), "显式夹具媒体文件不存在。");
        var runtimeResult = await RuntimeMediaManifestBoundary.LoadAndVerifyAsync(
            installationDirectory,
            MediaRuntimeBoundary.DefaultRuntimeVersion);
        Assert.IsTrue(runtimeResult.IsSuccess, runtimeResult.Error?.Message);
        Assert.IsNotNull(runtimeResult.Runtime);

        var loopPath = CreateFixtureCopy(mediaPath, "gpautolive-mpv-single-loop");
        using var hostWindow = NativeHostWindow.Create();
        await using var controller = new WindowsMpvPlaybackController();
        try
        {
            var source = CreateFixtureSource(loopPath);
            var loopZero = new MediaPlaybackIdentity(1, 1, 0, 0);
            var loopOne = new MediaPlaybackIdentity(1, 1, 0, 1);
            var loopTwo = new MediaPlaybackIdentity(1, 1, 0, 2);

            var started = await controller.StartAsync(
                runtimeResult.Runtime,
                source,
                loopZero,
                checked((uint)hostWindow.Handle.ToInt64()),
                MpvLaunchMode.Original,
                waitForFirstFrame: true);
            Assert.IsTrue(started.IsSuccess, started.Error?.Message);

            var firstEof = await WaitForObservedEofAsync(controller, loopZero, "单项第 1 个循环");
            Assert.AreEqual(loopZero, firstEof.Identity);

            var firstLoop = await controller.SwitchSourceAsync(source with { SourcePath = loopPath, PlaybackReference = loopPath }, loopOne);
            Assert.IsTrue(firstLoop.IsSuccess, firstLoop.Error?.Message);
            await AssertSourceIsActuallyPlayingAsync(controller, loopOne, loopPath, "单项第 2 个循环");

            var secondEof = await WaitForObservedEofAsync(controller, loopOne, "单项第 2 个循环");
            Assert.AreEqual(loopOne, secondEof.Identity);

            var secondLoop = await controller.SwitchSourceAsync(source with { SourcePath = loopPath, PlaybackReference = loopPath }, loopTwo);
            Assert.IsTrue(secondLoop.IsSuccess, secondLoop.Error?.Message);
            await AssertSourceIsActuallyPlayingAsync(controller, loopTwo, loopPath, "单项第 3 个循环");

            Assert.AreEqual(loopTwo, controller.Snapshot.ActiveIdentity);
            Assert.AreEqual(WindowsMpvPlaybackControllerState.Playing, controller.Snapshot.State);
        }
        finally
        {
            var stopped = await controller.ShutdownAsync();
            Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
            DeleteFixtureCopy(loopPath);
        }
    }

    [TestMethod]
    public async Task Real_gpu83_changes_captured_window_pixels_when_explicitly_enabled()
    {
        var installationDirectory = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_INSTALL_ROOT");
        var mediaPath = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA");
        if (string.IsNullOrWhiteSpace(installationDirectory)
            || string.IsNullOrWhiteSpace(mediaPath)
            || !string.Equals(
                Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_GPU83_PIXEL"),
                "1",
                StringComparison.Ordinal))
        {
            return;
        }

        Assert.IsTrue(OperatingSystem.IsWindows(), "真实 GPU83 像素夹具只支持 Windows。");
        Assert.IsTrue(File.Exists(mediaPath), "显式夹具媒体文件不存在。");
        var runtimeResult = await RuntimeMediaManifestBoundary.LoadAndVerifyAsync(
            installationDirectory,
            MediaRuntimeBoundary.DefaultRuntimeVersion);
        Assert.IsTrue(runtimeResult.IsSuccess, runtimeResult.Error?.Message);
        Assert.IsNotNull(runtimeResult.Runtime);

        using var hostWindow = NativeHostWindow.Create();
        var fileInfo = new FileInfo(mediaPath);
        var source = new SourceMediaDto(
            mediaPath,
            mediaPath,
            MediaKind.Video,
            MediaCompatibilityMode.Direct,
            fileInfo.Name,
            checked((ulong)fileInfo.Length),
            2_000,
            null,
            null,
            320,
            180,
            30,
            null,
            null,
            "mpeg4",
            null,
            null,
            "not_calculated");
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        Assert.IsTrue(
            MpvVideoEffectSnapshot.TryCreateGpu83(
                VideoEffectParams.Default,
                AdvancedEffectParams.Default,
                source.FrameRateFps ?? 30,
                epochStartSeconds: 0,
                randomSeed: 1,
                out var initialEffects,
                out _,
                out var initialError),
            initialError?.Message);

        await using var controller = new WindowsMpvPlaybackController();
        var started = await controller.StartAsync(
            runtimeResult.Runtime,
            source,
            identity,
            checked((uint)hostWindow.Handle.ToInt64()),
            MpvLaunchMode.Gpu83,
            initialEffectSnapshot: initialEffects,
            waitForFirstFrame: true);
        Assert.IsTrue(started.IsSuccess, started.Error?.Message);

        using var binding = new WindowsVirtualCameraSurfaceBinding();
        var bound = binding.Bind(checked((uint)hostWindow.Handle.ToInt64()));
        Assert.IsTrue(bound.IsSuccess, "GPU83 像素夹具无法绑定真实 mpv 窗口。");
        using var converter = new WindowsGraphicsCaptureGpuYuy2Converter();
        var originalFrame = NewFrameSignal();
        var processedFrame = NewFrameSignal();
        byte[]? baselineHash = null;
        ulong sequence = 0;
        var rawFrameCount = 0;
        var lastConversionCode = (int)WindowsGraphicsCaptureGpuYuy2ConversionCode.Pending;
        await using var capture = new WindowsGraphicsCaptureWindowSession();
        var captureResult = await capture.StartAsync(
            binding,
            TimeSpan.FromSeconds(10),
            frameConsumer: (frame, context) =>
            {
                Interlocked.Increment(ref rawFrameCount);
                var converted = converter.TryConvert(
                    frame,
                    context,
                    binding.Snapshot.Generation,
                    unchecked(++sequence));
                Volatile.Write(ref lastConversionCode, (int)converted.Code);
                if (converted.Frame is null)
                {
                    return;
                }

                var hash = SHA256.HashData(converted.Frame.Payload);
                var baseline = Volatile.Read(ref baselineHash);
                if (baseline is null)
                {
                    originalFrame.TrySetResult(hash);
                }
                else if (!hash.AsSpan().SequenceEqual(baseline))
                {
                    processedFrame.TrySetResult(hash);
                }
            });
        Assert.IsTrue(captureResult.IsSuccess, $"GPU83 像素夹具 WGC 启动失败：{captureResult.Code}");

        var originalHash = await WaitForFrameAsync(
            originalFrame,
            "GPU83 原始窗口帧",
            () => $"raw={Volatile.Read(ref rawFrameCount)}, converted={(WindowsGraphicsCaptureGpuYuy2ConversionCode)Volatile.Read(ref lastConversionCode)}, session={capture.Snapshot.FrameCount}");
        Volatile.Write(ref baselineHash, originalHash);
        Assert.IsTrue(
            MpvVideoEffectSnapshot.TryCreateGpu83(
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
                out var processedEffects,
                out _,
                out var processedError),
            processedError?.Message);
        var updated = await controller.UpdateEffectsAsync(
            identity,
            processedEffects,
            waitForNextFrame: true);
        Assert.IsTrue(updated.IsSuccess, updated.Error?.Message);

        var processedHash = await WaitForFrameAsync(
            processedFrame,
            "GPU83 处理窗口帧",
            () => $"raw={Volatile.Read(ref rawFrameCount)}, converted={(WindowsGraphicsCaptureGpuYuy2ConversionCode)Volatile.Read(ref lastConversionCode)}, session={capture.Snapshot.FrameCount}");
        Assert.IsFalse(
            processedHash.AsSpan().SequenceEqual(originalHash),
            "GPU83 参数已回读，但真实 mpv 窗口的最终 YUY2 帧没有变化。");

        await controller.ShutdownAsync();
    }

    private static TaskCompletionSource<byte[]> NewFrameSignal() =>
        new(TaskCreationOptions.RunContinuationsAsynchronously);

    private static async Task<byte[]> WaitForFrameAsync(
        TaskCompletionSource<byte[]> signal,
        string description,
        Func<string>? diagnostics = null)
    {
        try
        {
            return await signal.Task.WaitAsync(TimeSpan.FromSeconds(10));
        }
        catch (TimeoutException)
        {
            var detail = diagnostics is null ? string.Empty : $"（{diagnostics()}）";
            Assert.Inconclusive($"当前 Windows/WGC 环境在有界时间内没有产出{description}{detail}；未把无帧环境计为失败或通过。");
            return Array.Empty<byte>();
        }
    }

    private static string CreateFixtureCopy(string sourcePath, string name)
    {
        var path = Path.Combine(Path.GetTempPath(), $"{name}-{Guid.NewGuid():N}.mp4");
        File.Copy(sourcePath, path);
        return path;
    }

    private static void DeleteFixtureCopy(string path)
    {
        try
        {
            File.Delete(path);
        }
        catch (IOException)
        {
            // The controller owns process cleanup; the outer hard timeout also kills leftovers.
        }
        catch (UnauthorizedAccessException)
        {
            // Keep the real-fixture test from hiding the playback assertion behind cleanup noise.
        }
    }

    private static SourceMediaDto CreateFixtureSource(string path) =>
        new(
            path,
            path,
            MediaKind.Video,
            MediaCompatibilityMode.Direct,
            Path.GetFileName(path),
            checked((ulong)new FileInfo(path).Length),
            2_000,
            null,
            null,
            320,
            180,
            30,
            null,
            null,
            "mpeg4",
            null,
            null,
            "not_calculated");

    private static async Task<MpvPlaybackStateSnapshot> WaitForObservedEofAsync(
        WindowsMpvPlaybackController controller,
        MediaPlaybackIdentity identity,
        string description)
    {
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(8));
        await foreach (var result in controller.WatchPlaybackStateAsync(identity, timeout.Token))
        {
            if (!result.IsSuccess || result.Snapshot is null)
            {
                throw new AssertFailedException(
                    $"{description} 未在有界观察中得到有效 mpv 状态：{result.Error?.Message ?? "未知错误"}");
            }

            if (result.Snapshot.EofReached)
            {
                return result.Snapshot;
            }
        }

        throw new AssertFailedException($"{description} 未在 8 秒内观察到真实 EOF。");
    }

    private static async Task AssertSourceIsActuallyPlayingAsync(
        WindowsMpvPlaybackController controller,
        MediaPlaybackIdentity identity,
        string expectedPath,
        string description)
    {
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(3));
        while (!timeout.IsCancellationRequested)
        {
            var path = await controller.ReadPropertyAsync(identity, MpvIpcProperty.MediaPath, timeout.Token);
            var eof = await controller.ReadPropertyAsync(identity, MpvIpcProperty.EofReached, timeout.Token);
            var paused = await controller.ReadPropertyAsync(identity, MpvIpcProperty.Paused, timeout.Token);
            var frame = await controller.ReadPropertyAsync(identity, MpvIpcProperty.EstimatedFrameNumber, timeout.Token);
            var position = await controller.ReadPropertyAsync(identity, MpvIpcProperty.PlaybackTime, timeout.Token);
            if (path.IsSuccess
                && eof.IsSuccess
                && paused.IsSuccess
                && frame.IsSuccess
                && position.IsSuccess
                && MpvIpcValueReader.TryReadString(path.Frame!, out var activePath, out _)
                && MpvIpcValueReader.TryReadBoolean(eof.Frame!, out var eofReached, out _)
                && MpvIpcValueReader.TryReadBoolean(paused.Frame!, out var isPaused, out _)
                && TryReadPositiveNumber(frame, out var frameNumber)
                && TryReadPositiveNumber(position, out var playbackTime)
                && PathsEqual(activePath, expectedPath)
                && !eofReached
                && !isPaused
                && frameNumber > 0
                && playbackTime > 0)
            {
                return;
            }

            try
            {
                await Task.Delay(25, timeout.Token);
            }
            catch (OperationCanceledException)
            {
                break;
            }
        }

        throw new AssertFailedException($"{description} 未在 3 秒内确认目标路径已实际播放（path/eof/pause/frame/time）。");
    }

    private static bool TryReadPositiveNumber(MpvIpcDispatchResult result, out double value)
    {
        value = 0;
        return result.IsSuccess
            && result.Frame is not null
            && MpvIpcValueReader.TryReadFiniteDouble(result.Frame, out var read, out _)
            && read is double positive
            && double.IsFinite(positive)
            && positive > 0
            && (value = positive) > 0;
    }

    private static bool PathsEqual(string? actual, string expected)
    {
        if (string.IsNullOrWhiteSpace(actual))
        {
            return false;
        }

        return StringComparer.OrdinalIgnoreCase.Equals(
            Path.GetFullPath(actual),
            Path.GetFullPath(expected));
    }

    private static VideoEffectParams FullGpu83Video(
        double brightness,
        double contrast,
        double saturation,
        double hue) =>
        VideoEffectParams.Default with
        {
            BrightnessPercent = brightness,
            ContrastPercent = contrast,
            SaturationPercent = saturation,
            HueRotationDegrees = hue,
            BlurRadiusPx = 2,
            SharpenPercent = 8,
            NoisePercent = 2,
            DetailEnhancementPercent = 12,
            PixelScalePercent = 102,
            PixelJitterPx = 1,
            DynamicCropPercent = 1,
            SpaceXOffsetPx = 2,
            SpaceYOffsetPx = -1,
            RotationDegrees = 4,
            VignettePercent = 10,
            HighlightsPercent = 6,
            ShadowsPercent = -4,
            RedChannelLockEnabled = true,
            EdgeSoftnessPercent = 15,
            ImageRepairEnabled = true,
            ImageRepairStrengthPercent = 20,
        };

    private static AdvancedEffectParams FullGpu83Advanced()
    {
        var bandWeights = AdvancedEffectParams.Default.BandWeights
            .ToDictionary(pair => pair.Key, pair => pair.Key == 1_110 ? 1.25 : 1.0);
        return AdvancedEffectParams.Default with
        {
            BandWeights = bandWeights,
            TargetFrequencyHz = 1_110,
            CoreFrequencyHz = 777,
            WaveIntensity = 0.25,
            WaveLevel = 0.1,
            WaveGrainCount = 24,
            DynamicEqThreshold = 12,
            ChannelOffsetPercent = 3,
            FrequencySpaceXOffsetPx = 2,
            FrequencySpaceYOffsetPx = -2,
            RandomGraphicEnabled = true,
            RandomGraphicCount = 3,
            RandomGraphicOpacityPercent = 8,
            RandomGraphicSizePx = 6,
            PictureInPictureEnabled = true,
            PictureInPictureOpacityPercent = 60,
            LocalBlurEnabled = true,
            LocalBlurRegionPercent = 20,
            LocalBlurRadiusPx = 2,
            EdgeFillEnabled = true,
            EdgeFeatherPercent = 12,
            FramePerturbationProbabilityPercent = 5,
            SliceLengthMs = 5_000,
            SliceTriggerIntervalMs = 15_000,
            TransformSmoothingEnabled = true,
            HighlightPerturbationEnabled = true,
            AsynchronousRotationEnabled = true,
            AsynchronousRotationMinDegrees = -1,
            AsynchronousRotationMaxDegrees = 1,
        };
    }

    private sealed class NativeHostWindow : IDisposable
    {
        private const uint OverlappedWindowStyle = 0x00CF0000;
        private const uint PeekRemove = 0x0001;
        private const uint WmQuit = 0x0012;
        private readonly ManualResetEventSlim _ready = new(false);
        private readonly ManualResetEventSlim _stop = new(false);
        private readonly Thread _thread;
        private nint _handle;
        private Exception? _startupException;

        private NativeHostWindow()
        {
            _thread = new(ThreadMain)
            {
                IsBackground = true,
                Name = "GpAutoLive-real-mpv-host-window",
            };
            _thread.SetApartmentState(ApartmentState.STA);
            _thread.Start();
            if (!_ready.Wait(TimeSpan.FromSeconds(5)))
            {
                throw new AssertFailedException("真实 mpv 夹具 HWND 创建超时。");
            }

            if (_startupException is not null)
            {
                throw new AssertFailedException("真实 mpv 夹具 HWND 创建失败。", _startupException);
            }

            if (_handle == 0)
            {
                throw new AssertFailedException("真实 mpv 夹具 HWND 无效。");
            }
        }

        public nint Handle => _handle;

        public static NativeHostWindow Create() => new();

        public void Dispose()
        {
            _stop.Set();
            _thread.Join(TimeSpan.FromSeconds(2));
            _ready.Dispose();
            _stop.Dispose();
        }

        private void ThreadMain()
        {
            try
            {
                _handle = CreateWindowExW(
                    0,
                    "STATIC",
                    "GpAutoLive mpv real fixture",
                    OverlappedWindowStyle,
                    0,
                    0,
                    640,
                    360,
                    0,
                    0,
                    0,
                    0);
                if (_handle == 0)
                {
                    return;
                }

                ShowWindow(_handle, 5);
                UpdateWindow(_handle);
            }
            catch (Exception exception)
            {
                _startupException = exception;
            }
            finally
            {
                _ready.Set();
            }

            if (_handle == 0)
            {
                return;
            }

            try
            {
                while (!_stop.Wait(20))
                {
                    InvalidateRect(_handle, 0, true);
                    while (PeekMessageW(out var message, 0, 0, 0, PeekRemove))
                    {
                        if (message.Code == WmQuit)
                        {
                            return;
                        }

                        TranslateMessage(ref message);
                        DispatchMessageW(ref message);
                    }
                }
            }
            finally
            {
                DestroyWindow(_handle);
                _handle = 0;
            }
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct Point
        {
            public int X;
            public int Y;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct NativeMessage
        {
            public nint Window;
            public uint Code;
            public nuint WParam;
            public nint LParam;
            public uint Time;
            public Point Point;
        }

        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern nint CreateWindowExW(
            uint exStyle,
            string className,
            string windowName,
            uint style,
            int x,
            int y,
            int width,
            int height,
            nint parent,
            nint menu,
            nint instance,
            nint parameter);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern bool DestroyWindow(nint window);

        [DllImport("user32.dll")]
        private static extern bool ShowWindow(nint window, int command);

        [DllImport("user32.dll")]
        private static extern bool UpdateWindow(nint window);

        [DllImport("user32.dll")]
        private static extern bool InvalidateRect(nint window, nint updateRectangle, bool erase);

        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern bool PeekMessageW(
            out NativeMessage message,
            nint window,
            uint minimumMessage,
            uint maximumMessage,
            uint removeMessage);

        [DllImport("user32.dll")]
        private static extern bool TranslateMessage(ref NativeMessage message);

        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern nint DispatchMessageW(ref NativeMessage message);
    }
}
