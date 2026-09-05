using GpAutoLive.App.Features.Media;

namespace GpAutoLive.App;

public partial class MainWindow
{
    private void StartMediaThumbnailLoad() =>
        _mediaThumbnailCache.Start(
            _state.MediaItems,
            _verifiedMediaRuntime,
            Dispatcher,
            _windowCancellation.Token,
            UpdateMediaProjection);
}
