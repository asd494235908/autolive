using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>
/// 将麦克风输入环缓中的有限帧送入活动最终 PCM overlay 总线。
/// 该桥只负责有界的声道映射和发布，不宣称完成 AEC、降噪或 AGC。
/// </summary>
internal sealed class WindowsMicrophonePcmBridge
{
    private const int MaxDrainFrames = 4_096;
    private readonly int _inputChannels;
    private readonly float[] _inputScratch;
    private readonly float[] _monoScratch = new float[MaxDrainFrames];
    private readonly float[] _stereoScratch = new float[MaxDrainFrames * 2];

    public WindowsMicrophonePcmBridge(int inputChannels)
    {
        if (inputChannels is < 1 or > 2)
        {
            throw new ArgumentOutOfRangeException(nameof(inputChannels), "麦克风输入声道数必须在 1 到 2 之间。");
        }

        _inputChannels = inputChannels;
        _inputScratch = new float[checked(MaxDrainFrames * inputChannels)];
    }

    /// <summary>
    /// 只在 Speaking/Hangover 时发布输入帧；静音期间丢弃积压，避免旧麦克风帧泄漏。
    /// </summary>
    public bool TryDrain(
        AudioPcmRingBuffer input,
        FinalPcmBus? finalPcmBus,
        bool speaking,
        out WindowsPortAudioInputError? error)
    {
        ArgumentNullException.ThrowIfNull(input);
        error = null;

        if (finalPcmBus is null || finalPcmBus.Snapshot.IsClosed)
        {
            error = new(
                WindowsPortAudioInputFailureCode.OutputBusUnavailable,
                "麦克风最终 PCM 输出总线不可用。",
                Retryable: true);
            return false;
        }

        if (finalPcmBus.Channels is not (1 or 2))
        {
            error = new(
                WindowsPortAudioInputFailureCode.OutputBusUnavailable,
                "麦克风只支持 1 或 2 声道最终 PCM 输出。",
                Retryable: false);
            return false;
        }

        if (!speaking)
        {
            input.DiscardPending();
            return true;
        }

        if (!input.TryRead(_inputScratch, out var framesRead, out var readError))
        {
            error = new(
                WindowsPortAudioInputFailureCode.OutputBusUnavailable,
                readError?.Message ?? "麦克风输入帧读取失败。",
                Retryable: true);
            return false;
        }

        if (framesRead == 0)
        {
            return true;
        }

        var mono = _monoScratch.AsSpan(0, framesRead);
        for (var frame = 0; frame < framesRead; frame++)
        {
            var sampleOffset = frame * _inputChannels;
            var sum = 0F;
            for (var channel = 0; channel < _inputChannels; channel++)
            {
                var sample = _inputScratch[sampleOffset + channel];
                sum += float.IsFinite(sample) ? Math.Clamp(sample, -1F, 1F) : 0F;
            }

            mono[frame] = sum / _inputChannels;
        }

        ReadOnlySpan<float> output = mono;
        if (finalPcmBus.Channels == 2)
        {
            var stereo = _stereoScratch.AsSpan(0, checked(framesRead * 2));
            for (var frame = 0; frame < framesRead; frame++)
            {
                var sample = mono[frame];
                stereo[frame * 2] = sample;
                stereo[(frame * 2) + 1] = sample;
            }

            output = stereo;
        }

        if (finalPcmBus.TryPublishOverlay(output, out _, out var publishError))
        {
            return true;
        }

        error = new(
            WindowsPortAudioInputFailureCode.OutputBusUnavailable,
            publishError?.Message ?? "麦克风最终 PCM overlay 发布失败。",
            Retryable: true);
        return false;
    }
}
