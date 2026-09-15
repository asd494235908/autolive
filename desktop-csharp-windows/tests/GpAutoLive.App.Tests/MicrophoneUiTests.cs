namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class MicrophoneUiTests
{
    [TestMethod]
    public void Start_and_test_share_the_same_safe_gate_while_stop_requires_listening()
    {
        Assert.IsFalse(MainWindow.CanStartMicrophone(true, false, false, true, false));
        Assert.IsFalse(MainWindow.CanStartMicrophone(true, false, true, false, false));
        Assert.IsTrue(MainWindow.CanStartMicrophone(true, false, true, true, false));
        Assert.IsFalse(MainWindow.CanStartMicrophone(true, true, true, true, false));
        Assert.IsFalse(MainWindow.CanStartMicrophone(true, false, true, true, true));

        Assert.IsTrue(MainWindow.CanStopMicrophone(true, true, false));
        Assert.IsFalse(MainWindow.CanStopMicrophone(true, false, false));
        Assert.IsFalse(MainWindow.CanStopMicrophone(true, true, true));
    }

    [TestMethod]
    public void Idle_hint_tells_user_how_to_reach_the_next_gate()
    {
        var beforePlayback = InvokeIdleStatus(finalPcmBusAvailable: false, inputDeviceAvailable: false);
        StringAssert.Contains(beforePlayback, "播放带声音媒体");
        StringAssert.Contains(beforePlayback, "FinalPcmBus");

        var afterPlaybackBeforeDevice = InvokeIdleStatus(finalPcmBusAvailable: true, inputDeviceAvailable: false);
        StringAssert.Contains(afterPlaybackBeforeDevice, "刷新并选择");
        StringAssert.Contains(afterPlaybackBeforeDevice, "FinalPcmBus");
    }

    private static string InvokeIdleStatus(bool finalPcmBusAvailable, bool inputDeviceAvailable)
    {
        var method = typeof(MainWindow).GetMethod(
            "GetMicrophoneIdleStatus",
            System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.NonPublic);
        Assert.IsNotNull(method);
        return (string)method!.Invoke(null, [finalPcmBusAvailable, inputDeviceAvailable])!;
    }
}
