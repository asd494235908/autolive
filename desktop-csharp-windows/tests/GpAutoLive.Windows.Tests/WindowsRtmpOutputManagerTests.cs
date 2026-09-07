using System.Diagnostics;
using GpAutoLive.Contracts;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsRtmpOutputManagerTests
{
    [TestMethod]
    public async Task Stop_without_start_is_idempotent()
    {
        await using var manager = new WindowsRtmpOutputManager();

        var first = await manager.StopAsync();
        var second = await manager.StopAsync();

        Assert.IsTrue(first.IsSuccess, first.Error?.Message);
        Assert.IsTrue(second.IsSuccess, second.Error?.Message);
        Assert.AreEqual(RtmpOutputState.Idle, second.Snapshot.State);
    }

    [TestMethod]
    public async Task Start_with_missing_ffmpeg_fails_before_spawning()
    {
        using var fixture = MediaFixture.Create();
        await using var manager = new WindowsRtmpOutputManager();

        var result = await manager.StartAsync(
            RtmpOutputConfig.Default with
            {
                TargetUrl = "rtmp://127.0.0.1/live/test",
                AudioEnabled = false
            },
            fixture.Source,
            Path.Combine(fixture.DirectoryPath, "missing-ffmpeg.exe"));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsRtmpFailureCode.InvalidPlan, result.Error?.Code);
        Assert.IsNull(result.Snapshot.ProcessId);
    }

    [TestMethod]
    public async Task Pcm_write_without_audio_session_is_rejected()
    {
        await using var manager = new WindowsRtmpOutputManager();

        var result = await manager.WriteFinalPcmAsync(new float[] { 0.0f, 0.0f });

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsRtmpFailureCode.NotRunning, result.Error?.Code);
    }

    [TestMethod]
    public void Ffmpeg_progress_requires_positive_output_evidence()
    {
        Assert.IsFalse(WindowsRtmpOutputManager.IsProgressOutputAdvanced("progress=continue"));
        Assert.IsFalse(WindowsRtmpOutputManager.IsProgressOutputAdvanced("out_time_ms=0"));
        Assert.IsFalse(WindowsRtmpOutputManager.IsProgressOutputAdvanced("total_size=0"));
        Assert.IsTrue(WindowsRtmpOutputManager.IsProgressOutputAdvanced("out_time_ms=123000"));
        Assert.IsTrue(WindowsRtmpOutputManager.IsProgressOutputAdvanced("total_size=4096"));
        Assert.IsFalse(WindowsRtmpOutputManager.IsProgressOutputAdvanced("out_time_ms=not-a-number"));
    }

    [TestMethod]
    public async Task Start_after_dispose_returns_closed_without_validating_plan()
    {
        await using var manager = new WindowsRtmpOutputManager();
        await manager.DisposeAsync();

        var result = await manager.StartAsync(null, null, null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsRtmpFailureCode.Closed, result.Error?.Code);
    }

    [TestMethod]
    public async Task Unexpected_process_exit_closes_pcm_input_and_clears_process_identity()
    {
        using var fixture = MediaFixture.Create();
        await using var manager = new WindowsRtmpOutputManager();
        var failedNotification = new TaskCompletionSource<WindowsRtmpSnapshot>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        manager.SnapshotChanged += snapshot =>
        {
            if (snapshot.State == RtmpOutputState.Failed && snapshot.ProcessId is null)
            {
                failedNotification.TrySetResult(snapshot);
            }
        };

        var started = await manager.StartAsync(
            RtmpOutputConfig.Default with
            {
                TargetUrl = "rtmp://127.0.0.1/live/local-fixture",
                AudioEnabled = true,
            },
            fixture.Source,
            Path.Combine(Environment.SystemDirectory, "cmd.exe"));

        Assert.IsTrue(started.IsSuccess, started.Error?.Message);
        Assert.IsNotNull(started.Snapshot.ProcessId);
        Assert.IsTrue(started.Snapshot.FinalPcmInputOpen);

        using (var process = Process.GetProcessById(started.Snapshot.ProcessId.Value))
        {
            process.Kill(entireProcessTree: true);
            process.WaitForExit();
        }

        Assert.IsTrue(
            SpinWait.SpinUntil(
                () =>
                {
                    var snapshot = manager.Snapshot;
                    return snapshot.State == RtmpOutputState.Failed
                        && snapshot.ProcessId is null
                        && !snapshot.FinalPcmInputOpen;
                },
                TimeSpan.FromSeconds(2)),
            "本地夹具进程未进入失败终态。 ");

        var snapshot = manager.Snapshot;
        Assert.AreEqual("rtmp_process_exited", snapshot.ErrorCode);
        Assert.IsNull(snapshot.ProcessId);
        Assert.IsFalse(snapshot.FinalPcmInputOpen);
        var notified = await failedNotification.Task.WaitAsync(TimeSpan.FromSeconds(2));
        Assert.AreEqual("rtmp_process_exited", notified.ErrorCode);
        Assert.IsNull(notified.ProcessId);
    }

    [TestMethod]
    public async Task Audio_session_does_not_report_success_when_pump_fails_during_start()
    {
        using var fixture = MediaFixture.Create();
        using var bus = new FinalPcmBus();
        await using var manager = new WindowsRtmpOutputManager();
        await using var session = new WindowsRtmpAudioSession(manager);

        var result = await session.StartAsync(
            RtmpOutputConfig.Default with
            {
                TargetUrl = "rtmp://127.0.0.1/live/local-fixture",
                VideoEnabled = false,
                AudioEnabled = true,
            },
            fixture.Source,
            Path.Combine(Environment.SystemDirectory, "cmd.exe"),
            sourceIdentity: null,
            sharedFinalPcmBus: bus,
            sharedRtmpOutputSource: new FailingPcmOutputSource());

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsRtmpAudioSessionFailureCode.PumpFailed, result.Error?.Code);
        Assert.IsFalse(result.Snapshot.IsRunning);
        Assert.AreEqual(RtmpOutputState.Idle, manager.Snapshot.State);
    }

    [TestMethod]
    public async Task Audio_session_does_not_report_success_before_forwarding_final_pcm()
    {
        using var fixture = MediaFixture.Create();
        using var bus = new FinalPcmBus();
        await using var manager = new WindowsRtmpOutputManager();
        await using var session = new WindowsRtmpAudioSession(manager);

        var result = await session.StartAsync(
            RtmpOutputConfig.Default with
            {
                TargetUrl = "rtmp://127.0.0.1/live/no-pcm",
                VideoEnabled = false,
                AudioEnabled = true,
            },
            fixture.Source,
            Path.Combine(Environment.SystemDirectory, "cmd.exe"),
            sourceIdentity: null,
            sharedFinalPcmBus: bus,
            sharedRtmpOutputSource: new EmptyPcmOutputSource());

        Assert.IsFalse(result.IsSuccess, "未转发任何最终 PCM 时不能把 RTMP 声音会话报告为成功。");
        Assert.AreEqual(WindowsRtmpAudioSessionFailureCode.PumpFailed, result.Error?.Code);
        Assert.AreEqual<ulong>(0, result.Snapshot.ForwardedFrames);
        Assert.IsFalse(result.Snapshot.IsRunning);
        Assert.AreEqual(RtmpOutputState.Idle, manager.Snapshot.State);
    }

    private sealed class FailingPcmOutputSource : IAudioPcmOutputSource
    {
        public int Channels => FinalPcmBus.DefaultChannels;

        public bool IsClosed => false;

        public bool TryRead(Span<float> destination, out int framesRead, out PcmRingBufferError? error)
        {
            framesRead = 0;
            error = new(PcmRingBufferFailureCode.MixFailed, "测试 PCM 来源失败。");
            return false;
        }

        public bool TryReadRealtime(Span<float> destination, out int framesRead, out PcmRingBufferError? error) =>
            TryRead(destination, out framesRead, out error);
    }

    private sealed class EmptyPcmOutputSource : IAudioPcmOutputSource
    {
        public int Channels => FinalPcmBus.DefaultChannels;

        public bool IsClosed => false;

        public bool TryRead(Span<float> destination, out int framesRead, out PcmRingBufferError? error)
        {
            framesRead = 0;
            error = null;
            return true;
        }

        public bool TryReadRealtime(Span<float> destination, out int framesRead, out PcmRingBufferError? error) =>
            TryRead(destination, out framesRead, out error);
    }

    private static SourceMediaDto CreateSource(string path) => new(
        path,
        path,
        MediaKind.Video,
        MediaCompatibilityMode.Direct,
        Path.GetFileName(path),
        1,
        10_000,
        null,
        null,
        1_280,
        720,
        30,
        48_000,
        2,
        "h264",
        "aac",
        null,
        "disabled");

    private sealed class MediaFixture : IDisposable
    {
        private MediaFixture(string directoryPath, string mediaPath)
        {
            DirectoryPath = directoryPath;
            Source = CreateSource(mediaPath);
        }

        public string DirectoryPath { get; }
        public SourceMediaDto Source { get; }

        public static MediaFixture Create()
        {
            var directory = Path.Combine(
                Path.GetTempPath(),
                "gpautolive-rtmp-host-tests",
                Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(directory);
            var mediaPath = Path.Combine(directory, "sample.mp4");
            File.WriteAllBytes(mediaPath, [0x00, 0x01]);
            return new(directory, mediaPath);
        }

        public void Dispose()
        {
            if (Directory.Exists(DirectoryPath))
            {
                Directory.Delete(DirectoryPath, recursive: true);
            }
        }
    }
}
