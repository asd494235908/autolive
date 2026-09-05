using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class VirtualCameraOutputManagerTests
{
    [TestMethod]
    public void Lifecycle_requires_install_start_and_gpu_gate_before_ready()
    {
        var manager = new VirtualCameraOutputManager();

        Assert.AreEqual(VirtualCameraState.Unavailable, manager.Snapshot.State);
        Assert.IsFalse(manager.BeginStart().IsSuccess);
        Assert.IsTrue(manager.MarkInstalled().IsSuccess);
        Assert.IsTrue(manager.BeginStart().IsSuccess);
        Assert.IsFalse(manager.MarkReady(WarpFacts()).IsSuccess);
        Assert.AreEqual(VirtualCameraState.Failed, manager.Snapshot.State);
        Assert.IsTrue(manager.MarkInstalled().IsSuccess);
        Assert.IsTrue(manager.BeginStart().IsSuccess);
        Assert.IsTrue(manager.MarkReady(ValidFacts()).IsSuccess);
        Assert.AreEqual(VirtualCameraState.Ready, manager.Snapshot.State);
    }

    [TestMethod]
    public void Downstream_count_controls_ready_and_streaming_without_guessing()
    {
        var manager = ReadyManager();

        Assert.IsNull(manager.Snapshot.DownstreamClientCount);
        Assert.IsTrue(manager.SetDownstreamClientCount(2).IsSuccess);
        Assert.AreEqual(VirtualCameraState.Streaming, manager.Snapshot.State);
        Assert.IsTrue(manager.SetDownstreamClientCount(0).IsSuccess);
        Assert.AreEqual(VirtualCameraState.Ready, manager.Snapshot.State);
    }

    [TestMethod]
    public void Latest_wins_rejects_stale_generation_and_counts_drop()
    {
        var manager = ReadyManager();
        var generation = manager.Snapshot.Generation;
        var first = Frame(generation, 1, 1);
        var second = Frame(generation, 2, 2);

        Assert.IsTrue(manager.SubmitFrame(first).IsSuccess);
        Assert.IsTrue(manager.SubmitFrame(second).IsSuccess);
        Assert.AreEqual(1UL, manager.Snapshot.Metrics.FramesDropped);
        Assert.AreSame(second, manager.TakeLatestFrame());

        var stale = manager.SubmitFrame(Frame(generation - 1, 3, 3));
        Assert.IsFalse(stale.IsSuccess);
        Assert.AreEqual("virtual_camera_stale_generation", stale.Error!.Code);
        Assert.AreEqual(1UL, manager.Snapshot.Metrics.StaleFramesRejected);
    }

    [TestMethod]
    public void Output_policy_is_black_when_paused_locked_or_without_valid_frame()
    {
        var manager = ReadyManager();
        var live = new VirtualCameraOutputContext(true, true, false, false, false, true);
        Assert.AreEqual(VirtualCameraOutputPolicy.LatestFrame, manager.GetOutputPolicy(live));
        Assert.AreEqual(VirtualCameraOutputPolicy.Black, manager.GetOutputPolicy(live with { Paused = true }));
        Assert.AreEqual(VirtualCameraOutputPolicy.Black, manager.GetOutputPolicy(live with { Locked = true }));
        Assert.AreEqual(VirtualCameraOutputPolicy.Black, manager.GetOutputPolicy(live with { HasValidFrame = false }));
    }

    [TestMethod]
    public void Stop_invalidates_generation_and_discards_pending_frame()
    {
        var manager = ReadyManager();
        var before = manager.Snapshot.Generation;
        Assert.IsTrue(manager.SubmitFrame(Frame(before, 4, 4)).IsSuccess);
        Assert.IsTrue(manager.Stop().IsSuccess);

        var snapshot = manager.Snapshot;
        Assert.AreEqual(VirtualCameraState.Installed, snapshot.State);
        Assert.AreNotEqual(before, snapshot.Generation);
        Assert.IsNull(manager.TakeLatestFrame());
        Assert.AreEqual(1UL, snapshot.Metrics.FramesDropped);
    }

    [TestMethod]
    public void Readback_percentiles_are_bounded_and_observable()
    {
        var manager = new VirtualCameraOutputManager();
        for (var index = 1; index <= VirtualCameraRules.ReadbackSampleCapacity + 3; index++)
        {
            manager.RecordReadback(TimeSpan.FromMicroseconds(index));
        }

        var metrics = manager.Snapshot.Metrics;
        Assert.AreEqual((ulong)VirtualCameraRules.ReadbackSampleCapacity + 3, metrics.ReadbackCount);
        Assert.IsTrue(metrics.ReadbackAverageUs is > 3);
        Assert.IsTrue(metrics.ReadbackP50Us is >= 3);
        Assert.IsTrue(metrics.ReadbackP50Us.HasValue);
        Assert.IsTrue(metrics.ReadbackP95Us.HasValue);
        Assert.IsTrue(metrics.ReadbackP99Us.HasValue);
        Assert.IsTrue(metrics.ReadbackP95Us!.Value >= metrics.ReadbackP50Us!.Value);
        Assert.IsTrue(metrics.ReadbackP99Us!.Value >= metrics.ReadbackP95Us!.Value);
    }

    private static VirtualCameraOutputManager ReadyManager()
    {
        var manager = new VirtualCameraOutputManager();
        Assert.IsTrue(manager.MarkInstalled().IsSuccess);
        Assert.IsTrue(manager.BeginStart().IsSuccess);
        Assert.IsTrue(manager.MarkReady(ValidFacts()).IsSuccess);
        return manager;
    }

    private static VirtualCameraFrame Frame(ulong generation, ulong sequence, ulong timestamp) =>
        new(generation, sequence, timestamp, new byte[1280 * 720 * 2]);

    private static GpuCaptureFacts ValidFacts() => new(
        VirtualCameraRules.CaptureApi,
        "test-adapter-luid",
        "Test GPU",
        0x1002,
        0x744c,
        "11_0",
        false,
        true,
        true,
        VirtualCameraRules.Transport,
        false,
        VirtualCameraRules.Width,
        VirtualCameraRules.Height,
        VirtualCameraRules.Fps);

    private static GpuCaptureFacts WarpFacts() => ValidFacts() with { IsWarp = true };
}
