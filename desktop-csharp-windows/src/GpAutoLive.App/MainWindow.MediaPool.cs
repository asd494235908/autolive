using System.IO;
using Microsoft.Win32;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using GpAutoLive.App.Features.Media;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Configuration;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private MediaListItemViewModel? _pendingMediaSelection;

    private async void ImportButton_Click(object sender, RoutedEventArgs e) =>
        await ImportMediaAsync().ConfigureAwait(true);

    private async void ImportPlaylistButton_Click(object sender, RoutedEventArgs e) =>
        await ImportPlaylistAsync().ConfigureAwait(true);

    private void MediaSearchTextBox_TextChanged(object sender, TextChangedEventArgs e)
    {
        if (!_isClosing && sender is TextBox searchBox)
        {
            _state.MediaSearchText = searchBox.Text;
        }
    }

    private void MediaPool_DragOver(object sender, DragEventArgs e)
    {
        e.Effects = _login.CanEnterWorkbench
            && !_importBusy
            && MediaDropPayload.HasCandidateFiles(e.Data)
            ? DragDropEffects.Copy
            : DragDropEffects.None;
        e.Handled = true;
    }

    private async void MediaPool_Drop(object sender, DragEventArgs e)
    {
        e.Handled = true;
        if (!_login.CanEnterWorkbench)
        {
            e.Effects = DragDropEffects.None;
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        if (_importBusy)
        {
            e.Effects = DragDropEffects.None;
            return;
        }

        if (!MediaDropPayload.TryReadPaths(e.Data, out var paths))
        {
            e.Effects = DragDropEffects.None;
            _state.SetStatus("拖放内容无有效媒体文件，已保留原播放池");
            return;
        }

        e.Effects = DragDropEffects.Copy;
        await RunImportAsync(() => Task.FromResult<MediaImportRequest?>(
            new(MediaImportOperation.Append, paths))).ConfigureAwait(true);
    }

    private void MediaListBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (e.AddedItems.Count > 0
            && MediaListBox.SelectedItem is MediaListItemViewModel selected
            && !ReferenceEquals(selected, _pendingMediaSelection))
        {
            _pendingMediaSelection = null;
        }

        if (!_login.CanEnterWorkbench || _importBusy || _isClosing)
        {
            SetMediaMutationButtonsEnabled(false);
            return;
        }

        UpdateMediaProjection();
        var selectedIndex = GetSelectedMediaPoolIndex();
        if (e.AddedItems.Count == 0 || selectedIndex < 0)
        {
            return;
        }

        var snapshot = _mediaPool.Snapshot;
        if (selectedIndex == snapshot.SourceMediaIndex)
        {
            return;
        }

        if (snapshot.PlaybackState is PlaybackState.Playing or PlaybackState.Paused)
        {
            SelectMediaPoolIndex(snapshot.SourceMediaIndex);
            _state.SetStatus("播放中请使用上一项/下一项切换，避免列表选择与输出会话脱节");
            return;
        }

        var result = _mediaPool.SelectAt(selectedIndex, _mediaPool.CurrentIdentity);
        ApplyMediaOperation(
            result,
            result.IsSuccess ? $"已选择第 {selectedIndex + 1} 项媒体" : result.Error?.Message ?? "选择媒体失败");
    }

    private async void MoveUpButton_Click(object sender, RoutedEventArgs e)
    {
        if (!_login.CanEnterWorkbench || _importBusy || _isClosing)
        {
            return;
        }

        var selectedIndex = GetSelectedMediaPoolIndex();
        if (selectedIndex <= 0)
        {
            return;
        }

        if (!await StopMediaForMutationAsync().ConfigureAwait(true))
        {
            return;
        }

        var result = _mediaPool.MoveUp(selectedIndex);
        ApplyMediaOperation(result, result.IsSuccess ? "媒体已上移" : result.Error?.Message ?? "媒体上移失败");
        if (result.IsSuccess)
        {
            SelectMediaPoolIndex(selectedIndex - 1);
        }
    }

    private async void MoveDownButton_Click(object sender, RoutedEventArgs e)
    {
        if (!_login.CanEnterWorkbench || _importBusy || _isClosing)
        {
            return;
        }

        var selectedIndex = GetSelectedMediaPoolIndex();
        if (selectedIndex < 0 || selectedIndex >= _state.MediaItems.Count - 1)
        {
            return;
        }

        if (!await StopMediaForMutationAsync().ConfigureAwait(true))
        {
            return;
        }

        var result = _mediaPool.MoveDown(selectedIndex);
        ApplyMediaOperation(result, result.IsSuccess ? "媒体已下移" : result.Error?.Message ?? "媒体下移失败");
        if (result.IsSuccess)
        {
            SelectMediaPoolIndex(selectedIndex + 1);
        }
    }

    private async void RemoveMediaButton_Click(object sender, RoutedEventArgs e) =>
        await RemoveSelectedMediaAsync().ConfigureAwait(true);

    private async Task RemoveSelectedMediaAsync()
    {
        if (!_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        if (_importBusy || _isClosing)
        {
            return;
        }

        var selectedIndex = GetSelectedMediaPoolIndex();
        if (selectedIndex < 0)
        {
            return;
        }

        if (!ConfirmMediaMutation(
                "移除媒体",
                "确定移除当前选中的媒体项吗？\n\n当前播放池中的其他项目不会改变。"))
        {
            _state.SetStatus("已取消移除媒体");
            return;
        }

        if (!await StopMediaForMutationAsync().ConfigureAwait(true))
        {
            return;
        }

        var result = _mediaPool.RemoveAt(selectedIndex);
        ApplyMediaOperation(result, result.IsSuccess ? "媒体已移除" : result.Error?.Message ?? "移除媒体失败");
        if (result.IsSuccess && result.Snapshot.SourceMediaPool.Length > 0)
        {
            SelectMediaPoolIndex(Math.Min(selectedIndex, result.Snapshot.SourceMediaPool.Length - 1));
        }
    }

    private async void ClearMediaButton_Click(object sender, RoutedEventArgs e)
    {
        if (!_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        if (_importBusy || _isClosing)
        {
            return;
        }

        if (!ConfirmMediaMutation(
                "清空媒体池",
                "确定清空全部媒体吗？\n\n此操作只会移除播放池中的引用，不会删除本地文件。"))
        {
            _state.SetStatus("已取消清空媒体池");
            return;
        }

        if (!await StopMediaForMutationAsync().ConfigureAwait(true))
        {
            return;
        }

        var result = _mediaPool.Clear();
        ApplyMediaOperation(result, result.IsSuccess ? "媒体池已清空" : result.Error?.Message ?? "清空媒体池失败");
    }

    /// <summary>
    /// 媒体池成功编辑前统一停止所有输出资源，避免旧媒体身份在池修订后继续运行。
    /// 调用方仍负责在该方法成功后提交原子播放池编辑。
    /// </summary>
    private async Task<bool> StopMediaForMutationAsync()
    {
        if (!await StopRtmpForMediaMutationAsync().ConfigureAwait(true))
        {
            return false;
        }

        if (!await StopVirtualCameraForMediaMutationAsync().ConfigureAwait(true))
        {
            return false;
        }

        await StopVideoStateWatcherAsync().ConfigureAwait(true);
        await StopAudioCompletionWatcherAsync().ConfigureAwait(true);
        await StopInterludeForPriorityAsync().ConfigureAwait(true);
        if (!await StopMicrophoneAsync().ConfigureAwait(true))
        {
            return false;
        }
        var audioState = _audioPlaybackController.Snapshot.State;
        if (audioState is WindowsAudioPlaybackState.Starting
            or WindowsAudioPlaybackState.Playing
            or WindowsAudioPlaybackState.Paused
            or WindowsAudioPlaybackState.Stopping)
        {
            var stoppedAudio = await _audioPlaybackController.StopAsync().ConfigureAwait(true);
            if (!stoppedAudio.IsSuccess)
            {
                _state.SetStatus(stoppedAudio.Error?.Message ?? "媒体池编辑前停止纯音频失败");
                return false;
            }

            AudioDeviceStatusText.Text = "PortAudio 输出流已停止，等待媒体池编辑";
        }

        var controllerIdentity = _mpvController.Snapshot.ActiveIdentity;
        if (controllerIdentity is MediaPlaybackIdentity identity)
        {
            var stoppedVideo = await _mpvController.ShutdownAsync(_windowCancellation.Token)
                .ConfigureAwait(true);
            if (!stoppedVideo.IsSuccess)
            {
                _state.SetStatus(stoppedVideo.Error?.Message ?? "媒体池编辑前停止视频失败");
                return false;
            }

            _state.SetStatus($"已停止旧视频会话（媒体代次 {identity.PlaybackGeneration}）");
        }

        var playbackState = _mediaPool.Snapshot.PlaybackState;
        if (playbackState is PlaybackState.Playing or PlaybackState.Paused)
        {
            var stoppedPool = _mediaPool.StopPlayback();
            if (!stoppedPool.IsSuccess)
            {
                _state.SetStatus(stoppedPool.Error?.Message ?? "媒体池编辑前提交停止状态失败");
                return false;
            }
        }

        return true;
    }

    private void ApplyMediaOperation(MediaPoolOperationResult result, string status) =>
        ApplyMediaSnapshot(result.Snapshot, status);

    private void ApplyMediaOperation(MediaImportResult result, string status) =>
        ApplyMediaSnapshot(result.IsSuccess ? result.Snapshot : _mediaPool.Snapshot, status);

    private void ApplyMediaSnapshot(AppState snapshot, string status)
    {
        _state.ApplyMediaSnapshot(snapshot);
        SyncVirtualCameraOutputContext(snapshot);
        SelectMediaPoolIndex(snapshot.SourceMediaPool.IsEmpty ? -1 : snapshot.SourceMediaIndex);
        UpdateMediaProjection();
        StartMediaThumbnailLoad();
        _state.SetStatus(status);
        _finalEffectController.Update(CreateFinalEffectSnapshot());
        SetImportButtonsEnabled(_login.CanEnterWorkbench && !_importBusy);
    }

    private Task ImportMediaAsync() => RunImportAsync(CreateOpenFileImportRequestAsync);

    private Task ImportPlaylistAsync() => RunImportAsync(CreateOpenPlaylistImportRequestAsync);

    private Task<MediaImportRequest?> CreateOpenFileImportRequestAsync()
    {
        var dialog = new OpenFileDialog
        {
            Multiselect = true,
            CheckFileExists = true,
            Filter = BuildMediaFilter(),
            Title = "选择要加入播放池的媒体",
        };
        return Task.FromResult<MediaImportRequest?>(dialog.ShowDialog(this) == true
            ? CreateFileImportRequest(dialog.FileNames)
            : null);
    }

    internal static MediaImportRequest CreateFileImportRequest(IReadOnlyList<string?> paths) =>
        new(MediaImportOperation.Append, paths);

    private async Task<MediaImportRequest?> CreateOpenPlaylistImportRequestAsync()
    {
        var dialog = new OpenFileDialog
        {
            Multiselect = false,
            CheckFileExists = true,
            Filter = "GpAutoLive 媒体列表 (*.json)|*.json",
            Title = "选择要导入的媒体列表",
        };
        if (dialog.ShowDialog(this) != true)
        {
            return null;
        }

        var paths = await new MediaPlaylistReader(dialog.FileName)
            .ReadPathsAsync(_windowCancellation.Token)
            .ConfigureAwait(true);
        return new MediaImportRequest(MediaImportOperation.ReplaceAll, paths);
    }

    private async Task RunImportAsync(Func<Task<MediaImportRequest?>> requestFactory)
    {
        if (!_login.CanEnterWorkbench)
        {
            _state.SetStatus("请先完成登录与设备授权");
            return;
        }

        if (_importBusy)
        {
            return;
        }

        _importBusy = true;
        SetImportButtonsEnabled(false);
        var playbackGateAcquired = false;
        try
        {
            var request = await requestFactory().ConfigureAwait(true);
            if (request is null || _isClosing)
            {
                return;
            }

            await _playbackCommandSerial
                .WaitAsync(_windowCancellation.Token)
                .ConfigureAwait(true);
            playbackGateAcquired = true;

            var importer = await EnsureMediaImporterAsync().ConfigureAwait(true);
            if (importer is null || _isClosing)
            {
                return;
            }

            var pathCount = request.Paths?.Count ?? 0;
            _state.SetStatus($"正在探测 {pathCount} 项媒体…");
            var result = await importer.ImportAsync(
                    request,
                    _windowCancellation.Token,
                    PrepareMediaPoolCommitAsync)
                .ConfigureAwait(true);
            ApplyMediaOperation(
                result,
                result.IsSuccess
                    ? request.Operation is MediaImportOperation.Append
                        ? $"已追加 {result.ProbedCount} 项媒体"
                        : $"已导入 {result.ProbedCount} 项媒体"
                    : result.Error?.Message ?? "媒体导入失败，已保留原播放池");
        }
        catch (OperationCanceledException)
        {
            _state.SetStatus("媒体导入已取消，已保留原播放池");
        }
        catch (ConfigurationValidationException exception)
        {
            _state.SetStatus(exception.Message);
        }
        catch (IOException)
        {
            _state.SetStatus("媒体列表文件读取失败，已保留原播放池");
        }
        catch (UnauthorizedAccessException)
        {
            _state.SetStatus("没有权限读取媒体列表文件，已保留原播放池");
        }
        catch (InvalidOperationException)
        {
            _state.SetStatus("媒体导入界面当前不可用，已保留原播放池");
        }
        finally
        {
            if (playbackGateAcquired)
            {
                _playbackCommandSerial.Release();
            }

            _importBusy = false;
            SetImportButtonsEnabled(_login.CanEnterWorkbench);
        }
    }

    private void SetImportButtonsEnabled(bool enabled)
    {
        ImportButton.IsEnabled = enabled;
        TopImportButton.IsEnabled = enabled;
        TopImportPlaylistButton.IsEnabled = enabled;
        SetMediaMutationButtonsEnabled(enabled);
    }

    private void SetMediaMutationButtonsEnabled(bool enabled)
    {
        var selectedIndex = GetSelectedMediaPoolIndex();
        if (selectedIndex < 0 && _state.MediaItems.Count > 0 && MediaListBox.Items.Count == 0)
        {
            // WPF applies a changed ItemsSource on the next binding pass. During that
            // short window the pool snapshot still has a valid current item even though
            // the ListBox has not received its items yet.
            var snapshotIndex = _mediaPool.Snapshot.SourceMediaIndex;
            if (snapshotIndex < _state.MediaItems.Count
                && _state.VisibleMediaItems.Contains(_state.MediaItems[snapshotIndex]))
            {
                selectedIndex = snapshotIndex;
            }
        }

        MoveUpButton.IsEnabled = enabled && selectedIndex > 0;
        MoveDownButton.IsEnabled = enabled
            && selectedIndex >= 0
            && selectedIndex < _state.MediaItems.Count - 1;
        RemoveMediaButton.IsEnabled = enabled && selectedIndex >= 0;
        ClearMediaButton.IsEnabled = enabled && _state.HasMedia;
    }

    private int GetSelectedMediaPoolIndex() =>
        MediaListBox.SelectedItem is MediaListItemViewModel selected
            && MediaListBox.Items.Contains(selected)
            ? FindMediaPoolIndex(selected)
            : -1;

    private int FindMediaPoolIndex(MediaListItemViewModel item)
    {
        for (var index = 0; index < _state.MediaItems.Count; index++)
        {
            if (ReferenceEquals(_state.MediaItems[index], item))
            {
                return index;
            }
        }

        return -1;
    }

    private void SelectMediaPoolIndex(int index)
    {
        if (index < 0 || index >= _state.MediaItems.Count)
        {
            _pendingMediaSelection = null;
            MediaListBox.SelectedItem = null;
            return;
        }

        var item = _state.MediaItems[index];
        if (!_state.VisibleMediaItems.Contains(item))
        {
            _pendingMediaSelection = null;
            MediaListBox.SelectedItem = null;
            return;
        }

        _pendingMediaSelection = item;
        ApplyPendingMediaSelection();
        if (_pendingMediaSelection is not null)
        {
            Dispatcher.BeginInvoke(
                DispatcherPriority.Background,
                new Action(ApplyPendingMediaSelection));
        }
    }

    private void ApplyPendingMediaSelection()
    {
        var item = _pendingMediaSelection;
        if (item is null)
        {
            return;
        }

        if (_isClosing || !_state.MediaItems.Contains(item))
        {
            _pendingMediaSelection = null;
            return;
        }

        if (!MediaListBox.Items.Contains(item))
        {
            return;
        }

        _pendingMediaSelection = null;
        MediaListBox.SelectedItem = item;
    }

    private async Task<bool> PrepareMediaPoolCommitAsync(CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        if (Dispatcher.CheckAccess())
        {
            return await StopMediaForMutationAsync().ConfigureAwait(true);
        }

        var operation = Dispatcher.InvokeAsync(
            StopMediaForMutationAsync,
            DispatcherPriority.Send,
            cancellationToken);
        return await operation.Task.Unwrap().ConfigureAwait(false);
    }

    private bool ConfirmMediaMutation(string title, string message) =>
        MessageBox.Show(
            this,
            message,
            title,
            MessageBoxButton.YesNo,
            MessageBoxImage.Question,
            MessageBoxResult.No) == MessageBoxResult.Yes;

    private async Task<MediaImportCoordinator?> EnsureMediaImporterAsync()
    {
        if (_mediaImporter is not null)
        {
            return _mediaImporter;
        }

        var runtime = await EnsureVerifiedMediaRuntimeAsync().ConfigureAwait(true);
        if (runtime is null)
        {
            return null;
        }

        if (!runtime.TryCreateFfprobeProbe(
                new WindowsExternalProcessRunner(),
                out var probe,
                out var error)
            || probe is null)
        {
            _state.SetStatus(FormatRuntimeFailure(error, "媒体探测资源不可用"));
            return null;
        }

        _mediaImporter = new MediaImportCoordinator(_mediaPool, probe);
        MediaStatusText.Text = "媒体运行时：资源已校验，等待导入";
        return _mediaImporter;
    }

    private async Task<VerifiedMediaRuntime?> EnsureVerifiedMediaRuntimeAsync()
    {
        if (_verifiedMediaRuntime is not null)
        {
            return _verifiedMediaRuntime;
        }

        _runtimeVerificationTask ??= MediaRuntimeBoundary.VerifyInstalledAsync(
            cancellationToken: _windowCancellation.Token);
        RuntimeMediaVerificationResult verification;
        try
        {
            verification = await _runtimeVerificationTask.ConfigureAwait(true);
        }
        catch (OperationCanceledException)
        {
            _state.SetStatus("媒体运行资源校验已取消");
            return null;
        }

        if (!verification.IsSuccess || verification.Runtime is null)
        {
            _runtimeVerificationTask = null;
            var status = FormatRuntimeFailure(verification.Error, "媒体运行资源不可用");
            MediaStatusText.Text = $"媒体运行时：{status}";
            _state.SetStatus(status);
            return null;
        }

        _verifiedMediaRuntime = verification.Runtime;
        MediaStatusText.Text = "媒体运行时：资源已校验";
        return _verifiedMediaRuntime;
    }

    private static string FormatRuntimeFailure(RuntimeResourceError? error, string fallback) =>
        $"{error?.Message ?? fallback} 请安装或修复 C# 媒体运行包后重试。";

    private static string BuildMediaFilter()
    {
        static string Pattern(IEnumerable<string> extensions) =>
            string.Join(';', extensions
                .OrderBy(extension => extension, StringComparer.Ordinal)
                .Select(extension => $"*{extension}"));

        var video = Pattern(MediaFormatCatalog.VideoExtensions);
        var audio = Pattern(MediaFormatCatalog.AudioExtensions);
        return $"支持的媒体|{video};{audio}|视频|{video}|音频|{audio}|所有文件|*.*";
    }
}
