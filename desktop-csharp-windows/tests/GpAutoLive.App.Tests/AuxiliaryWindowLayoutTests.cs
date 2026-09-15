using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using GpAutoLive.App.Features.Auth;
using GpAutoLive.App.Features.Settings;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class AuxiliaryWindowLayoutTests
{
    [TestMethod]
    public void Login_card_stays_inside_narrow_viewport_and_scrolls_to_all_actions()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var view = new LoginView();
            view.Measure(new Size(360, 280));
            view.Arrange(new Rect(0, 0, 360, 280));
            view.UpdateLayout();

            var scroll = Descendants<ScrollViewer>(view).FirstOrDefault();
            Assert.IsNotNull(scroll, "登录内容必须可纵向滚动，避免小工作区裁切。 ");
            var account = (TextBox)view.FindName("AccountBox");
            var left = account.TranslatePoint(new Point(), view).X;
            Assert.IsTrue(left >= 0 && left + account.ActualWidth <= 360);
            Assert.IsTrue(scroll.ScrollableHeight > 0);
            scroll.ScrollToEnd();
            view.UpdateLayout();
            var activation = Descendants<Button>(view).Single(button => Equals(button.Content, "设备需要激活？"));
            var bottom = activation.TranslatePoint(new Point(0, activation.ActualHeight), view).Y;
            Assert.IsTrue(bottom <= 280, $"激活入口底部 {bottom} 超过视口。");
        });
    }

    [TestMethod]
    public void Settings_footer_keeps_status_and_save_actions_separate_at_small_size()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new SettingsWindow(DesktopSettingsDraft.From(UserPreferences.Defaults));
            try
            {
                Assert.IsTrue(window.MinWidth <= 360 && window.MinHeight <= 320);
                Assert.AreNotEqual(ResizeMode.NoResize, window.ResizeMode);
                var root = (FrameworkElement)window.Content;
                root.Measure(new Size(360, 280));
                root.Arrange(new Rect(0, 0, 360, 280));
                root.UpdateLayout();
                var status = (TextBlock)window.FindName("StatusText");
                status.Text = "请选择下次输出入口，确认后保存本机设置";
                root.UpdateLayout();
                var cancel = Descendants<Button>(root).Single(button => Equals(button.Content, "取消"));
                var statusRight = status.TranslatePoint(new Point(status.ActualWidth, 0), root).X;
                var buttonLeft = cancel.TranslatePoint(new Point(), root).X;
                Assert.IsTrue(statusRight <= buttonLeft, $"提示右侧 {statusRight} 超过按钮左侧 {buttonLeft}。");
            }
            finally
            {
                window.Close();
            }
        });
    }

    private static IEnumerable<T> Descendants<T>(DependencyObject root) where T : DependencyObject
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(root); index++)
        {
            var child = VisualTreeHelper.GetChild(root, index);
            if (child is T match)
            {
                yield return match;
            }

            foreach (var descendant in Descendants<T>(child))
            {
                yield return descendant;
            }
        }
    }
}
