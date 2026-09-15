using System.Reflection;
using GpAutoLive.Media;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsRtmpTimelineTests
{
    [TestMethod]
    public async Task Concurrent_dispose_waits_for_the_same_owned_shutdown()
    {
        await VerifySharedDisposeAsync(new WindowsAudioPlaybackController());
        await using var manager = new WindowsRtmpOutputManager();
        await VerifySharedDisposeAsync(new WindowsRtmpAudioSession(manager));
    }

    [TestMethod]
    public async Task Interlude_cancel_is_inside_the_same_lock_as_owner_detachment()
    {
        await using var controller = new WindowsAudioPlaybackController();
        await using var manager = new WindowsRtmpOutputManager();
        await using var session = new WindowsRtmpAudioSession(manager);
        foreach (var owner in new object[] { controller, session })
        {
            var gate = GetField(owner, "_gate");
            using var cancellation = new CancellationTokenSource();
            using var registration = cancellation.Token.Register(() =>
                Assert.IsTrue(Monitor.IsEntered(gate), "摘除/释放 CTS 与 Cancel 必须共享所有权锁。"));
            SetField(owner, "_overlayCancellation", cancellation);
            SetField(owner, "_overlayTask", Task.CompletedTask);
            try
            {
                if (owner is WindowsAudioPlaybackController local)
                {
                    Assert.IsTrue((await local.StopInterludeAsync()).IsSuccess);
                }
                else
                {
                    Assert.IsTrue((await session.StopInterludeAsync()).IsSuccess);
                }
            }
            finally
            {
                SetField(owner, "_overlayCancellation", null);
                SetField(owner, "_overlayTask", null);
            }
        }
    }

    [TestMethod]
    public async Task Controller_capture_uses_unread_pcm_source_anchor_and_keeps_local_frames()
    {
        using var bus = new FinalPcmBus(capacityFrames: 16, channels: 1);
        using var tracks = new FinalPcmBusTrackSwitch(7, bus);
        await using var controller = new WindowsAudioPlaybackController();
        var plan = new FfmpegPcmDecodePlan("unused", [], "fixture.wav", 1_000, 1, TimeSpan.FromSeconds(1))
        {
            SourceStartMs = 30_000,
        };
        SetField(controller, "_activePlan", plan);
        SetField(controller, "_activePlanCandidateId", 7UL);
        SetField(controller, "_finalPcmBusTrackSwitch", tracks);
        SetField(controller, "_state", WindowsAudioPlaybackState.Playing);
        Assert.IsTrue(bus.TryPublish([1F, 2F, 3F, 4F, 5F], out _, out _));
        Assert.IsTrue(tracks.OutputSource.TryRead(new float[2], out _, out _));

        Assert.IsTrue(controller.TryAttachRtmpSource("fixture.wav", out var captured, out var error), error?.Message);
        Assert.IsNotNull(captured);
        Assert.AreEqual(30_002UL, captured.SourcePositionMs);
        var local = new float[3];
        Assert.IsTrue(tracks.OutputSource.TryRead(local, out _, out _));
        CollectionAssert.AreEqual(new[] { 3F, 4F, 5F }, local);
        var remote = new float[3];
        Assert.IsTrue(captured.OutputSource.TryRead(remote, out var frames, out _));
        Assert.AreEqual(3, frames);
        CollectionAssert.AreEqual(local, remote);
    }

    [TestMethod]
    public async Task Controller_does_not_combine_retired_plan_with_promoted_bus()
    {
        using var bus = new FinalPcmBus(capacityFrames: 16, channels: 1);
        using var tracks = new FinalPcmBusTrackSwitch(8, bus);
        await using var controller = new WindowsAudioPlaybackController();
        SetField(controller, "_activePlan", new FfmpegPcmDecodePlan("unused", [], "fixture.wav", 48_000, 1, TimeSpan.FromSeconds(1)));
        SetField(controller, "_activePlanCandidateId", 7UL);
        SetField(controller, "_finalPcmBusTrackSwitch", tracks);
        SetField(controller, "_state", WindowsAudioPlaybackState.Playing);

        Assert.IsFalse(controller.TryAttachRtmpSource("fixture.wav", out _, out var error));
        Assert.AreEqual("rtmp_candidate_transition", error?.Code);
        Assert.IsFalse(tracks.RtmpConsumerAttached);
    }

    [TestMethod]
    public async Task Non_unit_rate_is_rejected_before_attaching_a_shared_av_consumer()
    {
        using var bus = new FinalPcmBus(capacityFrames: 16, channels: 1);
        using var tracks = new FinalPcmBusTrackSwitch(1, bus);
        await using var controller = new WindowsAudioPlaybackController();
        SetField(controller, "_activePlan", new FfmpegPcmDecodePlan("unused", [], "fixture.wav", 48_000, 1, TimeSpan.FromSeconds(1))
        {
            PlaybackRate = 1.2,
        });
        SetField(controller, "_activePlanCandidateId", 1UL);
        SetField(controller, "_finalPcmBusTrackSwitch", tracks);
        SetField(controller, "_state", WindowsAudioPlaybackState.Playing);

        Assert.IsFalse(controller.TryAttachRtmpSource("fixture.wav", out _, out var error));
        Assert.AreEqual("rtmp_speed_unsupported", error?.Code);
        Assert.IsFalse(tracks.RtmpConsumerAttached);
    }

    private static async Task VerifySharedDisposeAsync(IAsyncDisposable owner)
    {
        var lifecycle = (SemaphoreSlim)GetField(owner, "_lifecycle");
        Assert.IsTrue(lifecycle.Wait(0));
        Task first;
        Task second;
        try
        {
            first = owner.DisposeAsync().AsTask();
            second = owner.DisposeAsync().AsTask();
            Assert.AreSame(first, second);
            Assert.IsFalse(second.IsCompleted, "第二个调用不能把未结束的关闭伪装为完成。");
        }
        finally
        {
            lifecycle.Release();
        }
        await Task.WhenAll(first, second);
    }

    private static object GetField(object target, string name)
    {
        var field = target.GetType().GetField(name, BindingFlags.Instance | BindingFlags.NonPublic);
        Assert.IsNotNull(field);
        var value = field.GetValue(target);
        Assert.IsNotNull(value);
        return value;
    }

    private static void SetField(object target, string name, object? value)
    {
        var field = target.GetType().GetField(name, BindingFlags.Instance | BindingFlags.NonPublic);
        Assert.IsNotNull(field);
        field.SetValue(target, value);
    }
}
