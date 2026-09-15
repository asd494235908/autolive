using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class MpvAudioClockSynchronizationTests
{
    private static readonly MediaPlaybackIdentity Identity = new(1, 1, 0, 0);

    [TestMethod]
    [DataRow(1000UL, 1020UL, 1.25, 1.25)]
    [DataRow(1000UL, 1100UL, 1.25, 1.3125)]
    [DataRow(1100UL, 1000UL, 0.5, 0.475)]
    [DataRow(1250UL, 1000UL, 1.0, 0.95)]
    public void Normal_drift_changes_video_speed_without_seeking(
        ulong videoMs, ulong audioMs, double audioRate, double expectedRate)
    {
        var result = CreatePlayingSession().SynchronizeAudioClock(Identity, videoMs, audioMs, audioRate);
        Assert.IsTrue(result.IsSuccess);
        Assert.HasCount(1, result.Commands);
        Assert.AreEqual(MpvIpcCommandKind.SetPlaybackSpeed, result.Commands[0].Kind);
        Assert.IsTrue(result.Commands[0].TrySerialize(1, out var json, out _));
        using var parsed = JsonDocument.Parse(json!);
        Assert.AreEqual(expectedRate, parsed.RootElement.GetProperty("command")[2].GetDouble(), 0.000001);
    }

    [TestMethod]
    public void Severe_drift_seeks_once_to_audio_source_position_and_restores_base_speed()
    {
        var result = CreatePlayingSession().SynchronizeAudioClock(Identity, 1000, 2000, 1.25);
        Assert.IsTrue(result.IsSuccess);
        Assert.HasCount(2, result.Commands);
        Assert.AreEqual(MpvIpcCommandKind.SetPlaybackSpeed, result.Commands[0].Kind);
        Assert.AreEqual(MpvIpcCommandKind.SeekAbsoluteMs, result.Commands[1].Kind);
        Assert.IsTrue(result.Commands[1].TrySerialize(1, out var json, out _));
        using var parsed = JsonDocument.Parse(json!);
        Assert.AreEqual(2.0, parsed.RootElement.GetProperty("command")[1].GetDouble());
    }

    [TestMethod]
    public void Stale_identity_invalid_rate_and_outside_source_clocks_never_dispatch()
    {
        var session = CreatePlayingSession();
        Assert.IsFalse(session.SynchronizeAudioClock(Identity with { LoopIndex = 1 }, 1000, 1100, 1).IsSuccess);
        Assert.IsFalse(session.SynchronizeAudioClock(Identity, 1000, 1100, double.NaN).IsSuccess);
        Assert.IsFalse(session.SynchronizeAudioClock(Identity, 1000, 60000, 1).IsSuccess);
        session.Pause();
        Assert.IsTrue(session.SynchronizeAudioClock(Identity, 1000, 2000, 1).Commands.IsEmpty);
    }

    [TestMethod]
    public void Loadfile_resets_speed_for_next_source_without_audio()
    {
        var session = CreatePlayingSession();
        var result = session.BindSource(session.Snapshot.ActiveSource! with { Identity = Identity with { LoopIndex = 1 } });
        Assert.IsTrue(result.Commands[0].TrySerialize(1, out var json, out _));
        using var parsed = JsonDocument.Parse(json!);
        Assert.AreEqual("1", parsed.RootElement.GetProperty("command")[4].GetProperty("speed").GetString());
    }

    private static MpvPlaybackSession CreatePlayingSession()
    {
        const string path = @"C:\media\sample.mp4";
        var source = new SourceMediaDto(path, path, MediaKind.Video, MediaCompatibilityMode.Direct,
            "sample.mp4", 1, 60000, null, null, 1280, 720, 30, 48000, 2, "h264", "aac", null, "disabled");
        Assert.IsTrue(MpvActiveSource.TryCreate(source, Identity, out var active, out _));
        var session = new MpvPlaybackSession();
        Assert.IsTrue(session.BindSource(active).IsSuccess);
        Assert.IsTrue(session.Start().IsSuccess);
        return session;
    }
}
