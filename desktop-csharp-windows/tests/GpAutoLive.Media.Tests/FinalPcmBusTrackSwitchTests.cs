using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class FinalPcmBusTrackSwitchTests
{
    [TestMethod]
    public void Overlay_follows_each_consumers_base_while_microphone_targets_local_promoted_bus()
    {
        using var current = new FinalPcmBus(capacityFrames: 16, channels: 1);
        using var next = new FinalPcmBus(capacityFrames: 16, channels: 1);
        using var tracks = new FinalPcmBusTrackSwitch(1, current);
        tracks.SetRtmpConsumerAttached(true);
        Assert.IsTrue(current.TryPublish([0F, 0F, 0F, 0F], out _, out _));
        Assert.IsTrue(current.TryPublishOverlay([0.1F, 0.2F], out _, out _));
        Assert.IsTrue(tracks.TryPrepareNext(2, next, out _));
        Assert.IsTrue(next.TryPublish([0F, 0F, 0F], out _, out _));
        Assert.IsTrue(tracks.TryCommitNextAtFrames(2, 2, out _));
        Assert.IsTrue(tracks.OutputSource.TryRead(new float[3], out _, out _));
        Assert.IsFalse(tracks.TryAcknowledgePromotion(2, out _, out _));
        Assert.AreSame(next, tracks.ActiveBus, "麦克风生产者应立即跟随本机已播放的新候选。");
        Assert.IsTrue(tracks.ActiveBus.TryPublishOverlay([0.75F], out _, out _));
        var localVoice = new float[1];
        Assert.IsTrue(tracks.OutputOverlaySource.TryRead(localVoice, out _, out _));
        Assert.AreEqual(0.75F, localVoice[0]);

        Assert.IsTrue(tracks.RtmpSource.TryRead(new float[1], out _, out _));
        var oldRemoteVoice = new float[1];
        Assert.IsTrue(tracks.RtmpOverlaySource.TryRead(oldRemoteVoice, out _, out _));
        Assert.AreEqual(0.1F, oldRemoteVoice[0], "远端旧基础轨不能提前混入新候选插话。");
        Assert.IsTrue(tracks.RtmpOverlaySource.TryRead(new float[1], out _, out _));
        current.Close();
        Assert.IsTrue(tracks.RtmpOverlaySource.TryRead(new float[1], out var closedOverlayFrames, out _));
        Assert.AreEqual(0, closedOverlayFrames, "旧插话关闭排空也不能越过仍有尾帧的旧基础轨。");
        Assert.IsTrue(tracks.RtmpSource.TryRead(new float[2], out _, out _));
        var newRemoteVoice = new float[1];
        Assert.IsTrue(tracks.RtmpOverlaySource.TryRead(newRemoteVoice, out _, out _));
        Assert.AreEqual(0.75F, newRemoteVoice[0]);
    }

    [TestMethod]
    public void Mixed_read_crossing_boundary_pairs_each_base_segment_with_its_own_overlay()
    {
        using var current = new FinalPcmBus(capacityFrames: 16, channels: 1);
        using var next = new FinalPcmBus(capacityFrames: 16, channels: 1);
        using var tracks = new FinalPcmBusTrackSwitch(1, current);
        Assert.IsTrue(current.TryPublish([0.1F, 0.2F, 0.3F], out _, out _));
        Assert.IsTrue(current.TryPublishOverlay([0.01F, 0.02F, 0.03F], out _, out _));
        Assert.IsTrue(tracks.TryPrepareNext(2, next, out _));
        Assert.IsTrue(next.TryPublish([0.6F, 0.7F], out _, out _));
        Assert.IsTrue(next.TryPublishOverlay([0.1F, 0.2F], out _, out _));
        Assert.IsTrue(tracks.TryCommitNextAtFrames(2, 2, out _));
        var mix = new AudioPcmMixingOutputSource(tracks.OutputSource, tracks.OutputOverlaySource, 1);
        var output = new float[4];
        Assert.IsTrue(mix.TryRead(output, out var frames, out _));
        Assert.AreEqual(4, frames);
        var expected = new[] { 0.11F, 0.22F, 0.7F, 0.9F };
        for (var index = 0; index < output.Length; index++)
        {
            Assert.AreEqual(expected[index], output[index], 0.00001F);
        }
    }

    [TestMethod]
    public void Rtmp_overlay_cannot_advance_without_base_but_local_overlay_remains_independent()
    {
        using var bus = new FinalPcmBus(capacityFrames: 8, channels: 1);
        Assert.IsTrue(bus.TryPublishOverlay([0.5F, 0.6F], out _, out _));
        bus.AttachRtmpFromPendingOutput();
        var remote = new AudioPcmMixingOutputSource(
            new AudioPcmRingBufferOutputSource(bus.RtmpBuffer),
            new AudioPcmRingBufferOutputSource(bus.RtmpOverlayBuffer), 1, overlayFollowsBase: true);
        var local = new AudioPcmMixingOutputSource(bus.OutputBuffer, bus.OutputOverlayBuffer, 1);
        Assert.IsTrue(remote.TryRead(new float[2], out var remoteFrames, out _));
        Assert.AreEqual(0, remoteFrames);
        Assert.AreEqual(2, bus.RtmpOverlayBuffer.Snapshot.AvailableFrames);
        Assert.IsTrue(local.TryRead(new float[2], out var localFrames, out _));
        Assert.AreEqual(2, localFrames);
    }

    [TestMethod]
    public void Midplay_attach_preserves_unread_current_tail_and_preloaded_next_head()
    {
        using var current = new FinalPcmBus(capacityFrames: 16, channels: 1);
        using var next = new FinalPcmBus(capacityFrames: 16, channels: 1);
        using var trackSwitch = new FinalPcmBusTrackSwitch(1, current);
        Assert.IsTrue(current.TryPublish([1F, 2F, 3F, 4F, 5F, 6F], out _, out _));
        Assert.IsTrue(trackSwitch.OutputSource.TryRead(new float[2], out _, out _));
        Assert.IsTrue(next.TryPublish([10F, 11F, 12F], out _, out _));
        Assert.IsTrue(trackSwitch.TryPrepareNext(2, next, out _));

        Assert.IsTrue(trackSwitch.TryAttachRtmpFromPendingOutput(out var firstFrame));
        Assert.AreEqual(2UL, firstFrame);
        Assert.IsTrue(trackSwitch.RtmpSource.TryRead(new float[4], out var beforeLocalFrames, out _));
        Assert.AreEqual(0, beforeLocalFrames, "远端不能抢跑到尚未提交的本机周期边界之后。");
        Assert.IsTrue(trackSwitch.TryCommitNextAtFrames(2, 2, out _));
        var local = new float[4];
        var remote = new float[4];
        Assert.IsTrue(trackSwitch.OutputSource.TryRead(local, out var localFrames, out _));
        Assert.IsTrue(trackSwitch.RtmpSource.TryRead(remote, out var remoteFrames, out _));
        Assert.AreEqual(4, localFrames);
        Assert.AreEqual(4, remoteFrames);
        CollectionAssert.AreEqual(new[] { 3F, 4F, 10F, 11F }, local);
        CollectionAssert.AreEqual(local, remote);
    }

    [TestMethod]
    public void Attach_during_committed_transition_is_rejected_without_enabling_consumer()
    {
        using var current = new FinalPcmBus(capacityFrames: 8, channels: 1);
        using var next = new FinalPcmBus(capacityFrames: 8, channels: 1);
        using var trackSwitch = new FinalPcmBusTrackSwitch(1, current);
        Assert.IsTrue(trackSwitch.TryPrepareNext(2, next, out _));
        Assert.IsTrue(trackSwitch.TryCommitNextAtFrames(2, 2, out _));
        Assert.IsFalse(trackSwitch.TryAttachRtmpFromPendingOutput(out _));
        Assert.IsFalse(trackSwitch.RtmpConsumerAttached);
    }

    [TestMethod]
    public void Copied_startup_pcm_rejects_overflow_instead_of_publishing_wrong_timeline()
    {
        using var bus = new FinalPcmBus(capacityFrames: 4, channels: 1);
        Assert.IsTrue(bus.TryPublish([1F, 2F, 3F, 4F], out _, out _));
        Assert.AreEqual(0UL, bus.AttachRtmpFromPendingOutput());
        Assert.AreEqual(8, bus.RtmpBuffer.Snapshot.CapacityFrames);
        Assert.IsTrue(bus.TryPublish([5F, 6F, 7F, 8F], out _, out _));
        Assert.IsTrue(bus.TryPublish([9F], out _, out _));
        Assert.IsFalse(bus.RtmpBuffer.TryRead(new float[2], out var frames, out var error));
        Assert.AreEqual(0, frames);
        Assert.AreEqual(PcmRingBufferFailureCode.Discontinuity, error?.Code);
    }

    [TestMethod]
    public void Scheduled_boundary_is_exact_inside_one_read_and_equal_for_lagging_consumers()
    {
        using var current = new FinalPcmBus(capacityFrames: 16, channels: 1);
        using var next = new FinalPcmBus(capacityFrames: 16, channels: 1);
        using var trackSwitch = new FinalPcmBusTrackSwitch(1, current);
        trackSwitch.SetRtmpConsumerAttached(true);
        Assert.IsTrue(current.TryPublish([1F, 2F, 3F, 4F, 5F, 6F], out _, out _));
        Assert.IsTrue(trackSwitch.TryPrepareNext(2, next, out _));
        Assert.IsTrue(next.TryPublish([10F, 11F, 12F, 13F], out _, out _));
        var localPrefix = new float[2];
        Assert.IsTrue(trackSwitch.OutputSource.TryRead(localPrefix, out _, out _));
        Assert.IsTrue(trackSwitch.TryCommitNextAtFrames(2, 2, out _));

        var local = new float[4];
        var remote = new float[6];
        Assert.IsTrue(trackSwitch.OutputSource.TryRead(local, out var localFrames, out _));
        Assert.IsTrue(trackSwitch.RtmpSource.TryRead(remote, out var remoteFrames, out _));
        Assert.AreEqual(4, localFrames);
        Assert.AreEqual(6, remoteFrames);
        CollectionAssert.AreEqual(new[] { 3F, 4F, 10F, 11F }, local);
        CollectionAssert.AreEqual(new[] { 1F, 2F, 3F, 4F, 10F, 11F }, remote);
    }

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
