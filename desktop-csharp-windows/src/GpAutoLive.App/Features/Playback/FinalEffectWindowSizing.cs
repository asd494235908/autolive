namespace GpAutoLive.App.Features.Playback;

internal readonly record struct FinalEffectClientSize(int Width, int Height);

internal readonly record struct FinalEffectWindowSize(
    int Width,
    int Height,
    int ClientWidth,
    int ClientHeight);

/// <summary>最终效果窗口的等比尺寸计算；只处理尺寸，不拥有窗口或媒体状态。</summary>
internal static class FinalEffectWindowSizing
{
    internal const int MinimumClientWidth = 320;
    internal const int MinimumClientHeight = 180;
    internal const double DefaultAspectRatio = 16d / 9d;

    internal static double ResolveAspectRatio(uint? videoWidth, uint? videoHeight) =>
        videoWidth is uint width and > 0
        && videoHeight is uint height and > 0
            ? width / (double)height
            : DefaultAspectRatio;

    internal static FinalEffectWindowSize CalculateInitialWindowSize(
        uint videoWidth,
        uint videoHeight,
        double workAreaWidth,
        double workAreaHeight,
        double frameWidth,
        double frameHeight)
    {
        if (videoWidth == 0 || videoHeight == 0)
        {
            throw new ArgumentOutOfRangeException(nameof(videoWidth), "视频宽高必须为正数。");
        }

        if (!double.IsFinite(workAreaWidth)
            || !double.IsFinite(workAreaHeight)
            || !double.IsFinite(frameWidth)
            || !double.IsFinite(frameHeight)
            || workAreaWidth <= 0
            || workAreaHeight <= 0
            || frameWidth < 0
            || frameHeight < 0)
        {
            throw new ArgumentOutOfRangeException(nameof(workAreaWidth), "窗口工作区尺寸无效。");
        }

        var availableClientWidth = Math.Max(1d, workAreaWidth - frameWidth);
        var availableClientHeight = Math.Max(1d, workAreaHeight - frameHeight);
        var videoWidthAsDouble = (double)videoWidth;
        var videoHeightAsDouble = (double)videoHeight;
        var maxFitScale = Math.Min(
            availableClientWidth / videoWidthAsDouble,
            availableClientHeight / videoHeightAsDouble);
        var scale = Math.Min(1d, maxFitScale);
        var minimumScale = Math.Max(
            MinimumClientWidth / videoWidthAsDouble,
            MinimumClientHeight / videoHeightAsDouble);
        if (minimumScale <= maxFitScale)
        {
            scale = Math.Max(scale, minimumScale);
        }

        var clientWidth = RoundDimension(videoWidthAsDouble * scale);
        var clientHeight = RoundDimension(videoHeightAsDouble * scale);
        return new FinalEffectWindowSize(
            RoundDimension(clientWidth + frameWidth),
            RoundDimension(clientHeight + frameHeight),
            clientWidth,
            clientHeight);
    }

    internal static FinalEffectClientSize CalculateClientSizeFromWidth(
        int requestedWidth,
        double aspectRatio,
        int minimumWidth,
        int minimumHeight)
    {
        ValidateAspectRatio(aspectRatio);
        var width = Math.Max(requestedWidth, Math.Max(1, minimumWidth));
        width = Math.Max(width, RoundDimension(minimumHeight * aspectRatio));
        var height = RoundDimension(width / aspectRatio);
        if (height < minimumHeight)
        {
            height = minimumHeight;
            width = RoundDimension(height * aspectRatio);
        }

        return new FinalEffectClientSize(width, height);
    }

    internal static FinalEffectClientSize CalculateClientSizeFromHeight(
        int requestedHeight,
        double aspectRatio,
        int minimumWidth,
        int minimumHeight)
    {
        ValidateAspectRatio(aspectRatio);
        var height = Math.Max(requestedHeight, Math.Max(1, minimumHeight));
        height = Math.Max(height, RoundDimension(minimumWidth / aspectRatio));
        var width = RoundDimension(height * aspectRatio);
        if (width < minimumWidth)
        {
            width = minimumWidth;
            height = RoundDimension(width / aspectRatio);
        }

        return new FinalEffectClientSize(width, height);
    }

    private static void ValidateAspectRatio(double aspectRatio)
    {
        if (!double.IsFinite(aspectRatio) || aspectRatio <= 0)
        {
            throw new ArgumentOutOfRangeException(nameof(aspectRatio), "视频画幅比无效。");
        }
    }

    private static int RoundDimension(double value) =>
        checked((int)Math.Clamp(
            Math.Round(value, MidpointRounding.AwayFromZero),
            1,
            int.MaxValue));
}
