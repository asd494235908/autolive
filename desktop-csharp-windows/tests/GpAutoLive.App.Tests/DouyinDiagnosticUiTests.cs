using System.Reflection;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class DouyinDiagnosticUiTests
{
    [TestMethod]
    [DataRow(WindowsDouyinDiagnosticLogState.Ready, "已保存")]
    [DataRow(WindowsDouyinDiagnosticLogState.WriteFailed, "保存失败")]
    public void Existing_log_path_remains_copyable_but_failed_write_is_never_shown_as_success(
        WindowsDouyinDiagnosticLogState state, string expected)
    {
        var path = Path.Combine(Path.GetTempPath(), "gpautolive-diagnostic-ui-" + Guid.NewGuid().ToString("N") + ".ndjson");
        File.WriteAllText(path, "{\"event\":\"layout_fixture\"}\n");
        try
        {
            WpfTestApplicationHost.Run(() =>
            {
                var window = new MainWindow();
                try
                {
                    Project(window, new(WindowsDouyinProbeHostState.Ready, null, null,
                        new DouyinLiveManager().Snapshot, null, null, 0, DiagnosticLogPath: path, DiagnosticLogState: state));
                    var status = (TextBlock)window.FindName("DouyinDiagnosticStatusText");
                    StringAssert.Contains(status.Text, expected);
                    if (state == WindowsDouyinDiagnosticLogState.WriteFailed)
                        Assert.IsFalse(status.Text.Contains("已保存", StringComparison.Ordinal));
                    var location = (FrameworkElement)window.FindName("DouyinDiagnosticLocationPanel");
                    var copy = (Button)window.FindName("CopyDouyinDiagnosticPathButton");
                    var text = (TextBlock)window.FindName("DouyinDiagnosticPathText");
                    Assert.IsTrue(copy.IsEnabled);
                    Assert.AreEqual(path, text.Text);
                    Assert.AreEqual(Visibility.Visible, location.Visibility);
                    location.Measure(new Size(200, 28));
                    location.Arrange(new Rect(0, 0, 200, 28));
                    location.UpdateLayout();
                    Assert.IsTrue(text.ActualWidth > 0);
                    Assert.IsTrue(copy.TranslatePoint(new Point(copy.ActualWidth, 0), location).X <= 200);
                    Assert.IsTrue(text.TranslatePoint(new Point(text.ActualWidth, 0), location).X
                        <= copy.TranslatePoint(new Point(), location).X);
                    Assert.AreEqual(TextTrimming.CharacterEllipsis, text.TextTrimming);
                }
                finally { window.Close(); }
            });
        }
        finally { File.Delete(path); }
    }

    [TestMethod]
    [DataRow(WindowsDouyinDiagnosticLogState.Ready, "暂不可用")]
    [DataRow(WindowsDouyinDiagnosticLogState.WriteFailed, "保存失败")]
    public void Missing_log_path_disables_copy_and_reports_actual_availability(WindowsDouyinDiagnosticLogState state, string expected)
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                Project(window, new(WindowsDouyinProbeHostState.Ready, null, null,
                    new DouyinLiveManager().Snapshot, null, null, 0,
                    DiagnosticLogPath: Path.Combine(Path.GetTempPath(), Guid.NewGuid().ToString("N") + ".ndjson"), DiagnosticLogState: state));
                StringAssert.Contains(((TextBlock)window.FindName("DouyinDiagnosticStatusText")).Text, expected);
                Assert.IsFalse(((Button)window.FindName("CopyDouyinDiagnosticPathButton")).IsEnabled);
                Assert.AreEqual(string.Empty, ((TextBlock)window.FindName("DouyinDiagnosticPathText")).Text);
            }
            finally { window.Close(); }
        });
    }

    [TestMethod]
    public void No_log_file_does_not_claim_saved_or_offer_copy()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var window = new MainWindow();
            try
            {
                Project(window, new(WindowsDouyinProbeHostState.Ready, null, null,
                    new DouyinLiveManager().Snapshot, null, null, 0));
                var status = (TextBlock)window.FindName("DouyinDiagnosticStatusText");
                Assert.IsNotNull(status, "抖音卡片应显示诊断日志当前保存状态。");
                StringAssert.Contains(status.Text, "尚未创建");
                Assert.IsFalse(((Button)window.FindName("CopyDouyinDiagnosticPathButton")).IsEnabled);
                Assert.AreEqual(Visibility.Collapsed, ((FrameworkElement)window.FindName("DouyinDiagnosticLocationPanel")).Visibility);
            }
            finally { window.Close(); }
        });
    }

    private static void Project(MainWindow window, WindowsDouyinProbeHostSnapshot snapshot) =>
        typeof(MainWindow).GetMethod("UpdateDouyinProjection", BindingFlags.Instance | BindingFlags.NonPublic)!.Invoke(window, [snapshot]);
}
