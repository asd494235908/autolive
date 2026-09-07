using GpAutoLive.Contracts;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class AudioEffectProgressWatcherTests
{
    [TestMethod]
    public void Playing_video_with_a_new_pcm_session_starts_the_audio_cycle_watcher()
    {
        Assert.IsTrue(MainWindow.ShouldStartAudioCompletionWatcher(
            PlaybackState.Playing,
            audioSessionStarted: true));
        Assert.IsFalse(MainWindow.ShouldStartAudioCompletionWatcher(
            PlaybackState.Playing,
            audioSessionStarted: false));
        Assert.IsFalse(MainWindow.ShouldStartAudioCompletionWatcher(
            PlaybackState.Paused,
            audioSessionStarted: true));
    }
}
