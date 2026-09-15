using System.IO;
using System.Reflection;
using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using GpAutoLive.App.Features.Auth;

namespace GpAutoLive.App.Tests;

[TestClass]
[DoNotParallelize]
public sealed class ResponsiveWorkbenchLayoutTests
{
    [TestMethod]
    public void Feature_inputs_and_actions_share_the_same_card()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                foreach (var (input, action) in new[]
                {
                    ("StopVirtualCameraButton", "StartVirtualCameraButton"),
                    ("AudioInputDeviceComboBox", "StartMicrophoneButton"),
                    ("FixedSpeechTextBox", "SpeakFixedSpeechButton"),
                    ("DouyinRoomTextBox", "StartDouyinButton"),
                    ("StartRtmpButton", "RtmpVideoCheckBox"),
                    ("DouyinRoomTextBox", "DouyinRepliesTextBox"),
                })
                {
                    Border? Card(string name)
                    {
                        for (var current = window.FindName(name) as FrameworkElement; current is not null;
                             current = current.Parent as FrameworkElement)
                        {
                            if (current is Border border && border.Style == window.FindResource("CardStyle"))
                            {
                                return border;
                            }
                        }
                        return null;
                    }
                    Assert.IsNotNull(Card(input), input);
                    Assert.AreSame(Card(input), Card(action), $"{input} / {action} 分散在不同功能区");
                }
            }
            finally
            {
                window.Close();
            }
        });
    }

    [TestMethod]
    [DataRow(640, 320)]
    [DataRow(640, 360)]
    [DataRow(853, 480)]
    [DataRow(1024, 600)]
    [DataRow(999, 700)]
    [DataRow(1000, 700)]
    [DataRow(1249, 700)]
    [DataRow(1250, 700)]
    [DataRow(1280, 720)]
    [DataRow(1366, 768)]
    [DataRow(1586, 992)]
    [DataRow(1920, 1080)]
    [DataRow(2560, 1440)]
    [DataRow(3840, 2160)]
    public void Workbench_controls_stay_inside_their_functional_regions(int width, int height)
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                // Layout-only fixture: do not Show(), restore credentials or start media.
                ((FrameworkElement)window.FindName("LoginGate")).Visibility = Visibility.Collapsed;
                ((LoginViewModel)typeof(MainWindow).GetField("_login", BindingFlags.Instance | BindingFlags.NonPublic)!
                    .GetValue(window)!).ApplyActivated("GUI布局夹具");
                var root = (FrameworkElement)window.Content;
                var scale = MainWindow.CalculateResponsiveShellScale(new Size(width, height));
                var transform = (ScaleTransform)window.FindName("ResponsiveShellScaleTransform");
                transform.ScaleX = scale;
                transform.ScaleY = scale;
                window.GetType().GetMethod("UpdateResponsiveLayout", BindingFlags.Instance | BindingFlags.NonPublic)
                    ?.Invoke(window, [new Size(width, height)]);
                root.Measure(new Size(width, height));
                root.Arrange(new Rect(0, 0, width, height));
                root.UpdateLayout();

                var output = Environment.GetEnvironmentVariable("AUTOLIVE_GUI_SCREENSHOT_DIR");
                if (!string.IsNullOrWhiteSpace(output))
                {
                    Directory.CreateDirectory(output);
                    var bitmap = new RenderTargetBitmap(width, height, 96, 96, PixelFormats.Pbgra32);
                    bitmap.Render(root);
                    var encoder = new PngBitmapEncoder();
                    encoder.Frames.Add(BitmapFrame.Create(bitmap));
                    using var file = File.Create(Path.Combine(output, $"workbench-{width}x{height}.png"));
                    encoder.Save(file);
                }

                var search = (FrameworkElement)window.FindName("MediaSearchTextBox");
                var mediaHeader = (FrameworkElement)((FrameworkElement)search.Parent).Parent;
                AssertInside(search, mediaHeader);
                var status = (FrameworkElement)window.FindName("RtmpStatusText");
                var start = (FrameworkElement)window.FindName("StartRtmpButton");
                AssertSeparate(start, status, (FrameworkElement)status.Parent);
                var slider = (FrameworkElement)window.FindName("PlaybackSlider");
                var time = (FrameworkElement)window.FindName("PlaybackPositionText");
                AssertSeparate(slider, time, (FrameworkElement)slider.Parent);
                AssertInside(slider, (FrameworkElement)slider.Parent);
                Assert.IsTrue(root.DesiredSize.Width <= width + 1, $"布局需要 {root.DesiredSize.Width}，可用 {width}");
                AssertTextDoesNotOverlap((FrameworkElement)window.FindName("WorkbenchSurface"));
                AssertTextDoesNotOverlap(root);
                // Inspect the less common visible states without invoking any service action.
                foreach (var expander in Descendants(root).OfType<Expander>().ToArray())
                {
                    expander.IsExpanded = true;
                }
                ((FrameworkElement)window.FindName("DouyinQrPanel")).Visibility = Visibility.Visible;
                ((TextBlock)window.FindName("DouyinQrStatusText")).Text = "布局测试：等待用户扫码，二维码已过期时请重新获取。";
                ((TextBlock)window.FindName("RtmpStatusText")).Text = "布局测试：服务器连接失败，请检查推流地址、网络及鉴权配置；确认后可以重新连接。";
                root.UpdateLayout();
                foreach (var name in new[] { "ParameterScrollViewer", "OutputWorkspace" })
                {
                    var scroll = (ScrollViewer)window.FindName(name);
                    for (var offset = 0d; offset < scroll.ExtentHeight; offset += Math.Max(100, scroll.ViewportHeight / 2))
                    {
                        scroll.ScrollToVerticalOffset(offset);
                        root.UpdateLayout();
                        AssertTextDoesNotOverlap((FrameworkElement)window.FindName("WorkbenchSurface"));
                    }
                }
            }
            finally
            {
                window.Close();
            }
        });
    }

    private static void AssertInside(FrameworkElement element, FrameworkElement parent)
    {
        var rect = element.TransformToAncestor(parent).TransformBounds(new Rect(element.RenderSize));
        Assert.IsTrue(rect.Left >= -1 && rect.Right <= parent.ActualWidth + 1,
            $"{element.Name}: {rect} 超出容器宽度 {parent.ActualWidth}");
    }

    private static void AssertSeparate(FrameworkElement first, FrameworkElement second, FrameworkElement parent)
    {
        var a = first.TransformToAncestor(parent).TransformBounds(new Rect(first.RenderSize));
        var b = second.TransformToAncestor(parent).TransformBounds(new Rect(second.RenderSize));
        a.Intersect(b);
        Assert.IsTrue(a.IsEmpty || a.Width < 1 || a.Height < 1, $"{first.Name} 与 {second.Name} 重叠 {a}");
    }

    private static void AssertTextDoesNotOverlap(FrameworkElement region)
    {
        var texts = Descendants(region).OfType<TextBlock>()
            .Where(text => !string.IsNullOrWhiteSpace(text.Text) && text.ActualWidth > 0 && text.ActualHeight > 0)
            .Select(text =>
            {
                var formatted = new FormattedText(text.Text, CultureInfo.CurrentCulture, text.FlowDirection,
                    new Typeface(text.FontFamily, text.FontStyle, text.FontWeight, text.FontStretch),
                    text.FontSize, text.Foreground, VisualTreeHelper.GetDpi(text).PixelsPerDip);
                if (text.TextWrapping != TextWrapping.NoWrap)
                {
                    formatted.MaxTextWidth = text.ActualWidth;
                }
                var size = new Size(Math.Min(text.ActualWidth, formatted.WidthIncludingTrailingWhitespace),
                    Math.Min(text.ActualHeight, formatted.Height));
                var offset = text.TextAlignment switch
                {
                    TextAlignment.Right => text.ActualWidth - size.Width,
                    TextAlignment.Center => (text.ActualWidth - size.Width) / 2,
                    _ => 0,
                };
                var bounds = text.TransformToAncestor(region).TransformBounds(new Rect(new Point(offset, 0), size));
                for (var ancestor = VisualTreeHelper.GetParent(text); ancestor is not null && ancestor != region;
                     ancestor = VisualTreeHelper.GetParent(ancestor))
                {
                    if (ancestor is FrameworkElement clip && (clip is ScrollViewer || clip.ClipToBounds))
                    {
                        bounds.Intersect(clip.TransformToAncestor(region).TransformBounds(new Rect(clip.RenderSize)));
                    }
                }
                return (text.Text, Bounds: bounds);
            }).ToArray();
        for (var i = 0; i < texts.Length; i++)
        {
            for (var j = i + 1; j < texts.Length; j++)
            {
                var intersection = Rect.Intersect(texts[i].Bounds, texts[j].Bounds);
                Assert.IsTrue(intersection.IsEmpty || intersection.Width < 2 || intersection.Height < 2,
                    $"文字重叠：{texts[i].Text} / {texts[j].Text}，交集 {intersection}");
            }
        }
    }

    private static IEnumerable<DependencyObject> Descendants(DependencyObject parent)
    {
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
        {
            var child = VisualTreeHelper.GetChild(parent, i);
            if (child is UIElement { Visibility: not Visibility.Visible })
            {
                continue;
            }
            yield return child;
            foreach (var nested in Descendants(child))
            {
                yield return nested;
            }
        }
    }
}
