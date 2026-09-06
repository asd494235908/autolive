using GpAutoLive.Core;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class InterludePriorityReleaseTests
{
    [TestMethod]
    public async Task Decoder_completion_releases_main_audio_duck_without_waiting_for_ui_projection()
    {
        var priority = new AudioPriorityCoordinator();
        Assert.IsTrue(priority.BeginInterludeFile().IsAccepted);
        var completion = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);

        var observed = MainWindow.ObserveInterludePriorityCompletionAsync(
            completion.Task,
            priority);
        Assert.IsTrue(priority.Snapshot.MediaDucked);

        completion.SetResult();
        await observed;

        Assert.IsFalse(priority.Snapshot.InterludeActive);
        Assert.IsFalse(priority.Snapshot.MediaDucked);
        Assert.IsFalse(priority.Snapshot.MediaMuted);
    }

    [TestMethod]
    public async Task Decoder_failure_also_releases_main_audio_duck()
    {
        var priority = new AudioPriorityCoordinator();
        Assert.IsTrue(priority.BeginInterludeFile().IsAccepted);

        await Assert.ThrowsExactlyAsync<InvalidOperationException>(
            () => MainWindow.ObserveInterludePriorityCompletionAsync(
                Task.FromException(new InvalidOperationException("decode failed")),
                priority));

        Assert.IsFalse(priority.Snapshot.InterludeActive);
        Assert.IsFalse(priority.Snapshot.MediaDucked);
    }
}
