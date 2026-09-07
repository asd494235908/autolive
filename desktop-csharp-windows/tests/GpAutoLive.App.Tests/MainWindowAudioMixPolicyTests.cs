using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class MainWindowAudioMixPolicyTests
{
    [TestMethod]
    public void Interlude_volume_percent_maps_to_the_safe_decibel_range()
    {
        Assert.AreEqual(-60d, MainWindow.InterludeVolumePercentToDb(0));
        Assert.AreEqual(-18d, MainWindow.InterludeVolumePercentToDb(70));
        Assert.AreEqual(0d, MainWindow.InterludeVolumePercentToDb(100));
        Assert.AreEqual(0d, MainWindow.InterludeVolumeDbToPercent(-60));
        Assert.AreEqual(70d, MainWindow.InterludeVolumeDbToPercent(-18));
        Assert.AreEqual(100d, MainWindow.InterludeVolumeDbToPercent(0));
        Assert.AreEqual(100d, MainWindow.InterludeVolumeDbToPercent(12));
    }

    [TestMethod]
    public void Audio_diagnostics_focuses_interlude_spectrum_only_while_interlude_is_active()
    {
        using var bus = new FinalPcmBus(capacityFrames: 1_024, channels: 1);
        var mainPcm = Enumerable.Range(0, 480)
            .Select(index => MathF.Sin(2 * MathF.PI * 500 * index / 48_000) * 0.2f)
            .ToArray();
        var interludePcm = Enumerable.Range(0, 480)
            .Select(index => MathF.Sin(2 * MathF.PI * 4_000 * index / 48_000) * 0.5f)
            .ToArray();
        Assert.IsTrue(bus.TryPublish(mainPcm, out _, out _));
        Assert.IsTrue(bus.TryPublishOverlay(interludePcm, out _, out _));

        var priority = new AudioPriorityCoordinator();
        var mainLevels = new float[PcmSpectrumAnalyzer.BandCount];
        var interludeLevels = new float[PcmSpectrumAnalyzer.BandCount];
        var expectedMainLevels = new float[PcmSpectrumAnalyzer.BandCount];
        var expectedInterludeLevels = new float[PcmSpectrumAnalyzer.BandCount];
        bus.OutputSpectrum.CopyTo(expectedMainLevels);
        bus.OverlaySpectrum.CopyTo(expectedInterludeLevels);

        MainWindow.ProjectAudioDiagnosticsSpectrumLevels(
            bus,
            priority.Snapshot,
            mainLevels,
            interludeLevels);
        CollectionAssert.AreEqual(expectedMainLevels, mainLevels);
        Assert.IsTrue(interludeLevels.All(level => level == 0));

        Assert.IsTrue(priority.BeginInterludeFile().IsAccepted);
        MainWindow.ProjectAudioDiagnosticsSpectrumLevels(
            bus,
            priority.Snapshot,
            mainLevels,
            interludeLevels);
        Assert.IsTrue(mainLevels.All(level => level == 0));
        CollectionAssert.AreEqual(expectedInterludeLevels, interludeLevels);

        Assert.IsTrue(priority.End(AudioPriorityLayer.InterludeFile).IsAccepted);
        MainWindow.ProjectAudioDiagnosticsSpectrumLevels(
            bus,
            priority.Snapshot,
            mainLevels,
            interludeLevels);
        CollectionAssert.AreEqual(expectedMainLevels, mainLevels);
        Assert.IsTrue(interludeLevels.All(level => level == 0));
    }

    [TestMethod]
    public void Fixed_speech_and_microphone_keep_overlay_audible_while_muting_base()
    {
        var fixedSpeech = new AudioPriorityCoordinator();
        fixedSpeech.BeginFixedSpeech();

        var fixedPolicy = MainWindow.CreateAudioMixPolicy(
            fixedSpeech.Snapshot,
            outputGainDb: 0,
            duckingDepthDb: -60,
            overlayGainDb: 0);

        Assert.IsTrue(fixedPolicy.MuteBase);
        Assert.IsFalse(fixedPolicy.MuteOverlay);
        Assert.AreEqual(0, fixedPolicy.OverlayGainDb);

        var microphone = new AudioPriorityCoordinator();
        microphone.SetMicrophoneSpeaking(true);

        var microphonePolicy = MainWindow.CreateAudioMixPolicy(
            microphone.Snapshot,
            outputGainDb: 0,
            duckingDepthDb: -60,
            overlayGainDb: -3);

        Assert.IsTrue(microphonePolicy.MuteBase);
        Assert.IsFalse(microphonePolicy.MuteOverlay);
        Assert.AreEqual(-3, microphonePolicy.OverlayGainDb);
    }

    [TestMethod]
    public void Interlude_volume_changes_only_overlay_gain_and_keeps_duck_depth()
    {
        var priority = new AudioPriorityCoordinator();
        Assert.IsTrue(priority.BeginInterludeFile().IsAccepted);

        var policy = MainWindow.CreateAudioMixPolicy(
            priority.Snapshot,
            outputGainDb: -2,
            duckingDepthDb: -60,
            overlayGainDb: -18);

        Assert.AreEqual(-2, policy.BaseGainDb);
        Assert.AreEqual(-60, policy.BaseDuckingDb);
        Assert.AreEqual(-18, policy.OverlayGainDb);
        Assert.IsFalse(policy.MuteBase);
        Assert.IsFalse(policy.MuteOverlay);
    }
}
