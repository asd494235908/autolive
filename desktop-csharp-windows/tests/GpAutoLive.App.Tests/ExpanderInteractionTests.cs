using System.Reflection;
using GpAutoLive.App.Features.Auth;
using System.Windows;
using System.Windows.Automation.Peers;
using System.Windows.Automation.Provider;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;

namespace GpAutoLive.App.Tests;

[TestClass]
[DoNotParallelize]
public sealed class ExpanderInteractionTests
{
    [TestMethod]
    [DataRow("FixedSpeechExpander", "FixedSpeechTextBox", "FixedSpeechStatePillText")]
    [DataRow("DouyinExpander", "DouyinRoomTextBox", "DouyinStatePillText")]
    public void Feature_cards_collapse_content_but_keep_title_and_status_visible(string expanderName, string inputName, string statusName)
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                window.Show();
                ((LoginViewModel)typeof(MainWindow).GetField("_login", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(window)!).ApplyActivated("expander-fixture");
                window.UpdateLayout();
                var expander = window.FindName(expanderName) as Expander;
                Assert.IsNotNull(expander);
                Assert.IsTrue(expander.IsExpanded);
                var input = (TextBox)window.FindName(inputName);
                var status = (TextBlock)window.FindName(statusName);
                var originalText = input.Text;
                var header = Descendants<ToggleButton>(expander).First();
                header.BringIntoView();
                window.UpdateLayout();
                Assert.IsNotNull(header.InputHitTest(new Point(10, header.ActualHeight / 2)));
                Assert.IsNotNull(header.InputHitTest(new Point(60, header.ActualHeight / 2)));
                var expandedHeight = expander.ActualHeight;
                var toggle = (IToggleProvider)new ToggleButtonAutomationPeer(header).GetPattern(PatternInterface.Toggle)!;
                toggle.Toggle();
                window.UpdateLayout();
                Assert.IsFalse(expander.IsExpanded);
                Assert.IsFalse(input.IsVisible);
                Assert.IsTrue(header.IsVisible);
                Assert.IsTrue(status.IsVisible);
                Assert.IsLessThan(expandedHeight, expander.ActualHeight);
                toggle.Toggle();
                window.UpdateLayout();
                Assert.IsTrue(expander.IsExpanded);
                Assert.IsTrue(input.IsVisible);
                Assert.AreEqual(originalText, input.Text);
            }
            finally { window.Close(); }
        });
    }

    [TestMethod]
    public void Advanced_header_content_is_hit_testable_and_toggles_both_ways()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                window.Show();
                ((LoginViewModel)typeof(MainWindow).GetField("_login", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(window)!).ApplyActivated("expander-fixture");
                window.UpdateLayout();
                var expanders = Descendants<Expander>(window).Where(item => item.Header is string text && text.EndsWith("（高级）", StringComparison.Ordinal)).ToArray();
                Assert.HasCount(3, expanders);
                foreach (var expander in expanders)
                {
                    var header = Descendants<ToggleButton>(expander).First();
                    Assert.IsTrue(header.IsEnabled, $"{expander.Header}: header disabled");
                    Assert.IsTrue(header.ActualWidth >= expander.ActualWidth - 24,
                        $"{expander.Header}: header width {header.ActualWidth}, expander width {expander.ActualWidth}");
                    header.BringIntoView();
                    window.UpdateLayout();
                    foreach (var x in new[] { 20d, 70d, header.ActualWidth - 12 })
                    {
                        var hit = header.InputHitTest(new Point(x, header.ActualHeight / 2));
                        Assert.IsNotNull(hit, $"{expander.Header}: header does not receive clicks at {x}");
                    }
                    var peer = new ToggleButtonAutomationPeer(header);
                    var toggle = (IToggleProvider)peer.GetPattern(PatternInterface.Toggle)!;
                    toggle.Toggle();
                    window.UpdateLayout();
                    Assert.IsTrue(expander.IsExpanded, $"{expander.Header}: expand");
                    Assert.IsTrue(((UIElement)expander.Content).IsVisible);
                    toggle.Toggle();
                    window.UpdateLayout();
                    Assert.IsFalse(expander.IsExpanded, $"{expander.Header}: collapse");
                    Assert.IsFalse(((UIElement)expander.Content).IsVisible);
                }
            }
            finally { window.Close(); }
        });
    }

    [TestMethod]
    public void Expander_header_space_is_not_intercepted_by_playback_shortcut()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                window.Show();
                ((LoginViewModel)typeof(MainWindow).GetField("_login", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(window)!).ApplyActivated("expander-fixture");
                window.UpdateLayout();
                foreach (var expander in Descendants<Expander>(window).ToArray())
                {
                    var initialExpanded = expander.IsExpanded;
                    var header = Descendants<ToggleButton>(expander).First();
                    header.BringIntoView();
                    window.UpdateLayout();
                    Assert.IsTrue(header.Focus());
                    var key = new KeyEventArgs(Keyboard.PrimaryDevice, PresentationSource.FromVisual(window), 0, Key.Space)
                    {
                        RoutedEvent = Keyboard.PreviewKeyDownEvent,
                    };
                    header.RaiseEvent(key);
                    Assert.IsFalse(key.Handled, $"{expander.Header}: playback consumed header Space");
                    for (var press = 0; press < 2; press++)
                    {
                        header.RaiseEvent(new KeyEventArgs(Keyboard.PrimaryDevice, PresentationSource.FromVisual(window), 0, Key.Space)
                        {
                            RoutedEvent = Keyboard.KeyDownEvent,
                        });
                        header.RaiseEvent(new KeyEventArgs(Keyboard.PrimaryDevice, PresentationSource.FromVisual(window), 0, Key.Space)
                        {
                            RoutedEvent = Keyboard.KeyUpEvent,
                        });
                        window.UpdateLayout();
                        Assert.AreEqual(press == 0 ? !initialExpanded : initialExpanded, expander.IsExpanded, $"{expander.Header}: Space toggle {press}");
                    }
                }
            }
            finally { window.Close(); }
        });
    }

    private static IEnumerable<T> Descendants<T>(DependencyObject root) where T : DependencyObject
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(root); index++)
        {
            var child = VisualTreeHelper.GetChild(root, index);
            if (child is T match) yield return match;
            foreach (var descendant in Descendants<T>(child)) yield return descendant;
        }
    }
}
