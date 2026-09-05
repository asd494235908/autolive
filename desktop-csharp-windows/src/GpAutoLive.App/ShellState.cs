using System.ComponentModel;
using System.Runtime.CompilerServices;
using GpAutoLive.App.Features.Effects;
using GpAutoLive.App.Features.Media;
using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.App;

public sealed class ShellState : INotifyPropertyChanged
{
    private static readonly IReadOnlyList<MediaListItemViewModel> EmptyMediaItems = Array.Empty<MediaListItemViewModel>();
    private PlaybackState _playbackState = PlaybackState.Stopped;
    private bool _hasMedia;
    private IReadOnlyList<MediaListItemViewModel> _mediaItems = EmptyMediaItems;
    private IReadOnlyList<MediaListItemViewModel> _visibleMediaItems = EmptyMediaItems;
    private string _mediaSearchText = string.Empty;
    private bool _videoProcessing = true;
    private bool _audioProcessing = true;
    private long _audioProcessingRevision;
    private double _playbackProgress;
    private string _statusMessage = "就绪 · C# Windows 壳";
    private GeneratedVideoEffectSnapshot _videoParameterSnapshot = GeneratedVideoEffectSnapshot.Create();
    private GeneratedAudioEffectSnapshot _audioParameterSnapshot = GeneratedAudioEffectSnapshot.Create();

    public VideoEffectEditorState VideoEffects { get; } = new();

    /// <summary>
    /// 当前周期由系统自动生成的只读视频/声音结果；用户不能通过界面写入这些值。
    /// </summary>
    public GeneratedVideoEffectSnapshot VideoParameterSnapshot
    {
        get => _videoParameterSnapshot;
        private set
        {
            _videoParameterSnapshot = value;
            OnPropertyChanged();
        }
    }

    public GeneratedAudioEffectSnapshot AudioParameterSnapshot
    {
        get => _audioParameterSnapshot;
        private set
        {
            _audioParameterSnapshot = value;
            OnPropertyChanged();
        }
    }

    /// <summary>由系统重新生成下一轮只读结果，不接受调用方传入任意效果值。</summary>
    public void RegenerateParameterSnapshots()
    {
        RegenerateVideoParameterSnapshot();
        AudioParameterSnapshot = GeneratedAudioEffectSnapshot.Create(
            generation: AudioParameterSnapshot.Generation + 1);
        StatusMessage = "已生成新的本地只读参数快照；尚未提交给播放运行时";
    }

    /// <summary>仅生成视频周期快照；声音保持现有连续 PCM 处理链。</summary>
    public void RegenerateVideoParameterSnapshot() =>
        VideoParameterSnapshot = GeneratedVideoEffectSnapshot.Create(
            generation: VideoParameterSnapshot.Generation + 1);

    /// <summary>仅生成声音周期快照；视频保持现有 mpv 参数。</summary>
    public void RegenerateAudioParameterSnapshot() =>
        AudioParameterSnapshot = GeneratedAudioEffectSnapshot.Create(
            generation: AudioParameterSnapshot.Generation + 1);

    /// <summary>当前壳是否已有可播放媒体；媒体导入接入后由媒体所有者更新。</summary>
    public bool HasMedia
    {
        get => _hasMedia;
        private set
        {
            if (_hasMedia == value)
            {
                return;
            }

            _hasMedia = value;
            OnPropertyChanged();
        }
    }

    public IReadOnlyList<MediaListItemViewModel> MediaItems
    {
        get => _mediaItems;
        private set
        {
            if (ReferenceEquals(_mediaItems, value))
            {
                return;
            }

            _mediaItems = value;
            OnPropertyChanged();
            OnPropertyChanged(nameof(MediaCountLabel));
            RefreshVisibleMediaItems();
        }
    }

    /// <summary>媒体池搜索文本；只影响界面投影，不改变媒体所有者的播放池。</summary>
    public string MediaSearchText
    {
        get => _mediaSearchText;
        set
        {
            var next = value ?? string.Empty;
            if (string.Equals(_mediaSearchText, next, StringComparison.Ordinal))
            {
                return;
            }

            _mediaSearchText = next;
            OnPropertyChanged();
            RefreshVisibleMediaItems();
        }
    }

    public IReadOnlyList<MediaListItemViewModel> VisibleMediaItems => _visibleMediaItems;

    public string MediaCountLabel => $"{MediaItems.Count} / {MediaPoolRules.MaxItems}";

