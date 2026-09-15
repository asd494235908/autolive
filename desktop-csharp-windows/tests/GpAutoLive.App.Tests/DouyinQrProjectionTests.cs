using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using GpAutoLive.Contracts;
using GpAutoLive.App.Features.Auth;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class DouyinQrProjectionTests
{
    [TestMethod]
    [DataRow(WindowsDouyinProbeHostState.Ready, true)]
    [DataRow(WindowsDouyinProbeHostState.Running, false)]
    public void Retained_login_allows_reconnect_only_after_room_disconnect(
        WindowsDouyinProbeHostState hostState, bool canReconnect)
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                var login = (LoginViewModel)typeof(MainWindow).GetField("_login", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(window)!;
                login.ApplyActivated("douyin-ui@example.com");
                var snapshot = new WindowsDouyinProbeHostSnapshot(hostState,
                    123, null, new DouyinLiveManager().Snapshot with { State = DouyinLiveState.LoggedIn },
                    null, "live_disconnected", 0, Authenticated: true);
                typeof(MainWindow).GetMethod("UpdateDouyinProjection", BindingFlags.Instance | BindingFlags.NonPublic)!
                    .Invoke(window, [snapshot]);
                Assert.AreEqual(canReconnect, ((Button)window.FindName("StartDouyinButton")).IsEnabled);
                Assert.AreEqual(canReconnect, ((TextBox)window.FindName("DouyinRoomTextBox")).IsEnabled);
                Assert.AreEqual(!canReconnect, ((Button)window.FindName("StopDouyinButton")).IsEnabled);
                Assert.AreEqual(Visibility.Collapsed, ((FrameworkElement)window.FindName("DouyinQrPanel")).Visibility);
                var status = ((TextBlock)window.FindName("DouyinStatusText")).Text;
                if (canReconnect) StringAssert.Contains(status, "直播间：已断开，可直接重连");
                else StringAssert.Contains(status, "正在解析直播间");
            }
            finally { window.Close(); }
        });
    }

    [TestMethod]
    [DataRow(DouyinLiveState.LoggedIn)]
    [DataRow(DouyinLiveState.Listening)]
    [DataRow(DouyinLiveState.Failed)]
    [DataRow(DouyinLiveState.Idle)]
    public void Qr_is_shown_only_while_waiting_for_scan(DouyinLiveState nextState)
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                var image = (Image)window.FindName("DouyinQrImage");
                var panel = (FrameworkElement)window.FindName("DouyinQrPanel");
                var pathField = typeof(MainWindow).GetField("_douyinQrImagePath", BindingFlags.Instance | BindingFlags.NonPublic)!;
                var project = typeof(MainWindow).GetMethod("UpdateDouyinQrProjection", BindingFlags.Instance | BindingFlags.NonPublic)!;
                var cachedImage = BitmapSource.Create(1, 1, 96, 96, PixelFormats.Bgra32, null, new byte[4], 4);
                image.Source = cachedImage;
                pathField.SetValue(window, "cached-qr.png");
                var snapshot = new WindowsDouyinProbeHostSnapshot(WindowsDouyinProbeHostState.Running,
                    123, null, new DouyinLiveManager().Snapshot with { State = DouyinLiveState.WaitingQr },
                    "cached-qr.png", null, 0);
                project.Invoke(window, [snapshot]);
                Assert.AreEqual(Visibility.Visible, panel.Visibility);
                Assert.AreSame(cachedImage, image.Source);

                project.Invoke(window, [snapshot with { Douyin = snapshot.Douyin with { State = nextState } }]);
                Assert.AreEqual(Visibility.Collapsed, panel.Visibility);
                Assert.IsNull(image.Source);
                Assert.IsNull(pathField.GetValue(window));
            }
            finally { window.Close(); }
        });
    }
}
