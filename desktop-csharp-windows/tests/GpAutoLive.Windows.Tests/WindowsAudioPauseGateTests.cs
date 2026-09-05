namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsAudioPauseGateTests
{
    [TestMethod]
    public async Task Open_gate_completes_immediately()
    {
        var gate = new WindowsAudioPauseGate();

        await gate.WaitIfPausedAsync();

        Assert.IsFalse(gate.IsPaused);
    }

    [TestMethod]
    public async Task Paused_gate_waits_until_resume()
    {
        var gate = new WindowsAudioPauseGate();
        gate.Pause();
        var waiting = gate.WaitIfPausedAsync().AsTask();

        Assert.IsFalse(waiting.IsCompleted);
        gate.Resume();
        await waiting;
        Assert.IsFalse(gate.IsPaused);
    }

    [TestMethod]
    public async Task Paused_gate_honors_cancellation()
    {
        var gate = new WindowsAudioPauseGate();
        gate.Pause();
        using var cancellation = new CancellationTokenSource();
        var waiting = gate.WaitIfPausedAsync(cancellation.Token).AsTask();

        cancellation.Cancel();

        try
        {
            await waiting;
            Assert.Fail("暂停等待未响应取消。");
        }
        catch (OperationCanceledException)
        {
            // expected
        }
    }

    [TestMethod]
    public async Task Closing_gate_releases_waiters_and_is_idempotent()
    {
        var gate = new WindowsAudioPauseGate();
        gate.Pause();
        var waiting = gate.WaitIfPausedAsync().AsTask();

        gate.Close();
        gate.Close();
        await waiting;

        Assert.IsFalse(gate.IsPaused);
        await gate.WaitIfPausedAsync();
    }

    [TestMethod]
    public void Resume_and_pause_are_idempotent()
    {
        var gate = new WindowsAudioPauseGate();

        gate.Resume();
        gate.Pause();
        gate.Pause();
        Assert.IsTrue(gate.IsPaused);
        gate.Resume();
        gate.Resume();
        Assert.IsFalse(gate.IsPaused);
    }
}
