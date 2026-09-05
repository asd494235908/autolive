using GpAutoLive.App.Features.Playback;
using GpAutoLive.Contracts;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class FinalEffectWindowControllerTests
{
    [TestMethod]
    public void Open_is_idempotent_and_keeps_one_projection_owner()
    {
        var controller = new FinalEffectWindowController();
        var video = FinalEffectSnapshot.Create(
            PlaybackState.Playing,
            FinalEffectSurfaceKind.VideoHwndReserved,
            TimeSpan.FromSeconds(3),
            TimeSpan.FromSeconds(10),
            7,
            11,
            2);
        var audio = video with
        {
            PlaybackState = PlaybackState.Paused,
            SurfaceKind = FinalEffectSurfaceKind.AudioBlack
        };

        Assert.IsTrue(controller.Open(video));
        Assert.IsFalse(controller.Open(audio));
        Assert.IsTrue(controller.IsOpen);
        Assert.AreSame(audio, controller.Snapshot);
        Assert.IsTrue(controller.Snapshot.IsPureAudio);
    }

    [TestMethod]
    public void Close_clears_sensitive_surface_state_and_can_reopen()
    {
        var controller = new FinalEffectWindowController();
        var snapshot = FinalEffectSnapshot.Create(
            PlaybackState.Playing,
            FinalEffectSurfaceKind.VideoHwndReserved,
            TimeSpan.Zero,
            TimeSpan.FromMinutes(1),
            3,
            4,
            5);

        controller.Open(snapshot);

        Assert.IsTrue(controller.Close());
        Assert.IsFalse(controller.IsOpen);
        Assert.AreSame(FinalEffectSnapshot.Empty, controller.Snapshot);
        Assert.IsFalse(controller.RequestCommand(FinalEffectPlaybackCommand.Stop));
        Assert.IsTrue(controller.Open(FinalEffectSnapshot.Empty));
    }

    [TestMethod]
    public void Commands_are_emitted_only_while_window_is_open()
    {
        var controller = new FinalEffectWindowController();
        var received = new List<FinalEffectPlaybackCommand>();
        controller.CommandRequested += (_, args) => received.Add(args.Command);

        Assert.IsFalse(controller.RequestCommand(FinalEffectPlaybackCommand.Stop));

        controller.Open(FinalEffectSnapshot.Empty with
        {
            SurfaceKind = FinalEffectSurfaceKind.AudioBlack,
            PlaybackState = PlaybackState.Playing
        });
        Assert.IsTrue(controller.RequestCommand(FinalEffectPlaybackCommand.TogglePlayPause));
        controller.Close();
        Assert.IsFalse(controller.RequestCommand(FinalEffectPlaybackCommand.Stop));

        CollectionAssert.AreEqual(
            new[] { FinalEffectPlaybackCommand.TogglePlayPause },
            received);
    }

    [TestMethod]
    public void Projection_clamps_progress_without_exposing_source_path()
    {
        var snapshot = FinalEffectSnapshot.Create(
            PlaybackState.Playing,
            FinalEffectSurfaceKind.VideoHwndReserved,
            TimeSpan.FromSeconds(30),
            TimeSpan.FromSeconds(10),
            1,
            2,
            3);

        Assert.AreEqual(1d, snapshot.Progress);
        Assert.IsFalse(typeof(FinalEffectSnapshot).GetProperties()
            .Any(property => property.Name.Contains("Path", StringComparison.OrdinalIgnoreCase)));
    }
}
