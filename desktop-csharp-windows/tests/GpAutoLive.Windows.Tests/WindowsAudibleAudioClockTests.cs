using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsAudibleAudioClockTests
{
    [TestMethod]
    public void Candidate_commit_subtracts_already_queued_dac_frames_and_uses_active_rate()
    {
        var clock = new WindowsAudibleAudioClockSnapshot(0, 0, 48_000, 20_000, true, 1_000, 2.0);
        Assert.AreEqual<ulong>(1_440, WindowsAudioPlaybackController.CalculateFramesUntilPosition(clock, 1_100));
        Assert.AreEqual<ulong>(0, WindowsAudioPlaybackController.CalculateFramesUntilPosition(clock, 1_010));
        Assert.AreEqual<ulong>(0, WindowsAudioPlaybackController.CalculateFramesUntilPosition(clock, 900));
    }

    [TestMethod]
    public void Mixed_overlay_frames_do_not_advance_base_media_clock()
    {
        var clock = new WindowsAudibleAudioClock();
        clock.Anchor(0, 0, 1.5);
        var output = new WindowsPortAudioOutputSnapshot(true, 1, 2, 48_000, 256, 0, 0, null, null)
        {
            HasTimeInfo = true,
            OutputFramesWritten = 9_600,
            BaseFramesWritten = 4_800,
        };
        var snapshot = clock.Project(output)!;
        Assert.AreEqual<ulong>(150, snapshot.PlaybackTimeMs!.Value);
        Assert.AreEqual(1.5, snapshot.PlaybackRate);
        clock.Anchor(output.MediaFramesWritten, 5_000);
        Assert.AreEqual<ulong>(5_000, clock.Project(output)!.PlaybackTimeMs!.Value);
    }

    [TestMethod]
    public void Startup_and_later_underrun_silence_does_not_advance_source_time()
    {
        var clock = new WindowsAudibleAudioClock();
        clock.Anchor(0, sourcePositionMs: 2_000);
        var output = new WindowsPortAudioOutputSnapshot(true, 1, 2, 48_000, 256,
            4_800, 0, null, null)
        {
            HasTimeInfo = true,
            OutputFramesWritten = 4_800,
        };
        Assert.AreEqual<ulong>(2_000, clock.Project(output)!.PlaybackTimeMs!.Value);
        output = output with { OutputFramesWritten = 9_600 };
        Assert.AreEqual<ulong>(2_100, clock.Project(output)!.PlaybackTimeMs!.Value);
        output = output with { OutputFramesWritten = 14_400, UnderrunFrames = 9_600 };
        Assert.AreEqual<ulong>(2_100, clock.Project(output)!.PlaybackTimeMs!.Value);
    }

    [TestMethod]
    public void Audible_position_subtracts_output_latency_from_callback_frames()
    {
        var clock = new WindowsAudibleAudioClock();
        clock.Anchor(1_000, sourcePositionMs: 20_000);

        var snapshot = clock.Project(new WindowsPortAudioOutputSnapshot(
            IsRunning: true,
            DeviceIndex: 1,
            Channels: 1,
            SampleRate: 48_000,
            FramesPerBuffer: 256,
            UnderrunFrames: 0,
            CallbackFailures: 0,
            ErrorCode: null,
            Error: null)
        {
            HasTimeInfo = true,
            OutputFramesWritten = 5_800,
            OutputLatencyMicroseconds = 20_000,
        });

        Assert.IsNotNull(snapshot);
        Assert.AreEqual<ulong>(4_840, snapshot!.AudibleFrames);
        Assert.AreEqual<ulong>(20_080, snapshot.PlaybackTimeMs!.Value);
        Assert.IsTrue(snapshot.HasTimeInfo);
    }

    [TestMethod]
    public void Audible_position_is_unavailable_without_portaudio_time_info()
    {
        var clock = new WindowsAudibleAudioClock();
        clock.Anchor(0, sourcePositionMs: 0);

        var snapshot = clock.Project(new WindowsPortAudioOutputSnapshot(
            IsRunning: true,
            DeviceIndex: 1,
            Channels: 2,
            SampleRate: 48_000,
            FramesPerBuffer: 256,
            UnderrunFrames: 0,
            CallbackFailures: 0,
            ErrorCode: null,
            Error: null)
        {
            OutputFramesWritten = 4_800,
        });

        Assert.IsNotNull(snapshot);
        Assert.IsNull(snapshot!.PlaybackTimeMs);
    }

    [TestMethod]
    public void Reanchoring_resets_media_position_at_a_candidate_boundary()
    {
        var clock = new WindowsAudibleAudioClock();
        clock.Anchor(0, sourcePositionMs: 0);
        clock.Anchor(4_000, sourcePositionMs: 0);

        var snapshot = clock.Project(new WindowsPortAudioOutputSnapshot(
            IsRunning: true,
            DeviceIndex: 1,
            Channels: 2,
            SampleRate: 48_000,
            FramesPerBuffer: 256,
            UnderrunFrames: 0,
            CallbackFailures: 0,
            ErrorCode: null,
            Error: null)
        {
            HasTimeInfo = true,
            OutputFramesWritten = 5_000,
            OutputLatencyMicroseconds = 0,
        });

        Assert.IsNotNull(snapshot);
        Assert.AreEqual<ulong>(21, snapshot!.PlaybackTimeMs!.Value);
    }
}
