using System.Net.Http;
using System.IO;
using System.ComponentModel;
using System.Windows;
using System.Windows.Input;
using System.Windows.Threading;
using GpAutoLive.App.Features.Auth;
using GpAutoLive.App.Features.Performance;
using GpAutoLive.App.Features.Playback;
using GpAutoLive.App.Features.Media;
using GpAutoLive.App.Features.Settings;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Configuration;
using GpAutoLive.Media;
using GpAutoLive.Windows;
using GpAutoLive.Windows.Security;

namespace GpAutoLive.App;

public partial class MainWindow : Window
{
    private const double ResponsiveDesignWidth = 1586;
    private const double ResponsiveDesignHeight = 992;
    private const double MaximumResponsiveShellScale = 1.25;
    private const string ControlPlaneBaseUriVariable = "AUTOLIVE_CONTROL_PLANE_BASE_URI";
    private const string ControlPlaneEnvironmentVariable = "AUTOLIVE_CONTROL_PLANE_ENV";
    private const string TestControlPlaneEnvironment = "test";
    private const string LocalDevelopmentControlPlaneEnvironment = "development";
    private const string DevelopmentControlPlaneBaseUri = "http://101.96.208.132:9090";

    private readonly ShellState _state = new();
    private readonly MediaPoolService _mediaPool = new();
    private readonly InterludeFilePoolService _interludePool = new();
    private readonly InterludeAudioConfigStore? _interludeConfigStore;
    private readonly InterludeAudioSelector _interludeSelector = new();
    private readonly ControlPlaneAuthCoordinator? _authCoordinator;
    private readonly ControlPlaneHeartbeatScheduler? _heartbeatScheduler;
    private readonly LoginViewModel _login;
    private readonly DesktopPreferencesCoordinator? _preferences;
    private readonly FinalEffectWindowController _finalEffectController = new();
    private readonly MediaThumbnailCache _mediaThumbnailCache = new();
    private readonly WindowsMpvPlaybackController _mpvController = new();
    private readonly WindowsAudioPlaybackController _audioPlaybackController = new();
    private readonly WindowsRtmpOutputManager _rtmpOutputManager = new();
    private readonly WindowsRtmpAudioSession _rtmpAudioSession;
    private readonly WindowsRtmpReconnectCoordinator _rtmpReconnectCoordinator = new();
    private RtmpOutputConfig? _lastRtmpConfig;
    private readonly AudioPriorityCoordinator _audioPriority = new();
    private readonly VirtualCameraOutputManager _virtualCameraOutput = new();
    private readonly WindowsVirtualCameraSurfaceBinding _virtualCameraSurfaceBinding = new();
    private WindowsVirtualCameraSidecarProbeResult _virtualCameraSidecarProbe;
    private WindowsVirtualCameraInstallationProbeResult? _virtualCameraInstallationProbe;
    private WindowsD3D11CapabilityResult? _virtualCameraD3D11Probe;
    private WindowsGraphicsCaptureCapabilityResult? _virtualCameraWgcProbe;
    private bool _virtualCameraProbeBusy;
    private bool _virtualCameraOutputCommandBusy;
    private WindowsVirtualCameraOutputCoordinator? _virtualCameraOutputCoordinator;
    private readonly DouyinLiveManager _douyinLive = new();
    private readonly WindowsDouyinProbeHost _douyinProbeHost;
    private readonly DouyinLiveConfigStore? _douyinConfigStore;
    private readonly WindowsProcessPerformanceSampler _performanceSampler =
        new(new WindowsCurrentProcessPerformanceSource());
    private readonly LatestWinsAsyncUpdateQueue _microphoneUiUpdates;
    private readonly LatestWinsAsyncUpdateQueue _douyinUiUpdates;
    private readonly DispatcherTimer _performanceTimer;
    private readonly DispatcherTimer _spectrumTimer;
    private readonly DispatcherTimer _interludeScheduleTimer;
    private readonly InterludeSchedulePlanner _interludeSchedulePlanner = new();
    private readonly SemaphoreSlim _playbackCommandSerial = new(1, 1);
    private readonly CancellationTokenSource _windowCancellation = new();
    private InterludeAudioConfig _interludeConfig = InterludeAudioConfig.Default;
    private EffectCycleSettings _effectCycleSettings = EffectCycleSettings.Default;
    private WindowsPortAudioDeviceEnumerator? _portAudioEnumerator;
    private WindowsMicrophoneInterludeController? _microphoneInterludeController;
    private WindowsSystemSpeechAdapter? _speechAdapter;
    private CancellationTokenSource? _audioCompletionCancellation;
    private CancellationTokenSource? _rtmpReconnectCancellation;
    private Task? _audioCompletionTask;
    private Task? _interludeObservationTask;
    private bool _interludeScheduleStartInFlight;
    private CancellationTokenSource? _videoStateCancellation;
    private Task? _videoStateTask;
    private string? _speechOperationId;
    private FinalEffectWindow? _finalEffectWindow;
    private SettingsWindow? _settingsWindow;
    private MediaImportCoordinator? _mediaImporter;
    private VerifiedMediaRuntime? _verifiedMediaRuntime;
    private Task<RuntimeMediaVerificationResult>? _runtimeVerificationTask;
    private bool _importBusy;
    private bool _performanceSampleInFlight;
    private bool _isClosing;
    private MediaPlaybackIdentity? _projectedPositionIdentity;
    private MediaPlaybackIdentity? _audioPlaybackIdentity;
    private ulong? _projectedPositionMs;

