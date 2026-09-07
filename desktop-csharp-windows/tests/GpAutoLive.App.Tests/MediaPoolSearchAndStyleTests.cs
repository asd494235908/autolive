using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Media;
using Ellipse = System.Windows.Shapes.Ellipse;
using System.Windows.Threading;
using GpAutoLive.App.Features.Auth;
using GpAutoLive.App.Features.Media;
using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.App.Tests;

[TestClass]
[DoNotParallelize]
public sealed class MediaPoolSearchAndStyleTests
{
    [TestMethod]
    public void Search_box_filters_visible_items_without_mutating_the_source_pool()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                window.Show();
                GetPrivateField<LoginViewModel>(window, "_login").ApplyActivated("fixture-account");

                var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                var result = mediaPool.ReplaceAll([CreateMedia("alpha.mp4"), CreateMedia("music.wav", MediaKind.Audio)]);
                Assert.IsTrue(result.IsSuccess, result.Error?.Message);
                InvokeVoidPrivate(window, "ApplyMediaSnapshot", result.Snapshot, "测试媒体池投影");

                var search = GetPrivateField<TextBox>(window, "MediaSearchTextBox");
                var list = GetPrivateField<ListBox>(window, "MediaListBox");
                Dispatcher.CurrentDispatcher.Invoke(DispatcherPriority.Background, new Action(static () => { }));
                search.Text = "music";
                Dispatcher.CurrentDispatcher.Invoke(DispatcherPriority.Background, new Action(static () => { }));

                Assert.AreEqual("music", GetPrivateField<ShellState>(window, "_state").MediaSearchText);
                Assert.AreEqual(1, list.Items.Count);
                Assert.AreEqual("music.wav", ((MediaListItemViewModel)list.Items[0]).FileName);
                Assert.AreEqual(2, mediaPool.Snapshot.SourceMediaPool.Length);

                list.SelectedIndex = 0;
                var selectedIndex = (int)(window.GetType()
                    .GetMethod("GetSelectedMediaPoolIndex", BindingFlags.Instance | BindingFlags.NonPublic)
                    ?.Invoke(window, null) ?? -1);
                Assert.AreEqual(1, selectedIndex);

                search.Text = " ";
                Dispatcher.CurrentDispatcher.Invoke(DispatcherPriority.Background, new Action(static () => { }));
                Assert.AreEqual(2, list.Items.Count);

                var kindFilter = GetPrivateField<ComboBox>(window, "MediaKindFilterComboBox");
                kindFilter.SelectedIndex = 1;
                Dispatcher.CurrentDispatcher.Invoke(DispatcherPriority.Background, new Action(static () => { }));
                Assert.AreEqual(MediaKindFilter.Video, GetPrivateField<ShellState>(window, "_state").MediaKindFilter);
                Assert.AreEqual(1, list.Items.Count);

                kindFilter.SelectedIndex = 0;
                Dispatcher.CurrentDispatcher.Invoke(DispatcherPriority.Background, new Action(static () => { }));
                Assert.AreEqual(MediaKindFilter.All, GetPrivateField<ShellState>(window, "_state").MediaKindFilter);
                Assert.AreEqual(2, list.Items.Count);

