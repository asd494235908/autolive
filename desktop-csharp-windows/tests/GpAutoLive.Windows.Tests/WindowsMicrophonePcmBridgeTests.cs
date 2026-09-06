using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsMicrophonePcmBridgeTests
{
    [TestMethod]
    public void Speaking_mono_input_is_published_to_stereo_overlay()
    {
        using var bus = new FinalPcmBus(capacityFrames: 8, channels: 2);
        var input = new AudioPcmRingBuffer(capacityFrames: 8, channels: 1);
        Assert.IsTrue(input.TryWrite([0.25F, -0.5F], out _, out _));
        var bridge = new WindowsMicrophonePcmBridge(inputChannels: 1);

        var drained = bridge.TryDrain(input, bus, speaking: true, out var error);

        Assert.IsTrue(drained, error?.Message);
        Assert.AreEqual(2, bus.OutputOverlayBuffer.Snapshot.AvailableFrames);
        var output = new float[4];
        Assert.IsTrue(bus.OutputOverlayBuffer.TryRead(output, out var frames, out var readError), readError?.Message);
        Assert.AreEqual(2, frames);
        CollectionAssert.AreEqual(new[] { 0.25F, 0.25F, -0.5F, -0.5F }, output);
    }

    [TestMethod]
    public void Non_speaking_input_is_discarded_without_publishing()
    {
        using var bus = new FinalPcmBus(capacityFrames: 8, channels: 1);
        var input = new AudioPcmRingBuffer(capacityFrames: 8, channels: 1);
        Assert.IsTrue(input.TryWrite([0.25F], out _, out _));
        var bridge = new WindowsMicrophonePcmBridge(inputChannels: 1);

        var drained = bridge.TryDrain(input, bus, speaking: false, out var error);

        Assert.IsTrue(drained, error?.Message);
        Assert.AreEqual(0, input.Snapshot.AvailableFrames);
        Assert.AreEqual(0, bus.OutputOverlayBuffer.Snapshot.AvailableFrames);
    }

    [TestMethod]
    public void Missing_final_bus_fails_closed()
    {
        var input = new AudioPcmRingBuffer(capacityFrames: 8, channels: 1);
        Assert.IsTrue(input.TryWrite([0.25F], out _, out _));
        var bridge = new WindowsMicrophonePcmBridge(inputChannels: 1);

        var drained = bridge.TryDrain(input, finalPcmBus: null, speaking: true, out var error);

        Assert.IsFalse(drained);
        Assert.AreEqual(WindowsPortAudioInputFailureCode.OutputBusUnavailable, error?.Code);
        Assert.IsTrue(error?.Retryable);
    }
}
