using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class AppStateTests
{
    [TestMethod]
    public void Initial_state_is_stopped_and_has_empty_pool()
    {
        var state = AppState.Initial;

        Assert.AreEqual(PlaybackState.Stopped, state.PlaybackState);
        Assert.AreEqual(0UL, state.PlaybackGeneration);
        Assert.IsTrue(state.SourceMediaPool.IsEmpty);
    }

    [TestMethod]
    public void Record_update_keeps_original_snapshot_unchanged()
    {
        var initial = AppState.Initial;
        var updated = initial with
        {
            PlaybackState = PlaybackState.Ready,
            PlaybackGeneration = 1,
            SourceRevision = 1
        };

        Assert.AreEqual(PlaybackState.Stopped, initial.PlaybackState);
        Assert.AreEqual(0UL, initial.PlaybackGeneration);
        Assert.AreEqual(PlaybackState.Ready, updated.PlaybackState);
        Assert.AreEqual(1UL, updated.SourceRevision);
    }
}
