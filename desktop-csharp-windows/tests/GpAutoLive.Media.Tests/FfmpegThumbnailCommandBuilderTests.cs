using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class FfmpegThumbnailCommandBuilderTests
{
    [TestMethod]
    public void Creates_a_bounded_single_frame_jpeg_plan()
    {
        var plan = FfmpegThumbnailCommandBuilder.Create(
            @"C:\media\ffmpeg.exe",
            @"C:\media\sample.mp4",
            @"C:\Users\tester\AppData\Local\Temp\sample.jpg");

        Assert.AreEqual(@"C:\media\ffmpeg.exe", plan.ExecutablePath);
        CollectionAssert.Contains(plan.Arguments, "-nostdin");
        CollectionAssert.Contains(plan.Arguments, "-frames:v");
        CollectionAssert.Contains(plan.Arguments, "1");
        CollectionAssert.Contains(plan.Arguments, "-an");
        CollectionAssert.Contains(plan.Arguments, @"C:\Users\tester\AppData\Local\Temp\sample.jpg");
        Assert.AreEqual(TimeSpan.FromSeconds(12), plan.Timeout);
        Assert.IsTrue(plan.Arguments.Length <= 32);
    }

    [TestMethod]
    public void Rejects_non_video_or_non_jpeg_output()
    {
        FfmpegThumbnailCommandValidationException? sourceError = null;
        try
        {
            _ = FfmpegThumbnailCommandBuilder.Create(
                @"C:\media\ffmpeg.exe",
                @"C:\media\sample.mp3",
                @"C:\temp\sample.jpg");
        }
        catch (FfmpegThumbnailCommandValidationException exception)
        {
            sourceError = exception;
        }

        Assert.IsNotNull(sourceError);
        Assert.AreEqual(MediaProbeFailureCode.UnsupportedMediaStream, sourceError.Code);

        FfmpegThumbnailCommandValidationException? outputError = null;
        try
        {
            _ = FfmpegThumbnailCommandBuilder.Create(
                @"C:\media\ffmpeg.exe",
                @"C:\media\sample.mp4",
                @"C:\temp\sample.png");
        }
        catch (FfmpegThumbnailCommandValidationException exception)
        {
            outputError = exception;
        }

        Assert.IsNotNull(outputError);
        Assert.AreEqual(MediaProbeFailureCode.PathTooLong, outputError.Code);
    }
}
