using System.Windows;
using System.Windows.Controls;
using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private bool _audioDevicesEnumerated;
    private bool _audioDeviceListCurrent;

    private Task RefreshAudioDevicesAsync() => RunPlaybackCommandAsync(async () =>
    {
        if (_mediaPool.Snapshot.PlaybackState is PlaybackState.Playing or PlaybackState.Paused)
        {
            AudioDeviceStatusText.Text = "请先停止播放和麦克风，再刷新设备";
            return;
        }
        await RefreshAudioDevicesCoreAsync().ConfigureAwait(true);
    });

    private async void RefreshAudioDevicesButton_Click(object sender, RoutedEventArgs e) =>
        await RefreshAudioDevicesAsync().ConfigureAwait(true);

    internal static bool CanRefreshAudioDevices(
        WindowsAudioPlaybackState outputState, WindowsMicrophoneInterludeState inputState) =>
        outputState is not (WindowsAudioPlaybackState.Starting or WindowsAudioPlaybackState.Playing
            or WindowsAudioPlaybackState.Paused or WindowsAudioPlaybackState.Stopping or WindowsAudioPlaybackState.Closed)
        && inputState is not (WindowsMicrophoneInterludeState.Starting or WindowsMicrophoneInterludeState.Listening
            or WindowsMicrophoneInterludeState.Stopping or WindowsMicrophoneInterludeState.Closed);

    internal static WindowsPortAudioDevice? MatchAudioDevice(
        WindowsPortAudioDevice[] devices, WindowsPortAudioDevice? previous, int? defaultIndex, bool firstEnumeration)
    {
        if (previous is null)
            return firstEnumeration ? devices.FirstOrDefault(device => device.Index == defaultIndex) : null;
        var matches = devices.Where(device => device.Name == previous.Name && device.HostApi == previous.HostApi)
            .Take(2).ToArray();
        return matches.Length == 1 ? matches[0] : null;
    }

    // 调用者持有播放命令闸门；首次播放准备直接进入此处，避免重复获取同一闸门。
    private async Task RefreshAudioDevicesCoreAsync()
    {
        if (_isClosing || !_login.CanEnterWorkbench) return;
        await _microphoneLifecycle.WaitAsync(_windowCancellation.Token).ConfigureAwait(true);
        try
        {
            if (!CanRefreshAudioDevices(_audioPlaybackController.Snapshot.State,
                    _microphoneInterludeController?.Snapshot.State ?? WindowsMicrophoneInterludeState.Idle))
            {
                AudioDeviceStatusText.Text = "请先停止播放和麦克风，再刷新设备";
                return;
            }

            var previousOutput = AudioOutputDeviceComboBox.SelectedItem as WindowsPortAudioDevice;
            var previousInput = AudioInputDeviceComboBox.SelectedItem as WindowsPortAudioDevice;
            var firstEnumeration = !_audioDevicesEnumerated;
            _audioDeviceListCurrent = false;
            UpdateAudioDeviceProjection();
            AudioDeviceStatusText.Text = "正在刷新 PortAudio 输入/输出设备…";

            // Failed/Completed 仍可能持有原生资源；必须由原所有者确认释放后才能重新初始化。
            var stoppedOutput = await _audioPlaybackController.StopAsync().ConfigureAwait(true);
            if (!stoppedOutput.IsSuccess)
            {
                AudioDeviceStatusText.Text = stoppedOutput.Error?.Message ?? "输出资源尚未释放，请停止后重试";
                return;
            }
            if (_microphoneInterludeController is not null)
            {
                var stoppedInput = await _microphoneInterludeController.StopAsync().ConfigureAwait(true);
                if (!stoppedInput.IsSuccess)
                {
                    AudioDeviceStatusText.Text = stoppedInput.Error?.Message ?? "输入资源尚未释放，请停止后重试";
                    return;
                }
            }

            var runtime = await EnsureVerifiedMediaRuntimeAsync().ConfigureAwait(true);
            if (runtime is null || !runtime.TryGetResource("portaudio_x64.dll", out var resource) || resource is null)
            {
                AudioDeviceStatusText.Text = "PortAudio 运行资源未校验或未安装";
                return;
            }
            _portAudioEnumerator ??= new WindowsPortAudioDeviceEnumerator();
            var result = await _portAudioEnumerator.ProbeAsync(resource.AbsolutePath, _windowCancellation.Token)
                .ConfigureAwait(true);
            if (_isClosing || _windowCancellation.IsCancellationRequested) return;
            if (!result.IsSuccess)
            {
                AudioDeviceStatusText.Text = result.Error?.Message ?? "PortAudio 设备刷新失败，请重试";
                return;
            }

            var outputs = result.Snapshot.Devices.Where(device => device.MaxOutputChannels > 0).ToArray();
            var inputs = result.Snapshot.Devices.Where(device => device.MaxInputChannels > 0).ToArray();
            var selectedOutput = MatchAudioDevice(outputs, previousOutput, result.Snapshot.DefaultOutputDevice, firstEnumeration);
            var selectedInput = MatchAudioDevice(inputs, previousInput, null, firstEnumeration: false);
            AudioOutputDeviceComboBox.ItemsSource = outputs;
            AudioOutputDeviceComboBox.SelectedItem = selectedOutput;
            AudioInputDeviceComboBox.ItemsSource = inputs;
            AudioInputDeviceComboBox.SelectedItem = selectedInput;
            _audioDevicesEnumerated = true;
            _audioDeviceListCurrent = true;
            AudioDeviceStatusText.Text = outputs.Length == 0
                ? "未发现输出设备；连接设备后点击刷新"
                : selectedOutput is null
                    ? firstEnumeration ? "系统默认输出设备不可用，请手动选择输出设备"
                        : "请手动选择输出设备；原设备可能已断开或存在同名设备"
                    : $"已选择 {selectedOutput.Name} · {selectedOutput.HostApi}；下次播放生效";
            _state.SetStatus($"设备刷新完成：{outputs.Length} 个输出、{inputs.Length} 个输入");
        }
        finally
        {
            _microphoneLifecycle.Release();
            UpdateMicrophoneProjection();
        }
    }

    private void AudioOutputDeviceComboBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_audioDeviceListCurrent && AudioOutputDeviceComboBox.SelectedItem is WindowsPortAudioDevice device)
            AudioDeviceStatusText.Text = $"已选择 {device.Name} · {device.HostApi}；下次播放生效";
    }

    private void UpdateAudioDeviceProjection()
    {
        if (!IsInitialized || _isClosing) return;
        var canEdit = _login.CanEnterWorkbench && !_importBusy
            && _playbackCommandSerial.CurrentCount > 0 && _microphoneLifecycle.CurrentCount > 0
            && _mediaPool.Snapshot.PlaybackState is not (PlaybackState.Playing or PlaybackState.Paused)
            && CanRefreshAudioDevices(_audioPlaybackController.Snapshot.State,
                _microphoneInterludeController?.Snapshot.State ?? WindowsMicrophoneInterludeState.Idle);
        RefreshAudioDevicesButton.IsEnabled = canEdit;
        AudioOutputDeviceComboBox.IsEnabled = canEdit && _audioDeviceListCurrent
            && AudioOutputDeviceComboBox.Items.Count > 0;
        AudioOutputDeviceComboBox.ToolTip = canEdit
            ? "所选设备在下次播放生效" : "停止播放和麦克风后可刷新、选择输出设备";
    }
}
