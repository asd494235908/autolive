namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class FinalEffectWindowSizingTests
{
    [TestMethod]
    public void Initial_window_size_fits_video_to_work_area_without_changing_ratio()
    {
        var size = GpAutoLive.App.Features.Playback.FinalEffectWindowSizing.CalculateInitialWindowSize(
            videoWidth: 1920,
            videoHeight: 1080,
            workAreaWidth: 1600,
            workAreaHeight: 900,
            frameWidth: 16,
            frameHeight: 39);

        Assert.AreEqual(1547, size.Width);
        Assert.AreEqual(900, size.Height);
        Assert.AreEqual(16d / 9d, size.ClientWidth / (double)size.ClientHeight, 0.002d);
    }

    [TestMethod]
    public void Interactive_resize_uses_the_video_ratio_from_either_axis()
    {
        var fromWidth = GpAutoLive.App.Features.Playback.FinalEffectWindowSizing.CalculateClientSizeFromWidth(
            requestedWidth: 400,
            aspectRatio: 4d / 3d,
            minimumWidth: 320,
            minimumHeight: 180);
        var fromHeight = GpAutoLive.App.Features.Playback.FinalEffectWindowSizing.CalculateClientSizeFromHeight(
            requestedHeight: 300,
            aspectRatio: 4d / 3d,
            minimumWidth: 320,
            minimumHeight: 180);

        Assert.AreEqual(400, fromWidth.Width);
        Assert.AreEqual(300, fromWidth.Height);
        Assert.AreEqual(400, fromHeight.Width);
        Assert.AreEqual(300, fromHeight.Height);
    }
}
