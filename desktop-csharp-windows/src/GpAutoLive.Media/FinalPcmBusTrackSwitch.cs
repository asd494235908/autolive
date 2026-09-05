namespace GpAutoLive.Media;

/// <summary>
/// 在同一个本机/RTMP PCM 出口内承载一个活动最终总线和一个预载总线。
/// 四个消费者分支共享候选 ID；不创建第三个预载候选。
/// </summary>
public sealed class FinalPcmBusTrackSwitch : IDisposable
{
    private readonly object _gate = new();
    private readonly AudioPcmTrackSwitchOutputSource _outputSource;
    private readonly AudioPcmTrackSwitchOutputSource _rtmpSource;
    private readonly AudioPcmTrackSwitchOutputSource _outputOverlaySource;
    private readonly AudioPcmTrackSwitchOutputSource _rtmpOverlaySource;
    private FinalPcmBus _activeBus;
    private FinalPcmBus? _preparedBus;
    private ulong _preparedCandidateId;
    private bool _rtmpConsumerAttached;
    private bool _disposed;

    public FinalPcmBusTrackSwitch(ulong activeCandidateId, FinalPcmBus activeBus)
    {
        if (activeCandidateId == 0)
        {
            throw new ArgumentOutOfRangeException(nameof(activeCandidateId));
        }

        _activeBus = activeBus ?? throw new ArgumentNullException(nameof(activeBus));
        _outputSource = new(
            activeCandidateId,
            new AudioPcmRingBufferOutputSource(activeBus.OutputBuffer),
            activeBus.Channels);
        _rtmpSource = new(
            activeCandidateId,
            new AudioPcmRingBufferOutputSource(activeBus.RtmpBuffer),
            activeBus.Channels);
        _outputOverlaySource = new(
            activeCandidateId,
            new AudioPcmRingBufferOutputSource(activeBus.OutputOverlayBuffer),
            activeBus.Channels);
        _rtmpOverlaySource = new(
            activeCandidateId,
            new AudioPcmRingBufferOutputSource(activeBus.RtmpOverlayBuffer),
            activeBus.Channels);
    }

    public int Channels => _activeBus.Channels;

    public ulong ActiveCandidateId => _outputSource.ActiveCandidateId;

    public ulong? PreparedCandidateId
    {
        get
        {
            lock (_gate)
            {
                return _preparedBus is null ? null : _preparedCandidateId;
            }
        }
    }

    public bool HasPreparedNext => PreparedCandidateId is not null;

    public bool RtmpConsumerAttached
    {
        get
        {
            lock (_gate)
            {
                return _rtmpConsumerAttached;
            }
        }
    }

    public FinalPcmBus ActiveBus
    {
        get
        {
            lock (_gate)
            {
                return _activeBus;
            }
        }
    }

    public IAudioPcmOutputSource OutputSource => _outputSource;

    public IAudioPcmOutputSource RtmpSource => _rtmpSource;

    public IAudioPcmOutputSource OutputOverlaySource => _outputOverlaySource;

    public IAudioPcmOutputSource RtmpOverlaySource => _rtmpOverlaySource;

    /// <summary>告诉切换器 RTMP 是否有真实消费者；没有消费者的分支可安全跳过旧尾部。</summary>
    public void SetRtmpConsumerAttached(bool attached)
    {
        lock (_gate)
        {
            _rtmpConsumerAttached = attached;
            _activeBus.SetRtmpConsumerAttached(attached);
            _preparedBus?.SetRtmpConsumerAttached(attached);
        }
    }

    public bool TryPrepareNext(
        ulong candidateId,
        FinalPcmBus? nextBus,
        out AudioPcmTrackSwitchError? error)
    {
        error = null;
        if (candidateId == 0)
        {
            error = new(
                AudioPcmTrackSwitchFailureCode.InvalidCandidateId,
                "下一最终 PCM 总线候选 ID 必须大于 0。");
            return false;
        }

        if (nextBus is null)
        {
            error = new(
                AudioPcmTrackSwitchFailureCode.InvalidSource,
                "下一最终 PCM 总线不可用。");
            return false;
        }

        lock (_gate)
        {
            if (_disposed)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.InvalidSource,
                    "最终 PCM 总线切换器已关闭。");
                return false;
            }

