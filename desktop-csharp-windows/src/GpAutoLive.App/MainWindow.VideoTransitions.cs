using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.App;

public partial class MainWindow
{
    // 仅在已拥有视频时保帧；首次播放没有旧画面，错误恢复可继续使用已保留的画面。
    private async Task<bool> PreserveVideoTransitionFrameAsync()
    {
        var window = _finalEffectWindow;
        if (_isClosing || window is null || !window.IsVisible || window.HasHeldVideoFrame)
            return true;
        if (_mpvController.Snapshot.ActiveIdentity is not MediaPlaybackIdentity identity)
            return true;

        var capture = await _mpvController.CapturePresentationAsync(identity, _windowCancellation.Token)
            .ConfigureAwait(true);
        if (_isClosing || !ReferenceEquals(window, _finalEffectWindow) || !window.IsVisible)
            return false;
        if (!capture.IsSuccess || capture.PngBytes is null)
        {
            _state.SetStatus(capture.Error?.Message ?? "无法保留当前画面，未切换视频");
            return false;
        }
        if (!window.TryHoldVideoFrame(capture.PngBytes, out var error))
        {
            _state.SetStatus(error ?? "无法显示保留画面，未切换视频");
            return false;
        }
        return true;
    }

    // 池导入后的新进程没有换源事件门禁；保帧层存在时额外确认真实输出可截图。
    private async Task<bool> ConfirmRestartedVideoFrameAsync(MediaPlaybackIdentity identity)
    {
        if (_finalEffectWindow?.HasHeldVideoFrame != true) return true;
        var capture = await _mpvController.CapturePresentationAsync(identity, _windowCancellation.Token)
            .ConfigureAwait(true);
        if (capture.IsSuccess && capture.PngBytes is not null
            && _mpvController.Snapshot.ActiveIdentity == identity
            && _mediaPool.CurrentIdentity == identity)
            return true;
        _state.SetStatus(capture.Error?.Message ?? "新视频画面尚未确认，继续保留上一帧");
        return false;
    }

    private void ReleaseVideoTransitionFrame() => _finalEffectWindow?.ReleaseHeldVideoFrame();

    private void CancelVideoTransitionBeforeLoad()
    {
        var snapshot = _mpvController.Snapshot;
        if (snapshot.ActiveIdentity == _mediaPool.CurrentIdentity
            && snapshot.State is WindowsMpvPlaybackControllerState.Playing or WindowsMpvPlaybackControllerState.Paused)
        {
            ReleaseVideoTransitionFrame();
            if (snapshot.State is WindowsMpvPlaybackControllerState.Playing)
                StartVideoStateWatcher(_mediaPool.CurrentIdentity);
        }
    }
}
