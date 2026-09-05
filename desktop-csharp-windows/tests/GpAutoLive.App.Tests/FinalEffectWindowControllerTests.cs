using GpAutoLive.App.Features.Playback;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class FinalEffectWindowControllerTests
{
    [TestMethod]
    public void Open_is_idempotent_and_keeps_one_projection_owner()
    {
        var controller = new FinalEffectWindowController();
        var video = FinalEffectSnapshot.Create(FinalEffectSurfaceKind.VideoHwndReserved);
        var audio = FinalEffectSnapshot.Create(FinalEffectSurfaceKind.AudioBlack);

        Assert.IsTrue(controller.Open(video));
        Assert.IsFalse(controller.Open(audio));
        Assert.IsTrue(controller.IsOpen);
        Assert.AreSame(audio, controller.Snapshot);
        Assert.AreEqual(FinalEffectSurfaceKind.AudioBlack, controller.Snapshot.SurfaceKind);
    }

    [TestMethod]
    public void Close_clears_sensitive_surface_state_and_can_reopen()
    {
        var controller = new FinalEffectWindowController();
        var snapshot = FinalEffectSnapshot.Create(FinalEffectSurfaceKind.VideoHwndReserved);

        controller.Open(snapshot);

        Assert.IsTrue(controller.Close());
        Assert.IsFalse(controller.IsOpen);
        Assert.AreSame(FinalEffectSnapshot.Empty, controller.Snapshot);
        Assert.IsTrue(controller.Open(FinalEffectSnapshot.Empty));
    }

    [TestMethod]
    public void Projection_contains_only_the_surface_contract()
    {
        var controller = new FinalEffectWindowController();
        var snapshot = FinalEffectSnapshot.Create(FinalEffectSurfaceKind.AudioBlack);

        controller.Open(snapshot);

        Assert.AreEqual(FinalEffectSurfaceKind.AudioBlack, controller.Snapshot.SurfaceKind);
        Assert.IsFalse(typeof(FinalEffectSnapshot).GetProperties()
            .Any(property => property.Name.Contains("Path", StringComparison.OrdinalIgnoreCase)));
    }
}
