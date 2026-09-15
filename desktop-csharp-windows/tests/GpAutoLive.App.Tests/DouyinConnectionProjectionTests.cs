using System.Reflection;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using GpAutoLive.App.Features.Auth;
using GpAutoLive.App.Features.Douyin;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class DouyinConnectionProjectionTests
{
    [TestMethod]
    public void Login_and_room_projection_fit_compact_chat_window_with_manual_send_visible()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var main = new MainWindow();
            var chat = new DouyinChatWindow(() => { });
            try
            {
                typeof(MainWindow).GetField("_douyinChatWindow", Flags)!.SetValue(main, chat);
                Project(main, new(WindowsDouyinProbeHostState.Ready, null, null,
                    new DouyinLiveManager().Snapshot, null, null, 0));
                var root = (FrameworkElement)chat.Content;
                root.Measure(new Size(308, 265));
                root.Arrange(new Rect(0, 0, 308, 265));
                root.UpdateLayout();
                var status = (TextBlock)chat.FindName("ConnectionStatusText");
                StringAssert.Contains(status.Text, "抖音登录：");
                StringAssert.Contains(status.Text, "\n直播间：");
                Assert.IsTrue(status.ActualHeight > 0);
                Assert.IsTrue(status.ActualHeight <= 40, "小窗口只显示两行状态，详细原因放在提示中。");
                StringAssert.Contains((string)status.ToolTip, "本次运行尚未登录");
                var send = (Button)chat.FindName("SendChatButton");
                Assert.IsTrue(send.ActualHeight > 0);
                Assert.IsTrue(send.TranslatePoint(new Point(0, send.ActualHeight), root).Y <= 265);
                var output = Environment.GetEnvironmentVariable("AUTOLIVE_GUI_SCREENSHOT_DIR");
                if (!string.IsNullOrWhiteSpace(output))
                {
                    Directory.CreateDirectory(output);
                    var bitmap = new RenderTargetBitmap(340, 297, 96, 96, PixelFormats.Pbgra32);
                    bitmap.Render(root);
                    var encoder = new PngBitmapEncoder();
                    encoder.Frames.Add(BitmapFrame.Create(bitmap));
                    using var file = File.Create(Path.Combine(output, "douyin-chat-compact-login.png"));
                    encoder.Save(file);
                }
            }
            finally { chat.Close(); main.Close(); }
        });
    }

    [TestMethod]
    [DataRow(WindowsDouyinLoginClearReason.NotAuthenticatedThisRun, "本次运行尚未登录")]
    [DataRow(WindowsDouyinLoginClearReason.AuthenticationExpired, "平台登录已失效")]
    [DataRow(WindowsDouyinLoginClearReason.LoginFailed, "扫码未完成或失败")]
    [DataRow(WindowsDouyinLoginClearReason.ProcessExited, "辅助进程已退出")]
    [DataRow(WindowsDouyinLoginClearReason.LocalCommunicationError, "本地通信异常")]
    [DataRow(WindowsDouyinLoginClearReason.ExplicitStop, "已退出抖音登录")]
    public void Login_clear_reason_is_separate_from_room_connection(WindowsDouyinLoginClearReason reason, string expected)
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                Project(window, new(WindowsDouyinProbeHostState.Ready, null, null,
                    new DouyinLiveManager().Snapshot, null, null, 0, LoginClearReason: reason));
                var auth = (TextBlock)window.FindName("DouyinAuthStatusText");
                Assert.IsNotNull(auth);
                StringAssert.Contains(auth.Text, expected);
                StringAssert.Contains(((TextBlock)window.FindName("DouyinStatusText")).Text, "直播间：");
                Assert.AreEqual(Visibility.Collapsed, ((FrameworkElement)window.FindName("DouyinQrPanel")).Visibility);
            }
            finally { window.Close(); }
        });
    }

    [TestMethod]
    public void Same_room_connected_is_idempotent_but_room_edit_and_active_send_stay_gated()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            var pending = new TaskCompletionSource<WindowsDouyinChatSendResult>();
            try
            {
                ((LoginViewModel)typeof(MainWindow).GetField("_login", Flags)!.GetValue(window)!).ApplyActivated("connection-ui@example.com");
                typeof(MainWindow).GetField("_douyinDisplayRoomId", Flags)!.SetValue(window, "265580978886");
                var room = (TextBox)window.FindName("DouyinRoomTextBox");
                room.Text = "https://live.douyin.com/265580978886";
                var snapshot = new WindowsDouyinProbeHostSnapshot(WindowsDouyinProbeHostState.Running, 123, null,
                    new DouyinLiveManager().Snapshot with { State = DouyinLiveState.Listening },
                    null, null, 0, Authenticated: true, LoginClearReason: WindowsDouyinLoginClearReason.None);
                Project(window, snapshot);
                var connect = (Button)window.FindName("StartDouyinButton");
                Assert.IsTrue(connect.IsEnabled);
                Assert.AreEqual("已连接", connect.Content);
                Assert.IsFalse(room.IsEnabled);
                StringAssert.Contains(((TextBlock)window.FindName("DouyinAuthStatusText")).Text, "已登录");
                typeof(MainWindow).GetField("_douyinManualSendTask", Flags)!.SetValue(window, pending.Task);
                Project(window, snapshot);
                Assert.IsFalse(connect.IsEnabled);
                pending.SetResult(new(DouyinSendOutcome.NotSent, "未发送"));
                room.Text = "12345";
                Project(window, snapshot);
                Assert.IsFalse(connect.IsEnabled, "已连接时不能直接改目标房间或重新登录。");
            }
            finally { pending.TrySetResult(new(DouyinSendOutcome.NotSent, "未发送")); window.Close(); }
        });
    }

    private const BindingFlags Flags = BindingFlags.Instance | BindingFlags.NonPublic;
    [TestMethod]
    public void Clicking_connected_room_does_not_validate_or_persist_unapplied_reply_changes()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                ((LoginViewModel)typeof(MainWindow).GetField("_login", Flags)!.GetValue(window)!).ApplyActivated("idempotent-ui@example.com");
                var manager = (DouyinLiveManager)typeof(MainWindow).GetField("_douyinLive", Flags)!.GetValue(window)!;
                Assert.IsTrue(manager.TryStart(new DouyinLiveConfig { RoomId = "265580978886", Enabled = false, Replies = [] }).IsSuccess);
                Assert.IsTrue(manager.MarkLoggedIn().IsSuccess);
                Assert.IsTrue(manager.MarkRoomResolved().IsSuccess);
                Assert.IsTrue(manager.BeginListening().IsSuccess);
                var generation = manager.Snapshot.Generation;
                var host = (WindowsDouyinProbeHost)typeof(MainWindow).GetField("_douyinProbeHost", Flags)!.GetValue(window)!;
                typeof(WindowsDouyinProbeHost).GetField("_authenticated", Flags)!.SetValue(host, true);
                typeof(WindowsDouyinProbeHost).GetField("_state", Flags)!.SetValue(host, WindowsDouyinProbeHostState.Running);
                typeof(MainWindow).GetField("_douyinDisplayRoomId", Flags)!.SetValue(window, "265580978886");
                ((TextBox)window.FindName("DouyinRoomTextBox")).Text = "https://live.douyin.com/265580978886";
                ((CheckBox)window.FindName("DouyinEnabledCheckBox")).IsChecked = true;
                ((TextBox)window.FindName("DouyinRepliesTextBox")).Text = string.Empty;
                Project(window, host.Snapshot);
                ((Button)window.FindName("StartDouyinButton")).RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
                StringAssert.Contains(((TextBlock)window.FindName("DouyinStatusText")).Text, "无需重复连接");
                Assert.AreEqual(generation, manager.Snapshot.Generation);
                Assert.AreEqual(DouyinLiveState.Listening, manager.Snapshot.State);
            }
            finally { window.Close(); }
        });
    }

    private static void Project(MainWindow window, WindowsDouyinProbeHostSnapshot snapshot) =>
        typeof(MainWindow).GetMethod("UpdateDouyinProjection", Flags)!.Invoke(window, [snapshot]);
}