    public event PropertyChangedEventHandler? PropertyChanged;

    public PlaybackState PlaybackState
    {
        get => _playbackState;
        private set
        {
            if (_playbackState == value)
            {
                return;
            }

            _playbackState = value;
            OnPropertyChanged();
            OnPropertyChanged(nameof(IsPlaying));
            OnPropertyChanged(nameof(PlaybackGlyph));
            OnPropertyChanged(nameof(PlaybackLabel));
        }
    }

    public bool IsPlaying => PlaybackState is PlaybackState.Playing;

    public bool VideoProcessing
    {
        get => _videoProcessing;
        set
        {
            if (_videoProcessing == value)
            {
                return;
            }

            _videoProcessing = value;
            OnPropertyChanged();
            OnPropertyChanged(nameof(VideoProcessingLabel));
            StatusMessage = value ? "视频处理已开启（壳状态）" : "视频处理已关闭（壳状态）";
        }
    }

    public bool AudioProcessing
    {
        get => _audioProcessing;
        set
        {
            if (_audioProcessing == value)
            {
                return;
            }

            _audioProcessing = value;
            _audioProcessingRevision++;
            OnPropertyChanged();
            OnPropertyChanged(nameof(AudioProcessingLabel));
            StatusMessage = value ? "声音处理已开启（壳状态）" : "声音处理已关闭（壳状态）";
        }
    }

    /// <summary>声音处理开关每次实际变化的单调版本，供异步重配置拒绝过期请求。</summary>
    public long AudioProcessingRevision => _audioProcessingRevision;

    public double PlaybackProgress
    {
        get => _playbackProgress;
        set
        {
            if (Math.Abs(_playbackProgress - value) < 0.001)
            {
                return;
            }

            _playbackProgress = value;
            OnPropertyChanged();
        }
    }

    public string PlaybackGlyph => IsPlaying ? "Ⅱ" : "▶";

    public string VideoProcessingLabel => VideoProcessing ? "已启用" : "已关闭";

    public string AudioProcessingLabel => AudioProcessing ? "已启用" : "已关闭";

    public string PlaybackLabel => PlaybackState switch
    {
        PlaybackState.Playing => "播放中",
        PlaybackState.Paused => "已暂停",
        PlaybackState.Ready => "待播放",
        _ => "已停止",
    };

    public string StatusMessage
    {
        get => _statusMessage;
        private set
        {
            if (_statusMessage == value)
            {
                return;
            }

            _statusMessage = value;
            OnPropertyChanged();
        }
    }

    public void TogglePlayback()
    {
        if (!HasMedia)
        {
            StatusMessage = "播放入口已触发；媒体池为空";
            return;
        }

        PlaybackState = IsPlaying ? PlaybackState.Paused : PlaybackState.Playing;
        StatusMessage = IsPlaying ? "播放入口已触发（壳状态）" : "播放已暂停（壳状态）";
    }

    public void StopPlayback()
    {
        PlaybackState = PlaybackState.Stopped;
        PlaybackProgress = 0;
        StatusMessage = "播放已停止（壳状态）";
    }

    public void SetStatus(string message) => StatusMessage = message;

    /// <summary>
    /// 将媒体所有者的不可变快照投影到壳层；壳层不自行推算媒体状态。
    /// </summary>
    public void ApplyMediaSnapshot(AppState snapshot)
    {
        ArgumentNullException.ThrowIfNull(snapshot);

        MediaItems = snapshot.SourceMediaPool
            .Select(static source => new MediaListItemViewModel(source))
            .ToArray();
        HasMedia = MediaItems.Count > 0;
        PlaybackState = snapshot.PlaybackState;
        if (!HasMedia || snapshot.PlaybackState is PlaybackState.Stopped)
        {
            PlaybackProgress = 0;
        }
    }

    private void OnPropertyChanged([CallerMemberName] string? propertyName = null) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));

    private void RefreshVisibleMediaItems()
    {
        var query = MediaSearchText.Trim();
        var visible = string.IsNullOrEmpty(query)
            ? MediaItems
            : MediaItems
                .Where(item => item.FileName.Contains(query, StringComparison.OrdinalIgnoreCase)
                    || item.MediaKindLabel.Contains(query, StringComparison.OrdinalIgnoreCase))
                .ToArray();
        if (ReferenceEquals(_visibleMediaItems, visible))
        {
            return;
        }

        _visibleMediaItems = visible;
        OnPropertyChanged(nameof(VisibleMediaItems));
    }
}