    public MainWindow()
    {
        DesktopPreferencesCoordinator.TryCreateDefault(out _preferences, out var preferencesError);
        _interludeConfigStore = TryCreateInterludeConfigStore();
        _douyinConfigStore = TryCreateDouyinConfigStore();
        _authCoordinator = TryCreateAuthCoordinator();
        _heartbeatScheduler = _authCoordinator is null ? null : TryCreateHeartbeatScheduler(_authCoordinator);
        _login = new LoginViewModel(_authCoordinator);
        _douyinProbeHost = new(_douyinLive);
        _rtmpAudioSession = new WindowsRtmpAudioSession(_rtmpOutputManager);
        _virtualCameraSidecarProbe = WindowsVirtualCameraSidecarLocator.ProbeFromEnvironment();
        InitializeComponent();
        _performanceTimer = new DispatcherTimer(DispatcherPriority.Background, Dispatcher)
        {
            Interval = TimeSpan.FromSeconds(1),
        };
        _spectrumTimer = new DispatcherTimer(DispatcherPriority.Background, Dispatcher)
        {
            Interval = TimeSpan.FromMilliseconds(120),
        };
        _interludeScheduleTimer = new DispatcherTimer(DispatcherPriority.Background, Dispatcher)
        {
            Interval = TimeSpan.FromMilliseconds(250),
        };
        _microphoneUiUpdates = new(
            callback => _ = Dispatcher.InvokeAsync(callback, DispatcherPriority.Background),
            HandleBackgroundUiUpdateError);
        _douyinUiUpdates = new(
            callback => _ = Dispatcher.InvokeAsync(callback, DispatcherPriority.Background),
            HandleBackgroundUiUpdateError);
        _performanceTimer.Tick += PerformanceTimer_Tick;
        _spectrumTimer.Tick += SpectrumTimer_Tick;
        _interludeScheduleTimer.Tick += InterludeScheduleTimer_Tick;
        _interludeScheduleTimer.Start();
        _spectrumTimer.Start();
        InitializeAudioDiagnostics();
        DataContext = _state;
        _state.PropertyChanged += ShellState_PropertyChanged;
        LoginGate.DataContext = _login;
        LoginGate.IsEnabled = _authCoordinator is null;
        _login.PropertyChanged += Login_PropertyChanged;
        _douyinProbeHost.SnapshotChanged += DouyinProbeHost_SnapshotChanged;
        _rtmpOutputManager.SnapshotChanged += RtmpOutputManager_SnapshotChanged;
        _finalEffectController.StateChanged += FinalEffectController_StateChanged;
        _state.ApplyMediaSnapshot(_mediaPool.Snapshot);
        // Keep the media assembly cold on the login shell. Resource probing is deferred until
        // the authorized workbench is actually entered, matching the startup memory budget.
        MediaResourceText.Text = "外置媒体运行资源 · Windows 10/11 x64（登录后按需加载）";
        WindowsStatusText.Text = $"Windows 集成：{WindowsCapabilityBoundary.Status}";
        MediaStatusText.Text = "媒体运行时：登录后按需加载";
        UpdateMediaProjection();
        UpdateLoginProjection();
        UpdateFixedSpeechProjection();
        UpdateInterludeProjection();
        UpdateEffectCycleProjection();
        UpdateVirtualCameraProjection();
        UpdateDouyinProjection();
        if (preferencesError is not null)
        {
            _state.SetStatus(preferencesError);
        }
    }

