using GpAutoLive.Contracts;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsRtmpAudioSessionTests
{
    [TestMethod]
    public void Pump_failure_is_projected_but_stop_cancellation_is_not_a_failure()
    {
        var failure = new WindowsRtmpPcmPumpResult(
            false,
            new WindowsRtmpPcmPumpSnapshot(false, 0, "write_failed", "PCM 写入失败"),
            new WindowsRtmpPcmPumpError(
                WindowsRtmpPcmPumpFailureCode.WriteFailed,
                "PCM 写入失败",
                Retryable: true));

        var projected = WindowsRtmpAudioSession.MapPumpFailure(failure, sessionCancellationRequested: false);
        Assert.IsNotNull(projected);
        Assert.AreEqual(WindowsRtmpAudioSessionFailureCode.PumpFailed, projected.Code);
        Assert.IsTrue(projected.Retryable);

        Assert.IsNull(WindowsRtmpAudioSession.MapPumpFailure(failure, sessionCancellationRequested: true));
        Assert.IsNull(WindowsRtmpAudioSession.MapPumpFailure(
            new WindowsRtmpPcmPumpResult(true, new WindowsRtmpPcmPumpSnapshot(false, 1, null, null)),
            sessionCancellationRequested: false));
    }

    [TestMethod]
    public async Task Audio_disabled_is_rejected_before_starting_the_host()
    {
        await using var manager = new WindowsRtmpOutputManager();
        await using var session = new WindowsRtmpAudioSession(manager);

        var result = await session.StartAsync(
            RtmpOutputConfig.Default with
            {
                TargetUrl = "rtmp://127.0.0.1/live/test",
                AudioEnabled = false,
            },
            null,
            null,
            null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsRtmpAudioSessionFailureCode.InvalidArguments, result.Error?.Code);
        Assert.IsFalse(session.Snapshot.IsRunning);
        Assert.AreEqual(RtmpOutputState.Idle, manager.Snapshot.State);
    }

    [TestMethod]
    public async Task Source_without_audio_is_rejected_before_building_a_decode_plan()
    {
        await using var manager = new WindowsRtmpOutputManager();
        await using var session = new WindowsRtmpAudioSession(manager);

        var result = await session.StartAsync(
            RtmpOutputConfig.Default with
            {
                TargetUrl = "rtmp://127.0.0.1/live/test",
                VideoEnabled = false,
                AudioEnabled = true,
            },
            null,
            null,
            null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsRtmpAudioSessionFailureCode.InvalidArguments, result.Error?.Code);
        Assert.AreEqual(RtmpOutputState.Idle, manager.Snapshot.State);
    }

    [TestMethod]
    public async Task Disposed_session_rejects_future_start()
    {
        var manager = new WindowsRtmpOutputManager();
        var session = new WindowsRtmpAudioSession(manager);
        await session.DisposeAsync();

        var result = await session.StartAsync(null, null, null, null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsRtmpAudioSessionFailureCode.Closed, result.Error?.Code);
        await manager.DisposeAsync();
    }

    [TestMethod]
    public async Task Stop_without_start_is_idempotent_and_keeps_host_idle()
    {
        await using var manager = new WindowsRtmpOutputManager();
        await using var session = new WindowsRtmpAudioSession(manager);

        var first = await session.StopAsync();
        var second = await session.StopAsync();

        Assert.IsTrue(first.IsSuccess);
        Assert.IsTrue(second.IsSuccess);
        Assert.IsFalse(session.Snapshot.IsRunning);
        Assert.AreEqual(RtmpOutputState.Idle, manager.Snapshot.State);
    }

    [TestMethod]
    public async Task Interlude_start_without_audio_session_fails_closed()
    {
        await using var manager = new WindowsRtmpOutputManager();
        await using var session = new WindowsRtmpAudioSession(manager);

        var result = await session.StartInterludeAsync(null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsRtmpAudioSessionFailureCode.InvalidArguments, result.Error?.Code);
    }

    [TestMethod]
    public async Task Interlude_stop_without_start_is_idempotent()
    {
        await using var manager = new WindowsRtmpOutputManager();
        await using var session = new WindowsRtmpAudioSession(manager);

        var result = await session.StopInterludeAsync();

        Assert.IsTrue(result.IsSuccess);
        Assert.IsFalse(session.Snapshot.IsRunning);
    }

    [TestMethod]
    public async Task Invalid_audio_effects_are_rejected_before_rtmp_host_start()
    {
        var sourcePath = Path.Combine(Path.GetTempPath(), $"gpalive-rtmp-{Guid.NewGuid():N}.mp3");
        File.WriteAllBytes(sourcePath, new byte[] { 1 });
        try
        {
            await using var manager = new WindowsRtmpOutputManager();
            await using var session = new WindowsRtmpAudioSession(manager);

            var result = await session.StartAsync(
                RtmpOutputConfig.Default with
                {
                    TargetUrl = "not-a-valid-rtmp-target",
                    VideoEnabled = false,
                    AudioEnabled = true,
                },
                new SourceMediaDto(
                    sourcePath,
                    "media://test",
                    MediaKind.Audio,
                    MediaCompatibilityMode.Direct,
                    "audio.mp3",
                    1,
                    1_000,
                    0,
                    1_000,
                    null,
                    null,
                    null,
                    48_000,
                    2,
                    null,
                    "aac",
                    null,
                    "not-computed"),
                Environment.ProcessPath,
                null,
                audioEffects: new AudioEffectParams { PlaybackSpeed = 0.1 });

            Assert.IsFalse(result.IsSuccess);
            Assert.AreEqual(WindowsRtmpAudioSessionFailureCode.InvalidArguments, result.Error?.Code);
            Assert.AreEqual(RtmpOutputState.Idle, manager.Snapshot.State);
        }
        finally
        {
            File.Delete(sourcePath);
        }
    }

    [TestMethod]
    public async Task Shared_mono_final_bus_is_used_by_the_rtmp_audio_chain()
    {
        var directory = Path.Combine(
            Path.GetTempPath(),
            "gpautolive-rtmp-mono-tests",
            Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        var sourcePath = Path.Combine(directory, "mono.mp4");
        File.WriteAllBytes(sourcePath, [0x00, 0x01]);

        try
        {
            using var bus = new FinalPcmBus(channels: 1);
            await using var manager = new WindowsRtmpOutputManager();
            await using var session = new WindowsRtmpAudioSession(manager);

            var result = await session.StartAsync(
                RtmpOutputConfig.Default with
                {
                    TargetUrl = "rtmp://127.0.0.1/live/mono",
                    VideoEnabled = false,
                    AudioEnabled = true,
                },
                new SourceMediaDto(
                    sourcePath,
                    "media://mono",
                    MediaKind.Video,
                    MediaCompatibilityMode.Direct,
                    "mono.mp4",
                    1,
                    10_000,
                    null,
                    null,
                    1_280,
                    720,
                    30,
                    48_000,
                    1,
                    "h264",
                    "aac",
                    null,
                    "disabled"),
                Path.Combine(Environment.SystemDirectory, "cmd.exe"),
                sourceIdentity: null,
                sharedFinalPcmBus: bus,
                sharedRtmpOutputSource: new SingleFramePcmOutputSource());

            Assert.IsTrue(result.IsSuccess, result.Error?.Message);
            Assert.IsTrue(result.Snapshot.ForwardedFrames > 0, "RTMP 启动成功必须已经消费一帧最终 PCM。");
            Assert.IsTrue(session.Snapshot.IsRunning);

            var stopped = await session.StopAsync();
            Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
        }
        finally
        {
            if (Directory.Exists(directory))
            {
                Directory.Delete(directory, recursive: true);
            }
        }
    }

    private sealed class SingleFramePcmOutputSource : IAudioPcmOutputSource
    {
        private int _served;

        public int Channels => 1;

        public bool IsClosed => Volatile.Read(ref _served) != 0;

        public bool TryRead(Span<float> destination, out int framesRead, out PcmRingBufferError? error)
        {
            if (destination.Length < Channels || Interlocked.Exchange(ref _served, 1) != 0)
            {
                framesRead = 0;
                error = null;
                return true;
            }

            destination[0] = 0.25F;
            framesRead = 1;
            error = null;
            return true;
        }

        public bool TryReadRealtime(Span<float> destination, out int framesRead, out PcmRingBufferError? error) =>
            TryRead(destination, out framesRead, out error);
    }
}
