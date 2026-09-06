using GpAutoLive.App.Features.Effects;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class EffectCycleProgressProjectionTests
{
    [TestMethod]
    public void Projects_only_the_supplied_real_cycle_window()
    {
        Assert.AreEqual(0, EffectCycleProgressProjection.Calculate(null, 1_000, 5_000));
        Assert.AreEqual(0, EffectCycleProgressProjection.Calculate(1_000, 1_000, 5_000));
        Assert.AreEqual(50, EffectCycleProgressProjection.Calculate(3_000, 1_000, 5_000));
        Assert.AreEqual(100, EffectCycleProgressProjection.Calculate(6_000, 1_000, 5_000));
        Assert.AreEqual(0, EffectCycleProgressProjection.Calculate(3_000, 5_000, 5_000));
    }

    [TestMethod]
    public void Shell_progress_bindings_are_bounded()
    {
        var state = new ShellState();

        state.SetVideoEffectCycleProgress(37.5);
        state.SetAudioEffectCycleProgress(101);
        state.SetInterludeEffectCycleProgress(double.NaN);

        Assert.AreEqual(37.5, state.VideoEffectCycleProgress);
        Assert.AreEqual(100, state.AudioEffectCycleProgress);
        Assert.AreEqual(0, state.InterludeEffectCycleProgress);
    }
}