    protected override void OnClosed(EventArgs e)
    {
        _isClosing = true;
        if (_preferences?.IsLoaded == true)
        {
            try
            {
                _ = _preferences.SaveAsync(this).GetAwaiter().GetResult();
            }
            catch (OperationCanceledException)
            {
                // 关闭过程不应因偏好保存取消而阻止窗口退出。
            }
        }

        CancelRtmpReconnect();
        _windowCancellation.Cancel();
        try
        {
            _mediaThumbnailCache.DisposeAsync().AsTask().GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
            // 关闭时取消缩略图任务，不阻止 WPF 退出。
        }
        _douyinProbeHost.SnapshotChanged -= DouyinProbeHost_SnapshotChanged;
        _rtmpOutputManager.SnapshotChanged -= RtmpOutputManager_SnapshotChanged;
        try
        {
            _douyinProbeHost.DisposeAsync().AsTask().GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
            // 关闭时 sidecar 已请求取消；不阻止 WPF 退出。
        }
        _douyinLive.Stop();
        _audioCompletionCancellation?.Cancel();
        _videoStateCancellation?.Cancel();
        _microphoneUiUpdates.Dispose();
        _douyinUiUpdates.Dispose();
        _performanceTimer.Stop();
        _performanceTimer.Tick -= PerformanceTimer_Tick;
        _spectrumTimer.Stop();
        _spectrumTimer.Tick -= SpectrumTimer_Tick;
        _interludeScheduleTimer.Stop();
        _interludeScheduleTimer.Tick -= InterludeScheduleTimer_Tick;
        _mediaImporter?.Dispose();
        _mediaImporter = null;
        _login.PropertyChanged -= Login_PropertyChanged;
        _state.PropertyChanged -= ShellState_PropertyChanged;
        _finalEffectController.StateChanged -= FinalEffectController_StateChanged;
        if (_virtualCameraOutputCoordinator is not null)
        {
            _virtualCameraOutputCoordinator.SnapshotChanged -= VirtualCameraOutputCoordinator_SnapshotChanged;
        }
        _finalEffectWindow?.Close();
        _finalEffectWindow = null;
        _settingsWindow?.Close();
        _settingsWindow = null;
        try
        {
            _microphoneInterludeController?.DisposeAsync().AsTask().GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
            // 关闭窗口时麦克风门控已请求取消；不阻止窗口退出。
        }
        if (_microphoneInterludeController is not null)
        {
            _microphoneInterludeController.SnapshotChanged -= MicrophoneInterludeController_SnapshotChanged;
        }
        _microphoneInterludeController = null;
        try
        {
            _mpvController.DisposeAsync().AsTask().GetAwaiter().GetResult();
        }
        catch (InvalidOperationException)
        {
            // 关闭窗口时媒体宿主可能已经由取消路径回收；不阻止 WPF 退出。
        }
        catch (IOException)
        {
            // 关闭窗口时命名管道可能已断开；不阻止 WPF 退出。
        }
        try
        {
            _audioPlaybackController.DisposeAsync().AsTask().GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
            // 关闭窗口时音频解码已请求取消；不阻止 WPF 退出。
        }
        try
        {
            _rtmpAudioSession.DisposeAsync().AsTask().GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
            // 关闭时 RTMP PCM 解码与分流已请求取消；不阻止窗口退出。
        }
        try
        {
            _rtmpOutputManager.DisposeAsync().AsTask().GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
            // 关闭窗口时 RTMP 进程已请求退出；不阻止 WPF 退出。
        }
        try
        {
            _heartbeatScheduler?.DisposeAsync().AsTask().GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
            // 退出时心跳循环应在取消后 Join；取消异常不阻止窗口退出。
        }
        try
        {
            _speechAdapter?.DisposeAsync().AsTask().GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
            // 退出时 SAPI 操作已请求取消；不阻止窗口退出。
        }
        _portAudioEnumerator?.Dispose();
        _portAudioEnumerator = null;
        _authCoordinator?.Dispose();
        try
        {
            _virtualCameraOutputCoordinator?.DisposeAsync().AsTask().GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
            // 关闭窗口时虚拟摄像头输出已请求取消；不阻止 WPF 退出。
        }
        _virtualCameraSurfaceBinding.Dispose();
        _windowCancellation.Dispose();
        base.OnClosed(e);
    }

