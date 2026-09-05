namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class AudioPriorityCoordinatorTests
{
    [TestMethod]
    public void Fixed_speech_mutes_media_and_interlude()
    {
        var coordinator = new AudioPriorityCoordinator();

        var started = coordinator.BeginFixedSpeech();

        Assert.IsTrue(started.IsAccepted);
        Assert.AreEqual(AudioPriorityLayer.FixedSpeech, started.Snapshot.FocusLayer);
        Assert.IsTrue(started.Snapshot.MediaMuted);
        Assert.IsTrue(started.Snapshot.InterludeMuted);
        Assert.IsFalse(started.Snapshot.MediaDucked);
    }

    [TestMethod]
    public void Interlude_ducks_media_but_does_not_mute_it()
    {
        var coordinator = new AudioPriorityCoordinator();

        var started = coordinator.BeginInterludeFile();

        Assert.IsTrue(started.IsAccepted);
        Assert.IsFalse(started.Snapshot.MediaMuted);
        Assert.IsTrue(started.Snapshot.MediaDucked);
        Assert.IsFalse(started.Snapshot.InterludeMuted);
    }

    [TestMethod]
    public void Fixed_speech_is_rejected_while_microphone_is_speaking()
    {
        var coordinator = new AudioPriorityCoordinator();
        var microphone = coordinator.SetMicrophoneSpeaking(true);

        var blocked = coordinator.BeginFixedSpeech();

        Assert.IsTrue(microphone.IsAccepted);
        Assert.IsNull(microphone.PreemptedLayer);
        Assert.IsFalse(blocked.IsAccepted);
        Assert.AreEqual(AudioPriorityDecisionKind.Rejected, blocked.Kind);
        Assert.AreEqual(AudioPriorityLayer.Microphone, blocked.Snapshot.FocusLayer);
    }

    [TestMethod]
    public void Microphone_preempts_fixed_speech_without_auto_resume()
    {
        var coordinator = new AudioPriorityCoordinator();
        coordinator.BeginFixedSpeech();

        var microphone = coordinator.SetMicrophoneSpeaking(true);
        var ended = coordinator.SetMicrophoneSpeaking(false);

        Assert.AreEqual(AudioPriorityLayer.FixedSpeech, microphone.PreemptedLayer);
        Assert.AreEqual(AudioPriorityLayer.Microphone, microphone.Snapshot.FocusLayer);
        Assert.IsNull(ended.Snapshot.FocusLayer);
        Assert.IsFalse(ended.Snapshot.FixedSpeechActive);
        Assert.IsFalse(ended.Snapshot.MediaMuted);
    }

    [TestMethod]
    public void New_fixed_speech_replaces_an_existing_fixed_operation()
    {
        var coordinator = new AudioPriorityCoordinator();
        coordinator.BeginFixedSpeech();

        var replacement = coordinator.BeginFixedSpeech();

        Assert.IsTrue(replacement.IsAccepted);
        Assert.AreEqual(AudioPriorityLayer.FixedSpeech, replacement.PreemptedLayer);
        Assert.AreEqual(AudioPriorityLayer.FixedSpeech, replacement.Snapshot.FocusLayer);
    }

    [TestMethod]
    public void Ending_a_stale_layer_is_idempotent_and_does_not_change_generation()
    {
        var coordinator = new AudioPriorityCoordinator();
        var before = coordinator.Snapshot;

        var ended = coordinator.End(AudioPriorityLayer.InterludeFile);

        Assert.IsFalse(ended.IsAccepted);
        Assert.AreEqual(AudioPriorityDecisionKind.Ignored, ended.Kind);
        Assert.AreEqual(before.Generation, ended.Snapshot.Generation);
    }
}
