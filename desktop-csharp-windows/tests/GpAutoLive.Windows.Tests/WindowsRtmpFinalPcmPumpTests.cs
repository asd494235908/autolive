using GpAutoLive.Media;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsRtmpFinalPcmPumpTests
{
    [TestMethod]
    public async Task Missing_manager_is_rejected_without_consuming_source()
    {
        var source = new AudioPcmRingBuffer(32, 2);
        Assert.IsTrue(source.TryWrite(new float[] { 1, 2 }, out _, out _));
        using var pump = new WindowsRtmpFinalPcmPump(source, channels: 2);

        var result = await pump.RunAsync(null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsRtmpPcmPumpFailureCode.InvalidArguments, result.Error?.Code);
        Assert.AreEqual(1, source.Snapshot.AvailableFrames);
    }

    [TestMethod]
    public async Task Not_running_manager_stops_the_pump_with_write_failure()
    {
        var source = new AudioPcmRingBuffer(32, 2);
        Assert.IsTrue(source.TryWrite(new float[] { 1, 2, 3, 4 }, out _, out _));
        using var pump = new WindowsRtmpFinalPcmPump(source, channels: 2);
        await using var manager = new WindowsRtmpOutputManager();

        var result = await pump.RunAsync(manager);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsRtmpPcmPumpFailureCode.WriteFailed, result.Error?.Code);
        Assert.IsFalse(pump.Snapshot.IsRunning);
    }

    [TestMethod]
    public async Task Closed_source_drains_then_finishes_without_busy_loop()
    {
        var source = new AudioPcmRingBuffer(32, 2);
        source.Close();
        using var pump = new WindowsRtmpFinalPcmPump(source, channels: 2);
        await using var manager = new WindowsRtmpOutputManager();

        var result = await pump.RunAsync(manager);

        Assert.IsTrue(result.IsSuccess);
        Assert.AreEqual((ulong)0, result.Snapshot.ForwardedFrames);
    }
}
