using GpAutoLive.App.Features.Effects;
using GpAutoLive.Core;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class AudioEffectCyclePlannerTests
{
    [TestMethod]
    public void Prepares_one_second_before_target_and_commits_prepared_candidate()
    {
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        var planner = new AudioEffectCyclePlanner();

        Assert.AreEqual(
            AudioEffectCycleAction.None,
            planner.GetAction(identity, 0, 20_000, enabled: true, hasPreparedCandidate: false, out _));
        Assert.AreEqual(
            AudioEffectCycleAction.Prepare,
            planner.GetAction(identity, 3_000, 20_000, enabled: true, hasPreparedCandidate: false, out var target));
        Assert.AreEqual(4_000UL, target);
        Assert.AreEqual(
            AudioEffectCycleAction.Commit,
            planner.GetAction(identity, 3_200, 20_000, enabled: true, hasPreparedCandidate: true, out target));
        Assert.AreEqual(4_000UL, target);
    }

    [TestMethod]
    public void Short_media_does_not_schedule_a_cycle()
    {
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        var planner = new AudioEffectCyclePlanner();

        Assert.AreEqual(
            AudioEffectCycleAction.None,
            planner.GetAction(identity, 0, 4_000, enabled: true, hasPreparedCandidate: false, out _));
    }
}
