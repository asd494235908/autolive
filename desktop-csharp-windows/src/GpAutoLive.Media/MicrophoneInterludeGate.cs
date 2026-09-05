namespace GpAutoLive.Media;

/// <summary>麦克风本地能量门控状态。</summary>
public enum MicrophoneInterludeGateState
{
    Disabled,
    Armed,
    Speaking,
    Hangover,
}

/// <summary>不包含音频正文的门控快照。</summary>
public sealed record MicrophoneInterludeGateSnapshot(
    MicrophoneInterludeGateState State,
    float LevelDb,
    bool IsSpeaking,
    long LastSpeechTimestampMs,
    ulong Generation);

/// <summary>
/// 固定容量本地能量 VAD 门控。它只计算 RMS 电平和迟滞，不做 AEC、降噪、AGC、ASR 或变声；
/// 输入由调用方限制为有限交错 float PCM，时间戳必须使用单调毫秒。
/// </summary>
public sealed class MicrophoneInterludeGate
{
    private const int MaxFramesPerObservation = 4_096;
    private readonly object _gate = new();
    private readonly int _channels;
    private readonly float _startThresholdDb;
    private readonly float _stopThresholdDb;
    private readonly long _hangoverMs;
    private MicrophoneInterludeGateState _state = MicrophoneInterludeGateState.Disabled;
    private float _levelDb = -96;
    private long _lastSpeechTimestampMs;
    private ulong _generation;

    public MicrophoneInterludeGate(
        int channels = 1,
        float startThresholdDb = -42,
        float stopThresholdDb = -48,
        long hangoverMs = 250)
    {
        if (channels is < 1 or > 2)
        {
            throw new ArgumentOutOfRangeException(nameof(channels), "麦克风门控声道数必须在 1 到 2 之间。");
        }

        if (!float.IsFinite(startThresholdDb)
            || !float.IsFinite(stopThresholdDb)
            || startThresholdDb is < -96 or > 0
            || stopThresholdDb is < -96 or > 0
            || stopThresholdDb > startThresholdDb)
        {
            throw new ArgumentOutOfRangeException(nameof(startThresholdDb), "麦克风门控阈值必须在 -96 到 0 dB 且停止阈值不高于启动阈值。");
        }

        if (hangoverMs is < 0 or > 2_000)
        {
            throw new ArgumentOutOfRangeException(nameof(hangoverMs), "麦克风门控挂起必须在 0 到 2000 毫秒内。");
        }

        _channels = channels;
        _startThresholdDb = startThresholdDb;
        _stopThresholdDb = stopThresholdDb;
        _hangoverMs = hangoverMs;
    }

    public MicrophoneInterludeGateSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshot();
            }
        }
    }

    public void Arm()
    {
        lock (_gate)
        {
            if (_state is MicrophoneInterludeGateState.Disabled)
            {
                _state = MicrophoneInterludeGateState.Armed;
                IncrementGeneration();
            }
        }
    }

    public void Disable()
    {
        lock (_gate)
        {
            if (_state is not MicrophoneInterludeGateState.Disabled)
            {
                _state = MicrophoneInterludeGateState.Disabled;
                IncrementGeneration();
            }
        }
    }

    /// <summary>处理一段有限 PCM；返回更新后的脱敏门控状态。</summary>
    public MicrophoneInterludeGateSnapshot Process(
        ReadOnlySpan<float> interleavedPcm,
        long timestampMs)
    {
        if (interleavedPcm.Length % _channels != 0)
        {
            throw new ArgumentException("麦克风 PCM 样本数必须完整对齐到交错声道帧。", nameof(interleavedPcm));
        }

        var frameCount = interleavedPcm.Length / _channels;
        if (frameCount > MaxFramesPerObservation)
        {
            throw new ArgumentOutOfRangeException(nameof(interleavedPcm), "麦克风门控观测分片超过有界容量。");
        }

        var sumSquares = 0d;
        foreach (var sample in interleavedPcm)
        {
            var finiteSample = float.IsFinite(sample) ? Math.Clamp(sample, -1, 1) : 0;
            sumSquares += finiteSample * finiteSample;
        }

        var levelDb = frameCount == 0
            ? -96
            : Math.Clamp((float)(20 * Math.Log10(Math.Sqrt(sumSquares / interleavedPcm.Length))), -96, 0);
        lock (_gate)
        {
            var nowMs = Math.Max(timestampMs, _lastSpeechTimestampMs);
            _levelDb = levelDb;
            if (_state is MicrophoneInterludeGateState.Disabled)
            {
                return CreateSnapshot();
            }

            if (levelDb >= _startThresholdDb)
            {
                if (_state is not MicrophoneInterludeGateState.Speaking)
                {
                    _state = MicrophoneInterludeGateState.Speaking;
                    IncrementGeneration();
                }

                _lastSpeechTimestampMs = nowMs;
            }
            else if (_state is MicrophoneInterludeGateState.Speaking)
            {
                _state = MicrophoneInterludeGateState.Hangover;
                IncrementGeneration();
            }
            else if (_state is MicrophoneInterludeGateState.Hangover
                && nowMs - _lastSpeechTimestampMs >= _hangoverMs)
            {
                _state = MicrophoneInterludeGateState.Armed;
                IncrementGeneration();
            }

            return CreateSnapshot();
        }
    }

    private MicrophoneInterludeGateSnapshot CreateSnapshot() =>
        new(_state, _levelDb, _state is MicrophoneInterludeGateState.Speaking, _lastSpeechTimestampMs, _generation);

    private void IncrementGeneration()
    {
        if (_generation < ulong.MaxValue)
        {
            _generation++;
        }
    }
}
