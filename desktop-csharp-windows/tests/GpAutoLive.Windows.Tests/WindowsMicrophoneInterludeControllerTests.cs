using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsMicrophoneInterludeControllerTests
{
    [TestMethod]
    public async Task Stop_timeout_retains_monitor_and_cancellation_until_retry_can_join()
    {
        var controller = new WindowsMicrophoneInterludeController(new AudioPriorityCoordinator());
        var monitor = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var cancellation = new CancellationTokenSource();
        var fields = System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic;
        var monitorField = typeof(WindowsMicrophoneInterludeController).GetField("_monitorTask", fields)!;
        var cancellationField = typeof(WindowsMicrophoneInterludeController).GetField("_sessionCancellation", fields)!;
        monitorField.SetValue(controller, monitor.Task);
        cancellationField.SetValue(controller, cancellation);

        try
        {
            var stopped = await controller.StopAsync();
            Assert.IsFalse(stopped.IsSuccess);
            Assert.AreSame(monitor.Task, monitorField.GetValue(controller));
            Assert.AreSame(cancellation, cancellationField.GetValue(controller));
            Assert.IsTrue(cancellation.Token.IsCancellationRequested);
            Assert.IsFalse((await controller.StartAsync(null, new(0))).IsSuccess);
            monitor.SetResult();
            Assert.IsTrue((await controller.StopAsync()).IsSuccess);
            Assert.IsNull(monitorField.GetValue(controller));
            Assert.IsNull(cancellationField.GetValue(controller));
        }
        finally
        {
            monitor.TrySetResult();
            await controller.DisposeAsync();
        }
    }

    [TestMethod]
    public async Task Repeated_dispose_does_not_cancel_an_already_disposed_source()
    {
        var controller = new WindowsMicrophoneInterludeController(new AudioPriorityCoordinator());
        await controller.DisposeAsync();
        await controller.DisposeAsync();
        Assert.AreEqual(WindowsMicrophoneInterludeState.Closed, controller.Snapshot.State);
    }

    [TestMethod]
    public async Task Faulted_monitor_reports_failure_once_and_still_releases_its_owner()
    {
        await using var controller = new WindowsMicrophoneInterludeController(new AudioPriorityCoordinator());
        var fields = System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic;
        var monitorField = typeof(WindowsMicrophoneInterludeController).GetField("_monitorTask", fields)!;
        monitorField.SetValue(controller, Task.FromException(new InvalidOperationException("private provider failure")));

        var stopped = await controller.StopAsync();
        Assert.IsFalse(stopped.IsSuccess);
        Assert.AreEqual(WindowsPortAudioInputFailureCode.CallbackFailed, stopped.Error?.Code);
        Assert.IsFalse(stopped.Error?.Message.Contains("private provider failure", StringComparison.Ordinal) ?? true);
        Assert.IsNull(monitorField.GetValue(controller));
        Assert.IsTrue((await controller.StopAsync()).IsSuccess);
    }

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
