using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class AudioPcmRingBufferTests
{
    [TestMethod]
    public void WriteAndReadPreserveInterleavedFrameOrderAcrossWrap()
    {
        var ring = new AudioPcmRingBuffer(capacityFrames: 3, channels: 2);

        Assert.IsTrue(ring.TryWrite([1, 10, 2, 20], out var firstWritten, out var firstError));
        Assert.AreEqual(2, firstWritten);
        Assert.IsNull(firstError);
        Assert.IsTrue(ring.TryWrite([3, 30, 4, 40], out var secondWritten, out var secondError));
        Assert.AreEqual(2, secondWritten);
        Assert.IsNull(secondError);

        var output = new float[6];
        Assert.IsTrue(ring.TryRead(output, out var framesRead, out var readError));

        Assert.AreEqual(3, framesRead);
        Assert.IsNull(readError);
        CollectionAssert.AreEqual(new float[] { 2, 20, 3, 30, 4, 40 }, output);
        Assert.AreEqual(0, ring.Snapshot.AvailableFrames);
        Assert.AreEqual(1UL, ring.Snapshot.DroppedFrames);
    }

    [TestMethod]
    public void OversizedWriteKeepsNewestCapacityAndCountsDroppedFrames()
    {
        var ring = new AudioPcmRingBuffer(capacityFrames: 2, channels: 1);

        Assert.IsTrue(ring.TryWrite([1, 2, 3], out var framesWritten, out var error));

        Assert.AreEqual(2, framesWritten);
        Assert.IsNull(error);
        var output = new float[2];
        Assert.IsTrue(ring.TryRead(output, out framesWritten, out error));
        Assert.AreEqual(2, framesWritten);
        CollectionAssert.AreEqual(new float[] { 2, 3 }, output);
        Assert.AreEqual(1UL, ring.Snapshot.DroppedFrames);
    }

    [TestMethod]
    public void InvalidShapeAndClosedWriteFailWithoutChangingBufferedFrames()
    {
        var ring = new AudioPcmRingBuffer(capacityFrames: 4, channels: 2);

        Assert.IsFalse(ring.TryWrite([1], out var framesWritten, out var error));
        Assert.AreEqual(0, framesWritten);
        Assert.AreEqual(PcmRingBufferFailureCode.InvalidFrameShape, error?.Code);
        Assert.AreEqual(0, ring.Snapshot.AvailableFrames);

        Assert.IsTrue(ring.TryWrite([1, 2], out framesWritten, out error));
        Assert.IsNull(error);
        ring.Close();
        Assert.IsFalse(ring.TryWrite([3, 4], out framesWritten, out error));
        Assert.AreEqual(PcmRingBufferFailureCode.Closed, error?.Code);
        Assert.AreEqual(1, ring.Snapshot.AvailableFrames);

        var output = new float[2];
        Assert.IsTrue(ring.TryRead(output, out var framesRead, out error));
        Assert.AreEqual(1, framesRead);
        Assert.IsNull(error);
        CollectionAssert.AreEqual(new float[] { 1, 2 }, output);
    }

    [TestMethod]
    public void Realtime_read_and_write_use_same_bounded_storage()
    {
        var ring = new AudioPcmRingBuffer(capacityFrames: 2, channels: 1);
        Assert.IsTrue(ring.TryWriteRealtime([1, 2], out var written, out var writeError));
        Assert.AreEqual(2, written);
        Assert.IsNull(writeError);

        var output = new float[2];
        Assert.IsTrue(ring.TryReadRealtime(output, out var read, out var readError));
        Assert.AreEqual(2, read);
        Assert.IsNull(readError);
        CollectionAssert.AreEqual(new float[] { 1, 2 }, output);
    }

    [TestMethod]
    public void Realtime_write_after_close_fails_closed()
    {
        var ring = new AudioPcmRingBuffer(capacityFrames: 2, channels: 1);
        ring.Close();

        Assert.IsFalse(ring.TryWriteRealtime([1], out var written, out var error));
        Assert.AreEqual(0, written);
        Assert.AreEqual(PcmRingBufferFailureCode.Closed, error?.Code);
    }
}
