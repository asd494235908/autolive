using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Windows.Media;
using GpAutoLive.Contracts;

namespace GpAutoLive.App.Features.Media;

/// <summary>媒体池条目的 UI 投影；不改变 Core 的不可变媒体快照。</summary>
public sealed class MediaListItemViewModel : INotifyPropertyChanged
{
    private ImageSource? _thumbnail;

    public MediaListItemViewModel(SourceMediaDto source)
    {
        Source = source ?? throw new ArgumentNullException(nameof(source));
    }

    public SourceMediaDto Source { get; }

    public string FileName => Source.FileName;

    public MediaKind MediaKind => Source.MediaKind;

    public string MediaKindLabel => Source.MediaKind is MediaKind.Video ? "视频" : "音频";

    public ulong FileSizeBytes => Source.FileSizeBytes;

    public ulong? DurationMs => Source.DurationMs;

    public string DurationLabel => Source.DurationMs is ulong durationMs
        ? TimeSpan.FromMilliseconds(durationMs).ToString(
            durationMs >= TimeSpan.FromDays(1).TotalMilliseconds
                ? @"d\.hh\:mm\:ss"
                : @"hh\:mm\:ss")
        : "--:--:--";

    public uint? Width => Source.Width;

    public uint? Height => Source.Height;

    public double? FrameRateFps => Source.FrameRateFps;

    public string VideoFormatLabel => Source.Width is uint width and > 0
        && Source.Height is uint height and > 0
        ? $"{width}×{height} {FormatAspectRatio(width, height)}"
        : "—";

    public uint? AudioSampleRateHz => Source.AudioSampleRateHz;

    public ushort? AudioChannelCount => Source.AudioChannelCount;

    public string AudioFormatLabel => Source.AudioSampleRateHz is uint sampleRate
        && Source.AudioChannelCount is ushort channelCount
        && sampleRate > 0
        && channelCount > 0
        ? $"{sampleRate / 1000d:0.#}kHz {FormatChannelCount(channelCount)}"
        : "—";

    public ImageSource? Thumbnail
    {
        get => _thumbnail;
        private set
        {
            if (ReferenceEquals(_thumbnail, value))
            {
                return;
            }

            _thumbnail = value;
            OnPropertyChanged();
        }
    }

    public void SetThumbnail(ImageSource? thumbnail) => Thumbnail = thumbnail;

    public event PropertyChangedEventHandler? PropertyChanged;

    private static string FormatAspectRatio(uint width, uint height)
    {
        var divisor = GreatestCommonDivisor(width, height);
        return $"{width / divisor}:{height / divisor}";
    }

    private static string FormatChannelCount(ushort channelCount) => channelCount switch
    {
        1 => "单声道",
        2 => "立体声",
        _ => $"{channelCount}声道",
    };

    private static uint GreatestCommonDivisor(uint left, uint right)
    {
        while (right != 0)
        {
            (left, right) = (right, left % right);
        }

        return left;
    }

    private void OnPropertyChanged([CallerMemberName] string? propertyName = null) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
}