    private async void ShellState_PropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (_isClosing)
        {
            return;
        }

        if (e.PropertyName is nameof(ShellState.MediaSearchText)
            or nameof(ShellState.MediaKindFilter)
            or nameof(ShellState.VisibleMediaItems))
        {
            UpdateMediaProjection();
            var snapshot = _mediaPool.Snapshot;
            if (!snapshot.SourceMediaPool.IsEmpty)
            {
                SelectMediaPoolIndex(snapshot.SourceMediaIndex);
            }

            return;
        }

        if (e.PropertyName == nameof(ShellState.VideoProcessing))
        {
            await RunPlaybackCommandAsync(
                    ApplyVideoProcessingModeAsync,
                    _windowCancellation.Token)
                .ConfigureAwait(true);
        }
        else if (e.PropertyName == nameof(ShellState.AudioProcessing))
        {
            await RunPlaybackCommandAsync(
                    ApplyAudioProcessingAsync,
                    _windowCancellation.Token)
                .ConfigureAwait(true);
        }
    }

    private void HandleBackgroundUiUpdateError(Exception _)
    {
        if (!_isClosing)
        {
            _state.SetStatus("后台状态更新失败；当前界面状态已保留");
        }
    }

    private async void Window_Loaded(object sender, RoutedEventArgs e)
    {
        ParameterScrollViewer.ScrollToTop();

        if (_authCoordinator is not null)
        {
            try
            {
                await RestoreControlPlaneSessionAsync().ConfigureAwait(true);
            }
            finally
            {
                if (!_isClosing)
                {
                    LoginGate.IsEnabled = true;
                }
            }
        }

        if (_preferences is not null)
        {
            try
            {
                var warning = await _preferences.LoadAsync(_windowCancellation.Token).ConfigureAwait(true);
                _preferences.ApplyTo(this);
                _effectCycleSettings = EffectCycleSettings.From(_preferences.Current);
                if (warning is not null)
                {
                    _state.SetStatus(warning);
                }
                ApplyRuntimePreferences();
                UpdateEffectCycleProjection();
            }
            catch (OperationCanceledException)
            {
                // 窗口关闭时取消加载；不再更新 UI。
            }
        }
        else
        {
            ApplyRuntimePreferences();
        }

        var interludeRestoreError = await LoadInterludeAudioConfigAsync().ConfigureAwait(true);
        UpdateInterludeProjection();
        if (interludeRestoreError is not null)
        {
            InterludePoolStatusText.Text = interludeRestoreError;
        }
        UpdateEffectCycleProjection();
        await LoadDouyinConfigAsync().ConfigureAwait(true);
        _ = RefreshVirtualCameraProbesAsync();

    }

    private void TitleBar_MouseLeftButtonDown(object sender, MouseButtonEventArgs e)
    {
        if (e.ClickCount == 2)
        {
            ToggleMaximize();
            return;
        }

        if (e.LeftButton == MouseButtonState.Pressed)
        {
            try
            {
                DragMove();
            }
            catch (InvalidOperationException)
            {
                // 拖动手势在窗口状态改变时可能被 WPF 取消；不应打断壳层。
            }
        }
    }

    private void Window_SizeChanged(object sender, SizeChangedEventArgs e)
    {
        if (ResponsiveShellScaleTransform is null)
        {
            return;
        }

        var scale = CalculateResponsiveShellScale(e.NewSize);
        ResponsiveShellScaleTransform.ScaleX = scale;
        ResponsiveShellScaleTransform.ScaleY = scale;
    }

    internal static double CalculateResponsiveShellScale(Size size)
    {
        if (!double.IsFinite(size.Width)
            || !double.IsFinite(size.Height)
            || size.Width <= 0
            || size.Height <= 0)
        {
            return 1;
        }

        var availableScale = Math.Min(
            size.Width / ResponsiveDesignWidth,
            size.Height / ResponsiveDesignHeight);
        return Math.Clamp(availableScale, 1, MaximumResponsiveShellScale);
    }

    private void MinimizeButton_Click(object sender, RoutedEventArgs e) =>
        WindowState = WindowState.Minimized;

    private void MaximizeButton_Click(object sender, RoutedEventArgs e) => ToggleMaximize();

    private void CloseButton_Click(object sender, RoutedEventArgs e) => Close();

    private async void LogoutButton_Click(object sender, RoutedEventArgs e)
    {
        await CancelFixedSpeechIfActiveAsync().ConfigureAwait(true);
        await _login.LogoutAsync(_windowCancellation.Token).ConfigureAwait(true);
        LoginGate.ClearPasswordInput();
    }

    private void OpenFinalEffectButton_Click(object sender, RoutedEventArgs e)
    {
        if (EnsureFinalEffectWindowVisible())
        {
            _state.SetStatus("最终效果窗口已打开；不会自动播放");
        }
    }

    private void CloseFinalEffectButton_Click(object sender, RoutedEventArgs e) =>
        _finalEffectWindow?.Close();

    private async void SettingsButton_Click(object sender, RoutedEventArgs e) =>
        await OpenSettingsAsync().ConfigureAwait(true);

    private async Task OpenSettingsAsync()
    {
        if (_isClosing)
        {
            return;
        }

        if (_preferences is not null && !_preferences.IsLoaded)
        {
            _state.SetStatus("本地偏好正在加载，请稍后再打开设置");
            return;
        }

        if (_settingsWindow?.IsVisible == true)
        {
            _settingsWindow.Activate();
            return;
        }

        var initial = DesktopSettingsDraft.From(_preferences?.Current ?? UserPreferences.Defaults);
        var window = new SettingsWindow(initial)
        {
            Owner = this,
        };
        _settingsWindow = window;

        try
        {
            if (window.ShowDialog() != true || window.Draft is null)
            {
                return;
            }

            if (_preferences is null)
            {
                _state.SetStatus("本地偏好未初始化，设置未保存");
                return;
            }

            var warning = await _preferences
                .SaveAsync(this, window.Draft, _windowCancellation.Token)
                .ConfigureAwait(true);
            if (warning is not null)
            {
                _state.SetStatus(warning);
                return;
            }

            ApplyRuntimePreferences();
            _state.SetStatus("设置已保存");
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 主窗口关闭时取消保存；不再更新 UI。
        }
        finally
        {
            if (ReferenceEquals(_settingsWindow, window))
            {
                _settingsWindow = null;
            }
        }
    }

    private void Login_PropertyChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e)
    {
        UpdateLoginProjection();
        if (e.PropertyName is nameof(LoginViewModel.Message) or nameof(LoginViewModel.Status) or null)
        {
            _state.SetStatus(_login.Message);
        }

        if (e.PropertyName == nameof(LoginViewModel.Status)
            && _login.Status is LoginStatus.Activated or LoginStatus.Offline)
        {
            _heartbeatScheduler?.Start();
            _ = SendHeartbeatNowSafelyAsync();
        }
        else if (e.PropertyName == nameof(LoginViewModel.Status)
            && !_login.CanEnterWorkbench)
        {
            _ = StopMicrophoneAsync();
            _ = RunPlaybackCommandAsync(StopVirtualCameraCoreAsync);
            StopDouyinSession();
        }
    }

    private void FinalEffectController_StateChanged(object? sender, EventArgs e) => UpdateFinalEffectButtons();

    private async void FinalEffectWindow_Closed(object? sender, EventArgs e)
    {
        if (!_isClosing
            && _virtualCameraOutputCoordinator is not null
            && _virtualCameraOutputCoordinator.HasActiveResources)
        {
            await StopVirtualCameraCoreAsync().ConfigureAwait(true);
        }

        _virtualCameraSurfaceBinding.Unbind();
        if (_finalEffectWindow is not null)
        {
            _finalEffectWindow.Closed -= FinalEffectWindow_Closed;
        }

        _finalEffectWindow = null;
        _finalEffectController.Close();
        UpdateFinalEffectButtons();
        if (!_isClosing)
        {
            await RunPlaybackCommandAsync(
                    StopPlaybackAfterFinalEffectWindowClosedAsync,
                    _windowCancellation.Token)
                .ConfigureAwait(true);
        }
    }

    private void UpdateLoginProjection()
    {
        var canEnterWorkbench = _login.CanEnterWorkbench;
        LoginGate.Visibility = canEnterWorkbench ? Visibility.Collapsed : Visibility.Visible;
        WorkbenchSurface.IsEnabled = canEnterWorkbench;
        PlaybackBar.IsEnabled = canEnterWorkbench;
        OperationBar.IsEnabled = canEnterWorkbench;
        AuthStatusText.Text = _login.StatusLabel;
        DeviceStatusText.Text = canEnterWorkbench ? "设备已授权" : _login.StatusLabel;
        if (DeviceStatusText.TryFindResource(canEnterWorkbench ? "AccentBrush" : "WarningBrush") is System.Windows.Media.Brush deviceStatusBrush)
        {
            DeviceStatusText.Foreground = deviceStatusBrush;
        }

        var account = canEnterWorkbench && !string.IsNullOrWhiteSpace(_login.Account)
            ? _login.Account.Trim()
            : "—";
        AccountStatusText.Text = $"账户：{account}";
        var activationExpiresAt = _authCoordinator?.Snapshot.ActivationExpiresAt;
        ActivationExpiryText.Text = activationExpiresAt is { } expiresAt
            ? $"授权到期：{expiresAt.ToLocalTime():yyyy/M/d HH:mm:ss}"
            : "授权到期：—";
        LogoutMenuItem.IsEnabled = _login.CanLogout;
        LogoutMenuItem.Visibility = _login.CanLogout ? Visibility.Visible : Visibility.Collapsed;
        if (canEnterWorkbench)
        {
            MediaResourceText.Text = MediaRuntimeBoundary.ResourceSummary;
            MediaStatusText.Text = $"媒体运行时：{MediaRuntimeBoundary.Status}";
        }

        SetImportButtonsEnabled(canEnterWorkbench && !_importBusy);
        UpdateFixedSpeechProjection();
        UpdateInterludeProjection();
        UpdateEffectCycleProjection();
        UpdateMicrophoneProjection();
        UpdateRtmpProjection();
        UpdateVirtualCameraProjection();
        UpdateDouyinProjection();
    }

    private void SetStatusPill(System.Windows.Controls.TextBlock target, string text, bool active)
    {
        target.Text = text;
        if (target.TryFindResource(active ? "AccentBrush" : "WarningBrush") is System.Windows.Media.Brush statusBrush)
        {
            target.Foreground = statusBrush;
        }
    }

    private AudioPcmMixPolicy CreateBaseAudioMixPolicy()
    {
        return CreateAudioMixPolicy(
            _audioPriority.Snapshot,
            GetOutputVolumeGainDb(),
            _interludeConfig.DuckingDepthDb,
            _interludeConfig.VolumeDb);
    }

    /// <summary>
    /// 将优先级快照转换为最终 PCM 混音策略。
    /// 固定话术和麦克风都写入同一 overlay 总线；它们只应静音/duck 基础轨，
    /// 不能因为 <see cref="AudioPrioritySnapshot.InterludeMuted" /> 而把自身也静音。
    /// </summary>
    internal static AudioPcmMixPolicy CreateAudioMixPolicy(
        AudioPrioritySnapshot priority,
        double outputGainDb,
        double duckingDepthDb,
        double overlayGainDb) =>
        new(
            BaseGainDb: outputGainDb,
            BaseDuckingDb: priority.MediaDucked ? duckingDepthDb : 0,
            MuteBase: priority.MediaMuted,
            OverlayGainDb: overlayGainDb,
            MuteOverlay: false);

    private static ControlPlaneAuthCoordinator? TryCreateAuthCoordinator()
    {
        var controlPlaneEnvironment = Environment.GetEnvironmentVariable(ControlPlaneEnvironmentVariable)?.Trim();
        var isTestEnvironment = string.Equals(
            controlPlaneEnvironment,
            TestControlPlaneEnvironment,
            StringComparison.OrdinalIgnoreCase);
        var isLocalDevelopmentEnvironment = string.Equals(
            controlPlaneEnvironment,
            LocalDevelopmentControlPlaneEnvironment,
            StringComparison.OrdinalIgnoreCase);
        var allowsDevelopmentHttp = isTestEnvironment || isLocalDevelopmentEnvironment;
        var baseUriText = Environment.GetEnvironmentVariable(ControlPlaneBaseUriVariable);
        if (allowsDevelopmentHttp && string.IsNullOrWhiteSpace(baseUriText))
        {
            baseUriText = DevelopmentControlPlaneBaseUri;
        }

        if (string.IsNullOrWhiteSpace(baseUriText)
            || !Uri.TryCreate(baseUriText.Trim(), UriKind.Absolute, out var baseUri))
        {
            return null;
        }

        try
        {
            var secretStore = new CredentialManagerSecretStore();
            var deviceId = WindowsDeviceIdentity.GetOrCreate(secretStore);
            var version = typeof(MainWindow).Assembly.GetName().Version?.ToString() ?? "0.1.0";
            var registration = new DeviceRegistrationDto(
                ControlPlaneContractValues.Product,
                deviceId,
                "GPAL Desktop",
                "windows",
                version,
                Environment.OSVersion.VersionString[..Math.Min(Environment.OSVersion.VersionString.Length, AuthInputLimits.OsVersionMaxLength)]);
            var handler = new HttpClientHandler
            {
                AllowAutoRedirect = false,
                UseCookies = false,
            };
            var httpClient = new HttpClient(handler)
            {
                Timeout = Timeout.InfiniteTimeSpan,
            };
            var transport = new ControlPlaneHttpClient(
                httpClient,
                new ControlPlaneHttpClientOptions
                {
                    BaseUri = baseUri,
                    AllowLoopbackHttp = allowsDevelopmentHttp,
                    AllowDevelopmentTestHttp = allowsDevelopmentHttp
                        && ControlPlaneHttpClientOptions.IsFixedDevelopmentTestHttp(baseUri),
                });
            return new ControlPlaneAuthCoordinator(transport, secretStore, registration, httpClient);
        }
        catch (ArgumentException)
        {
            return null;
        }
        catch (InvalidDataException)
        {
            return null;
        }
        catch (System.ComponentModel.Win32Exception)
        {
            return null;
        }
        catch (System.Security.Cryptography.CryptographicException)
        {
            return null;
        }
        catch (System.Security.SecurityException)
        {
            return null;
        }
    }

    private ControlPlaneHeartbeatScheduler? TryCreateHeartbeatScheduler(
        ControlPlaneAuthCoordinator auth)
    {
        var localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        if (string.IsNullOrWhiteSpace(localAppData))
        {
            return null;
        }

        try
        {
            var outboxPath = Path.Combine(localAppData, "GpAutoLive", "outbox", "heartbeat.json");
            return new ControlPlaneHeartbeatScheduler(
                auth,
                CreateHeartbeatStatus,
                new HeartbeatOutboxStore(outboxPath),
                transitionObserver: transition =>
                {
                    _ = Dispatcher.InvokeAsync(() =>
                    {
                        if (!_isClosing)
                        {
                            _login.ApplyTransition(transition);
                        }
                    });
                });
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

    private async Task RestoreControlPlaneSessionAsync()
    {
        if (_authCoordinator is null || _isClosing)
        {
            return;
        }

        try
        {
            var pendingLogoutWarning = await _authCoordinator
                .RetryPendingLogoutAsync(_windowCancellation.Token)
                .ConfigureAwait(true);
            var transition = await _authCoordinator.RestoreAsync(_windowCancellation.Token).ConfigureAwait(true);
            if (_isClosing || _windowCancellation.IsCancellationRequested)
            {
                return;
            }

            if (transition is not null)
            {
                _login.ApplyTransition(transition);
                _state.SetStatus(transition.Error?.Message ?? "已尝试恢复控制面会话");
            }

            if (pendingLogoutWarning is not null)
            {
                _login.ApplyWarning(pendingLogoutWarning);
                _state.SetStatus(pendingLogoutWarning);
            }

            if (_login.CanEnterWorkbench)
            {
                _heartbeatScheduler?.Start();
            }
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 窗口关闭时不再投影恢复结果。
        }
        catch (InvalidOperationException exception)
        {
            _login.ApplyError(exception.Message);
        }
        catch (Exception)
        {
            _login.ApplyError("控制面会话恢复失败，请重新登录。");
        }
    }

    private async Task SendHeartbeatNowSafelyAsync()
    {
        try
        {
            await (_heartbeatScheduler?.SendNowAsync(_windowCancellation.Token) ?? Task.CompletedTask)
                .ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (_windowCancellation.IsCancellationRequested)
        {
            // 关闭窗口时取消即时心跳。
        }
    }

    private HeartbeatStatusDto CreateHeartbeatStatus()
    {
        long diskFreeBytes = 0;
        try
        {
            var systemRoot = Path.GetPathRoot(Environment.SystemDirectory);
            if (!string.IsNullOrWhiteSpace(systemRoot))
            {
                diskFreeBytes = Math.Max(0, new DriveInfo(systemRoot).AvailableFreeSpace);
            }
        }
        catch (IOException)
        {
            // 磁盘指标不可用时按 0 上报，不能阻断心跳。
        }
        catch (UnauthorizedAccessException)
        {
            // 同上。
        }

        var playbackState = MapHeartbeatPlaybackState(_mediaPool.Snapshot.PlaybackState);
        return new HeartbeatStatusDto(
            diskFreeBytes,
            Environment.WorkingSet,
            null,
            Environment.ProcessorCount,
            "Windows",
            Environment.OSVersion.VersionString[..Math.Min(Environment.OSVersion.VersionString.Length, AuthInputLimits.OsVersionMaxLength)],
            null,
            null,
            playbackState);
    }

    internal static string MapHeartbeatPlaybackState(PlaybackState state) => state switch
    {
        PlaybackState.Ready or PlaybackState.Stopped => "idle",
        PlaybackState.Playing => "playing",
        PlaybackState.Paused => "paused",
        _ => "error",
    };

    private void UpdateFinalEffectButtons()
    {
        var isOpen = _finalEffectWindow?.IsVisible == true && _finalEffectController.IsOpen;
        OpenFinalEffectButton.IsEnabled = !isOpen;
        CloseFinalEffectButton.IsEnabled = isOpen;
    }

    private bool EnsureFinalEffectWindowVisible()
    {
        if (_finalEffectWindow is null)
        {
            _finalEffectWindow = new FinalEffectWindow(_finalEffectController)
            {
                Owner = this
            };
            _finalEffectWindow.Closed += FinalEffectWindow_Closed;
        }

        _finalEffectController.Open(CreateFinalEffectSnapshot());
        if (!_finalEffectWindow.IsVisible)
        {
            _finalEffectWindow.Show();
        }
        else
        {
            if (_finalEffectWindow.WindowState == WindowState.Minimized)
            {
                _finalEffectWindow.WindowState = WindowState.Normal;
            }

            _finalEffectWindow.Activate();
        }

        _finalEffectWindow.UpdateLayout();
        RefreshVirtualCameraSurfaceBinding();
        UpdateFinalEffectButtons();
        return true;
    }

    private void ToggleMaximize() =>
        WindowState = WindowState == WindowState.Maximized ? WindowState.Normal : WindowState.Maximized;
}
