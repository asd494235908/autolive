using System.ComponentModel;
using System.Windows.Threading;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private Task? _shutdownTask;
    private Task? _loadedTask;
    private Task? _finalEffectCloseTask;
    private bool _allowClose;
    internal Task? ShutdownCompletion => _shutdownTask;

    protected override void OnClosing(CancelEventArgs e)
    {
        base.OnClosing(e);
        if (_allowClose || e.Cancel) return;
        e.Cancel = true;
        if (_shutdownTask is { IsCompleted: false }) return;
        _isClosing = true;
        _shutdownTask = ShutdownAndCloseAsync();
    }

    private async Task ShutdownAndCloseAsync()
    {
        // Leave the Closing callback before calling Close again, even when all resources are idle.
        await Dispatcher.Yield(DispatcherPriority.Normal);
        try
        {
            CancelRtmpReconnect();
            _windowCancellation.Cancel();
            _performanceTimer.Stop();
            _spectrumTimer.Stop();
            _interludeScheduleTimer.Stop();
            _microphoneUiUpdates.Dispose();
            _douyinUiUpdates.Dispose();
            _login.PropertyChanged -= Login_PropertyChanged;
            _state.PropertyChanged -= ShellState_PropertyChanged;
            _douyinProbeHost.SnapshotChanged -= DouyinProbeHost_SnapshotChanged;
            CloseDouyinChatWindow();
            _douyinProbeHost.ClearChatMessages();
            _rtmpOutputManager.SnapshotChanged -= RtmpOutputManager_SnapshotChanged;
            _finalEffectController.StateChanged -= FinalEffectController_StateChanged;

            // Existing commands retain the Dispatcher until their finally blocks release these gates.
            if (!await _playbackCommandSerial.WaitAsync(TimeSpan.FromSeconds(10)).ConfigureAwait(true))
                throw new TimeoutException("播放命令尚未退出");
            _playbackCommandSerial.Release();
            if (!await _microphoneLifecycle.WaitAsync(TimeSpan.FromSeconds(10)).ConfigureAwait(true))
                throw new TimeoutException("麦克风命令尚未退出");
            _microphoneLifecycle.Release();
            await JoinWindowOperationAsync(_finalEffectCloseTask).ConfigureAwait(true);
            await JoinWindowOperationAsync(_loadedTask).ConfigureAwait(true);
            await JoinWindowOperationAsync(_douyinAuthorizationStopTask).ConfigureAwait(true);
            await JoinWindowOperationAsync(_douyinManualSendTask).ConfigureAwait(true);
            await StopAudioCompletionWatcherAsync().ConfigureAwait(true);
            await StopVideoStateWatcherAsync().ConfigureAwait(true);

            if (_preferences?.IsLoaded == true)
                _ = await _preferences.SaveAsync(this).ConfigureAwait(true);
            await _mediaThumbnailCache.DisposeAsync().ConfigureAwait(true);
            await _douyinProbeHost.DisposeAsync().ConfigureAwait(true);
            _douyinLive.Stop();
            _mediaImporter?.Dispose();
            _mediaImporter = null;
            _finalEffectWindow?.Close();
            _settingsWindow?.Close();
            _settingsWindow = null;

            if (_microphoneInterludeController is not null)
            {
                await _microphoneInterludeController.DisposeAsync().ConfigureAwait(true);
                _microphoneInterludeController.SnapshotChanged -= MicrophoneInterludeController_SnapshotChanged;
                _microphoneInterludeController = null;
            }
            await _rtmpAudioSession.DisposeAsync().ConfigureAwait(true);
            await _rtmpOutputManager.DisposeAsync().ConfigureAwait(true);
            await _audioPlaybackController.DisposeAsync().ConfigureAwait(true);
            await JoinWindowOperationAsync(_interludeObservationTask).ConfigureAwait(true);
            await _mpvController.DisposeAsync().ConfigureAwait(true);
            if (_heartbeatScheduler is not null) await _heartbeatScheduler.DisposeAsync().ConfigureAwait(true);
            if (_speechAdapter is not null) await _speechAdapter.DisposeAsync().ConfigureAwait(true);
            _portAudioEnumerator?.Dispose();
            _portAudioEnumerator = null;
            _authCoordinator?.Dispose();
            if (_virtualCameraOutputCoordinator is not null)
            {
                _virtualCameraOutputCoordinator.SnapshotChanged -= VirtualCameraOutputCoordinator_SnapshotChanged;
                await _virtualCameraOutputCoordinator.DisposeAsync().ConfigureAwait(true);
            }
            _virtualCameraSurfaceBinding.Dispose();
            _performanceTimer.Tick -= PerformanceTimer_Tick;
            _spectrumTimer.Tick -= SpectrumTimer_Tick;
            _interludeScheduleTimer.Tick -= InterludeScheduleTimer_Tick;
            _windowCancellation.Dispose();
            _allowClose = true;
            Close();
        }
        catch (Exception error)
        {
            // Keep failed resource owners and the window alive. A later Close retries their cleanup.
            System.Diagnostics.Trace.TraceWarning("Window shutdown requires retry: {0}", error.GetType().Name);
            _state.SetStatus("资源退出尚未完成；已停止接受新操作，请再次关闭重试。");
        }
    }

    private static async Task JoinWindowOperationAsync(Task? operation)
    {
        if (operation is null) return;
        try
        {
            await operation.WaitAsync(TimeSpan.FromSeconds(10)).ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (operation.IsCompleted)
        {
            // Cancellation has already completed; the resource owners are disposed separately below.
        }
        catch (Exception error) when (operation.IsCompleted)
        {
            System.Diagnostics.Trace.TraceWarning("Window operation ended before shutdown: {0}", error.GetType().Name);
        }
    }
}
