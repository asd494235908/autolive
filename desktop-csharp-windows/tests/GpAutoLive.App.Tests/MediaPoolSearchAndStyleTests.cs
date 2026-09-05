using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
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

                var button = GetPrivateField<Button>(window, "ImportButton");
                Assert.AreEqual(32d, button.Height);
                Assert.AreEqual(HorizontalAlignment.Stretch, button.HorizontalAlignment);
                Assert.AreEqual(HorizontalAlignment.Center, button.HorizontalContentAlignment);
                Assert.AreEqual(VerticalAlignment.Center, button.VerticalContentAlignment);
                Assert.IsInstanceOfType(button.Content, typeof(StackPanel));
            }
            finally
            {
                window?.Close();
            }
        });
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
