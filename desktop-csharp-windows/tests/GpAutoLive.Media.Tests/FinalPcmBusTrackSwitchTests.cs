using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class FinalPcmBusTrackSwitchTests
{
    [TestMethod]
    public void Single_output_source_drains_current_bus_then_reads_prepared_bus()
    {
        using var current = new FinalPcmBus(capacityFrames: 8, channels: 1);
        using var next = new FinalPcmBus(capacityFrames: 8, channels: 1);
        Assert.IsTrue(current.TryPublish([0.1F, 0.2F], out _, out var currentError), currentError?.Message);
        Assert.IsTrue(next.TryPublish([0.3F, 0.4F], out _, out var nextError), nextError?.Message);
        current.Close();
        next.Close();

        using var trackSwitch = new FinalPcmBusTrackSwitch(1, current);
        Assert.IsTrue(trackSwitch.TryPrepareNext(2, next, out var prepareError), prepareError?.Message);
        Assert.IsTrue(trackSwitch.TryCommitNext(2, out var commitError), commitError?.Message);

        Span<float> samples = stackalloc float[4];
        Assert.IsTrue(trackSwitch.OutputSource.TryRead(samples, out var framesRead, out var readError), readError?.Message);
        Assert.AreEqual(4, framesRead);
        Assert.AreEqual(0.1F, samples[0]);
        Assert.AreEqual(0.2F, samples[1]);
        Assert.AreEqual(0.3F, samples[2]);
        Assert.AreEqual(0.4F, samples[3]);
        Assert.AreEqual(2UL, trackSwitch.ActiveCandidateId);

        Assert.IsTrue(trackSwitch.TryAcknowledgePromotion(2, out var retired, out var acknowledgeError), acknowledgeError?.Message);
        Assert.AreSame(current, retired);
        Assert.AreSame(next, trackSwitch.ActiveBus);
        Assert.IsFalse(trackSwitch.HasPreparedNext);
    }

    [TestMethod]
    public void Scheduled_commit_promotes_the_final_bus_without_rtmp_consumer()
    {
        using var current = new FinalPcmBus(capacityFrames: 8, channels: 1);
        using var next = new FinalPcmBus(capacityFrames: 8, channels: 1);
        Assert.IsTrue(current.TryPublish([0.1F, 0.2F, 0.3F], out _, out _));
        Assert.IsTrue(next.TryPublish([0.8F, 0.9F], out _, out _));
        next.Close();

        using var trackSwitch = new FinalPcmBusTrackSwitch(1, current);
        Assert.IsTrue(trackSwitch.TryPrepareNext(2, next, out _));
        Assert.IsTrue(trackSwitch.TryCommitNextAtFrames(2, 2, out _));

        Span<float> samples = stackalloc float[2];
        Assert.IsTrue(trackSwitch.OutputSource.TryRead(samples, out var framesRead, out _));
        Assert.AreEqual(2, framesRead);
        Assert.AreEqual(1UL, trackSwitch.ActiveCandidateId);

        Span<float> nextSample = stackalloc float[1];
        Assert.IsTrue(trackSwitch.OutputSource.TryRead(nextSample, out framesRead, out _));
        Assert.AreEqual(1, framesRead);
        Assert.AreEqual(2UL, trackSwitch.ActiveCandidateId);

        Assert.IsTrue(trackSwitch.TryAcknowledgePromotion(2, out var retired, out var error), error?.Message);
        Assert.AreSame(current, retired);
        Assert.AreSame(next, trackSwitch.ActiveBus);
    }

    [TestMethod]
    public void Scheduled_commit_promotes_overlay_branch_with_rtmp_consumer()
    {
        using var current = new FinalPcmBus(capacityFrames: 8, channels: 1);
        using var next = new FinalPcmBus(capacityFrames: 8, channels: 1);
        Assert.IsTrue(current.TryPublish([0.1F], out _, out _));
        Assert.IsTrue(current.TryPublishOverlay([0.3F], out _, out _));
        Assert.IsTrue(next.TryPublish([0.9F], out _, out _));

        using var trackSwitch = new FinalPcmBusTrackSwitch(1, current);
        trackSwitch.SetRtmpConsumerAttached(true);
        Assert.IsTrue(trackSwitch.TryPrepareNext(2, next, out _));
        Assert.IsTrue(trackSwitch.TryCommitNextAtFrames(2, 0, out _));

        Span<float> local = stackalloc float[1];
        Span<float> rtmp = stackalloc float[1];
        Assert.IsTrue(trackSwitch.OutputSource.TryRead(local, out _, out _));
        Assert.IsTrue(trackSwitch.RtmpSource.TryRead(rtmp, out _, out _));

        Assert.IsTrue(
            trackSwitch.TryAcknowledgePromotion(2, out var retired, out var error),
            error?.Message ?? "RTMP 插话分支不得阻塞已完成的主轨周期切换。");
        Assert.AreSame(current, retired);
        Assert.AreSame(next, trackSwitch.ActiveBus);
    }
}
