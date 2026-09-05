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
    private async void StartDouyinButton_Click(object sender, RoutedEventArgs e)
    {
        if (!_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
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
                out var sidecarConfigured)
            && probeRequest is not null)
        {
            var hostResult = await _douyinProbeHost.StartAsync(probeRequest, _windowCancellation.Token).ConfigureAwait(true);
            ApplyDouyinProbeHostResult(hostResult);
            if (hostResult.IsSuccess)
            {
                await PersistDouyinConfigAsync(config!).ConfigureAwait(true);
            }

            return;
        }

        if (sidecarConfigured)
        {
            DouyinStatusText.Text = probeError ?? "抖音 sidecar 环境配置无效";
            _state.SetStatus(DouyinStatusText.Text);
            return;
        }

        var result = _douyinLive.TryStart(config);
        ApplyDouyinOperationResult(result);
        if (result.IsSuccess)
        {
            await PersistDouyinConfigAsync(config!).ConfigureAwait(true);
        }
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

    private async void StopDouyinButton_Click(object sender, RoutedEventArgs e) => await StopDouyinSessionAsync();

    private void StopDouyinSession() => _ = StopDouyinSessionAsync();

    private async Task StopDouyinSessionAsync()
    {
        var result = await _douyinProbeHost.StopAsync().ConfigureAwait(true);
        if (_isClosing)
        {
            return;
        }

        ApplyDouyinProbeHostResult(result);
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

    private void UpdateDouyinProjection()
    {
        if (!IsInitialized)
        {
            return;
        }

        var status = _douyinLive.Snapshot;
        var authorized = _login.CanEnterWorkbench;
        var douyinActive = status.State is DouyinLiveState.WaitingQr
            or DouyinLiveState.LoggedIn
            or DouyinLiveState.RoomResolved
            or DouyinLiveState.Listening
            or DouyinLiveState.Paused
            or DouyinLiveState.Stopping;
        var douyinPill = status.State switch
        {
            DouyinLiveState.Listening => "已连接",
            DouyinLiveState.Paused => "已暂停",
            DouyinLiveState.WaitingQr or DouyinLiveState.LoggedIn or DouyinLiveState.RoomResolved => "连接中",
            DouyinLiveState.Failed or DouyinLiveState.Inconclusive => "异常",
            DouyinLiveState.Passed => "已通过",
            _ => "待连接",
        };
        SetStatusPill(DouyinStatePillText, douyinPill, douyinActive);
        var canEditConfig = status.State is DouyinLiveState.Idle
            or DouyinLiveState.Failed
            or DouyinLiveState.Inconclusive
            or DouyinLiveState.Passed;
        var canEditQueue = canEditConfig || status.State == DouyinLiveState.Paused;
        StartDouyinButton.IsEnabled = authorized && canEditConfig;
        PauseDouyinButton.IsEnabled = authorized && status.State == DouyinLiveState.Listening;
        ResumeDouyinButton.IsEnabled = authorized && status.State == DouyinLiveState.Paused;
        StopDouyinButton.IsEnabled = authorized && status.State != DouyinLiveState.Idle;
        DouyinEnabledCheckBox.IsEnabled = authorized && canEditConfig;
        DouyinRoomTextBox.IsEnabled = authorized && canEditConfig;
        DouyinRepliesTextBox.IsEnabled = authorized && canEditConfig;
        DouyinQueueCapacityTextBox.IsEnabled = authorized && canEditQueue;
        DouyinStatusText.Text = status.State switch
        {
            DouyinLiveState.Idle => "M1 本地合同已接入 · 未启动；扫码/sidecar 待验收",
            DouyinLiveState.WaitingQr => "等待扫码 · 凭据仅存当前进程内存",
            DouyinLiveState.LoggedIn => "扫码已确认 · 直播间解析待验收",
            DouyinLiveState.RoomResolved => "直播间已解析 · 公屏监听待验收",
            DouyinLiveState.Listening => $"公屏监听中 · 队列 {status.QueueCount}/{status.QueueCapacity} · sidecar 待验收",
            DouyinLiveState.Paused => $"已暂停 · 队列 {status.QueueCount}/{status.QueueCapacity}",
            DouyinLiveState.Stopping => "抖音 M1 正在停止",
            DouyinLiveState.Failed => $"M1 失败 · {status.Error ?? "请检查兼容探针"}",
            DouyinLiveState.Inconclusive => $"证据不足 · {status.Error ?? "本轮未观察到完整回显"}",
            DouyinLiveState.Passed => "兼容探针已通过 · 正式发布门禁仍待验收",
            _ => "抖音 M1 状态未知"
        };
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
