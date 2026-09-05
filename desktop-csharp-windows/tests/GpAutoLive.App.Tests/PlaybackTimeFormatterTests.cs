using GpAutoLive.App.Features.Playback;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class PlaybackTimeFormatterTests
{
    [TestMethod]
    public void Formats_missing_and_short_positions()
    {
        Assert.AreEqual("—", PlaybackTimeFormatter.Format(null));
        Assert.AreEqual("00:00:00", PlaybackTimeFormatter.Format(0));
        Assert.AreEqual("00:01:05", PlaybackTimeFormatter.Format(65_000));
    }

    [TestMethod]
    public void Formats_long_position_with_hours()
    {
        Assert.AreEqual("01:02:03", PlaybackTimeFormatter.Format(3_723_000));
    }
}
