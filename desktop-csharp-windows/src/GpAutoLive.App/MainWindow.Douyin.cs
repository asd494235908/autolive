using System.IO;
using System.Windows;
using GpAutoLive.App.Features.Douyin;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Configuration;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private string? _douyinQrImagePath;
    private DouyinChatWindow? _douyinChatWindow;
    private string? _douyinDisplayRoomId;
    private Task? _douyinAuthorizationStopTask;
    private Task<WindowsDouyinChatSendResult>? _douyinManualSendTask;
    private ulong? _douyinManualSendGeneration;

    private async void StartDouyinButton_Click(object sender, RoutedEventArgs e)
    {
        if (_isClosing || _douyinManualSendTask is { IsCompleted: false }) return;
        if (!_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        if (IsCurrentDouyinRoomConnected(_douyinProbeHost.Snapshot))
        {
            DouyinStatusText.Text = "直播间：当前直播间已连接，无需重复连接";
            _state.SetStatus(DouyinStatusText.Text);
            return;
        }

        if (!DouyinConfigDraft.TryCreate(
                DouyinEnabledCheckBox.IsChecked == true,
                DouyinRoomTextBox.Text,
                DouyinRepliesTextBox.Text,
                DouyinQueueCapacityTextBox.Text,
                out var config,
                out var error))
        {
            DouyinStatusText.Text = error ?? "抖音 M1 配置无效";
            _state.SetStatus(DouyinStatusText.Text);
            return;
        }

        if (DouyinProbeRequestFactory.TryCreate(
                config!,
                readEnvironment: null,
                temporaryDirectory: null,
                out var probeRequest,
                out var probeError,
                out _)
            && probeRequest is not null)
        {
            var hostResult = await _douyinProbeHost.StartAsync(probeRequest, _windowCancellation.Token).ConfigureAwait(true);
            if (hostResult.IsSuccess) _douyinDisplayRoomId = config!.RoomId;
            ApplyDouyinProbeHostResult(hostResult);
            if (hostResult.IsSuccess)
            {
                await PersistDouyinConfigAsync(config!).ConfigureAwait(true);
            }

            return;
        }

        DouyinStatusText.Text = probeError ?? "未配置抖音运行环境，请通过桌面开发启动脚本启动后重试";
        _state.SetStatus(DouyinStatusText.Text);
    }

    private void PauseDouyinButton_Click(object sender, RoutedEventArgs e) =>
        ApplyDouyinOperationResult(_douyinLive.Pause());

    private void ResumeDouyinButton_Click(object sender, RoutedEventArgs e)
    {
        if (!DouyinConfigDraft.TryParseQueueCapacity(
                DouyinQueueCapacityTextBox.Text,
                out var capacity,
                out var error))
        {
            DouyinStatusText.Text = error ?? "队列容量无效";
            _state.SetStatus(DouyinStatusText.Text);
            return;
        }

        var capacityResult = _douyinLive.SetQueueCapacity(capacity);
        if (!capacityResult.IsSuccess)
        {
            ApplyDouyinOperationResult(capacityResult);
            return;
        }

        ApplyDouyinOperationResult(_douyinLive.Resume());
    }

    private async void StopDouyinButton_Click(object sender, RoutedEventArgs e)
    {
        var result = await _douyinProbeHost.DisconnectAsync().ConfigureAwait(true);
        if (!_isClosing) ApplyDouyinProbeHostResult(result);
    }

    private void StopDouyinSession()
    {
        CloseDouyinChatWindow();
        _douyinDisplayRoomId = null;
        _douyinManualSendGeneration = null;
        _douyinProbeHost.ClearChatMessages();
        if (_douyinAuthorizationStopTask is not { IsCompleted: false })
            _douyinAuthorizationStopTask = StopDouyinSessionAsync(clearMessages: true);
    }

    private async Task StopDouyinSessionAsync(bool clearMessages = false)
    {
        var result = await _douyinProbeHost.StopAsync().ConfigureAwait(true);
        if (clearMessages) _douyinProbeHost.ClearChatMessages();
        if (_isClosing)
        {
            return;
        }

        ApplyDouyinProbeHostResult(result);
    }

    private void ViewDouyinChatButton_Click(object sender, RoutedEventArgs e)
    {
        if (!_login.CanEnterWorkbench || _isClosing) return;
        if (_douyinChatWindow is null)
        {
            _douyinChatWindow = new DouyinChatWindow(_douyinProbeHost.ClearChatMessages, SendDouyinChatAsync) { Owner = this };
            _douyinChatWindow.Closed += DouyinChatWindow_Closed;
            UpdateDouyinProjection();
            _douyinChatWindow.Show();
        }
        else
        {
            if (_douyinChatWindow.WindowState == WindowState.Minimized)
                _douyinChatWindow.WindowState = WindowState.Normal;
            _douyinChatWindow.Activate();
        }
    }

    private void DouyinChatWindow_Closed(object? sender, EventArgs e)
    {
        if (sender is DouyinChatWindow window) window.Closed -= DouyinChatWindow_Closed;
        _douyinChatWindow = null;
    }

    private void CloseDouyinChatWindow() => _douyinChatWindow?.Close();

    private bool IsCurrentDouyinRoomConnected(WindowsDouyinProbeHostSnapshot snapshot) =>
        snapshot.Authenticated && snapshot.State == WindowsDouyinProbeHostState.Running
        && snapshot.Douyin.State == DouyinLiveState.Listening
        && DouyinLiveRules.TryNormalizeRoomId(DouyinRoomTextBox.Text, out var roomId)
        && string.Equals(roomId, _douyinDisplayRoomId, StringComparison.Ordinal);

    private async Task<WindowsDouyinChatSendResult?> SendDouyinChatAsync(string text)
    {
        if (_isClosing || !_login.CanEnterWorkbench || _douyinManualSendTask is { IsCompleted: false }) return null;
        var generation = _douyinProbeHost.Snapshot.Douyin.Generation;
        _douyinManualSendGeneration = generation;
        _douyinManualSendTask = _douyinProbeHost.SendChatAsync(text, _windowCancellation.Token);
        UpdateDouyinProjection();
        try
        {
            var result = await _douyinManualSendTask.ConfigureAwait(true);
            return _douyinManualSendGeneration == generation && _douyinProbeHost.Snapshot.Douyin.Generation == generation
                ? result : null;
        }
        catch (Exception)
        {
            // 未预期的终态由下方投影明确显示结果未知，正文和异常原文不进入日志。
            return null;
        }
        finally
        {
            if (!_isClosing) UpdateDouyinProjection();
        }
    }

    private void DouyinProbeHost_SnapshotChanged(object? sender, WindowsDouyinProbeHostSnapshot snapshot)
    {
        if (_isClosing)
        {
            return;
        }

        _douyinUiUpdates.Post(() =>
        {
            if (!_isClosing)
            {
                UpdateDouyinProjection();
            }

            return Task.CompletedTask;
        });
    }

    private void ApplyDouyinProbeHostResult(WindowsDouyinProbeHostResult result)
    {
        UpdateDouyinProjection();
        if (!result.IsSuccess)
        {
            DouyinStatusText.Text = result.Error?.Message ?? "抖音 sidecar 操作失败";
            _state.SetStatus(DouyinStatusText.Text);
        }
    }

    private void ApplyDouyinOperationResult(DouyinLiveOperationResult result)
    {
        UpdateDouyinProjection();
        if (!result.IsSuccess)
        {
            DouyinStatusText.Text = result.Error?.Message ?? "抖音 M1 操作失败";
            _state.SetStatus(DouyinStatusText.Text);
        }
    }

    private void UpdateDouyinProjection(WindowsDouyinProbeHostSnapshot? snapshot = null)
    {
        if (!IsInitialized)
        {
            return;
        }

        var hostSnapshot = snapshot ?? _douyinProbeHost.Snapshot;
        var status = hostSnapshot.Douyin;
        var currentRoomConnected = IsCurrentDouyinRoomConnected(hostSnapshot);
        var loggedInDisconnected = hostSnapshot.Authenticated
            && hostSnapshot.State == WindowsDouyinProbeHostState.Ready
            && status.State == DouyinLiveState.LoggedIn;
        UpdateDouyinQrProjection(hostSnapshot);
        UpdateDouyinDiagnosticProjection(hostSnapshot);
        DouyinAuthStatusText.Text = "抖音登录：" + (hostSnapshot.Authenticated
            ? "已登录（仅本次软件运行）"
            : hostSnapshot.LoginClearReason switch
            {
                WindowsDouyinLoginClearReason.NotAuthenticatedThisRun => "本次运行尚未登录；首次启动或软件重启后需扫码",
                WindowsDouyinLoginClearReason.AuthenticationExpired => "平台登录已失效；请重新连接并扫码",
                WindowsDouyinLoginClearReason.LoginFailed => "扫码未完成或失败；请重新连接并扫码",
                WindowsDouyinLoginClearReason.ExplicitStop => "已退出抖音登录；再次连接需扫码",
                WindowsDouyinLoginClearReason.ProcessExited => "辅助进程已退出，内存登录已清除；请重新连接并扫码",
                WindowsDouyinLoginClearReason.LocalCommunicationError => "本地通信异常，无法确认登录；请重新连接并扫码",
                _ => "未登录，请扫码"
            });
        var authorized = _login.CanEnterWorkbench;
        var douyinActive = status.State is DouyinLiveState.WaitingQr
            or DouyinLiveState.LoggedIn
            or DouyinLiveState.RoomResolved
            or DouyinLiveState.Listening
            or DouyinLiveState.Paused
            or DouyinLiveState.Stopping;
        var douyinPill = status.State switch
        {
            DouyinLiveState.LoggedIn when loggedInDisconnected => "已断开",
            DouyinLiveState.Listening when status.ReplySendingBlocked => "发送已暂停",
            DouyinLiveState.Listening => "已连接",
            DouyinLiveState.Paused => "已暂停",
            DouyinLiveState.WaitingQr or DouyinLiveState.LoggedIn or DouyinLiveState.RoomResolved => "连接中",
            DouyinLiveState.Failed or DouyinLiveState.Inconclusive => "异常",
            DouyinLiveState.Passed => "已通过",
            _ => "待连接",
        };
        var gapSuffix = status.Metrics.GapDroppedCount > 0
            ? $" · 数据缺口 {status.Metrics.GapDroppedCount} 条"
            : string.Empty;
        SetStatusPill(DouyinStatePillText, douyinPill, douyinActive && !loggedInDisconnected);
        var canEditConfig = loggedInDisconnected || status.State is DouyinLiveState.Idle
            or DouyinLiveState.Failed
            or DouyinLiveState.Inconclusive
            or DouyinLiveState.Passed;
        var canEditQueue = canEditConfig || status.State == DouyinLiveState.Paused;
        StartDouyinButton.IsEnabled = authorized && !_isClosing
            && _douyinManualSendTask is not { IsCompleted: false }
            && (canEditConfig || currentRoomConnected);
        StartDouyinButton.Content = currentRoomConnected ? "已连接" : loggedInDisconnected ? "重新连接" : "连接直播间";
        StartDouyinButton.ToolTip = currentRoomConnected ? "当前直播间已连接，重复点击不会重建连接或重新登录"
            : hostSnapshot.Authenticated ? "使用本次抖音登录连接直播间，无需重复扫码"
            : "连接直播间并扫码登录抖音；自动回应保持当前设置";
        ViewDouyinChatButton.IsEnabled = authorized;
        PauseDouyinButton.IsEnabled = authorized && status.State == DouyinLiveState.Listening;
        ResumeDouyinButton.IsEnabled = authorized && status.State == DouyinLiveState.Paused;
        StopDouyinButton.IsEnabled = authorized && !loggedInDisconnected && status.State != DouyinLiveState.Idle;
        DouyinEnabledCheckBox.IsEnabled = authorized && canEditConfig;
        DouyinRoomTextBox.IsEnabled = authorized && canEditConfig;
        DouyinRepliesTextBox.IsEnabled = authorized && canEditConfig;
        DouyinQueueCapacityTextBox.IsEnabled = authorized && canEditQueue;
        DouyinStatusText.Text = "直播间：" + (status.State switch
        {
            DouyinLiveState.Idle => "未连接 / 已断开 · 输入直播间链接后连接",
            DouyinLiveState.WaitingQr => "等待抖音扫码登录后连接",
            DouyinLiveState.LoggedIn when loggedInDisconnected => string.IsNullOrWhiteSpace(status.Error)
                ? "已断开，可直接重连"
                : $"已断开：{status.Error}；可直接重连",
            DouyinLiveState.LoggedIn => "正在解析直播间",
            DouyinLiveState.RoomResolved => "直播间已解析 · 正在连接公屏",
            DouyinLiveState.Listening when status.ReplySendingBlocked => $"公屏仍在监听 · 回复已因风控/限流暂停 · 队列 {status.QueueCount}/{status.QueueCapacity}{gapSuffix}",
            DouyinLiveState.Listening => $"公屏监听中 · 回复队列 {status.QueueCount}/{status.QueueCapacity}{gapSuffix}",
            DouyinLiveState.Paused => $"回复已暂停 · 公屏仍在监听 · 队列 {status.QueueCount}/{status.QueueCapacity}{gapSuffix}",
            DouyinLiveState.Stopping => "抖音 M1 正在停止",
            DouyinLiveState.Failed => $"M1 失败 · {status.Error ?? "请检查兼容探针"}",
            DouyinLiveState.Inconclusive => $"证据不足 · {status.Error ?? "本轮未观察到完整回显"}",
            DouyinLiveState.Passed => "兼容探针已通过 · 正式发布门禁仍待验收",
            _ => "抖音 M1 状态未知"
        });
        var popupLogin = hostSnapshot.Authenticated ? "已登录" : status.State == DouyinLiveState.WaitingQr ? "等待扫码" : "未登录";
        _douyinChatWindow?.Refresh(_douyinProbeHost.GetChatMessages(),
            $"抖音登录：{popupLogin}\n直播间：{douyinPill}", _douyinDisplayRoomId,
            $"{DouyinAuthStatusText.Text}\n{DouyinStatusText.Text}");
        if (_douyinChatWindow is not null)
        {
            var canSend = authorized && hostSnapshot.Authenticated
                && hostSnapshot.State == WindowsDouyinProbeHostState.Running
                && status.State == DouyinLiveState.Listening && !status.ReplySendingBlocked;
            var sendMessage = _douyinManualSendGeneration == status.Generation
                ? _douyinManualSendTask switch
                {
                    { IsCompletedSuccessfully: true } task => task.Result.Message,
                    { IsFaulted: true } or { IsCanceled: true } => "发送结果未知，请先核对本人弹幕；不会自动重试",
                    _ => null
                } : null;
            _douyinChatWindow.RefreshSendState(canSend, _douyinManualSendTask is { IsCompleted: false }, sendMessage);
        }
    }

    private void UpdateDouyinQrProjection(WindowsDouyinProbeHostSnapshot hostSnapshot)
    {
        var waitingForQr = hostSnapshot.Douyin.State == DouyinLiveState.WaitingQr;
        var qrPath = waitingForQr ? hostSnapshot.QrPath : null;
        if (string.IsNullOrWhiteSpace(qrPath))
        {
            _douyinQrImagePath = null;
            DouyinQrImage.Source = null;
            DouyinQrPanel.Visibility = Visibility.Collapsed;
            DouyinQrStatusText.Text = waitingForQr && hostSnapshot.State == WindowsDouyinProbeHostState.Running
                ? "等待 sidecar 发放二维码"
                : "未生成二维码";
            return;
        }

        if (!string.Equals(_douyinQrImagePath, qrPath, StringComparison.OrdinalIgnoreCase))
        {
            if (DouyinQrImageLoader.TryLoad(qrPath, out var image, out var error))
            {
                DouyinQrImage.Source = image;
                _douyinQrImagePath = qrPath;
                DouyinQrStatusText.Text = "请使用抖音扫码；凭据不会写入配置文件";
            }
            else
            {
                _douyinQrImagePath = null;
                DouyinQrImage.Source = null;
                DouyinQrStatusText.Text = error ?? "二维码文件不可用";
            }
        }

        DouyinQrPanel.Visibility = DouyinQrImage.Source is null
            ? Visibility.Collapsed
            : Visibility.Visible;
    }

    private async Task LoadDouyinConfigAsync()
    {
        if (_douyinConfigStore is null || _isClosing)
        {
            return;
        }

        try
        {
            var config = await _douyinConfigStore
                .ReadAsync(_windowCancellation.Token)
                .ConfigureAwait(true);
            DouyinEnabledCheckBox.IsChecked = config.Enabled;
            DouyinRoomTextBox.Text = config.RoomId;
            DouyinRepliesTextBox.Text = string.Join(Environment.NewLine, config.Replies);
            DouyinQueueCapacityTextBox.Text = config.QueueCapacity.ToString(System.Globalization.CultureInfo.InvariantCulture);
            UpdateDouyinProjection();
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 关闭时不再更新本地配置状态。
        }
        catch (ConfigurationValidationException)
        {
            _state.SetStatus("抖音 M1 配置无效，已回退到内存默认值");
        }
        catch (IOException)
        {
            _state.SetStatus("抖音 M1 配置读取失败，使用内存默认值");
        }
        catch (UnauthorizedAccessException)
        {
            _state.SetStatus("抖音 M1 配置不可访问，使用内存默认值");
        }
    }

    private async Task PersistDouyinConfigAsync(DouyinLiveConfig config)
    {
        if (_douyinConfigStore is null || _isClosing)
        {
            return;
        }

        try
        {
            await _douyinConfigStore
                .WriteAsync(config, _windowCancellation.Token)
                .ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 关闭时取消写入；内存会话仍然有效。
        }
        catch (ConfigurationValidationException)
        {
            _state.SetStatus("抖音 M1 配置保存失败；当前会话仍使用内存值");
        }
        catch (IOException)
        {
            _state.SetStatus("抖音 M1 配置保存失败；当前会话仍使用内存值");
        }
        catch (UnauthorizedAccessException)
        {
            _state.SetStatus("抖音 M1 配置保存被系统拒绝；当前会话仍使用内存值");
        }
    }

    private static DouyinLiveConfigStore? TryCreateDouyinConfigStore()
    {
        var localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        if (string.IsNullOrWhiteSpace(localAppData))
        {
            return null;
        }

        try
        {
            var path = Path.Combine(
                localAppData,
                "GpAutoLive",
                "profiles",
                "douyin",
                "default.json");
            return new DouyinLiveConfigStore(path);
        }
        catch (ArgumentException)
        {
            return null;
        }
        catch (IOException)
        {
            return null;
        }
        catch (NotSupportedException)
        {
            return null;
        }
    }
}
