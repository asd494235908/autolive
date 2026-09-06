namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class AudioPcmMixerTests
{
    [TestMethod]
    public void Output_volume_percent_maps_to_linear_gain_decibels()
    {
        Assert.IsTrue(AudioPcmMixer.TryGetOutputVolumeGainDb(100, out var fullGainDb));
        Assert.AreEqual(0, fullGainDb, 0.0001);

        Assert.IsTrue(AudioPcmMixer.TryGetOutputVolumeGainDb(50, out var halfGainDb));
        Assert.AreEqual(-6.0206, halfGainDb, 0.001);

        Assert.IsTrue(AudioPcmMixer.TryGetOutputVolumeGainDb(0, out var mutedGainDb));
        Assert.AreEqual(-120, mutedGainDb, 0.0001);
    }

    [TestMethod]
    public void Output_volume_percent_rejects_non_finite_or_out_of_range_values()
    {
        Assert.IsFalse(AudioPcmMixer.TryGetOutputVolumeGainDb(double.NaN, out _));
        Assert.IsFalse(AudioPcmMixer.TryGetOutputVolumeGainDb(-1, out _));
        Assert.IsFalse(AudioPcmMixer.TryGetOutputVolumeGainDb(101, out _));
        Assert.IsFalse(AudioPcmMixer.TryGetOutputVolumeGainDb(double.PositiveInfinity, out _));
    }

    [TestMethod]
    public void Output_volume_gain_is_consumed_by_the_base_pcm_mix_policy()
    {
        Assert.IsTrue(AudioPcmMixer.TryGetOutputVolumeGainDb(50, out var gainDb));
        var output = new float[1];

        Assert.IsTrue(AudioPcmMixer.TryMix(
            new[] { 1F },
            ReadOnlySpan<float>.Empty,
            output,
            channels: 1,
            new AudioPcmMixPolicy(BaseGainDb: gainDb),
            out var frames,
            out var error), error?.Message);

        Assert.AreEqual(1, frames);
        Assert.AreEqual(0.5F, output[0], 0.001F);
    }

    [TestMethod]
    public void Interlude_ducks_base_and_keeps_overlay()
    {
        var basePcm = new[] { 1F, -1F };
        var overlay = new[] { 0.5F, -0.5F };
        var output = new float[2];

        var ok = AudioPcmMixer.TryMix(
            basePcm,
            overlay,
            output,
            channels: 1,
            new AudioPcmMixPolicy(BaseDuckingDb: -6),
            out var frames,
            out var error);

        Assert.IsTrue(ok, error?.Message);
        Assert.AreEqual(2, frames);
        Assert.IsTrue(output[0] > 0.99F && output[0] <= 1F);
        Assert.IsTrue(output[1] < -0.99F && output[1] >= -1F);
    }

    [TestMethod]
    public void Fixed_speech_policy_mutes_base_without_allocating_a_second_source()
    {
        var output = new float[2];

        var ok = AudioPcmMixer.TryMix(
            new[] { 0.75F, 0.75F },
            new[] { 0.25F, 0.25F },
            output,
            channels: 1,
            new AudioPcmMixPolicy(MuteBase: true),
            out _,
            out var error);

        Assert.IsTrue(ok, error?.Message);
        CollectionAssert.AreEqual(new[] { 0.25F, 0.25F }, output);
    }

    [TestMethod]
    public void Non_finite_input_is_silenced_and_output_is_clamped()
    {
        var output = new float[3];

        var ok = AudioPcmMixer.TryMix(
            new[] { float.NaN, 4F, -4F },
            ReadOnlySpan<float>.Empty,
            output,
            channels: 1,
            default,
            out _,
            out var error);

        Assert.IsTrue(ok, error?.Message);
        CollectionAssert.AreEqual(new[] { 0F, 1F, -1F }, output);
    }

    [TestMethod]
    public void Invalid_shape_gain_and_destination_fail_closed()
    {
        var output = new float[4];
        Assert.IsFalse(AudioPcmMixer.TryMix(new[] { 1F, 2F }, ReadOnlySpan<float>.Empty, output, 3, default, out _, out var shape));
        Assert.AreEqual(AudioPcmMixFailureCode.InvalidFrameShape, shape?.Code);

        Assert.IsFalse(AudioPcmMixer.TryMix(new[] { 1F }, ReadOnlySpan<float>.Empty, output, 1, new AudioPcmMixPolicy(BaseGainDb: 25), out _, out var gain));
        Assert.AreEqual(AudioPcmMixFailureCode.InvalidGain, gain?.Code);

        Assert.IsFalse(AudioPcmMixer.TryMix(new[] { 1F, 2F }, ReadOnlySpan<float>.Empty, new float[1], 1, default, out _, out var capacity));
        Assert.AreEqual(AudioPcmMixFailureCode.DestinationTooSmall, capacity?.Code);
    }

    [TestMethod]
    public void Base_policy_applies_duck_and_mute_in_place()
    {
        var samples = new[] { 1F, -1F, 0.5F, -0.5F };

        var ducked = AudioPcmMixer.TryApplyBasePolicy(
            samples.AsSpan(0, 2),
            channels: 2,
            new AudioPcmMixPolicy(BaseDuckingDb: -6),
            out var duckError);
        Assert.IsTrue(ducked, duckError?.Message);
        Assert.IsTrue(samples[0] is > 0.49F and < 0.51F);
        Assert.IsTrue(samples[1] is < -0.49F and > -0.51F);

        var muted = AudioPcmMixer.TryApplyBasePolicy(
            samples.AsSpan(2),
            channels: 2,
            new AudioPcmMixPolicy(MuteBase: true),
            out var muteError);
        Assert.IsTrue(muted, muteError?.Message);
        CollectionAssert.AreEqual(new[] { 0F, 0F }, samples[2..]);
    }

    [TestMethod]
    public void Base_policy_rejects_unaligned_or_invalid_gain()
    {
        Assert.IsFalse(AudioPcmMixer.TryApplyBasePolicy(
            new float[3],
            channels: 2,
            default,
            out var shape));
        Assert.AreEqual(AudioPcmMixFailureCode.InvalidFrameShape, shape?.Code);

        Assert.IsFalse(AudioPcmMixer.TryApplyBasePolicy(
            new float[2],
            channels: 1,
            new AudioPcmMixPolicy(BaseDuckingDb: -121),
            out var gain));
        Assert.AreEqual(AudioPcmMixFailureCode.InvalidGain, gain?.Code);
    }

    [TestMethod]
    public void Final_bus_can_publish_preallocated_mixed_pcm_to_both_consumers()
    {
        using var bus = new FinalPcmBus(capacityFrames: 8, channels: 1);
        bus.SetRtmpConsumerAttached(true);
        var scratch = new float[4];

        var ok = bus.TryPublishMixed(
            new[] { 0.5F, 0.5F },
            new[] { 0.25F, 0.25F },
            scratch,
            new AudioPcmMixPolicy(BaseDuckingDb: -6),
            out var frames,
            out var error);

        Assert.IsTrue(ok, error?.Message);
        Assert.AreEqual(2, frames);
        var output = new float[2];
        var rtmp = new float[2];
        Assert.IsTrue(bus.OutputBuffer.TryRead(output, out var outputFrames, out _));
        Assert.IsTrue(bus.RtmpBuffer.TryRead(rtmp, out var rtmpFrames, out _));
        Assert.AreEqual(2, outputFrames);
        Assert.AreEqual(2, rtmpFrames);
        CollectionAssert.AreEqual(output, rtmp);
    }

    [TestMethod]
    public void Mixing_output_source_consumes_base_and_overlay_in_one_fixed_buffer()
    {
        var baseBuffer = new AudioPcmRingBuffer(capacityFrames: 8, channels: 2);
        var overlayBuffer = new AudioPcmRingBuffer(capacityFrames: 8, channels: 2);
        Assert.IsTrue(baseBuffer.TryWrite([0.5F, 0.5F, 0.5F, 0.5F], out _, out _));
        Assert.IsTrue(overlayBuffer.TryWrite([0.25F, -0.25F, 0.25F, -0.25F], out _, out _));
        var source = new AudioPcmMixingOutputSource(
            baseBuffer,
            overlayBuffer,
            channels: 2,
            maxFramesPerRead: 4,
            policyProvider: static () => new AudioPcmMixPolicy(BaseDuckingDb: -6));
        var output = new float[4];

        var ok = source.TryRead(output, out var frames, out var error);

        Assert.IsTrue(ok, error?.Message);
        Assert.AreEqual(2, frames);
        Assert.IsTrue(output[0] > 0.49F && output[0] < 0.51F);
        Assert.IsTrue(Math.Abs(output[1]) < 0.01F);
        Assert.IsTrue(output[2] > 0.49F && output[2] < 0.51F);
        Assert.IsTrue(Math.Abs(output[3]) < 0.01F);
    }

    [TestMethod]
    public void Mixing_output_source_applies_overlay_volume_without_changing_base_duck()
    {
        var baseBuffer = new AudioPcmRingBuffer(capacityFrames: 4, channels: 1);
        var overlayBuffer = new AudioPcmRingBuffer(capacityFrames: 4, channels: 1);
        Assert.IsTrue(baseBuffer.TryWrite([1F], out _, out _));
        Assert.IsTrue(overlayBuffer.TryWrite([1F], out _, out _));
        var source = new AudioPcmMixingOutputSource(
            baseBuffer,
            overlayBuffer,
            channels: 1,
            maxFramesPerRead: 1,
            policyProvider: static () => new AudioPcmMixPolicy(
                BaseDuckingDb: -6,
                OverlayGainDb: -12));
        var output = new float[1];

        Assert.IsTrue(source.TryRead(output, out var frames, out var error), error?.Message);

        Assert.AreEqual(1, frames);
        Assert.AreEqual(0.752F, output[0], 0.002F);
    }

    [TestMethod]
    public void Mixing_output_source_does_not_require_overlay_frames()
    {
        var baseBuffer = new AudioPcmRingBuffer(capacityFrames: 4, channels: 1);
        var overlayBuffer = new AudioPcmRingBuffer(capacityFrames: 4, channels: 1);
        Assert.IsTrue(baseBuffer.TryWrite([0.75F, -0.75F], out _, out _));
        var source = new AudioPcmMixingOutputSource(baseBuffer, overlayBuffer, channels: 1);
        var output = new float[2];

        var ok = source.TryRead(output, out var frames, out var error);

        Assert.IsTrue(ok, error?.Message);
        Assert.AreEqual(2, frames);
        CollectionAssert.AreEqual(new[] { 0.75F, -0.75F }, output);
    }

    [TestMethod]
    public void Mixing_output_source_can_drain_overlay_when_base_is_empty()
    {
        var baseBuffer = new AudioPcmRingBuffer(capacityFrames: 4, channels: 1);
        var overlayBuffer = new AudioPcmRingBuffer(capacityFrames: 4, channels: 1);
        Assert.IsTrue(overlayBuffer.TryWrite([0.25F, -0.25F], out _, out _));
        var source = new AudioPcmMixingOutputSource(baseBuffer, overlayBuffer, channels: 1);
        var output = new float[2];

        var ok = source.TryRead(output, out var frames, out var error);

        Assert.IsTrue(ok, error?.Message);
        Assert.AreEqual(2, frames);
        CollectionAssert.AreEqual(new[] { 0.25F, -0.25F }, output);
    }

    [TestMethod]
    public void Final_bus_keeps_overlay_consumers_separate_until_output_read()
    {
        using var bus = new FinalPcmBus(capacityFrames: 8, channels: 1);
        bus.SetRtmpConsumerAttached(true);
        Assert.IsTrue(bus.TryPublish([0.5F, 0.5F], out var baseFrames, out var baseError), baseError?.Message);
        Assert.IsTrue(bus.TryPublishOverlay([0.25F, 0.25F], out var overlayFrames, out var overlayError), overlayError?.Message);
        Assert.AreEqual(2, baseFrames);
        Assert.AreEqual(2, overlayFrames);

        var source = new AudioPcmMixingOutputSource(bus.OutputBuffer, bus.OutputOverlayBuffer, channels: 1);
        var output = new float[2];
        Assert.IsTrue(source.TryRead(output, out var frames, out var error), error?.Message);

        Assert.AreEqual(2, frames);
        CollectionAssert.AreEqual(new[] { 0.75F, 0.75F }, output);
        var rtmpBase = new float[2];
        var rtmpOverlay = new float[2];
        Assert.IsTrue(bus.RtmpBuffer.TryRead(rtmpBase, out var rtmpBaseFrames, out _));
        Assert.IsTrue(bus.RtmpOverlayBuffer.TryRead(rtmpOverlay, out var rtmpOverlayFrames, out _));
        Assert.AreEqual(2, rtmpBaseFrames);
        Assert.AreEqual(2, rtmpOverlayFrames);
    }

    [TestMethod]
    public void Mixing_output_source_applies_bounded_attack_without_allocating_a_second_stream()
    {
        var baseBuffer = new AudioPcmRingBuffer(capacityFrames: 8, channels: 1);
        var overlayBuffer = new AudioPcmRingBuffer(capacityFrames: 8, channels: 1);
        Assert.IsTrue(overlayBuffer.TryWrite([1F, 1F, 1F], out _, out _));
        var source = new AudioPcmMixingOutputSource(
            baseBuffer,
            overlayBuffer,
            channels: 1,
            envelopeOptions: new AudioPcmMixEnvelopeOptions(SampleRateHz: 1_000, AttackMs: 100, ReleaseMs: 100));
        var output = new float[3];

        Assert.IsTrue(source.TryRead(output, out var frames, out var error), error?.Message);
        Assert.AreEqual(3, frames);
        Assert.AreEqual(0F, output[0]);
        Assert.IsTrue(output[1] > output[0] && output[1] < output[2]);
        Assert.IsTrue(output[2] is > 0F and < 0.1F);
    }
}
