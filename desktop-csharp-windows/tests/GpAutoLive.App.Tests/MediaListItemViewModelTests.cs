using GpAutoLive.App.Features.Media;
using GpAutoLive.Contracts;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class MediaListItemViewModelTests
{
    [TestMethod]
    public void VideoFormatLabel_UsesResolutionAndReducedAspectRatio()
    {
        var item = new MediaListItemViewModel(CreateSource(
            MediaKind.Video,
            width: 1920,
            height: 1080,
            sampleRate: null,
            channels: null));

        Assert.AreEqual("1920×1080 16:9", item.VideoFormatLabel);
        Assert.AreEqual("00:00:12", item.DurationLabel);
    }

    [TestMethod]
    public void AudioFormatLabel_UsesKhzAndHumanReadableChannelName()
    {
        var item = new MediaListItemViewModel(CreateSource(
            MediaKind.Audio,
            width: null,
            height: null,
            sampleRate: 48_000,
            channels: 2));

        Assert.AreEqual("48kHz 立体声", item.AudioFormatLabel);
    }

    private static SourceMediaDto CreateSource(
        MediaKind kind,
        uint? width,
        uint? height,
        uint? sampleRate,
        ushort? channels) => new(
            SourcePath: @"C:\media\sample",
            PlaybackReference: @"C:\media\sample",
            MediaKind: kind,
            CompatibilityMode: MediaCompatibilityMode.Direct,
            FileName: "sample",
            FileSizeBytes: 1,
            DurationMs: 12_000,
            AudioStartMs: 0,
            AudioEndMs: 11_999,
            Width: width,
            Height: height,
            FrameRateFps: kind is MediaKind.Video ? 30 : null,
            AudioSampleRateHz: sampleRate,
            AudioChannelCount: channels,
            VideoCodecName: kind is MediaKind.Video ? "h264" : null,
            AudioCodecName: sampleRate is not null ? "aac" : null,
            Mp4Sha256: null,
            Mp4HashStatus: "test");
}
