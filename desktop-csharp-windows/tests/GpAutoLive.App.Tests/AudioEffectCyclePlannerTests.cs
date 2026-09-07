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
        planner.Configure(4_000, 4_000);

        Assert.AreEqual(
            AudioEffectCycleAction.None,
            planner.GetAction(identity, 0, 20_000, enabled: true, hasPreparedCandidate: false, out _));
        Assert.AreEqual(0UL, planner.CurrentCycleStartMs);
        Assert.AreEqual(4_000UL, planner.CurrentCycleTargetMs);
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

    [TestMethod]
    public void Uses_the_user_configured_audio_period_range()
    {
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        var planner = new AudioEffectCyclePlanner();
        planner.Configure(2_000, 2_000);

        Assert.AreEqual(
            AudioEffectCycleAction.None,
            planner.GetAction(identity, 0, 20_000, enabled: true, hasPreparedCandidate: false, out _));
        Assert.AreEqual(
            AudioEffectCycleAction.Prepare,
            planner.GetAction(identity, 1_000, 20_000, enabled: true, hasPreparedCandidate: false, out var target));
        Assert.AreEqual(2_000UL, target);
        Assert.AreEqual(2_000UL, planner.CurrentPeriodMs);
    }
}
