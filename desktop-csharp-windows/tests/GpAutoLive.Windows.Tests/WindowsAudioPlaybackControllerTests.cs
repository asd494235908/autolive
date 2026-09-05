using GpAutoLive.Media;
using GpAutoLive.Contracts;
using System.Diagnostics;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsAudioPlaybackControllerTests
{
    [TestMethod]
    public async Task Null_plan_fails_closed_without_starting_output()
    {
        await using var controller = new WindowsAudioPlaybackController();

        var result = await controller.StartAsync(
            null,
            null,
            new WindowsPortAudioOutputConfig(0, 2, 48_000),
            loop: false);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual("invalid_plan", result.Error?.Code);
        Assert.AreEqual(WindowsAudioPlaybackState.Failed, result.Snapshot.State);
    }

    [TestMethod]
    public async Task Mismatched_plan_and_output_sample_rates_fail_before_starting_output()
    {
        await using var controller = new WindowsAudioPlaybackController();
        var plan = new FfmpegPcmDecodePlan(
            Environment.ProcessPath!,
            [],
            "fixture.wav",
            44_100,
            2,
            TimeSpan.FromSeconds(1));

        var result = await controller.StartAsync(
            plan,
            null,
            new WindowsPortAudioOutputConfig(0, 2, 48_000),
            loop: false);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual("invalid_plan", result.Error?.Code);
        Assert.AreEqual(WindowsAudioPlaybackState.Failed, result.Snapshot.State);
    }

    [TestMethod]
    public async Task Cancellation_before_start_is_bounded()
    {
        await using var controller = new WindowsAudioPlaybackController();
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();

        var result = await controller.StartAsync(
            null,
            null,
            new WindowsPortAudioOutputConfig(0, 2, 48_000),
            loop: false,
            cancellation.Token);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual("cancelled", result.Error?.Code);
    }

    [TestMethod]
    public async Task Closed_controller_rejects_future_start()
    {
        var controller = new WindowsAudioPlaybackController();
        await controller.DisposeAsync();

        var result = await controller.StartAsync(
            null,
            null,
            new WindowsPortAudioOutputConfig(0, 2, 48_000),
            loop: false);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual("closed", result.Error?.Code);
    }

    [TestMethod]
    public async Task Stop_without_active_session_is_idempotent()
    {
        await using var controller = new WindowsAudioPlaybackController();

        var first = await controller.StopAsync();
        var second = await controller.StopAsync();

        Assert.IsTrue(first.IsSuccess);
        Assert.IsTrue(second.IsSuccess);
        Assert.AreEqual(WindowsAudioPlaybackState.Idle, second.Snapshot.State);
    }

    [TestMethod]
    public async Task Interlude_start_without_main_session_fails_closed()
    {
        await using var controller = new WindowsAudioPlaybackController();

        var result = await controller.StartInterludeAsync(null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual("interlude_mix_not_enabled", result.Error?.Code);
    }

    [TestMethod]
    public async Task Interlude_stop_without_active_session_is_idempotent()
    {
        await using var controller = new WindowsAudioPlaybackController();

        var result = await controller.StopInterludeAsync();

        Assert.IsTrue(result.IsSuccess);
        Assert.AreEqual(WindowsAudioPlaybackState.Idle, result.Snapshot.State);
    }

    [TestMethod]
    public async Task Pause_without_active_session_fails_closed()
    {
        await using var controller = new WindowsAudioPlaybackController();

        var result = await controller.PauseAsync();

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual("not_running", result.Error?.Code);
        Assert.AreEqual(WindowsAudioPlaybackState.Failed, result.Snapshot.State);
    }

    [TestMethod]
    public async Task Resume_without_paused_session_fails_closed()
    {
        await using var controller = new WindowsAudioPlaybackController();

        var result = await controller.ResumeAsync();

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual("not_paused", result.Error?.Code);
        Assert.AreEqual(WindowsAudioPlaybackState.Failed, result.Snapshot.State);
    }

    [TestMethod]
    public async Task Real_fixture_runs_through_decoder_and_portaudio_when_explicitly_enabled()
    {
        var ffmpeg = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_FFMPEG");
        var portAudio = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_PORTAUDIO_DLL");
        if (string.IsNullOrWhiteSpace(ffmpeg)
            || !File.Exists(ffmpeg)
            || string.IsNullOrWhiteSpace(portAudio)
            || !File.Exists(portAudio))
        {
            return;
        }

        var fixture = Path.Combine(Path.GetTempPath(), $"gpalive-output-{Guid.NewGuid():N}.wav");
        try
        {
            using (var generator = new Process
            {
                StartInfo = new ProcessStartInfo
                {
                    FileName = ffmpeg,
                    UseShellExecute = false,
                    CreateNoWindow = true,
                    RedirectStandardOutput = false,
                    RedirectStandardError = false,
                }
            })
            {
                generator.StartInfo.ArgumentList.Add("-hide_banner");
                generator.StartInfo.ArgumentList.Add("-loglevel");
                generator.StartInfo.ArgumentList.Add("error");
                generator.StartInfo.ArgumentList.Add("-f");
                generator.StartInfo.ArgumentList.Add("lavfi");
                generator.StartInfo.ArgumentList.Add("-i");
                generator.StartInfo.ArgumentList.Add("sine=frequency=440:sample_rate=48000:duration=1");
                generator.StartInfo.ArgumentList.Add("-y");
                generator.StartInfo.ArgumentList.Add(fixture);
                Assert.IsTrue(generator.Start());
                await generator.WaitForExitAsync();
                Assert.AreEqual(0, generator.ExitCode);
            }

            var source = new SourceMediaDto(
                fixture,
                "media://output-fixture",
                MediaKind.Audio,
                MediaCompatibilityMode.Direct,
                "fixture.wav",
                (ulong)new FileInfo(fixture).Length,
                1_000,
                0,
                1_000,
                null,
                null,
                null,
                48_000,
                1,
                null,
                "pcm_s16le",
                null,
                "not-computed");
            Assert.IsTrue(FfmpegPcmDecodePlanBuilder.TryCreate(ffmpeg, source, 48_000, 2, out var plan, out var planError), planError?.Message);

            using var enumerator = new WindowsPortAudioDeviceEnumerator();
            var devices = await enumerator.ProbeAsync(portAudio);
            Assert.IsTrue(devices.IsSuccess, devices.Error?.Message);
            var outputDevice = devices.Snapshot.Devices.First(device => device.MaxOutputChannels > 0);
            await using var controller = new WindowsAudioPlaybackController(capacityFrames: 24_000);
            var started = await controller.StartAsync(
                plan,
                portAudio,
                new WindowsPortAudioOutputConfig(outputDevice.Index, 2, 48_000, 256),
                loop: true);

            Assert.IsTrue(started.IsSuccess, started.Error?.Message);
            Assert.IsTrue(
                started.Snapshot.Decoder?.DecodedFrames > 0,
                "音频启动成功前必须已经产生首批 PCM 帧。");
            Assert.IsTrue(
                started.Snapshot.Output?.HardwareState is WindowsPortAudioHardwareState.Active
                    or WindowsPortAudioHardwareState.Unknown,
                $"PortAudio 健康状态异常：{started.Snapshot.Output?.HardwareState}");

            var secondStart = await controller.StartAsync(
                plan,
                portAudio,
                new WindowsPortAudioOutputConfig(outputDevice.Index, 2, 48_000, 256),
                loop: true);
            Assert.IsFalse(secondStart.IsSuccess);
            Assert.AreEqual("already_running", secondStart.Error?.Code);

            await Task.Delay(150);
            var beforePause = controller.Snapshot;
            Assert.IsTrue(beforePause.Output?.OutputFramesWritten > 0, "PortAudio 回调没有报告已送出帧数。");
            Assert.IsNotNull(beforePause.AudibleClock, "音频会话没有建立可听时钟快照。");
            Assert.IsTrue(beforePause.AudibleClock!.HasTimeInfo, "PortAudio callback 没有提供可用 timeInfo。");
            Assert.IsNotNull(beforePause.AudibleClock.PlaybackTimeMs, "音频会话没有产生可听媒体位置。");
            var paused = await controller.PauseAsync();
            Assert.IsTrue(
                paused.IsSuccess,
                $"{paused.Error?.Message}; beforeState={beforePause.State}; beforeError={beforePause.ErrorCode}/{beforePause.Error}; decoder={beforePause.Decoder?.ErrorCode}/{beforePause.Decoder?.Error}");
            Assert.AreEqual(WindowsAudioPlaybackState.Paused, paused.Snapshot.State);
            Assert.IsTrue(paused.Snapshot.Output?.IsPaused == true);

            var resumed = await controller.ResumeAsync();
            Assert.IsTrue(resumed.IsSuccess, resumed.Error?.Message);
            Assert.AreEqual(WindowsAudioPlaybackState.Playing, resumed.Snapshot.State);
            Assert.IsTrue(resumed.Snapshot.Output?.IsRunning == true);

            var stopped = await controller.StopAsync();
            Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
            Assert.AreEqual(WindowsAudioPlaybackState.Idle, stopped.Snapshot.State);

            using var finalPcmBus = new FinalPcmBus(capacityFrames: 24_000, channels: 2);
            var finiteStarted = await controller.StartAsync(
                plan,
                portAudio,
                new WindowsPortAudioOutputConfig(outputDevice.Index, 2, 48_000, 256),
                loop: false,
                finalPcmBus: finalPcmBus);
            Assert.IsTrue(finiteStarted.IsSuccess, finiteStarted.Error?.Message);
            controller.SetRtmpConsumerAttached(true);
            try
            {
                await controller.Completion.WaitAsync(TimeSpan.FromSeconds(12));
            }
            catch (TimeoutException)
            {
                var timeoutSnapshot = controller.Snapshot;
                var busSnapshot = finalPcmBus.Snapshot;
                Assert.Fail(
                    $"有限音频会话未在预算内完成：state={timeoutSnapshot.State}; "
                    + $"error={timeoutSnapshot.ErrorCode}/{timeoutSnapshot.Error}; "
                    + $"decoderRunning={timeoutSnapshot.Decoder?.IsRunning}; "
                    + $"decoderPid={timeoutSnapshot.Decoder?.ProcessId}; "
                    + $"decoded={timeoutSnapshot.Decoder?.DecodedFrames}; "
                    + $"callbacks={timeoutSnapshot.Output?.CallbackCount}; "
                    + $"output={busSnapshot.OutputAvailableFrames}; "
                    + $"published={busSnapshot.PublishedFrames}; "
                    + $"closed={busSnapshot.IsClosed}");
            }
            Assert.AreEqual(WindowsAudioPlaybackState.Completed, controller.Snapshot.State);
            Assert.IsNull(controller.Snapshot.Output);
            Assert.IsTrue(finalPcmBus.Snapshot.IsClosed);
            Assert.IsTrue(finalPcmBus.Snapshot.PublishedFrames > 0);
            Assert.IsTrue(finalPcmBus.Snapshot.RtmpAvailableFrames > 0);

            var realtimeIndex = plan!.Arguments.IndexOf("-re");
            Assert.IsTrue(realtimeIndex >= 0, "真实夹具计划缺少可移除的实时节流参数。");
            var noThrottlePlan = plan with { Arguments = plan.Arguments.RemoveAt(realtimeIndex) };
            using var backpressureBus = new FinalPcmBus(capacityFrames: 8_192, channels: 2);
            var backpressureStarted = await controller.StartAsync(
                noThrottlePlan,
                portAudio,
                new WindowsPortAudioOutputConfig(outputDevice.Index, 2, 48_000, 256),
                loop: false,
                finalPcmBus: backpressureBus);
            Assert.IsTrue(backpressureStarted.IsSuccess, backpressureStarted.Error?.Message);
            await controller.Completion.WaitAsync(TimeSpan.FromSeconds(12));
            Assert.AreEqual(WindowsAudioPlaybackState.Completed, controller.Snapshot.State);
            Assert.AreEqual<ulong>(0, backpressureBus.Snapshot.OutputDroppedFrames);
        }
        finally
        {
            File.Delete(fixture);
        }
    }

    [TestMethod]
    public async Task Download_fixture_consumes_realtime_audio_effects_when_explicitly_enabled()
    {
        var ffmpeg = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_FFMPEG");
        var portAudio = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_PORTAUDIO_DLL");
        var mediaPath = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA");
        if (string.IsNullOrWhiteSpace(ffmpeg)
            || !File.Exists(ffmpeg)
            || string.IsNullOrWhiteSpace(portAudio)
            || !File.Exists(portAudio)
            || string.IsNullOrWhiteSpace(mediaPath))
        {
            return;
        }

        Assert.IsTrue(OperatingSystem.IsWindows(), "真实 PortAudio 夹具只支持 Windows。");
        Assert.IsTrue(File.Exists(mediaPath), "显式下载夹具媒体文件不存在。");
        var fileInfo = new FileInfo(mediaPath);
        var source = new SourceMediaDto(
            mediaPath,
            mediaPath,
            MediaKind.Video,
            MediaCompatibilityMode.Direct,
            fileInfo.Name,
            (ulong)fileInfo.Length,
            2_500,
            0,
            2_500,
            320,
            180,
            30,
            48_000,
            1,
            "mpeg4",
            "aac",
            null,
            "not-computed");
        var effects = new AudioEffectParams
        {
            PlaybackSpeed = 1.1,
            PitchShiftSemitones = 1,
            SpectralPerturbationPercent = 10,
            SpectrumBlindSpotPercent = 2,
            HighFrequencyPerturbationEnabled = true,
            HighFrequencyPerturbationIntervalMs = 12_000,
            HighFrequencyPerturbationStrengthPercent = 4,
            HighFrequencyPerturbationLevelDb = -32,
            FadeInMs = 40,
            FadeOutMs = 1_000,
            ReverbWetPercent = 5,
            NoiseReductionPercent = 20,
            PhasePerturbationPercent = 5,
            VibratoFrequencyHz = 5,
            VibratoDepthPercent = 1,
        };
        Assert.IsTrue(
            FfmpegPcmDecodePlanBuilder.TryCreate(
                ffmpeg,
                source,
                48_000,
                1,
                out var plan,
                out var planError,
                effects),
            planError?.Message);
        Assert.IsNotNull(plan);
        var filterIndex = plan!.Arguments.IndexOf("-af");
        Assert.IsTrue(filterIndex >= 0 && filterIndex + 1 < plan.Arguments.Length);
        StringAssert.Contains(plan.Arguments[filterIndex + 1], "atempo=1.1");
        StringAssert.Contains(plan.Arguments[filterIndex + 1], "asetrate=50854");
        StringAssert.Contains(plan.Arguments[filterIndex + 1], "aresample=48000");
        StringAssert.Contains(plan.Arguments[filterIndex + 1], "afftfilt=");
        StringAssert.Contains(plan.Arguments[filterIndex + 1], "bandreject=f=8000.000000:t=h:w=400.000000");
        StringAssert.Contains(plan.Arguments[filterIndex + 1], "afade=t=out:st=1.5:d=1");
        StringAssert.Contains(plan.Arguments[filterIndex + 1], "aecho");
        StringAssert.Contains(plan.Arguments[filterIndex + 1], "afftdn");
        StringAssert.Contains(plan.Arguments[filterIndex + 1], "aphaser");
        StringAssert.Contains(plan.Arguments[filterIndex + 1], "vibrato");

        using var enumerator = new WindowsPortAudioDeviceEnumerator();
        var devices = await enumerator.ProbeAsync(portAudio);
        Assert.IsTrue(devices.IsSuccess, devices.Error?.Message);
        var outputDevice = devices.Snapshot.Devices.FirstOrDefault(device => device.MaxOutputChannels > 0);
        Assert.IsNotNull(outputDevice, "没有可用的 PortAudio 输出设备。");

        using var finalPcmBus = new FinalPcmBus(capacityFrames: 24_000, channels: 1);
        await using var controller = new WindowsAudioPlaybackController(capacityFrames: 24_000);
        var started = await controller.StartAsync(
            plan,
            portAudio,
            new WindowsPortAudioOutputConfig(outputDevice!.Index, 1, 48_000, 256),
            loop: false,
            finalPcmBus: finalPcmBus);

        Assert.IsTrue(started.IsSuccess, started.Error?.Message);
        await controller.Completion.WaitAsync(TimeSpan.FromSeconds(8));
        Assert.AreEqual(WindowsAudioPlaybackState.Completed, controller.Snapshot.State);
        Assert.IsTrue(finalPcmBus.Snapshot.PublishedFrames > 0, "实时效果链没有产生最终 PCM 帧。");
    }

    [TestMethod]
    public async Task Download_fixture_can_restart_video_audio_from_a_current_position()
    {
        var ffmpeg = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_FFMPEG");
        var portAudio = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_PORTAUDIO_DLL");
        var mediaPath = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA");
        if (string.IsNullOrWhiteSpace(ffmpeg)
            || !File.Exists(ffmpeg)
            || string.IsNullOrWhiteSpace(portAudio)
            || !File.Exists(portAudio)
            || string.IsNullOrWhiteSpace(mediaPath))
        {
            return;
        }

        Assert.IsTrue(OperatingSystem.IsWindows(), "真实 PortAudio 夹具只支持 Windows。");
        Assert.IsTrue(File.Exists(mediaPath), "显式下载夹具媒体文件不存在。");
        var fileInfo = new FileInfo(mediaPath);
        var source = new SourceMediaDto(
            mediaPath,
            mediaPath,
            MediaKind.Video,
            MediaCompatibilityMode.Direct,
            fileInfo.Name,
            (ulong)fileInfo.Length,
            2_500,
            0,
            2_500,
            320,
            180,
            30,
            48_000,
            1,
            "mpeg4",
            "aac",
            null,
            "not-computed");
        const ulong sourceStartMs = 1_000;
        Assert.IsTrue(
            FfmpegPcmDecodePlanBuilder.TryCreate(
                ffmpeg,
                source,
                48_000,
                1,
                out var plan,
                out var planError,
                new AudioEffectParams { LoudnessAdjustmentDb = 2 },
                sourceStartMs),
            planError?.Message);
        Assert.IsNotNull(plan);
        Assert.AreEqual(sourceStartMs, plan!.SourceStartMs);
        Assert.AreEqual("1.000", plan.Arguments[plan.Arguments.IndexOf("-ss") + 1]);

        using var enumerator = new WindowsPortAudioDeviceEnumerator();
        var devices = await enumerator.ProbeAsync(portAudio);
        Assert.IsTrue(devices.IsSuccess, devices.Error?.Message);
        var outputDevice = devices.Snapshot.Devices.FirstOrDefault(device => device.MaxOutputChannels > 0);
        Assert.IsNotNull(outputDevice, "没有可用的 PortAudio 输出设备。");

        using var finalPcmBus = new FinalPcmBus(capacityFrames: 24_000, channels: 1);
        await using var controller = new WindowsAudioPlaybackController(capacityFrames: 24_000);
        var started = await controller.StartAsync(
            plan,
            portAudio,
            new WindowsPortAudioOutputConfig(outputDevice!.Index, 1, 48_000, 256),
            loop: false,
            finalPcmBus: finalPcmBus);

        Assert.IsTrue(started.IsSuccess, started.Error?.Message);
        await controller.Completion.WaitAsync(TimeSpan.FromSeconds(8));
        Assert.AreEqual(WindowsAudioPlaybackState.Completed, controller.Snapshot.State);
        Assert.IsTrue(finalPcmBus.Snapshot.PublishedFrames > 0, "从当前时间点恢复的声音没有产生最终 PCM 帧。");
    }

    [TestMethod]
    public async Task Download_fixture_switches_to_preloaded_next_audio_without_restarting_output()
    {
        var ffmpeg = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_FFMPEG");
        var portAudio = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_PORTAUDIO_DLL");
        var mediaPath = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_MPV_MEDIA");
        if (string.IsNullOrWhiteSpace(ffmpeg)
            || !File.Exists(ffmpeg)
            || string.IsNullOrWhiteSpace(portAudio)
            || !File.Exists(portAudio)
            || string.IsNullOrWhiteSpace(mediaPath))
        {
            return;
        }

        Assert.IsTrue(OperatingSystem.IsWindows(), "真实 PortAudio 夹具只支持 Windows。");
        Assert.IsTrue(File.Exists(mediaPath), "显式下载夹具媒体文件不存在。");
        var fileInfo = new FileInfo(mediaPath);
        var source = new SourceMediaDto(
            mediaPath,
            mediaPath,
            MediaKind.Video,
            MediaCompatibilityMode.Direct,
            fileInfo.Name,
            (ulong)fileInfo.Length,
            2_500,
            0,
            2_500,
            320,
            180,
            30,
            48_000,
            1,
            "mpeg4",
            "aac",
            null,
            "not-computed");
        Assert.IsTrue(
            FfmpegPcmDecodePlanBuilder.TryCreate(
                ffmpeg,
                source,
                48_000,
                1,
                out var plan,
                out var planError),
            planError?.Message);
        Assert.IsNotNull(plan);

        using var enumerator = new WindowsPortAudioDeviceEnumerator();
        var devices = await enumerator.ProbeAsync(portAudio);
        Assert.IsTrue(devices.IsSuccess, devices.Error?.Message);
        var outputDevice = devices.Snapshot.Devices.FirstOrDefault(device => device.MaxOutputChannels > 0);
        Assert.IsNotNull(outputDevice, "没有可用的 PortAudio 输出设备。");

        using var finalPcmBus = new FinalPcmBus(capacityFrames: 24_000, channels: 1);
        await using var controller = new WindowsAudioPlaybackController(capacityFrames: 24_000);
        var started = await controller.StartAsync(
            plan,
            portAudio,
            new WindowsPortAudioOutputConfig(outputDevice!.Index, 1, 48_000, 256),
            loop: false,
            finalPcmBus: finalPcmBus);
        Assert.IsTrue(started.IsSuccess, started.Error?.Message);

        var prepared = await controller.PrepareNextAsync(plan);
        Assert.IsTrue(prepared.IsSuccess, prepared.Error?.Message);
        var firstCompletion = controller.CandidateCompletion;
        Assert.IsNotNull(firstCompletion);
        var firstResult = await firstCompletion!.WaitAsync(TimeSpan.FromSeconds(8));
        Assert.IsTrue(firstResult.IsSuccess, firstResult.Error?.Message);
        Assert.IsTrue(controller.HasPreparedNext);
        Assert.IsTrue(controller.Snapshot.Output?.OutputFramesWritten > 0, "N/N+1 夹具期间没有送出 PortAudio 帧。");

        var committed = await controller.CommitPreparedNextAsync();
        Assert.IsTrue(committed.IsSuccess, committed.Error?.Message);
        Assert.IsFalse(controller.HasPreparedNext);
        Assert.AreNotSame(finalPcmBus, controller.ActiveFinalPcmBus);

        await controller.Completion.WaitAsync(TimeSpan.FromSeconds(12));
        Assert.AreEqual(WindowsAudioPlaybackState.Completed, controller.Snapshot.State);
        Assert.IsTrue(finalPcmBus.Snapshot.PublishedFrames > 0);
    }

    [TestMethod]
    public async Task Explicit_real_audio_cycle_switches_same_source_candidate_at_target()
    {
        if (Environment.GetEnvironmentVariable("AUTOLIVE_TEST_REAL_AUDIO_CYCLE") != "1")
        {
            return;
        }

        var ffmpeg = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_FFMPEG");
        var portAudio = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_PORTAUDIO_DLL");
        if (string.IsNullOrWhiteSpace(ffmpeg)
            || !File.Exists(ffmpeg)
            || string.IsNullOrWhiteSpace(portAudio)
            || !File.Exists(portAudio))
        {
            return;
        }

        var fixture = Path.Combine(Path.GetTempPath(), $"gpalive-audio-cycle-{Guid.NewGuid():N}.wav");
        try
        {
            using (var generator = new Process
            {
                StartInfo = new ProcessStartInfo
                {
                    FileName = ffmpeg,
                    UseShellExecute = false,
                    CreateNoWindow = true,
                    RedirectStandardOutput = false,
                    RedirectStandardError = false,
                },
            })
            {
                generator.StartInfo.ArgumentList.Add("-hide_banner");
                generator.StartInfo.ArgumentList.Add("-loglevel");
                generator.StartInfo.ArgumentList.Add("error");
                generator.StartInfo.ArgumentList.Add("-f");
                generator.StartInfo.ArgumentList.Add("lavfi");
                generator.StartInfo.ArgumentList.Add("-i");
                generator.StartInfo.ArgumentList.Add("sine=frequency=440:sample_rate=48000:duration=8");
                generator.StartInfo.ArgumentList.Add("-y");
                generator.StartInfo.ArgumentList.Add(fixture);
                Assert.IsTrue(generator.Start());
                await generator.WaitForExitAsync();
                Assert.AreEqual(0, generator.ExitCode);
            }

            var fileInfo = new FileInfo(fixture);
            var source = new SourceMediaDto(
                fixture,
                "media://audio-cycle-fixture",
                MediaKind.Audio,
                MediaCompatibilityMode.Direct,
                fileInfo.Name,
                (ulong)fileInfo.Length,
                8_000,
                0,
                8_000,
                null,
                null,
                null,
                48_000,
                1,
                null,
                "pcm_s16le",
                null,
                "not-computed");
            Assert.IsTrue(FfmpegPcmDecodePlanBuilder.TryCreate(
                ffmpeg,
                source,
                48_000,
                1,
                out var initialPlan,
                out var initialError), initialError?.Message);
            Assert.IsNotNull(initialPlan);
            Assert.IsTrue(FfmpegPcmDecodePlanBuilder.TryCreate(
                ffmpeg,
                source,
                48_000,
                1,
                out var cyclePlan,
                out var cycleError,
                new AudioEffectParams
                {
                    NaturalVoiceMode = NaturalVoiceMode.NaturalDynamic,
                    RandomChangePeriodMs = 4_000,
                    VoiceLibraryId = "p02",
                    LoudnessAdjustmentDb = 2,
                },
                sourceStartMs: 4_000), cycleError?.Message);
            Assert.IsNotNull(cyclePlan);

            using var enumerator = new WindowsPortAudioDeviceEnumerator();
            var devices = await enumerator.ProbeAsync(portAudio);
            Assert.IsTrue(devices.IsSuccess, devices.Error?.Message);
            var outputDevice = devices.Snapshot.Devices.FirstOrDefault(device => device.MaxOutputChannels > 0);
            Assert.IsNotNull(outputDevice, "没有可用的 PortAudio 输出设备。");

            using var initialBus = new FinalPcmBus(capacityFrames: 24_000, channels: 1);
            await using var controller = new WindowsAudioPlaybackController(capacityFrames: 24_000);
            var started = await controller.StartAsync(
                initialPlan,
                portAudio,
                new WindowsPortAudioOutputConfig(outputDevice!.Index, 1, 48_000, 256),
                loop: false,
                finalPcmBus: initialBus);
            Assert.IsTrue(started.IsSuccess, started.Error?.Message);

            var prepared = await controller.PrepareNextAsync(cyclePlan);
            Assert.IsTrue(prepared.IsSuccess, prepared.Error?.Message);
            var committed = await controller.CommitPreparedNextAsync(
                targetPositionMs: 4_000);
            Assert.IsTrue(committed.IsSuccess, committed.Error?.Message);
            var cycleBus = controller.ActiveFinalPcmBus;
            Assert.IsNotNull(cycleBus);
            Assert.AreNotSame(initialBus, cycleBus);

            await controller.Completion.WaitAsync(TimeSpan.FromSeconds(15));
            Assert.AreEqual(WindowsAudioPlaybackState.Completed, controller.Snapshot.State);
            Assert.IsTrue(cycleBus!.Snapshot.PublishedFrames > 0);
            Assert.IsTrue(initialBus.Snapshot.PublishedFrames > 0);
        }
        finally
        {
            File.Delete(fixture);
        }
    }
}