            if (nextBus.Channels != _activeBus.Channels)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.InvalidSource,
                    "下一最终 PCM 总线声道数不一致。");
                return false;
            }

            nextBus.SetRtmpConsumerAttached(_rtmpConsumerAttached);

            if (_preparedBus is not null)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.AlreadyPrepared,
                    "已经存在一个待提交的最终 PCM 总线候选。");
                return false;
            }

            if (candidateId == _outputSource.ActiveCandidateId)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.SameAsActive,
                    "下一最终 PCM 总线候选不能与当前候选相同。");
                return false;
            }

            if (!_outputSource.TryPrepareNext(
                    candidateId,
                    new AudioPcmRingBufferOutputSource(nextBus.OutputBuffer),
                    out error))
            {
                return false;
            }

            if (!_rtmpSource.TryPrepareNext(
                    candidateId,
                    new AudioPcmRingBufferOutputSource(nextBus.RtmpBuffer),
                    out error))
            {
                _outputSource.TryCancelPrepared(candidateId, out _);
                return false;
            }

            if (!_outputOverlaySource.TryPrepareNext(
                    candidateId,
                    new AudioPcmRingBufferOutputSource(nextBus.OutputOverlayBuffer),
                    out error))
            {
                _outputSource.TryCancelPrepared(candidateId, out _);
                _rtmpSource.TryCancelPrepared(candidateId, out _);
                return false;
            }

            if (!_rtmpOverlaySource.TryPrepareNext(
                    candidateId,
                    new AudioPcmRingBufferOutputSource(nextBus.RtmpOverlayBuffer),
                    out error))
            {
                _outputSource.TryCancelPrepared(candidateId, out _);
                _rtmpSource.TryCancelPrepared(candidateId, out _);
                _outputOverlaySource.TryCancelPrepared(candidateId, out _);
                return false;
            }

            _preparedBus = nextBus;
            _preparedCandidateId = candidateId;
            return true;
        }
    }

    public bool TryCommitNext(
        ulong candidateId,
        out AudioPcmTrackSwitchError? error)
    {
        error = null;
        lock (_gate)
        {
            if (_preparedBus is null || _preparedCandidateId != candidateId)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CandidateNotPrepared,
                    "指定的最终 PCM 总线候选尚未完成预载。");
                return false;
            }

            if (!_outputSource.TryCommitPrepared(candidateId, out error)
                || !_rtmpSource.TryCommitPrepared(candidateId, out error)
                || !_outputOverlaySource.TryCommitPrepared(candidateId, out error)
                || !_rtmpOverlaySource.TryCommitPrepared(candidateId, out error))
            {
                return false;
            }

            return true;
        }
    }

    /// <summary>
    /// 请求在当前输出分支各自再消费指定帧数后切换；本机和 RTMP 分支仍共享一个候选 ID。
    /// </summary>
    public bool TryCommitNextAtFrames(
        ulong candidateId,
        ulong framesFromNow,
        out AudioPcmTrackSwitchError? error)
    {
        error = null;
        lock (_gate)
        {
            if (_preparedBus is null || _preparedCandidateId != candidateId)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CandidateNotPrepared,
                    "指定的最终 PCM 总线候选尚未完成预载。");
                return false;
            }

            if (!_outputSource.TryCommitPreparedAtFrames(candidateId, framesFromNow, out error))
            {
                return false;
            }

            var commitRtmp = _rtmpConsumerAttached
                ? _rtmpSource.TryCommitPreparedAtFrames(candidateId, framesFromNow, out error)
                : _rtmpSource.TryCommitPrepared(candidateId, out error);
            if (!commitRtmp)
            {
                return false;
            }

            if (!_outputOverlaySource.TryCommitPrepared(candidateId, out error))
            {
                return false;
            }

            return _rtmpOverlaySource.TryCommitPrepared(candidateId, out error);
        }
    }

    public bool TryCancelNext(
        ulong candidateId,
        out AudioPcmTrackSwitchError? error)
    {
        error = null;
        lock (_gate)
        {
            if (_preparedBus is null || _preparedCandidateId != candidateId)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CandidateNotPrepared,
                    "指定的最终 PCM 总线候选尚未完成预载。");
                return false;
            }

            if (!_outputSource.TryCancelPrepared(candidateId, out error)
                || !_rtmpSource.TryCancelPrepared(candidateId, out error)
                || !_outputOverlaySource.TryCancelPrepared(candidateId, out error)
                || !_rtmpOverlaySource.TryCancelPrepared(candidateId, out error))
            {
                return false;
            }

            _preparedBus.Close();
            _preparedBus = null;
            _preparedCandidateId = 0;
            return true;
        }
    }

    /// <summary>
    /// 在本机输出已经切换后确认总线身份；无 RTMP 消费者时同步推进 RTMP 分支。
    /// </summary>
    public bool TryAcknowledgePromotion(
        ulong candidateId,
        out FinalPcmBus? retiredBus,
        out AudioPcmTrackSwitchError? error)
    {
        retiredBus = null;
        error = null;
        lock (_gate)
        {
            if (_preparedBus is null || _preparedCandidateId != candidateId)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CandidateNotPrepared,
                    "指定的最终 PCM 总线候选尚未完成预载。");
                return false;
            }

            if (_outputSource.ActiveCandidateId != candidateId)
            {
                return false;
            }

            if (!_rtmpConsumerAttached)
            {
                _ = _rtmpSource.TryForcePromoteCommittedWithoutRead(candidateId, out _);
                _ = _rtmpOverlaySource.TryForcePromoteCommittedWithoutRead(candidateId, out _);
            }

            // 插话分支没有独立的候选消费者；其边界跟随主轨提交，避免未启用插话时
            // 因为没有回调读取 overlay source 而永远阻塞候选回收。
            _ = _outputOverlaySource.TryForcePromoteCommittedWithoutRead(candidateId, out _);

            if (_rtmpSource.ActiveCandidateId != candidateId
                || _rtmpOverlaySource.ActiveCandidateId != candidateId
                || _outputOverlaySource.ActiveCandidateId != candidateId)
            {
                return false;
            }

            retiredBus = _activeBus;
            _activeBus = _preparedBus;
            _preparedBus = null;
            _preparedCandidateId = 0;
            return true;
        }
    }

    public void Dispose()
    {
        FinalPcmBus? prepared;
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            prepared = _preparedBus;
            _preparedBus = null;
            _preparedCandidateId = 0;
            _activeBus.Close();
        }

        prepared?.Close();
    }
}
