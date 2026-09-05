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
}
