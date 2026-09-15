using System.IO;
using System.Runtime.InteropServices;
using System.Windows;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private void UpdateDouyinDiagnosticProjection(WindowsDouyinProbeHostSnapshot snapshot)
    {
        var hasFile = File.Exists(snapshot.DiagnosticLogPath);
        DouyinDiagnosticStatusText.Text = snapshot.DiagnosticLogState switch
        {
            WindowsDouyinDiagnosticLogState.WriteFailed => hasFile
                ? "诊断日志：保存失败；现有文件可能不完整"
                : "诊断日志：保存失败；请检查目录权限或磁盘空间",
            WindowsDouyinDiagnosticLogState.Ready when hasFile => "诊断日志：已保存（不含弹幕正文或凭据）",
            WindowsDouyinDiagnosticLogState.Ready => "诊断日志：文件暂不可用；请重新连接后查看",
            _ => "诊断日志：尚未创建，开始连接后记录"
        };
        DouyinDiagnosticPathText.Text = hasFile ? snapshot.DiagnosticLogPath : string.Empty;
        DouyinDiagnosticLocationPanel.Visibility = hasFile ? Visibility.Visible : Visibility.Collapsed;
        CopyDouyinDiagnosticPathButton.IsEnabled = hasFile && !_isClosing;
    }

    private void CopyDouyinDiagnosticPathButton_Click(object sender, RoutedEventArgs e)
    {
        var path = _douyinProbeHost.Snapshot.DiagnosticLogPath;
        if (!File.Exists(path))
        {
            UpdateDouyinDiagnosticProjection(_douyinProbeHost.Snapshot);
            _state.SetStatus("诊断日志文件暂不可用，未复制路径");
            return;
        }

        try
        {
            Clipboard.SetText(path!);
            _state.SetStatus("抖音诊断日志路径已复制到剪贴板");
        }
        catch (ExternalException)
        {
            _state.SetStatus("系统剪贴板当前不可用，未复制日志路径");
        }
        catch (System.Threading.ThreadStateException)
        {
            _state.SetStatus("系统剪贴板当前不可用，未复制日志路径");
        }
    }
}
