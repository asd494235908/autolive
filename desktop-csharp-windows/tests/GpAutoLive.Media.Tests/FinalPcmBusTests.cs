namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class FinalPcmBusTests
{
    [TestMethod]
    public void Publish_fans_out_in_order_to_both_consumers()
    {
        var bus = new FinalPcmBus(capacityFrames: 8, channels: 2);
        bus.SetRtmpConsumerAttached(true);

        var samples = new float[] { 1, 2, 3, 4, 5, 6 };
        var published = bus.TryPublish(samples, out var frames, out var error);

        Assert.IsTrue(published);
        Assert.AreEqual(3, frames);
        Assert.IsNull(error);
        var output = new float[6];
        Assert.IsTrue(bus.OutputBuffer.TryRead(output, out var outputFrames, out _));
        Assert.AreEqual(3, outputFrames);
        CollectionAssert.AreEqual(samples, output);
        var rtmp = new float[6];
        Assert.IsTrue(bus.RtmpBuffer.TryRead(rtmp, out var rtmpFrames, out _));
        Assert.AreEqual(3, rtmpFrames);
        CollectionAssert.AreEqual(samples, rtmp);
    }

    [TestMethod]
    public void Publish_skips_rtmp_branch_until_a_consumer_is_attached()
    {
        var bus = new FinalPcmBus(capacityFrames: 8, channels: 1);

        Assert.IsTrue(bus.TryPublish([1, 2, 3], out var frames, out var error), error?.Message);

        Assert.AreEqual(3, frames);
        Assert.AreEqual(3, bus.Snapshot.OutputAvailableFrames);
        Assert.AreEqual(0, bus.Snapshot.RtmpAvailableFrames);
        Assert.AreEqual((ulong)0, bus.Snapshot.RtmpDroppedFrames);

        bus.SetRtmpConsumerAttached(true);
        Assert.IsTrue(bus.TryPublish([4, 5], out _, out error), error?.Message);

        Assert.AreEqual(2, bus.Snapshot.RtmpAvailableFrames);
    }

    [TestMethod]
    public void Full_consumers_keep_realtime_bounds_and_report_drops_independently()
    {
        var bus = new FinalPcmBus(capacityFrames: 2, channels: 1);
        bus.SetRtmpConsumerAttached(true);

        Assert.IsTrue(bus.TryPublish(new float[] { 1, 2 }, out _, out _));
        Assert.IsTrue(bus.TryPublish(new float[] { 3, 4 }, out _, out _));

        var snapshot = bus.Snapshot;
        Assert.AreEqual((ulong)2, snapshot.OutputDroppedFrames);
        Assert.AreEqual((ulong)2, snapshot.RtmpDroppedFrames);
        Assert.AreEqual(2, snapshot.OutputAvailableFrames);
        Assert.AreEqual(2, snapshot.RtmpAvailableFrames);
    }

    [TestMethod]
    public void Invalid_shape_and_oversized_publish_are_rejected()
    {
        var bus = new FinalPcmBus(capacityFrames: 8, channels: 2);

        Assert.IsFalse(bus.TryPublish(new float[] { 1 }, out _, out var shapeError));
        Assert.AreEqual(FinalPcmBusFailureCode.InvalidFrameShape, shapeError?.Code);

        var oversized = new float[(FinalPcmBus.MaxFramesPerPublish + 1) * 2];
        Assert.IsFalse(bus.TryPublish(oversized, out _, out var sizeError));
        Assert.AreEqual(FinalPcmBusFailureCode.TooLarge, sizeError?.Code);
    }

    [TestMethod]
    public void Close_preserves_existing_frames_and_rejects_new_publish()
    {
        var bus = new FinalPcmBus(capacityFrames: 8, channels: 1);
        Assert.IsTrue(bus.TryPublish(new float[] { 7, 8 }, out _, out _));

        bus.Close();

        Assert.IsFalse(bus.TryPublish(new float[] { 9 }, out _, out var error));
        Assert.AreEqual(FinalPcmBusFailureCode.Closed, error?.Code);
        Assert.AreEqual(2, bus.Snapshot.OutputAvailableFrames);
        Assert.IsTrue(bus.Snapshot.IsClosed);
    }

    [TestMethod]
    public void Discard_overlay_pending_clears_local_and_rtmp_tails_without_closing_bus()
    {
        using var bus = new FinalPcmBus(capacityFrames: 8, channels: 1);
        bus.SetRtmpConsumerAttached(true);

        Assert.IsTrue(bus.TryPublishOverlay([1, 2], out _, out var error), error?.Message);
        Assert.AreEqual(2, bus.OutputOverlayBuffer.Snapshot.AvailableFrames);
        Assert.AreEqual(2, bus.RtmpOverlayBuffer.Snapshot.AvailableFrames);

        bus.DiscardOverlayPending();

        Assert.AreEqual(0, bus.OutputOverlayBuffer.Snapshot.AvailableFrames);
        Assert.AreEqual(0, bus.RtmpOverlayBuffer.Snapshot.AvailableFrames);
        Assert.IsFalse(bus.Snapshot.IsClosed);
    }

    [TestMethod]
    public void Spectrum_diagnostics_follow_base_and_overlay_pcm_without_consuming_output()
    {
        using var bus = new FinalPcmBus(capacityFrames: 8_192, channels: 1);
        var samples = Enumerable.Range(0, 4_096)
            .Select(index => (float)Math.Sin(index * Math.PI / 8))
            .ToArray();

        Assert.IsTrue(bus.TryPublish(samples, out _, out var error), error?.Message);
        Assert.IsTrue(bus.TryPublishOverlay(samples, out _, out error), error?.Message);

        Assert.IsTrue(bus.OutputSpectrum.HasSignal);
        Assert.IsTrue(bus.OverlaySpectrum.HasSignal);
        Assert.AreEqual(4_096, bus.OutputBuffer.Snapshot.AvailableFrames);
        Assert.AreEqual(4_096, bus.OutputOverlayBuffer.Snapshot.AvailableFrames);
    }
}
