namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class MicrophoneInterludeGateTests
{
    [TestMethod]
    public void Disabled_gate_does_not_enter_speaking()
    {
        var gate = new MicrophoneInterludeGate();

        var snapshot = gate.Process(new float[] { 1, 1, 1, 1 }, 100);

        Assert.AreEqual(MicrophoneInterludeGateState.Disabled, snapshot.State);
        Assert.IsFalse(snapshot.IsSpeaking);
    }

    [TestMethod]
    public void Armed_gate_enters_speaking_and_uses_hangover()
    {
        var gate = new MicrophoneInterludeGate(startThresholdDb: -42, stopThresholdDb: -48, hangoverMs: 250);
        gate.Arm();

        var speaking = gate.Process(new float[] { 0.2f, 0.2f }, 1_000);
        var hangover = gate.Process(new float[] { 0, 0 }, 1_010);
        var armed = gate.Process(new float[] { 0, 0 }, 1_300);

        Assert.AreEqual(MicrophoneInterludeGateState.Speaking, speaking.State);
        Assert.IsTrue(speaking.IsSpeaking);
        Assert.AreEqual(MicrophoneInterludeGateState.Hangover, hangover.State);
        Assert.AreEqual(MicrophoneInterludeGateState.Armed, armed.State);
    }

    [TestMethod]
    public void Low_noise_does_not_trigger_gate_and_nonfinite_samples_are_sanitized()
    {
        var gate = new MicrophoneInterludeGate();
        gate.Arm();

        var snapshot = gate.Process(new float[] { float.NaN, float.PositiveInfinity }, 100);

        Assert.AreEqual(MicrophoneInterludeGateState.Armed, snapshot.State);
        Assert.AreEqual(-96, snapshot.LevelDb);
    }

    [TestMethod]
    public void Invalid_shape_and_observation_size_are_rejected()
    {
        var gate = new MicrophoneInterludeGate(channels: 2);

        var shapeThrown = false;
        try
        {
            gate.Process(new float[] { 1 }, 0);
        }
        catch (ArgumentException)
        {
            shapeThrown = true;
        }

        var sizeThrown = false;
        try
        {
            gate.Process(new float[8_194], 0);
        }
        catch (ArgumentOutOfRangeException)
        {
            sizeThrown = true;
        }

        Assert.IsTrue(shapeThrown);
        Assert.IsTrue(sizeThrown);
    }
}
