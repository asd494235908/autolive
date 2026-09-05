using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsMpvPlaybackControllerTests
{
    [TestMethod]
    public async Task Start_requires_verified_runtime()
    {
        await using var controller = new WindowsMpvPlaybackController();

        var result = await controller.StartAsync(
            null,
            CreateSource(MediaKind.Video),
            new MediaPlaybackIdentity(1, 1, 0, 0),
            1);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsMpvPlaybackControllerFailureCode.RuntimeUnavailable, result.Error?.Code);
        Assert.AreEqual(WindowsMpvPlaybackControllerState.Ready, result.Snapshot.State);
    }

    [TestMethod]
    public async Task Start_rejects_pure_audio_before_creating_process()
    {
        await using var controller = new WindowsMpvPlaybackController();

        var result = await controller.StartAsync(
            null,
            CreateSource(MediaKind.Audio),
            new MediaPlaybackIdentity(1, 1, 0, 0),
            1);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsMpvPlaybackControllerFailureCode.SourceNotVideo, result.Error?.Code);
        Assert.AreEqual(WindowsMpvPlaybackControllerState.Ready, result.Snapshot.State);
    }

    [TestMethod]
    public async Task Start_rejects_missing_identity_and_invalid_host()
    {
        await using var controller = new WindowsMpvPlaybackController();

        var noIdentity = await controller.StartAsync(
            null,
            CreateSource(MediaKind.Video),
            null,
            1);
        Assert.AreEqual(WindowsMpvPlaybackControllerFailureCode.InvalidInput, noIdentity.Error?.Code);

        var invalidHost = await controller.StartAsync(
            null,
            CreateSource(MediaKind.Video),
            new MediaPlaybackIdentity(1, 1, 0, 0),
            0);
        Assert.AreEqual(WindowsMpvPlaybackControllerFailureCode.InvalidPlan, invalidHost.Error?.Code);
    }

    [TestMethod]
    public async Task Closed_controller_rejects_new_operations()
    {
        await using var controller = new WindowsMpvPlaybackController();
        await controller.DisposeAsync();

        var result = await controller.StartAsync(null, null, null, 1);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsMpvPlaybackControllerFailureCode.Closed, result.Error?.Code);
        Assert.AreEqual(WindowsMpvPlaybackControllerState.Closed, controller.Snapshot.State);
    }

    [TestMethod]
    public async Task Watch_requires_active_runtime_and_returns_bounded_error()
    {
        await using var controller = new WindowsMpvPlaybackController();
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);

        await foreach (var result in controller.WatchPlaybackStateAsync(identity))
        {
            Assert.IsFalse(result.IsSuccess);
            Assert.AreEqual(
                MpvPlaybackStateMonitorFailureCode.RuntimeUnavailable,
                result.Error?.Code);
            break;
        }
    }

    [TestMethod]
    public async Task UpdateEffects_requires_active_runtime()
    {
        await using var controller = new WindowsMpvPlaybackController();
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);

        var result = await controller.UpdateEffectsAsync(identity, MpvVideoEffectSnapshot.Default);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsMpvPlaybackControllerFailureCode.NotRunning, result.Error?.Code);
    }

    private static SourceMediaDto CreateSource(MediaKind kind) =>
        new("C:\\media\\sample.mp4", "C:\\media\\sample.mp4", kind, MediaCompatibilityMode.Direct, "sample.mp4", 1, 1_000, null, null, 1280, 720, 30, null, null, "h264", null, null, "not_calculated");
}
