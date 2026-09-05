using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsAudibleAudioClockTests
{
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
