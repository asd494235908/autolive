using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsMicrophoneInterludeControllerTests
{
    [TestMethod]
    public async Task Invalid_path_fails_closed_without_starting_priority()
    {
        await using var controller = new WindowsMicrophoneInterludeController(new AudioPriorityCoordinator());

        var result = await controller.StartAsync(
            @"C:\media\portaudio.dll",
            new WindowsPortAudioInputConfig(0));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioInputFailureCode.InvalidPath, result.Error?.Code);
        Assert.AreEqual(WindowsMicrophoneInterludeState.Failed, result.Snapshot.State);
        Assert.IsFalse(result.Snapshot.Priority.MicrophoneSpeaking);
    }

    [TestMethod]
    public async Task Missing_final_pcm_bus_is_rejected_before_input_start()
    {
        await using var controller = new WindowsMicrophoneInterludeController(
            new AudioPriorityCoordinator(),
            finalPcmBusProvider: static () => null);

        var result = await controller.StartAsync(
            @"C:\media\portaudio.dll",
            new WindowsPortAudioInputConfig(0));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioInputFailureCode.OutputBusUnavailable, result.Error?.Code);
        Assert.AreEqual(WindowsMicrophoneInterludeState.Failed, result.Snapshot.State);
        Assert.IsFalse(result.Snapshot.Input.IsRunning);
    }

    [TestMethod]
    public async Task Stop_without_active_session_is_idempotent()
    {
        await using var controller = new WindowsMicrophoneInterludeController(new AudioPriorityCoordinator());

        var result = await controller.StopAsync();

        Assert.IsTrue(result.IsSuccess);
        Assert.AreEqual(WindowsMicrophoneInterludeState.Idle, result.Snapshot.State);
        Assert.IsFalse(result.Snapshot.Input.IsRunning);
    }

    [TestMethod]
    public async Task Dispose_rejects_new_start()
    {
        var controller = new WindowsMicrophoneInterludeController(new AudioPriorityCoordinator());
        await controller.DisposeAsync();

        var result = await controller.StartAsync(null, new WindowsPortAudioInputConfig(0));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioInputFailureCode.Closed, result.Error?.Code);
        Assert.AreEqual(WindowsMicrophoneInterludeState.Closed, result.Snapshot.State);
    }
}
