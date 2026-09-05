using System.ComponentModel;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using Microsoft.Win32;

namespace GpAutoLive.Installer;

public partial class MainWindow : Window
{
    private readonly PowerShellInstallerRunner _runner;
    private CancellationTokenSource? _operationCancellation;
    private Task? _runningTask;
    private bool _closeRequested;
    private bool _allowClose;
    private bool _canCancel;

    public MainWindow()
    {
        InitializeComponent();
        _runner = new PowerShellInstallerRunner(Path.Combine(AppContext.BaseDirectory, "tools"));
        OperationBox.ItemsSource = new[]
        {
            new OperationOption(InstallerOperation.CheckRuntime, "探测 .NET 10 Runtime"),
            new OperationOption(InstallerOperation.VerifyPackage, "校验发布包"),
            new OperationOption(InstallerOperation.CheckInstallState, "刷新安装状态"),
            new OperationOption(InstallerOperation.InstallOrUpgrade, "安装或升级"),
            new OperationOption(InstallerOperation.Rollback, "回滚活动版本"),
            new OperationOption(InstallerOperation.UninstallVersion, "删除非活动版本")
        };
        OperationBox.SelectedIndex = 2;
        InstallRootBox.Text = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "GpAutoLive.CSharp.Windows.Install");
    }

    private async void OnRun(object sender, RoutedEventArgs e)
    {
        if (_runningTask is not null)
        {
            return;
        }

        _operationCancellation = new CancellationTokenSource();
        SetBusy(true, canCancel: true);
        _runningTask = RunSelectedOperationAsync(_operationCancellation.Token);
        try
        {
            await _runningTask;
        }
        finally
        {
            _operationCancellation.Dispose();
            _operationCancellation = null;
            _runningTask = null;
            SetBusy(false, canCancel: false);
        }
    }

    private async Task RunSelectedOperationAsync(CancellationToken cancellationToken)
    {
        InstallerRequest request;
        InstallerScriptInvocation invocation;
        try
        {
            request = CreateRequest();
            invocation = InstallerCommandFactory.Create(request);
        }
        catch (ArgumentException exception)
        {
            ResultBox.Text = exception.Message;
            return;
        }
        catch (InvalidOperationException exception)
        {
            ResultBox.Text = exception.Message;
            return;
        }

        if (invocation.IsMutation)
        {
            ResultBox.Text = "正在执行强制安全预演…";
            var previewRequest = request with { WhatIf = true };
            var preview = await RunAndFormatAsync(previewRequest, cancellationToken);
            ResultBox.Text = "预演结果" + Environment.NewLine + preview.Text;
            if (!preview.IsHealthy || cancellationToken.IsCancellationRequested)
            {
                return;
            }

            var confirmation = MessageBox.Show(
                GetConfirmationText(request.Operation),
                "确认真实变更",
                MessageBoxButton.OKCancel,
                MessageBoxImage.Warning,
                MessageBoxResult.Cancel);
            if (confirmation != MessageBoxResult.OK)
            {
                ResultBox.AppendText(Environment.NewLine + "用户取消了真实变更。 ");
                return;
            }

            SetBusy(true, canCancel: false);
        }

        ResultBox.Text = invocation.IsMutation ? "正在执行真实变更…" : "正在执行…";
        var summary = await RunAndFormatAsync(request, cancellationToken);
        ResultBox.Text = summary.Text;
        if (!invocation.IsMutation || cancellationToken.IsCancellationRequested)
        {
            return;
        }

        ResultBox.AppendText(Environment.NewLine + Environment.NewLine + "正在复核安装状态…");
        var stateRequest = request with
        {
            Operation = InstallerOperation.CheckInstallState,
            WhatIf = false
        };
        var state = await RunAndFormatAsync(stateRequest, cancellationToken);
        ResultBox.AppendText(Environment.NewLine + state.Text);
        if (!state.IsHealthy)
        {
            ResultBox.AppendText(Environment.NewLine + "真实变更已返回，但安装状态复核未通过；请勿启动或继续删除版本。 ");
        }
        else if (!summary.IsHealthy)
        {
            ResultBox.AppendText(Environment.NewLine + "真实变更脚本失败，但当前安装状态仍可验证；请根据活动版本决定是否重试。 ");
        }
    }

    private async Task<InstallerResultSummary> RunAndFormatAsync(
        InstallerRequest request,
        CancellationToken cancellationToken)
    {
        try
        {
            var invocation = InstallerCommandFactory.Create(request);
            var result = await _runner.RunAsync(invocation, cancellationToken);
            return InstallerResultFormatter.Format(request.Operation, result);
        }
        catch (Exception exception) when (
            exception is ArgumentException
            or InvalidOperationException
            or IOException
            or UnauthorizedAccessException)
        {
            return new(false, exception.Message);
        }
    }

    private InstallerRequest CreateRequest()
    {
        if (OperationBox.SelectedItem is not OperationOption option)
        {
            throw new InvalidOperationException("请选择安装维护操作。 ");
        }
        return new(
            option.Operation,
            PackageRootBox.Text,
            InstallRootBox.Text,
            VersionBox.Text,
            RequireSignedBox.IsChecked == true,
            WhatIfBox.IsChecked == true);
    }

    private void OnOperationChanged(object sender, SelectionChangedEventArgs e)
    {
        if (OperationBox.SelectedItem is not OperationOption option
            || InstallRootBox is null)
        {
            return;
        }

        var needsPackage = option.Operation is InstallerOperation.VerifyPackage or InstallerOperation.InstallOrUpgrade;
        var needsInstall = option.Operation is InstallerOperation.CheckInstallState
            or InstallerOperation.InstallOrUpgrade
            or InstallerOperation.Rollback
            or InstallerOperation.UninstallVersion;
        var needsVersion = option.Operation is InstallerOperation.InstallOrUpgrade
            or InstallerOperation.Rollback
            or InstallerOperation.UninstallVersion;
        PackageRootBox.IsEnabled = needsPackage;
        BrowsePackageRootButton.IsEnabled = needsPackage;
        InstallRootBox.IsEnabled = needsInstall;
        BrowseInstallRootButton.IsEnabled = needsInstall;
        VersionBox.IsEnabled = needsVersion;
        RequireSignedBox.IsEnabled = option.Operation is InstallerOperation.VerifyPackage
            or InstallerOperation.InstallOrUpgrade
            or InstallerOperation.Rollback;
        WhatIfBox.IsEnabled = option.Operation is InstallerOperation.CheckInstallState
            or InstallerOperation.InstallOrUpgrade
            or InstallerOperation.Rollback
            or InstallerOperation.UninstallVersion;
        OperationHelpText.Text = option.Operation switch
        {
            InstallerOperation.CheckRuntime => "只读探测；缺失时提示安装要求，不自动联网。 ",
            InstallerOperation.VerifyPackage => "复用发布包核验器检查清单、哈希、许可材料和可选签名门禁。 ",
            InstallerOperation.CheckInstallState => "只读检查 current.json、活动版本、回滚候选和所有版本清单。 ",
            InstallerOperation.InstallOrUpgrade => "版本目录不可覆盖；真实执行前会强制 WhatIf 并再次确认。 ",
            InstallerOperation.Rollback => "版本留空时回滚到 previous_version；只切换已验证的活动指针。 ",
            InstallerOperation.UninstallVersion => "只删除指定非活动版本，保留 current.json、活动版本和用户配置。 ",
            _ => string.Empty
        };
    }

    private void OnBrowseFolder(object sender, RoutedEventArgs e)
    {
        if (sender is not Button button)
        {
            return;
        }

        var target = string.Equals((string?)button.Tag, "Package", StringComparison.Ordinal)
            ? PackageRootBox
            : InstallRootBox;
        var dialog = new OpenFolderDialog
        {
            Title = "选择专用目录",
            Multiselect = false
        };
        if (Directory.Exists(target.Text))
        {
            dialog.InitialDirectory = target.Text;
        }
        if (dialog.ShowDialog(this) == true)
        {
            target.Text = dialog.FolderName;
        }
    }

    private void OnCancel(object sender, RoutedEventArgs e)
    {
        if (_canCancel)
        {
            _operationCancellation?.Cancel();
        }
    }

    private async void OnWindowClosing(object? sender, CancelEventArgs e)
    {
        if (_allowClose || _runningTask is null)
        {
            return;
        }
        e.Cancel = true;
        if (!_canCancel)
        {
            MessageBox.Show(
                "真实写入已经开始。现有脚本不支持安全中断，请等待完成后再关闭。 ",
                "GpAutoLive 安装维护助手",
                MessageBoxButton.OK,
                MessageBoxImage.Information);
            return;
        }
        if (_closeRequested)
        {
            return;
        }

        _closeRequested = true;
        _operationCancellation?.Cancel();
        try
        {
            await _runningTask;
        }
        finally
        {
            _allowClose = true;
            Close();
        }
    }

    private void SetBusy(bool busy, bool canCancel)
    {
        _canCancel = busy && canCancel;
        RunButton.IsEnabled = !busy;
        OperationBox.IsEnabled = !busy;
        CancelButton.IsEnabled = _canCancel;
        BusyIndicator.Visibility = busy ? Visibility.Visible : Visibility.Collapsed;
    }

    private static string GetConfirmationText(InstallerOperation operation) => operation switch
    {
        InstallerOperation.InstallOrUpgrade => "安全预演已通过。是否安装并激活这个版本？",
        InstallerOperation.Rollback => "安全预演已通过。是否切换活动版本？",
        InstallerOperation.UninstallVersion => "安全预演已通过。是否永久删除这个非活动版本？此操作不会删除用户配置。 ",
        _ => "是否执行真实变更？"
    };

    private sealed record OperationOption(InstallerOperation Operation, string Label);
}
