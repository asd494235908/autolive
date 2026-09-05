using GpAutoLive.Media;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsRtmpEncoderProbeFixtureTests
{
    [TestMethod]
    public async Task Real_ffmpeg_encoder_probe_runs_when_explicitly_enabled()
    {
        var ffmpeg = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_FFMPEG");
        if (string.IsNullOrWhiteSpace(ffmpeg))
        {
            return;
        }

        var result = await RtmpEncoderProbe.ProbeAsync(
            ffmpeg,
            preferredEncoder: null,
            new WindowsExternalProcessRunner());

        Assert.IsTrue(result.IsSuccess, result.Error?.Message);
        Assert.IsFalse(string.IsNullOrWhiteSpace(result.Snapshot.SelectedEncoder));
        Assert.IsTrue(result.Snapshot.Candidates.Any(static candidate => candidate.IsAvailable));
    }
}
