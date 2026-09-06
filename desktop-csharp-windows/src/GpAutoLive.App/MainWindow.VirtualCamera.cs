using System.Windows;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private MediaPlaybackIdentity? _virtualCameraOutputIdentity;

    private void SyncVirtualCameraOutputContext(AppState snapshot)
    {
        var source = snapshot.SourceMediaPool.IsEmpty
            ? null
            : snapshot.SourceMediaPool[snapshot.SourceMediaIndex];
        var identity = source is null ? null : _mediaPool.CurrentIdentity;
        var previous = _virtualCameraOutput.OutputContext;
        var sourceChanged = _virtualCameraOutputIdentity != identity;
        _virtualCameraOutputIdentity = identity;
        _virtualCameraOutput.SetOutputContext(new(
            PlaybackActive: snapshot.PlaybackState is PlaybackState.Playing or PlaybackState.Paused,
            VideoSourceActive: source?.MediaKind is MediaKind.Video,
            Paused: snapshot.PlaybackState is PlaybackState.Paused,
            Stopped: snapshot.PlaybackState is PlaybackState.Stopped || source is null,
            Locked: previous.Locked,
            HasValidFrame: !sourceChanged && previous.HasValidFrame));
    }

    private async Task<bool> StopVirtualCameraForMediaMutationAsync()
    {
        var status = _virtualCameraOutput.Snapshot;
        if (_virtualCameraOutputCoordinator is null
            || (!_virtualCameraOutputCoordinator.HasActiveResources
                && status.State is not (VirtualCameraState.Starting
                    or VirtualCameraState.Ready
                    or VirtualCameraState.Streaming
                    or VirtualCameraState.Recovering)))
        {
            return true;
        }

        var stopped = await _virtualCameraOutputCoordinator
            .StopAsync(_windowCancellation.Token)
            .ConfigureAwait(true);
        if (!stopped.IsSuccess)
        {
            VirtualCameraActionStatusText.Text = stopped.ErrorMessage ?? "媒体池编辑前停止虚拟摄像头失败";
            _state.SetStatus("媒体源未变化，虚拟摄像头停止失败");
            UpdateVirtualCameraProjection();
            return false;
        }

        VirtualCameraActionStatusText.Text = "媒体源即将变化，虚拟摄像头输出已停止";
        UpdateVirtualCameraProjection();
        return true;
    }

    private void UpdateVirtualCameraProjection()
    {
        if (!IsInitialized)
        {
            return;
        }

        var status = _virtualCameraOutput.Snapshot;
        VirtualCameraStatusText.Text = status.State switch
        {
            VirtualCameraState.Unavailable => FormatVirtualCameraUnavailableStatus(),
            VirtualCameraState.Installed => FormatVirtualCameraInstalledStatus(),
            VirtualCameraState.Starting => "正在准备 WGC/GPU→YUY2 输出链 · sidecar/DirectShow 待验收",
            VirtualCameraState.Ready => "GPU→YUY2 输出链已就绪 · 等待下游客户端（sidecar/DirectShow 待验收）",
            VirtualCameraState.Streaming => $"下游客户端 {status.DownstreamClientCount ?? 0} 个 · sidecar/DirectShow 待验收",
            VirtualCameraState.Recovering => "输出链恢复中 · 本地播放不受影响",
            VirtualCameraState.Failed => $"输出链失败 · {status.LastError ?? "请检查组件与 GPU 门禁"}",
            VirtualCameraState.Stopping => "输出链正在停止",
            _ => "虚拟摄像头状态未知"
        };
        RefreshVirtualCameraButton.IsEnabled = _login.CanEnterWorkbench && !_virtualCameraProbeBusy;
        var outputRunning = status.State is VirtualCameraState.Starting
            or VirtualCameraState.Ready
            or VirtualCameraState.Streaming
            or VirtualCameraState.Recovering;
        var canStop = outputRunning || (_virtualCameraOutputCoordinator?.HasActiveResources ?? false);
        var cameraPill = status.State switch
        {
            VirtualCameraState.Starting or VirtualCameraState.Recovering => "启动中",
            VirtualCameraState.Ready or VirtualCameraState.Streaming => "已启用",
            VirtualCameraState.Failed or VirtualCameraState.Unavailable => "不可用",
            _ => "待机",
        };
        SetStatusPill(VirtualCameraStatePillText, cameraPill, outputRunning);
        var canStart = _login.CanEnterWorkbench
            && !_virtualCameraProbeBusy
            && !_virtualCameraOutputCommandBusy
            && status.State == VirtualCameraState.Installed
            && _virtualCameraInstallationProbe?.IsAvailable == true
            && _virtualCameraSidecarProbe.IsTrusted
            && _virtualCameraD3D11Probe?.IsReady == true
            && _virtualCameraWgcProbe?.IsReady == true
            && _virtualCameraSurfaceBinding.Snapshot.IsBound
            && HasReadyVideoPlayback();
        StartVirtualCameraButton.IsEnabled = canStart;
        StopVirtualCameraButton.IsEnabled = _login.CanEnterWorkbench
            && !_virtualCameraProbeBusy
            && !_virtualCameraOutputCommandBusy
            && canStop;
    }

    private string FormatVirtualCameraUnavailableStatus()
    {
        if (_virtualCameraInstallationProbe is { IsAvailable: false } installation)
        {
            return installation.Code switch
            {
                WindowsVirtualCameraInstallationProbeCode.RegistryOwnerMissing => "未发现正式 x86/x64 安装所有者；请先安装签名组件",
                WindowsVirtualCameraInstallationProbeCode.RegistryOwnerMismatch => "x86/x64 安装路径不一致；输出门禁已拒绝",
                WindowsVirtualCameraInstallationProbeCode.ComponentMissing => "安装组件不完整；输出门禁已拒绝",
                WindowsVirtualCameraInstallationProbeCode.DeviceMissing => "未发现 GpAutoLive Camera PnP 设备；输出门禁已拒绝",
                WindowsVirtualCameraInstallationProbeCode.ProbeFailed => "Windows 设备安装探测失败；输出门禁已拒绝",
                _ => "虚拟摄像头安装门禁未通过"
            };
        }

        if (!_virtualCameraSidecarProbe.IsAvailable)
        {
            return "契约/GPU→YUY2 已接入 · sidecar 文件未通过探测；输出门禁未通过";
        }

        if (!_virtualCameraSidecarProbe.IsTrusted)
        {
            return FormatSidecarSignatureStatus();
        }

        return _virtualCameraD3D11Probe switch
        {
            null => "sidecar 文件已发现 · 正在检查 D3D11/WGC 前置",
            { IsReady: false } => "sidecar 文件已发现 · D3D11 前置未通过 · WGC 待验收",
            _ when _virtualCameraWgcProbe is null => "sidecar 文件已发现 · D3D11 通过 · 正在检查 WGC",
            _ when _virtualCameraWgcProbe.IsReady => "sidecar 文件已发现 · D3D11/WGC 前置通过 · 设备待验收",
            _ => "sidecar 文件已发现 · WGC 前置未通过 · 设备待验收"
        };
    }

    private string FormatVirtualCameraInstalledStatus() =>
        _virtualCameraSidecarProbe.IsTrusted
            ? "组件已安装 · 输出尚未启动（GPU→YUY2 已接入；sidecar/DirectShow 待验收）"
            : $"组件已安装 · {FormatSidecarSignatureStatus()}";

    private string FormatSidecarSignatureStatus() => _virtualCameraSidecarProbe.SignatureCode switch
    {
        WindowsAuthenticodeProbeCode.Unsigned => "sidecar 未签名；签名发布门禁未通过",
        WindowsAuthenticodeProbeCode.InvalidSignature => "sidecar 签名无效；输出门禁未通过",
        WindowsAuthenticodeProbeCode.FileMissing => "sidecar 签名文件缺失；输出门禁未通过",
        WindowsAuthenticodeProbeCode.InvalidPath => "sidecar 路径非法；输出门禁未通过",
        WindowsAuthenticodeProbeCode.ApiUnavailable => "Windows 签名 API 不可用；输出门禁未通过",
        WindowsAuthenticodeProbeCode.ProbeFailed => "sidecar 签名探测失败；输出门禁未通过",
        _ => "sidecar 签名待验收；输出门禁未通过",
    };

    private async Task ProbeVirtualCameraD3D11Async()
    {
        if (!_virtualCameraSidecarProbe.IsAvailable || _isClosing)
        {
            return;
        }

        try
        {
            var result = await Task.Run(
                    WindowsD3D11CapabilityProbe.Probe,
                    _windowCancellation.Token)
                .ConfigureAwait(true);
            if (!_isClosing)
            {
                _virtualCameraD3D11Probe = result;
                UpdateVirtualCameraProjection();
            }
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 关闭窗口时不再更新虚拟摄像头状态。
        }
    }

    private async void RefreshVirtualCameraButton_Click(object sender, RoutedEventArgs e) =>
        await RefreshVirtualCameraProbesAsync().ConfigureAwait(true);

    private async void StartVirtualCameraButton_Click(object sender, RoutedEventArgs e) =>
        await RunPlaybackCommandAsync(StartVirtualCameraCoreAsync).ConfigureAwait(true);

    private async void StopVirtualCameraButton_Click(object sender, RoutedEventArgs e) =>
        await RunPlaybackCommandAsync(StopVirtualCameraCoreAsync).ConfigureAwait(true);

    private async Task StartVirtualCameraCoreAsync()
    {
        if (!_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        var status = _virtualCameraOutput.Snapshot;
        if (status.State != VirtualCameraState.Installed)
        {
            VirtualCameraActionStatusText.Text = "安装门禁未通过；未启动虚拟摄像头输出";
            _state.SetStatus("虚拟摄像头尚未完成安装验收");
            return;
        }

        if (_virtualCameraInstallationProbe?.IsAvailable != true
            || !_virtualCameraSidecarProbe.IsTrusted)
        {
            VirtualCameraActionStatusText.Text = _virtualCameraSidecarProbe.IsAvailable
                ? "sidecar 未通过 Authenticode 签名门禁；未启动输出"
                : "未发现完整安装或有效 sidecar；未启动输出";
            _state.SetStatus("虚拟摄像头安装/sidecar 签名门禁未通过");
            return;
        }

        if (_virtualCameraD3D11Probe?.IsReady != true
            || _virtualCameraWgcProbe?.IsReady != true)
        {
            VirtualCameraActionStatusText.Text = "D3D11/WGC 前置未通过；未启动输出";
            _state.SetStatus("虚拟摄像头 GPU 前置未通过");
            return;
        }

        if (!HasReadyVideoPlayback())
        {
            VirtualCameraActionStatusText.Text = "请先播放视频并等待首帧后再启动虚拟摄像头输出";
            _state.SetStatus("虚拟摄像头需要当前视频播放首帧就绪");
            UpdateVirtualCameraProjection();
            return;
        }

        if (!EnsureFinalEffectWindowVisible()
            || !_virtualCameraSurfaceBinding.Snapshot.IsBound)
        {
            VirtualCameraActionStatusText.Text = "最终效果窗口没有可用 HWND；未启动输出";
            _state.SetStatus("请先打开最终效果窗口");
            UpdateVirtualCameraProjection();
            return;
        }

        _virtualCameraOutputCommandBusy = true;
        UpdateVirtualCameraProjection();
        try
        {
            var request = new WindowsVirtualCameraSidecarLaunchRequest(
                _virtualCameraSidecarProbe.ExecutablePath!,
                WindowsVirtualCameraSidecarLaunchPlanBuilder.CreateSessionToken(),
                status.Config,
                WindowsVirtualCameraSidecarLaunchPlanBuilder.DefaultStartupTimeout);
            if (!WindowsVirtualCameraSidecarLaunchPlanBuilder.TryCreate(
                    request,
                    out var plan,
                    out var planError)
                || plan is null)
            {
                VirtualCameraActionStatusText.Text = planError ?? "虚拟摄像头 sidecar 启动计划无效";
                _state.SetStatus("虚拟摄像头输出计划校验失败");
                return;
            }

            var coordinator = _virtualCameraOutputCoordinator;
            if (coordinator is null)
            {
                coordinator = new(
                    _virtualCameraOutput,
                    _virtualCameraSurfaceBinding);
                coordinator.SnapshotChanged += VirtualCameraOutputCoordinator_SnapshotChanged;
                _virtualCameraOutputCoordinator = coordinator;
            }
            var result = await coordinator.StartAsync(
                    plan,
                    cancellationToken: _windowCancellation.Token)
                .ConfigureAwait(true);
            VirtualCameraActionStatusText.Text = result.IsSuccess
                ? "虚拟摄像头输出已启动；等待 sidecar 下游客户端"
                : result.ErrorMessage ?? "虚拟摄像头输出启动失败";
            _state.SetStatus(VirtualCameraActionStatusText.Text);
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 窗口关闭时取消启动，不再更新 UI。
        }
        finally
        {
            _virtualCameraOutputCommandBusy = false;
            if (!_isClosing)
            {
                UpdateVirtualCameraProjection();
            }
        }
    }

    private void VirtualCameraOutputCoordinator_SnapshotChanged(
        object? sender,
        WindowsVirtualCameraOutputCoordinatorSnapshot snapshot)
    {
        if (_isClosing)
        {
            return;
        }

        _ = Dispatcher.InvokeAsync(() =>
        {
            if (_isClosing)
            {
                return;
            }

            if (snapshot.Output.State == VirtualCameraState.Failed)
            {
                VirtualCameraActionStatusText.Text = "虚拟摄像头输出已失败；请点击停止完成资源清理";
                _state.SetStatus("虚拟摄像头输出已失败，可停止清理后重新启动");
            }

            UpdateVirtualCameraProjection();
        }, System.Windows.Threading.DispatcherPriority.Background);
    }

    private async Task StopVirtualCameraCoreAsync()
    {
        if (_virtualCameraOutputCoordinator is null)
        {
            return;
        }

        _virtualCameraOutputCommandBusy = true;
        UpdateVirtualCameraProjection();
        try
        {
            var result = await _virtualCameraOutputCoordinator
                .StopAsync(_windowCancellation.Token)
                .ConfigureAwait(true);
            VirtualCameraActionStatusText.Text = result.IsSuccess
                ? "虚拟摄像头输出已停止"
                : result.ErrorMessage ?? "虚拟摄像头输出停止失败";
            _state.SetStatus(VirtualCameraActionStatusText.Text);
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 窗口关闭时取消停止；最终 DisposeAsync 仍会执行有界清理。
        }
        finally
        {
            _virtualCameraOutputCommandBusy = false;
            if (!_isClosing)
            {
                UpdateVirtualCameraProjection();
            }
        }
    }

    private async Task RefreshVirtualCameraProbesAsync()
    {
        if (_isClosing || _virtualCameraProbeBusy)
        {
            return;
        }

        _virtualCameraProbeBusy = true;
        RefreshVirtualCameraButton.IsEnabled = false;
        VirtualCameraActionStatusText.Text = "正在检查安装、D3D11 和 WGC 前置…";
        try
        {
            _virtualCameraSidecarProbe = WindowsVirtualCameraSidecarLocator.ProbeFromEnvironment();
            _virtualCameraInstallationProbe = await Task.Run(
                    () => WindowsVirtualCameraInstallationProbe.Probe(),
                    _windowCancellation.Token)
                .ConfigureAwait(true);

            if (_virtualCameraInstallationProbe.IsAvailable
                && _virtualCameraOutput.Snapshot.State is VirtualCameraState.Unavailable or VirtualCameraState.Failed)
            {
                _virtualCameraOutput.MarkInstalled();
            }

            UpdateVirtualCameraProjection();
            if (!_virtualCameraSidecarProbe.IsAvailable)
            {
                VirtualCameraActionStatusText.Text = "未发现通过文件/架构探测的 sidecar；仅更新安装状态，未启动任何进程";
                return;
            }

            if (!_virtualCameraSidecarProbe.IsTrusted)
            {
                VirtualCameraActionStatusText.Text = "sidecar 已发现但未通过 Authenticode 签名门禁；未启动任何进程";
                return;
            }

            await Task.WhenAll(
                    ProbeVirtualCameraD3D11Async(),
                    ProbeVirtualCameraWgcAsync())
                .ConfigureAwait(true);

            if (!_isClosing)
            {
                VirtualCameraActionStatusText.Text = _virtualCameraInstallationProbe.IsAvailable
                    ? "安装门禁与 GPU 前置已检查；真实 sidecar/DirectShow 仍需验收"
                    : "GPU 前置已检查；安装门禁未通过，未启动输出链";
            }
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 关闭窗口时取消探测，不再更新 UI。
        }
        finally
        {
            _virtualCameraProbeBusy = false;
            if (!_isClosing)
            {
                UpdateVirtualCameraProjection();
            }
        }
    }

    private async Task ProbeVirtualCameraWgcAsync()
    {
        if (!_virtualCameraSidecarProbe.IsAvailable || _isClosing)
        {
            return;
        }

        try
        {
            var result = await Task.Run(
                    WindowsGraphicsCaptureCapabilityProbe.Probe,
                    _windowCancellation.Token)
                .ConfigureAwait(true);
            if (!_isClosing)
            {
                _virtualCameraWgcProbe = result;
                UpdateVirtualCameraProjection();
            }
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 关闭窗口时不再更新虚拟摄像头状态。
        }
    }

    private void RefreshVirtualCameraSurfaceBinding()
    {
        if (_finalEffectWindow?.TryGetCaptureWindowHandle(out var windowId) == true)
        {
            _virtualCameraSurfaceBinding.Bind(windowId);
            return;
        }

        _virtualCameraSurfaceBinding.Unbind();
    }

    private bool HasReadyVideoPlayback()
    {
        var mediaSnapshot = _mediaPool.Snapshot;
        if (mediaSnapshot.SourceMediaPool.IsEmpty
            || mediaSnapshot.SourceMediaIndex < 0
            || mediaSnapshot.SourceMediaIndex >= mediaSnapshot.SourceMediaPool.Length)
        {
            return false;
        }

        var source = mediaSnapshot.SourceMediaPool[mediaSnapshot.SourceMediaIndex];
        var controller = _mpvController.Snapshot;
        return source.MediaKind is MediaKind.Video
            && mediaSnapshot.PlaybackState is (PlaybackState.Playing or PlaybackState.Paused)
            && controller.ActiveIdentity == _mediaPool.CurrentIdentity
            && controller.Runtime.State is WindowsMpvPlaybackRuntimeState.Running;
    }
}
