using System.Reflection;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsMpvShutdownOwnershipTests
{
    [TestMethod]
    public async Task Cancelled_runtime_stop_keeps_owner_for_shutdown_retry()
    {
        await using var controller = new WindowsMpvPlaybackController();
        var runtime = new WindowsMpvPlaybackRuntime();
        var runtimeField = typeof(WindowsMpvPlaybackController)
            .GetField("_runtime", BindingFlags.Instance | BindingFlags.NonPublic)!;
        runtimeField.SetValue(controller, runtime);
        // Occupy the existing gate, equivalent to an in-flight IPC command. No production fake runtime.
        var gate = (SemaphoreSlim)typeof(WindowsMpvPlaybackRuntime)
            .GetField("_lifecycle", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(runtime)!;
        await gate.WaitAsync();
        using var cancellation = new CancellationTokenSource();
        var shutdown = controller.ShutdownAsync(cancellation.Token);
        cancellation.Cancel();
        try
        {
            var result = await shutdown.WaitAsync(TimeSpan.FromSeconds(2));
            Assert.IsFalse(result.IsSuccess);
            Assert.AreSame(runtime, runtimeField.GetValue(controller));
        }
        finally
        {
            gate.Release();
            await shutdown.WaitAsync(TimeSpan.FromSeconds(5));
        }
        var retried = await controller.ShutdownAsync();
        Assert.IsTrue(retried.IsSuccess);
        Assert.IsNull(runtimeField.GetValue(controller));
    }
}
