namespace GpAutoLive.Windows;

/// <summary>基于 PortAudio 已送出帧数和 DAC 延迟计算的可听音频时钟快照。</summary>
public sealed record WindowsAudibleAudioClockSnapshot(
    ulong OutputFramesWritten,
    ulong AudibleFrames,
    int SampleRateHz,
    ulong OutputLatencyMicroseconds,
    bool HasTimeInfo,
    ulong? PlaybackTimeMs);

/// <summary>
/// 单一音频会话的可听位置计算器。它不拥有设备、不读文件，只在源切换边界重设输出帧锚点。
/// 缺少 PortAudio 时间信息时保留帧计数，但不发布可听媒体位置。
/// </summary>
public sealed class WindowsAudibleAudioClock
{
    private ulong _outputAnchorFrames;
    private ulong _sourceAnchorMs;
    private double _playbackRate = 1.0;

    internal void Anchor(
        ulong outputFramesWritten,
        ulong sourcePositionMs,
        double playbackRate = 1.0)
    {
        if (!double.IsFinite(playbackRate) || playbackRate is < 0.5 or > 2.0)
        {
            throw new ArgumentOutOfRangeException(nameof(playbackRate));
        }

        _outputAnchorFrames = outputFramesWritten;
        _sourceAnchorMs = sourcePositionMs;
        _playbackRate = playbackRate;
    }

    internal WindowsAudibleAudioClockSnapshot? Project(WindowsPortAudioOutputSnapshot? output)
    {
        if (output is null || output.SampleRate <= 0 || output.SampleRate > int.MaxValue)
        {
            return null;
        }

        var sampleRate = (int)Math.Round(output.SampleRate, MidpointRounding.AwayFromZero);
        if (sampleRate <= 0)
        {
            return null;
        }

        var latencyFrames = FramesFromMicroseconds(output.OutputLatencyMicroseconds, sampleRate);
        var audibleFrames = output.OutputFramesWritten > latencyFrames
            ? output.OutputFramesWritten - latencyFrames
            : 0;
        var playbackTimeMs = output.HasTimeInfo
            ? ToMilliseconds(
                audibleFrames > _outputAnchorFrames
                    ? audibleFrames - _outputAnchorFrames
                    : 0,
                sampleRate,
                _playbackRate,
                _sourceAnchorMs)
            : null;

        return new(
            output.OutputFramesWritten,
            audibleFrames,
            sampleRate,
            output.OutputLatencyMicroseconds,
            output.HasTimeInfo,
            playbackTimeMs);
    }

    private static ulong FramesFromMicroseconds(ulong microseconds, int sampleRate)
    {
        if (microseconds == 0)
        {
            return 0;
        }

        var product = (decimal)microseconds * sampleRate + 999_999;
        return product >= ulong.MaxValue * 1_000_000m
            ? ulong.MaxValue
            : (ulong)(product / 1_000_000m);
    }

    private static ulong? ToMilliseconds(
        ulong frames,
        int sampleRate,
        double playbackRate,
        ulong sourceAnchorMs)
    {
        var value = frames * 1_000d * playbackRate / sampleRate;
        if (!double.IsFinite(value) || value < 0 || value >= ulong.MaxValue)
        {
            return null;
        }

        var elapsedMs = (ulong)Math.Round(value, MidpointRounding.AwayFromZero);
        return ulong.MaxValue - sourceAnchorMs < elapsedMs
            ? null
            : sourceAnchorMs + elapsedMs;
    }
}
