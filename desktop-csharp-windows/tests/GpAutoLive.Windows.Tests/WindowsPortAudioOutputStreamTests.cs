using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsPortAudioOutputStreamTests
{
    [TestMethod]
    public async Task Invalid_path_is_rejected_before_native_load()
    {
        var buffer = new AudioPcmRingBuffer(256, 2);
        using var output = new WindowsPortAudioOutputStream(buffer);

        var result = await output.StartAsync(
            @"C:\media\portaudio.dll",
            new WindowsPortAudioOutputConfig(0, 2, 48_000));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioStreamFailureCode.InvalidPath, result.Error?.Code);
        Assert.IsFalse(output.Snapshot.IsRunning);
    }

    [TestMethod]
    public async Task Missing_resource_is_retryable_and_stop_is_idempotent()
    {
        var buffer = new AudioPcmRingBuffer(256, 2);
        using var output = new WindowsPortAudioOutputStream(buffer);
        var path = Path.Combine(
            Path.GetTempPath(),
            "gpautolive-portaudio-output-missing",
            Guid.NewGuid().ToString("N"),
            "portaudio_x64.dll");

        var result = await output.StartAsync(
            path,
            new WindowsPortAudioOutputConfig(0, 2, 48_000));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioStreamFailureCode.ResourceMissing, result.Error?.Code);
        Assert.IsTrue(result.Error?.Retryable);
        Assert.IsTrue(output.Stop().IsSuccess);
    }

    [TestMethod]
    public async Task Invalid_config_is_rejected_without_loading_native_library()
    {
        var buffer = new AudioPcmRingBuffer(256, 2);
        using var output = new WindowsPortAudioOutputStream(buffer);
        var path = Path.Combine(
            Path.GetTempPath(),
            "gpautolive-portaudio-output-config",
            Guid.NewGuid().ToString("N"),
            "portaudio_x64.dll");

        var result = await output.StartAsync(
            path,
            new WindowsPortAudioOutputConfig(0, 2, 48_000, FramesPerBuffer: 8));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioStreamFailureCode.InvalidConfig, result.Error?.Code);
    }

    [TestMethod]
    public async Task Cancellation_before_start_does_not_load_resource()
    {
        var buffer = new AudioPcmRingBuffer(256, 2);
        using var output = new WindowsPortAudioOutputStream(buffer);
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();

        var result = await output.StartAsync(
            null,
            new WindowsPortAudioOutputConfig(0, 2, 48_000),
            cancellation.Token);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioStreamFailureCode.Cancelled, result.Error?.Code);
    }

    [TestMethod]
    public async Task Dispose_rejects_new_start()
    {
        var buffer = new AudioPcmRingBuffer(256, 2);
        var output = new WindowsPortAudioOutputStream(buffer);
        output.Dispose();

        var result = await output.StartAsync(
            null,
            new WindowsPortAudioOutputConfig(0, 2, 48_000));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioStreamFailureCode.Closed, result.Error?.Code);
    }

    [TestMethod]
    public void Pause_and_resume_without_stream_are_rejected_without_native_load()
    {
        var buffer = new AudioPcmRingBuffer(256, 2);
        using var output = new WindowsPortAudioOutputStream(buffer);

        var paused = output.Pause();
        var resumed = output.Resume();

        Assert.IsFalse(paused.IsSuccess);
        Assert.AreEqual(WindowsPortAudioStreamFailureCode.InvalidConfig, paused.Error?.Code);
        Assert.IsFalse(resumed.IsSuccess);
        Assert.AreEqual(WindowsPortAudioStreamFailureCode.InvalidConfig, resumed.Error?.Code);
        Assert.IsFalse(output.Snapshot.IsPaused);
    }

    [TestMethod]
    public void Snapshot_without_stream_reports_not_created_health()
    {
        var buffer = new AudioPcmRingBuffer(256, 2);
        using var output = new WindowsPortAudioOutputStream(buffer);

        Assert.AreEqual(WindowsPortAudioHardwareState.NotCreated, output.Snapshot.HardwareState);
        Assert.AreEqual((ulong)0, output.Snapshot.CallbackCount);
        Assert.AreEqual((uint)0, output.Snapshot.LastCallbackStatusFlags);
        Assert.AreEqual((ulong)0, output.Snapshot.CallbackStatusFlagsCount);
        Assert.AreEqual((ulong)0, output.Snapshot.XrunCount);
    }

    [TestMethod]
    public void Sustained_xrun_policy_ignores_short_startup_glitch_and_uses_recovery_baseline()
    {
        var shortGlitch = new WindowsPortAudioOutputSnapshot(
            IsRunning: true,
            DeviceIndex: 0,
            Channels: 2,
            SampleRate: 48_000,
            FramesPerBuffer: 256,
            UnderrunFrames: 256,
            CallbackFailures: 0,
            ErrorCode: null,
            Error: null)
        {
            CallbackCount = 128,
            XrunCount = 128,
        };
        Assert.IsFalse(WindowsAudioPlaybackController.ShouldRecoverFromSustainedXrun(
            shortGlitch,
            baselineCallbackCount: 0,
            baselineXrunCount: 0));

        var sustained = shortGlitch with
        {
            CallbackCount = 1_024,
            XrunCount = 768,
        };
        Assert.IsTrue(WindowsAudioPlaybackController.ShouldRecoverFromSustainedXrun(
            sustained,
            baselineCallbackCount: 0,
            baselineXrunCount: 0));

        var recoveredBaseline = sustained with
        {
            CallbackCount = 2_048,
            XrunCount = 768,
        };
        Assert.IsFalse(WindowsAudioPlaybackController.ShouldRecoverFromSustainedXrun(
            recoveredBaseline,
            baselineCallbackCount: 1_024,
            baselineXrunCount: 768));
    }

    [TestMethod]
    public void Recovery_policy_is_bounded_and_uses_capped_backoff()
    {
        var policy = new WindowsPortAudioRecoveryPolicy(
            maxAttempts: 3,
            initialDelay: TimeSpan.FromMilliseconds(100),
            maxDelay: TimeSpan.FromMilliseconds(250));

        Assert.AreEqual(TimeSpan.FromMilliseconds(100), policy.GetDelay(0));
        Assert.AreEqual(TimeSpan.FromMilliseconds(200), policy.GetDelay(1));
        Assert.AreEqual(TimeSpan.FromMilliseconds(250), policy.GetDelay(2));
        try
        {
            _ = new WindowsPortAudioRecoveryPolicy(maxAttempts: 4);
            Assert.Fail("恢复预算不得超过 3 次。");
        }
        catch (ArgumentOutOfRangeException)
        {
            // expected
        }

        try
        {
            _ = policy.GetDelay(-1);
            Assert.Fail("负重试序号必须被拒绝。");
        }
        catch (ArgumentOutOfRangeException)
        {
            // expected
        }
    }

    [TestMethod]
    public async Task Recovery_attempts_are_bounded_even_when_every_restart_is_retryable()
    {
        var buffer = new AudioPcmRingBuffer(256, 1);
        using var output = new WindowsPortAudioOutputStream(buffer);
        var snapshot = output.Snapshot;
        var attempts = 0;
        var policy = new WindowsPortAudioRecoveryPolicy(
            maxAttempts: 3,
            initialDelay: TimeSpan.FromMilliseconds(1),
            maxDelay: TimeSpan.FromMilliseconds(1));
        var recovery = new WindowsPortAudioOutputRecovery(policy);

        var result = await recovery.RecoverWithAsync(
            _ =>
            {
                attempts++;
                return Task.FromResult(new WindowsPortAudioOutputResult(
                    false,
                    snapshot,
                    new WindowsPortAudioStreamError(
                        WindowsPortAudioStreamFailureCode.RestartFailed,
                        "模拟设备仍不可用。",
                        Retryable: true)));
            },
            () => snapshot);

        Assert.AreEqual(3, attempts);
        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioStreamFailureCode.RestartFailed, result.Error?.Code);
    }

    [TestMethod]
    public async Task Recovery_translates_native_restart_exception_to_bounded_failure()
    {
        var buffer = new AudioPcmRingBuffer(256, 1);
        using var output = new WindowsPortAudioOutputStream(buffer);
        var snapshot = output.Snapshot;
        var recovery = new WindowsPortAudioOutputRecovery(
            new WindowsPortAudioRecoveryPolicy(
                maxAttempts: 1,
                initialDelay: TimeSpan.FromMilliseconds(1),
                maxDelay: TimeSpan.FromMilliseconds(1)));

        var result = await recovery.RecoverWithAsync(
            _ => throw new InvalidOperationException("native restart failed"),
            () => snapshot);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioStreamFailureCode.RestartFailed, result.Error?.Code);
        Assert.IsTrue(result.Error?.Retryable);
        Assert.AreEqual("PortAudio 输出流恢复发生原生错误。", result.Error?.Message);
    }

    [TestMethod]
    public async Task Restart_failure_keeps_next_bounded_attempt_from_being_misclassified_as_initial_start()
    {
        var dllPath = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_PORTAUDIO_DLL");
        if (string.IsNullOrWhiteSpace(dllPath) || !File.Exists(dllPath))
        {
            return;
        }

        using var enumerator = new WindowsPortAudioDeviceEnumerator();
        var devices = await enumerator.ProbeAsync(dllPath);
        if (!devices.IsSuccess)
        {
            return;
        }

        var device = devices.Snapshot.Devices.FirstOrDefault(candidate => candidate.MaxOutputChannels > 0);
        if (device is null)
        {
            return;
        }

        var buffer = new AudioPcmRingBuffer(256, 1);
        using var output = new WindowsPortAudioOutputStream(buffer);
        var config = new WindowsPortAudioOutputConfig(device.Index, 1, 48_000, 256);
        var started = await output.StartAsync(dllPath, config);
        Assert.IsTrue(started.IsSuccess, started.Error?.Message);

        var missingPath = Path.Combine(
            Path.GetTempPath(),
            "gpautolive-portaudio-recovery-missing",
            Guid.NewGuid().ToString("N"),
            "portaudio_x64.dll");
        var first = await output.RestartAsync(missingPath, config);
        var second = await output.RestartAsync(missingPath, config);

        Assert.AreEqual(WindowsPortAudioStreamFailureCode.ResourceMissing, first.Error?.Code);
        Assert.AreEqual(WindowsPortAudioStreamFailureCode.ResourceMissing, second.Error?.Code);
        Assert.IsFalse(output.Snapshot.IsRunning);
    }

    [TestMethod]
    public async Task Real_device_can_reopen_the_same_output_stream_with_bounded_recovery()
    {
        var portAudio = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_PORTAUDIO_DLL");
        if (string.IsNullOrWhiteSpace(portAudio) || !File.Exists(portAudio))
        {
            return;
        }

        Assert.IsTrue(OperatingSystem.IsWindows(), "真实 PortAudio 夹具只支持 Windows。");
        using var enumerator = new WindowsPortAudioDeviceEnumerator();
        var devices = await enumerator.ProbeAsync(portAudio);
        Assert.IsTrue(devices.IsSuccess, devices.Error?.Message);
        var outputDevice = devices.Snapshot.Devices.FirstOrDefault(device => device.MaxOutputChannels > 0);
        Assert.IsNotNull(outputDevice, "没有可用的 PortAudio 输出设备。");

        var buffer = new AudioPcmRingBuffer(24_000, 1);
        using var output = new WindowsPortAudioOutputStream(buffer);
        var config = new WindowsPortAudioOutputConfig(outputDevice!.Index, 1, 48_000, 256);
        var started = await output.StartAsync(portAudio, config);
        Assert.IsTrue(started.IsSuccess, started.Error?.Message);
        await Task.Delay(150);
        var beforeRestart = output.Snapshot;

        var restarted = await output.RestartAsync(portAudio, config);
        Assert.IsTrue(restarted.IsSuccess, restarted.Error?.Message);
        await Task.Delay(150);
        var afterRestart = output.Snapshot;

        Assert.IsTrue(afterRestart.IsRunning);
        Assert.IsTrue(afterRestart.OutputFramesWritten > beforeRestart.OutputFramesWritten);
    }
}
