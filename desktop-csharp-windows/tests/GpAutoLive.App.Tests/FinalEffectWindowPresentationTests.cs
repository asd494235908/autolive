using GpAutoLive.App.Features.Playback;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class FinalEffectWindowPresentationTests
{
    [TestMethod]
    public void Final_effect_window_is_not_presented_as_a_second_taskbar_application()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new FinalEffectWindow(new FinalEffectWindowController());
            try
            {
                Assert.IsFalse(window.ShowInTaskbar);
            }
            finally
            {
                window.Close();
            }
        });
    }
}