                var button = GetPrivateField<Button>(window, "ImportButton");
                Assert.AreEqual(32d, button.Height);
                Assert.AreEqual(HorizontalAlignment.Stretch, button.HorizontalAlignment);
                Assert.AreEqual(HorizontalAlignment.Center, button.HorizontalContentAlignment);
                Assert.AreEqual(VerticalAlignment.Center, button.VerticalContentAlignment);
                Assert.IsInstanceOfType(button.Content, typeof(StackPanel));
                Assert.AreEqual("添加媒体", AutomationProperties.GetName(button));
                Assert.AreEqual("追加到当前播放池，不会删除已有条目或本地源文件", button.ToolTip);
            }
            finally
            {
                window?.Close();
            }
        });
    }

    [TestMethod]
    public void Clearing_search_restores_the_current_source_selection()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                window.Show();
                GetPrivateField<LoginViewModel>(window, "_login").ApplyActivated("fixture-account");

                var mediaPool = GetPrivateField<MediaPoolService>(window, "_mediaPool");
                var result = mediaPool.ReplaceAll([CreateMedia("alpha.mp4"), CreateMedia("music.wav", MediaKind.Audio)]);
                Assert.IsTrue(result.IsSuccess, result.Error?.Message);
                InvokeVoidPrivate(window, "ApplyMediaSnapshot", result.Snapshot, "测试媒体池投影");

                var search = GetPrivateField<TextBox>(window, "MediaSearchTextBox");
                var list = GetPrivateField<ListBox>(window, "MediaListBox");
                Dispatcher.CurrentDispatcher.Invoke(DispatcherPriority.ContextIdle, new Action(static () => { }));
                Assert.AreEqual("alpha.mp4", ((MediaListItemViewModel)list.SelectedItem!).FileName);

                search.Text = "music";
                Dispatcher.CurrentDispatcher.Invoke(DispatcherPriority.ContextIdle, new Action(static () => { }));
                Assert.IsNull(list.SelectedItem);

                search.Text = string.Empty;
                Dispatcher.CurrentDispatcher.Invoke(DispatcherPriority.ContextIdle, new Action(static () => { }));
                Assert.AreEqual("alpha.mp4", ((MediaListItemViewModel)list.SelectedItem!).FileName);
            }
            finally
            {
                window?.Close();
            }
        });
    }

    [TestMethod]
    public void Processing_switches_match_rust_small_switch_geometry()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                window.Show();
                AssertSmallSwitch(FindCheckBox(window, "VideoProcessingCheckBox"));
                AssertSmallSwitch(FindCheckBox(window, "AudioProcessingCheckBox"));
                Assert.AreEqual("等待视频", GetPrivateField<TextBlock>(window, "VideoProcessingPathText").Text);
                Assert.AreEqual("等待视频", GetPrivateField<TextBlock>(window, "VideoProcessingBackendText").Text);
            }
            finally
            {
                window?.Close();
            }
        });
    }

    [TestMethod]
    public void Advanced_expander_headers_use_the_dark_theme_text_and_header_template()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                window.Show();

                var expanders = FindVisualChildren<Expander>(window).ToArray();
                Assert.AreEqual(4, expanders.Length);
                Assert.IsTrue(expanders.All(static expander => expander.HeaderTemplate is not null));
                var expectedTextBrush = window.TryFindResource("TextBrush") as SolidColorBrush;
                Assert.IsNotNull(expectedTextBrush);
                var foregroundDetails = string.Join(
                    " | ",
                    expanders.Select(expander =>
                        $"{expander.Header}: {(expander.Foreground is SolidColorBrush brush ? brush.Color.ToString() : expander.Foreground?.ToString() ?? "null")}"));
                Assert.IsTrue(
                    expanders.All(expander =>
                        expander.Foreground is SolidColorBrush actual
                        && actual.Color == expectedTextBrush.Color),
                    $"预期颜色 {expectedTextBrush.Color}，实际：{foregroundDetails}");
            }
            finally
            {
                window?.Close();
            }
        });
    }

    [TestMethod]
    public void Parameter_scroll_does_not_resize_or_hide_the_fixed_audio_diagnostics_card()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                window.Show();
                Dispatcher.CurrentDispatcher.Invoke(DispatcherPriority.Background, new Action(static () => { }));

                var diagnostics = GetPrivateField<Border>(window, "AudioDiagnosticsCard");
                var previewRow = GetPrivateField<RowDefinition>(window, "PreviewRow");
                var parameterTabs = GetPrivateField<RowDefinition>(window, "ParameterTabRow");
                var scrollViewer = GetPrivateField<ScrollViewer>(window, "ParameterScrollViewer");

                Assert.AreEqual(Visibility.Visible, diagnostics.Visibility);
                Assert.AreEqual(GridUnitType.Pixel, previewRow.Height.GridUnitType);
                Assert.AreEqual(168d, previewRow.Height.Value, 0.1d);
                Assert.AreEqual(42d, parameterTabs.Height.Value, 0.1d);
                scrollViewer.ScrollToBottom();
                Dispatcher.CurrentDispatcher.Invoke(DispatcherPriority.Background, new Action(static () => { }));

                Assert.AreEqual(Visibility.Visible, diagnostics.Visibility);
                Assert.AreNotEqual(0d, previewRow.ActualHeight, 0.1d);
                Assert.AreEqual(42d, parameterTabs.Height.Value, 0.1d);
            }
            finally
            {
                window?.Close();
            }
        });
    }

    [TestMethod]
    public void Cycle_editors_are_present_for_video_audio_and_interlude_rules()
    {
        WpfTestApplicationHost.Run(() =>
        {
            MainWindow? window = null;
            try
            {
                window = new MainWindow();
                window.Show();

                Assert.AreEqual("5", GetPrivateField<TextBox>(window, "VideoCycleMinTextBox").Text);
                Assert.AreEqual("8", GetPrivateField<TextBox>(window, "VideoCycleMaxTextBox").Text);
                Assert.AreEqual("3", GetPrivateField<TextBox>(window, "AudioCycleMinTextBox").Text);
                Assert.AreEqual("5", GetPrivateField<TextBox>(window, "AudioCycleMaxTextBox").Text);
                Assert.IsNotNull(GetPrivateField<TextBox>(window, "InterludeCycleMinTextBox"));
                Assert.IsNotNull(GetPrivateField<TextBox>(window, "InterludeCycleMaxTextBox"));
                var interludeVolume = GetPrivateField<Slider>(window, "InterludeVolumeSlider");
                Assert.AreEqual(0d, interludeVolume.Minimum);
                Assert.AreEqual(100d, interludeVolume.Maximum);
                Assert.AreEqual(100d, interludeVolume.Value);
                Assert.AreEqual("100%", GetPrivateField<TextBlock>(window, "InterludeVolumeText").Text);
                interludeVolume.Value = 70;
                Assert.AreEqual(-18d, GetPrivateField<InterludeAudioConfig>(window, "_interludeConfig").VolumeDb);
                Assert.AreEqual("70%", GetPrivateField<TextBlock>(window, "InterludeVolumeText").Text);
                Assert.AreEqual(100d, GetPrivateField<ProgressBar>(window, "VideoEffectCycleProgressBar").Maximum);
                Assert.AreEqual(100d, GetPrivateField<ProgressBar>(window, "AudioEffectCycleProgressBar").Maximum);
                Assert.AreEqual(100d, GetPrivateField<ProgressBar>(window, "InterludeEffectCycleProgressBar").Maximum);
            }
            finally
            {
                window?.Close();
            }
        });
    }

    [TestMethod]
    public void Shell_text_scale_grows_only_on_larger_workspaces()
    {
        Assert.AreEqual(1d, MainWindow.CalculateResponsiveShellScale(new Size(1280, 800)), 0.001d);
        Assert.AreEqual(1d, MainWindow.CalculateResponsiveShellScale(new Size(1586, 992)), 0.001d);
        Assert.AreEqual(1.25d, MainWindow.CalculateResponsiveShellScale(new Size(2864, 1762)), 0.001d);
    }

    [TestMethod]
    public void Spectrum_display_gain_preserves_silence_and_lifts_weak_real_pcm()
    {
        Assert.AreEqual(0d, MainWindow.CalculateSpectrumDisplayLevel(0), 0.001d);
        Assert.IsTrue(MainWindow.CalculateSpectrumDisplayLevel(0.01f) > 0.1d);
        Assert.AreEqual(1d, MainWindow.CalculateSpectrumDisplayLevel(1), 0.001d);
    }

    private static void AssertSmallSwitch(CheckBox checkBox)
    {
        checkBox.ApplyTemplate();
        var templateRoot = VisualTreeHelper.GetChild(checkBox, 0) as Grid;
        Assert.IsNotNull(templateRoot);
        Assert.AreEqual(2, templateRoot.ColumnDefinitions.Count);
        Assert.AreEqual(36d, templateRoot.ColumnDefinitions[1].Width.Value);

        var track = checkBox.Template.FindName("SwitchTrack", checkBox) as Border;
        var thumb = checkBox.Template.FindName("SwitchThumb", checkBox) as Ellipse;
        Assert.IsNotNull(track);
        Assert.IsNotNull(thumb);
        Assert.AreEqual(28d, track.Width);
        Assert.AreEqual(16d, track.Height);
        Assert.AreEqual(12d, thumb.Width);
        Assert.AreEqual(12d, thumb.Height);
    }

    private static IEnumerable<T> FindVisualChildren<T>(DependencyObject root)
        where T : DependencyObject
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(root); index++)
        {
            var child = VisualTreeHelper.GetChild(root, index);
            if (child is T match)
            {
                yield return match;
            }

            foreach (var descendant in FindVisualChildren<T>(child))
            {
                yield return descendant;
            }
        }
    }

    private static CheckBox FindCheckBox(DependencyObject root, string automationId)
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(root); index++)
        {
            var child = VisualTreeHelper.GetChild(root, index);
            if (child is CheckBox checkBox
                && AutomationProperties.GetAutomationId(checkBox) == automationId)
            {
                return checkBox;
            }

            try
            {
                return FindCheckBox(child, automationId);
            }
            catch (InvalidOperationException)
            {
                // Continue searching sibling branches.
            }
        }

        throw new InvalidOperationException($"未找到自动化标识为 {automationId} 的 CheckBox。");
    }

    private static SourceMediaDto CreateMedia(string fileName, MediaKind kind = MediaKind.Video)
    {
        var path = Path.Combine(Path.GetTempPath(), "GpAutoLive.CSharp.App.MediaPool", fileName);
        return new(
            path,
            path,
            kind,
            MediaCompatibilityMode.Direct,
            fileName,
            1,
            1_000,
            null,
            null,
            kind is MediaKind.Video ? 320u : null,
            kind is MediaKind.Video ? 180u : null,
            kind is MediaKind.Video ? 30d : null,
            kind is MediaKind.Audio ? 48_000u : null,
            kind is MediaKind.Audio ? (ushort)2 : null,
            kind is MediaKind.Video ? "h264" : null,
            kind is MediaKind.Audio ? "pcm_s16le" : null,
            null,
            "disabled");
    }

    private static T GetPrivateField<T>(object instance, string fieldName) =>
        instance.GetType()
            .GetField(fieldName, BindingFlags.Instance | BindingFlags.NonPublic)
            ?.GetValue(instance) is T value
            ? value
            : throw new MissingFieldException(instance.GetType().FullName, fieldName);

    private static void InvokeVoidPrivate(object instance, string methodName, params object?[] arguments)
    {
        var method = instance.GetType()
            .GetMethod(methodName, BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new MissingMethodException(instance.GetType().FullName, methodName);
        method.Invoke(instance, arguments);
    }
}
