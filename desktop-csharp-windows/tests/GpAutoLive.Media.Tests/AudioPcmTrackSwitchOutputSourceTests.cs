using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class AudioPcmTrackSwitchOutputSourceTests
{
    [TestMethod]
    public void Prepared_next_source_switches_only_after_active_source_is_drained()
    {
        var currentBuffer = new AudioPcmRingBuffer(8, 1);
        var nextBuffer = new AudioPcmRingBuffer(8, 1);
        Assert.IsTrue(currentBuffer.TryWrite([0.1F, 0.2F], out _, out _));
        Assert.IsTrue(nextBuffer.TryWrite([0.3F, 0.4F], out _, out _));
        currentBuffer.Close();
        nextBuffer.Close();

        var output = new AudioPcmTrackSwitchOutputSource(
            activeCandidateId: 1,
            activeSource: new AudioPcmRingBufferOutputSource(currentBuffer),
            channels: 1);

        Assert.IsTrue(output.TryPrepareNext(
            2,
            new AudioPcmRingBufferOutputSource(nextBuffer),
            out var prepareError),
            prepareError?.Message);
        Assert.IsTrue(output.TryCommitPrepared(2, out var commitError), commitError?.Message);

        Span<float> samples = stackalloc float[4];
        Assert.IsTrue(output.TryRead(samples, out var framesRead, out var readError), readError?.Message);

        Assert.AreEqual(4, framesRead);
        CollectionAssert.AreEqual(new[] { 0.1F, 0.2F, 0.3F, 0.4F }, samples.ToArray());
        Assert.AreEqual(2UL, output.ActiveCandidateId);
    }

    [TestMethod]
    public void Temporary_underrun_does_not_switch_before_active_source_closes()
    {
        var currentBuffer = new AudioPcmRingBuffer(8, 1);
        var nextBuffer = new AudioPcmRingBuffer(8, 1);
        Assert.IsTrue(nextBuffer.TryWrite([0.5F], out _, out _));
        nextBuffer.Close();

        var output = new AudioPcmTrackSwitchOutputSource(
            activeCandidateId: 3,
            activeSource: new AudioPcmRingBufferOutputSource(currentBuffer),
            channels: 1);
        Assert.IsTrue(output.TryPrepareNext(
            4,
            new AudioPcmRingBufferOutputSource(nextBuffer),
            out _));
        Assert.IsTrue(output.TryCommitPrepared(4, out _));

        Span<float> samples = stackalloc float[1];
        Assert.IsTrue(output.TryRead(samples, out var framesRead, out _));
        Assert.AreEqual(0, framesRead);
        Assert.AreEqual(3UL, output.ActiveCandidateId);

        currentBuffer.Close();
        Assert.IsTrue(output.TryRead(samples, out framesRead, out _));
        Assert.AreEqual(1, framesRead);
        Assert.AreEqual(0.5F, samples[0]);
        Assert.AreEqual(4UL, output.ActiveCandidateId);
    }

    [TestMethod]
    public void Prepared_source_does_not_switch_without_explicit_commit()
    {
        var currentBuffer = new AudioPcmRingBuffer(8, 1);
        var nextBuffer = new AudioPcmRingBuffer(8, 1);
        Assert.IsTrue(currentBuffer.TryWrite([0.1F], out _, out _));
        Assert.IsTrue(nextBuffer.TryWrite([0.2F], out _, out _));
        currentBuffer.Close();
        nextBuffer.Close();

        var output = new AudioPcmTrackSwitchOutputSource(
            activeCandidateId: 8,
            activeSource: new AudioPcmRingBufferOutputSource(currentBuffer),
            channels: 1);
        Assert.IsTrue(output.TryPrepareNext(
            9,
            new AudioPcmRingBufferOutputSource(nextBuffer),
            out _));

        Span<float> samples = stackalloc float[2];
        Assert.IsTrue(output.TryRead(samples, out var framesRead, out _));
        Assert.AreEqual(1, framesRead);
        Assert.AreEqual(0.1F, samples[0]);
        Assert.AreEqual(8UL, output.ActiveCandidateId);
    }

    [TestMethod]
    public void Track_switch_keeps_only_one_prepared_candidate()
    {
        var current = new AudioPcmRingBufferOutputSource(new AudioPcmRingBuffer(8, 1));
        var next = new AudioPcmRingBufferOutputSource(new AudioPcmRingBuffer(8, 1));
        var output = new AudioPcmTrackSwitchOutputSource(5, current, channels: 1);

        Assert.IsTrue(output.TryPrepareNext(6, next, out _));
        Assert.IsFalse(output.TryPrepareNext(7, next, out var error));
        Assert.AreEqual(AudioPcmTrackSwitchFailureCode.AlreadyPrepared, error?.Code);
        Assert.IsFalse(output.TryCommitPrepared(7, out error));
        Assert.AreEqual(AudioPcmTrackSwitchFailureCode.CandidateNotPrepared, error?.Code);
    }

    [TestMethod]
    public void Scheduled_commit_switches_without_waiting_for_current_source_eof()
    {
        var currentBuffer = new AudioPcmRingBuffer(8, 1);
        var nextBuffer = new AudioPcmRingBuffer(8, 1);
        Assert.IsTrue(currentBuffer.TryWrite([0.1F, 0.2F, 0.3F, 0.4F], out _, out _));
        Assert.IsTrue(nextBuffer.TryWrite([0.9F, 1.0F], out _, out _));
        nextBuffer.Close();

        var output = new AudioPcmTrackSwitchOutputSource(
            activeCandidateId: 10,
            activeSource: new AudioPcmRingBufferOutputSource(currentBuffer),
            channels: 1);
        Assert.IsTrue(output.TryPrepareNext(
            11,
            new AudioPcmRingBufferOutputSource(nextBuffer),
            out _));
        Assert.IsTrue(output.TryCommitPreparedAtFrames(11, 2, out _));

        Span<float> first = stackalloc float[2];
        Assert.IsTrue(output.TryRead(first, out var firstFrames, out _));
        Assert.AreEqual(2, firstFrames);
        Assert.AreEqual(10UL, output.ActiveCandidateId);

        Span<float> second = stackalloc float[2];
        Assert.IsTrue(output.TryRead(second, out var secondFrames, out _));
        Assert.AreEqual(2, secondFrames);
        Assert.AreEqual(11UL, output.ActiveCandidateId);
        CollectionAssert.AreEqual(new[] { 0.9F, 1.0F }, second.ToArray());
    }
}
