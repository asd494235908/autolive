using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsMpvFrameCaptureTests
{
    [TestMethod]
    public async Task Capture_without_runtime_returns_no_image()
    {
        await using var controller = new WindowsMpvPlaybackController();
        var result = await controller.CapturePresentationAsync(new MediaPlaybackIdentity(1, 1, 0, 0));
        Assert.IsFalse(result.IsSuccess);
        Assert.IsNull(result.PngBytes);
        Assert.AreEqual(WindowsMpvPlaybackControllerFailureCode.NotRunning, result.Error?.Code);
    }

    [TestMethod]
    public void Screenshot_command_only_accepts_generated_identifier()
    {
        var id = Guid.NewGuid();
        var command = MpvIpcCommand.CapturePresentation(id);
        Assert.IsTrue(command.TrySerialize(1, out var line, out _));
        Assert.Contains("screenshot-to-file", line!);
        Assert.Contains("window", line!);
        Assert.Contains(id.ToString("N"), line!);
        Assert.IsFalse(MpvIpcCommand.CapturePresentation(Guid.Empty).TrySerialize(1, out _, out _));
    }

    [TestMethod]
    public void Png_header_rejects_bad_signature_truncation_and_oversized_dimensions()
    {
        Assert.IsFalse(WindowsMpvPlaybackController.IsBoundedPresentationPng(new byte[32]));
        var bytes = Convert.FromBase64String("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aN9sAAAAASUVORK5CYII=");
        Assert.IsTrue(WindowsMpvPlaybackController.IsBoundedPresentationPng(bytes));
        bytes[16] = 127;
        Assert.IsFalse(WindowsMpvPlaybackController.IsBoundedPresentationPng(bytes));
    }
}
