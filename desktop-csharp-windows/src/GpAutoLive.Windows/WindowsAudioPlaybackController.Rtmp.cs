using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>同一首 PCM 帧、活动源和稳定消费者的发布起点；资源仍属于本机声音控制器。</summary>
public sealed record WindowsRtmpAudioSource(
    FinalPcmBus Bus,
    IAudioPcmOutputSource OutputSource,
    IAudioPcmOutputSource OverlaySource,
    ulong SourcePositionMs);

public sealed partial class WindowsAudioPlaybackController
{
    private FfmpegPcmDecodePlan? _activePlan;
    private ulong _activePlanCandidateId;

    private void AnchorPromotedAudioClock(WindowsPortAudioOutputStream output,
        FinalPcmBusTrackSwitch tracks, FfmpegPcmDecodePlan plan)
    {
        lock (_gate)
        {
            var outputFrames = output.Snapshot.MediaFramesWritten;
            var candidateFrames = tracks.LocalCandidateFramesRead;
            _audibleAudioClock?.Anchor(outputFrames > candidateFrames ? outputFrames - candidateFrames : 0,
                plan.SourceStartMs, plan.PlaybackRate);
        }
    }

    public bool TryAttachRtmpSource(
        string expectedSourcePath,
        out WindowsRtmpAudioSource? source,
        out WindowsAudioPlaybackError? error)
    {
        source = null;
        error = null;
        lock (_gate)
        {
            if (_state != WindowsAudioPlaybackState.Playing
                || _activePlan is null || _finalPcmBusTrackSwitch is null
                || !string.Equals(_activePlan.SourcePath, expectedSourcePath, StringComparison.OrdinalIgnoreCase))
            {
                error = new("rtmp_source_changed", "活动声音源不可用或已变化，未启动推流。");
                return false;
            }
            if (Math.Abs(_activePlan.PlaybackRate - 1.0) > 0.001)
            {
                error = new("rtmp_speed_unsupported", "当前音画推流只支持保持源时长的声音处理。");
                return false;
            }
            if (!_finalPcmBusTrackSwitch.TryAttachRtmpFromPendingOutput(out var firstFrame, _activePlanCandidateId))
            {
                error = new("rtmp_candidate_transition", "声音候选正在切换，稍后重新捕获推流起点。", true);
                return false;
            }
            var position = (decimal)_activePlan.SourceStartMs
                + (decimal)firstFrame * 1_000 / _activePlan.SampleRateHz;
            if (position > ulong.MaxValue)
            {
                _finalPcmBusTrackSwitch.SetRtmpConsumerAttached(false);
                error = new("rtmp_position_invalid", "推流首 PCM 源位置超出有效范围。");
                return false;
            }
            source = new(
                _finalPcmBusTrackSwitch.ActiveBus,
                _finalPcmBusTrackSwitch.RtmpSource,
                _finalPcmBusTrackSwitch.RtmpOverlaySource,
                (ulong)position);
            return true;
        }
    }
}
