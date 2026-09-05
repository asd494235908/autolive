using GpAutoLive.App;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class AudioProcessingTransitionTests
{
    [TestMethod]
    public void Audio_processing_revision_changes_once_per_real_switch()
    {
        var state = new ShellState();
        var initialRevision = state.AudioProcessingRevision;

        state.AudioProcessing = false;
        Assert.AreEqual(initialRevision + 1, state.AudioProcessingRevision);

        state.AudioProcessing = false;
        Assert.AreEqual(initialRevision + 1, state.AudioProcessingRevision);

        state.AudioProcessing = true;
        Assert.AreEqual(initialRevision + 2, state.AudioProcessingRevision);
    }
}
