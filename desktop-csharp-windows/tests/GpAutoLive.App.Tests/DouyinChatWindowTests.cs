using System.Collections.Immutable;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using GpAutoLive.App.Features.Douyin;
using GpAutoLive.Contracts;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class DouyinChatWindowTests
{
    [TestMethod]
    public void Projection_keeps_existing_rows_and_clear_does_not_close_window()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var cleared = 0;
            var window = new DouyinChatWindow(() => cleared++);
            try
            {
                var first = new DouyinChatDisplayMessage("1", DateTimeOffset.UtcNow, "昵称", "中文😀", false);
                var second = new DouyinChatDisplayMessage("2", DateTimeOffset.UtcNow, "本人", "自己的消息", true);
                window.Refresh([first], "已连接", "12345");
                var list = (ListBox)window.FindName("MessagesList");
                Assert.AreSame(first, list.Items[0]);
                window.Refresh([first, second], "已断开", "12345");
                Assert.AreSame(first, list.Items[0]);
                Assert.AreEqual(2, list.Items.Count);
                Assert.IsTrue(VirtualizingPanel.GetIsVirtualizing(list));
                Assert.AreEqual(VirtualizationMode.Recycling, VirtualizingPanel.GetVirtualizationMode(list));
                Assert.AreEqual("已断开", ((TextBlock)window.FindName("ConnectionStatusText")).Text);
                ((Button)window.FindName("ClearButton")).RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
                Assert.AreEqual(1, cleared);
                Assert.AreEqual(0, list.Items.Count);
            }
            finally { window.Close(); }
        });
    }

    [TestMethod]
    public void Rolling_snapshot_replaces_expired_rows_and_empty_snapshot_clears_projection()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new DouyinChatWindow(() => { });
            try
            {
                var messages = Enumerable.Range(0, 501).Select(index => new DouyinChatDisplayMessage(
                    index.ToString(), DateTimeOffset.UtcNow, "昵称", new string('长', 300), false)).ToImmutableArray();
                window.Refresh(messages.Take(500).ToImmutableArray(), "已连接", "12345");
                window.Refresh(messages.Skip(1).ToImmutableArray(), "已连接", "12345");
                var list = (ListBox)window.FindName("MessagesList");
                Assert.AreEqual(500, list.Items.Count);
                Assert.AreSame(messages[1], list.Items[0]);
                Assert.AreSame(messages[500], list.Items[499]);
                window.Refresh([], "等待扫码", "67890");
                Assert.AreEqual(0, list.Items.Count);
                StringAssert.Contains(window.Title, "67890");
                Assert.AreEqual(Visibility.Visible, ((TextBlock)window.FindName("EmptyText")).Visibility);
            }
            finally { window.Close(); }
        });
    }

    [TestMethod]
    public void Reading_older_messages_keeps_scroll_position_until_return_to_latest()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new DouyinChatWindow(() => { });
            try
            {
                var messages = Enumerable.Range(0, 40).Select(index => new DouyinChatDisplayMessage(
                    index.ToString(), DateTimeOffset.UtcNow, "昵称", "第 " + index + " 条弹幕", false)).ToImmutableArray();
                window.Refresh(messages, "已连接", "12345");
                var root = (FrameworkElement)window.Content;
                root.Measure(new Size(340, 320));
                root.Arrange(new Rect(0, 0, 340, 320));
                root.UpdateLayout();
                var list = (ListBox)window.FindName("MessagesList");
                var scroll = FindScrollViewer(list);
                Assert.IsNotNull(scroll);
                scroll.ScrollToEnd();
                root.UpdateLayout();
                scroll.ScrollToHome();
                root.UpdateLayout();
                window.Refresh(messages.Add(new("40", DateTimeOffset.UtcNow, "新用户", "最新弹幕", false)), "已连接", "12345");
                root.UpdateLayout();
                Assert.IsTrue(scroll.VerticalOffset < 1, "新弹幕不能把正在阅读历史的用户推回底部。");
                ((Button)window.FindName("LatestButton")).RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
                root.UpdateLayout();
                Assert.IsTrue(scroll.VerticalOffset > 0, "回到最新应恢复跟随。");
            }
            finally { window.Close(); }
        });
    }

    private static ScrollViewer? FindScrollViewer(DependencyObject parent)
    {
        if (parent is ScrollViewer scroll) return scroll;
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(parent); index++)
            if (FindScrollViewer(VisualTreeHelper.GetChild(parent, index)) is { } child) return child;
        return null;
    }
}
