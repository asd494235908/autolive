using GpAutoLive.App.Features.Effects;
using GpAutoLive.Core;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class VideoEffectCyclePlannerTests
{
    [TestMethod]
    public void Uses_playback_position_to_emit_one_bounded_cycle_trigger()
    {
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        var planner = new VideoEffectCyclePlanner(new Random(7));

        Assert.IsFalse(planner.ShouldRegenerate(identity, 0, 20_000, enabled: true));
        Assert.AreEqual(0UL, planner.CurrentCycleStartMs);
        Assert.IsTrue(planner.CurrentCycleTargetMs is >= 5_000 and <= 8_000);
        Assert.IsFalse(planner.ShouldRegenerate(identity, 4_999, 20_000, enabled: true));
        Assert.IsTrue(planner.ShouldRegenerate(identity, 8_000, 20_000, enabled: true));
        Assert.IsFalse(planner.ShouldRegenerate(identity, 8_001, 20_000, enabled: true));
    }

    [TestMethod]
    public void Disable_and_seek_rearm_from_the_current_position()
    {
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        var planner = new VideoEffectCyclePlanner(new Random(7));

        _ = planner.ShouldRegenerate(identity, 0, 20_000, enabled: true);
        Assert.IsFalse(planner.ShouldRegenerate(identity, 9_000, 20_000, enabled: false));
        Assert.IsFalse(planner.ShouldRegenerate(identity, 9_100, 20_000, enabled: true));
        Assert.IsFalse(planner.ShouldRegenerate(identity, 2_000, 20_000, enabled: true));
        Assert.IsFalse(planner.ShouldRegenerate(identity, 2_001, 20_000, enabled: true));
    }

    [TestMethod]
    public void Uses_the_user_configured_video_period_range()
    {
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        var planner = new VideoEffectCyclePlanner(new Random(7));
        planner.Configure(1_000, 1_000);

        Assert.IsFalse(planner.ShouldRegenerate(identity, 0, 20_000, enabled: true));
        Assert.IsTrue(planner.ShouldRegenerate(identity, 1_000, 20_000, enabled: true));
    }
}
