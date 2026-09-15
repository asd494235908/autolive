using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using System.Reflection;
using GpAutoLive.App.Features.Auth;
using GpAutoLive.App.Features.Douyin;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class DouyinChatManualSendTests
{
    [TestMethod]
    public void Main_window_keeps_send_state_across_chat_window_reopen_and_hides_old_generation_result()
    {
        WpfTestApplicationHost.Run(() =>
        {
            const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic;
            var main = new MainWindow();
            var pending = new TaskCompletionSource<WindowsDouyinChatSendResult>();
            var chat = new DouyinChatWindow(() => { });
            try
            {
                ((LoginViewModel)typeof(MainWindow).GetField("_login", flags)!.GetValue(main)!).ApplyActivated("manual-ui@example.com");
                var chatField = typeof(MainWindow).GetField("_douyinChatWindow", flags)!;
                var project = typeof(MainWindow).GetMethod("UpdateDouyinProjection", flags)!;
                typeof(MainWindow).GetField("_douyinManualSendTask", flags)!.SetValue(main, pending.Task);
                typeof(MainWindow).GetField("_douyinManualSendGeneration", flags)!.SetValue(main, (ulong?)1);
                var snapshot = new WindowsDouyinProbeHostSnapshot(WindowsDouyinProbeHostState.Running, 123, null,
                    new DouyinLiveManager().Snapshot with { State = DouyinLiveState.Listening, Generation = 1 },
                    null, null, 0, Authenticated: true);
                chatField.SetValue(main, chat);
                project.Invoke(main, [snapshot]);
                StringAssert.Contains(((TextBlock)chat.FindName("ManualSendStatusText")).Text, "正在发送");
                chat.Close();
                Assert.IsFalse(pending.Task.IsCompleted);
                chat = new DouyinChatWindow(() => { });
                chatField.SetValue(main, chat);
                project.Invoke(main, [snapshot]);
                StringAssert.Contains(((TextBlock)chat.FindName("ManualSendStatusText")).Text, "正在发送");
                pending.SetResult(new(DouyinSendOutcome.Accepted, "平台已接受提交，请核对本人回显"));
                project.Invoke(main, [snapshot]);
                StringAssert.Contains(((TextBlock)chat.FindName("ManualSendStatusText")).Text, "平台已接受提交");
                project.Invoke(main, [snapshot with { Douyin = snapshot.Douyin with { Generation = 2 } }]);
                Assert.IsFalse(((TextBlock)chat.FindName("ManualSendStatusText")).Text.Contains("平台已接受提交", StringComparison.Ordinal));
            }
            finally
            {
                pending.TrySetResult(new(DouyinSendOutcome.NotSent, "未发送"));
                chat.Close();
                main.Close();
            }
        });
    }

    [TestMethod]
    public void Manual_send_requires_connection_and_ignores_duplicate_clicks()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var calls = 0;
            var completion = new TaskCompletionSource<WindowsDouyinChatSendResult?>(TaskCreationOptions.RunContinuationsAsynchronously);
            var window = new DouyinChatWindow(() => { }, text => { calls++; return completion.Task; });
            try
            {
                var input = (TextBox)window.FindName("ManualChatTextBox");
                var send = (Button)window.FindName("SendChatButton");
                input.Text = "大家好";
                Assert.IsFalse(send.IsEnabled);
                Assert.IsFalse(send.IsDefault, "Enter 不能默认发送弹幕。");
                window.RefreshSendState(true, false, null);
                Assert.IsTrue(send.IsEnabled);
                send.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
                send.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
                Assert.AreEqual(1, calls);
                Assert.IsFalse(send.IsEnabled);
                completion.SetResult(new(DouyinSendOutcome.Accepted, "平台已接受提交，请核对本人回显"));
                Dispatcher.CurrentDispatcher.Invoke(() => { }, DispatcherPriority.ApplicationIdle);
                window.RefreshSendState(true, false, "平台已接受提交，请核对本人回显");
                Assert.AreEqual(string.Empty, input.Text);
                StringAssert.Contains(((TextBlock)window.FindName("ManualSendStatusText")).Text, "平台已接受提交");
            }
            finally { completion.TrySetResult(new(DouyinSendOutcome.NotSent, "未发送")); window.Close(); }
        });
    }

    [TestMethod]
    public void Unknown_result_keeps_draft_without_retry_and_input_stays_visible_in_small_window()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var calls = 0;
            var window = new DouyinChatWindow(() => { }, _ =>
            {
                calls++;
                return Task.FromResult<WindowsDouyinChatSendResult?>(new(DouyinSendOutcome.OutcomeUnknown, "发送结果未知，请先核对本人弹幕"));
            });
            try
            {
                var input = (TextBox)window.FindName("ManualChatTextBox");
                var send = (Button)window.FindName("SendChatButton");
                window.RefreshSendState(true, false, null);
                input.Text = "";
                Assert.IsFalse(send.IsEnabled);
                input.Text = "大家好";
                send.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
                window.RefreshSendState(true, false, "发送结果未知，请先核对本人弹幕");
                Assert.AreEqual(1, calls);
                Assert.AreEqual("大家好", input.Text);
                StringAssert.Contains(((TextBlock)window.FindName("ManualSendStatusText")).Text, "结果未知");
                var root = (FrameworkElement)window.Content;
                root.Measure(new Size(308, 265));
                root.Arrange(new Rect(0, 0, 308, 265));
                root.UpdateLayout();
                Assert.IsTrue(input.ActualHeight > 0);
                Assert.IsTrue(send.TranslatePoint(new Point(0, send.ActualHeight), root).Y <= 265);
                Assert.IsTrue(input.TranslatePoint(new Point(), root).X >= 0);
                window.RefreshSendState(false, false, "已断开");
                Assert.IsFalse(send.IsEnabled);
            }
            finally { window.Close(); }
        });
    }
}
