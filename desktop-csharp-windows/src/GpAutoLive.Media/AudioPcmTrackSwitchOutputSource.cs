namespace GpAutoLive.Media;

/// <summary>PCM N/N+1 输出源切换的稳定失败分类。</summary>
public enum AudioPcmTrackSwitchFailureCode
{
    InvalidCandidateId,
    InvalidSource,
    SameAsActive,
    AlreadyPrepared,
    CandidateNotPrepared,
    CommitAlreadyRequested,
}

/// <summary>不包含音频正文的 PCM 音轨切换错误。</summary>
public sealed record AudioPcmTrackSwitchError(
    AudioPcmTrackSwitchFailureCode Code,
    string Message);

/// <summary>
/// 在一个输出流内承载 N/N+1 两个有限 PCM 源。
/// N+1 只能被显式提交，并且只会在 N 已关闭且排空到帧边界后切换；不创建第三个候选或输出流。
/// </summary>
public sealed class AudioPcmTrackSwitchOutputSource : IAudioPcmOutputSource
{
    private readonly object _gate = new();
    private readonly int _channels;
    private IAudioPcmOutputSource _activeSource;
    private ulong _activeCandidateId;
    private IAudioPcmOutputSource? _preparedSource;
    private ulong _preparedCandidateId;
    private bool _commitRequested;
    private ulong _framesRead;
    private ulong _totalFramesRead;
    private ulong _candidateStartTotalFrames;
    private AudioPcmTrackSwitchOutputSource? _readLeader;
    private AudioPcmTrackSwitchOutputSource? _candidateLeader;
    private ulong? _commitAtFramesRead;

    public AudioPcmTrackSwitchOutputSource(
        ulong activeCandidateId,
        IAudioPcmOutputSource activeSource,
        int channels)
    {
        if (activeCandidateId == 0)
        {
            throw new ArgumentOutOfRangeException(
                nameof(activeCandidateId),
                "当前音轨候选 ID 必须大于 0。");
        }

        if (channels is < 1 or > 8)
        {
            throw new ArgumentOutOfRangeException(
                nameof(channels),
                "音轨切换声道数必须在 1 到 8 之间。");
        }

        _activeSource = activeSource ?? throw new ArgumentNullException(nameof(activeSource));
        _channels = channels;
        _activeCandidateId = activeCandidateId;
    }

    /// <summary>当前实际提供 PCM 的候选 ID。</summary>
    public ulong ActiveCandidateId
    {
        get
        {
            lock (_gate)
            {
                return _activeCandidateId;
            }
        }
    }

    /// <inheritdoc />
    public int Channels => _channels;

    internal bool HasPendingCommit { get { lock (_gate) { return _commitRequested; } } }

    internal ulong CandidateFramesRead { get { lock (_gate) { return _framesRead; } } }

    internal void FollowReadPosition(AudioPcmTrackSwitchOutputSource leader) => _readLeader = leader;

    internal void FollowCandidate(AudioPcmTrackSwitchOutputSource leader) => _candidateLeader = leader;

    internal void SynchronizeCandidate()
    {
        lock (_gate)
        {
            if (_candidateLeader is not null && _preparedSource is not null
                && _candidateLeader.ActiveCandidateId == _preparedCandidateId)
            {
                _activeSource = _preparedSource;
                _activeCandidateId = _preparedCandidateId;
                _preparedSource = null;
                _preparedCandidateId = 0;
                _framesRead = 0;
                _candidateStartTotalFrames = _totalFramesRead;
                _commitRequested = false;
                _commitAtFramesRead = null;
            }
        }
    }

    internal void SetInitialFramePosition(ulong framesRead)
    {
        lock (_gate)
        {
            _framesRead = framesRead;
            if (_readLeader is not null)
            {
                lock (_readLeader._gate)
                {
                    _candidateStartTotalFrames = _readLeader._candidateStartTotalFrames;
                }
            }
            _totalFramesRead = _candidateStartTotalFrames + framesRead;
        }
    }

    internal static bool TryCommitTogether(
        AudioPcmTrackSwitchOutputSource local,
        AudioPcmTrackSwitchOutputSource remote,
        ulong candidateId,
        ulong framesFromNow,
        bool remoteAttached,
        out AudioPcmTrackSwitchError? error)
    {
        // 与远端读取 leader 的锁序一致，防止本机已经跨边界而远端尚未取得边界。
        lock (remote._gate)
        lock (local._gate)
        {
            if (!local.TryCommitPreparedAtFrames(candidateId, framesFromNow, out error, out var boundary))
            {
                return false;
            }
            return remoteAttached
                ? remote.TryCommitPreparedAtPosition(candidateId, boundary, out error)
                : remote.TryCommitPrepared(candidateId, out error);
        }
    }

    /// <inheritdoc />
    public bool IsClosed
    {
        get
        {
            lock (_gate)
            {
                return _activeSource.IsClosed
                    && (!_commitRequested || _preparedSource is null);
            }
        }
    }

    /// <summary>登记唯一 N+1 源；登记本身不改变当前输出。</summary>
    public bool TryPrepareNext(
        ulong candidateId,
        IAudioPcmOutputSource source,
        out AudioPcmTrackSwitchError? error)
    {
        error = null;
        if (candidateId == 0)
        {
            error = new(
                AudioPcmTrackSwitchFailureCode.InvalidCandidateId,
                "下一音轨候选 ID 必须大于 0。");
            return false;
        }

        if (source is null)
        {
            error = new(
                AudioPcmTrackSwitchFailureCode.InvalidSource,
                "下一音轨输出源不可用。");
            return false;
        }

        lock (_gate)
        {
            if (candidateId == _activeCandidateId)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.SameAsActive,
                    "下一音轨候选不能与当前音轨相同。");
                return false;
            }

            if (_preparedSource is not null)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.AlreadyPrepared,
                    "已经存在一个待提交的下一音轨候选。");
                return false;
            }

            _preparedCandidateId = candidateId;
            _preparedSource = source;
            return true;
        }
    }

    /// <summary>
    /// 请求切换到已登记的 N+1；真正切换延迟到当前源关闭并排空，避免临时欠载被误判为 EOF。
    /// </summary>
    public bool TryCommitPrepared(
        ulong candidateId,
        out AudioPcmTrackSwitchError? error)
    {
        error = null;
        if (candidateId == 0)
        {
            error = new(
                AudioPcmTrackSwitchFailureCode.InvalidCandidateId,
                "待提交音轨候选 ID 必须大于 0。");
            return false;
        }

        lock (_gate)
        {
            if (_preparedSource is null || _preparedCandidateId != candidateId)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CandidateNotPrepared,
                    "指定的下一音轨候选尚未完成登记。");
                return false;
            }

            if (_commitRequested)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CommitAlreadyRequested,
                    "下一音轨候选已经请求提交。");
                return false;
            }

            _commitRequested = true;
            _commitAtFramesRead = null;
            return true;
        }
    }

    /// <summary>
    /// 请求在当前输出源再提供指定帧数后切换；用于同一播放会话内的有界声音周期。
    /// </summary>
    public bool TryCommitPreparedAtFrames(
        ulong candidateId,
        ulong framesFromNow,
        out AudioPcmTrackSwitchError? error)
        => TryCommitPreparedAtFrames(candidateId, framesFromNow, out error, out _);

    internal bool TryCommitPreparedAtFrames(
        ulong candidateId,
        ulong framesFromNow,
        out AudioPcmTrackSwitchError? error,
        out ulong commitPosition)
    {
        lock (_gate)
        {
            commitPosition = ulong.MaxValue - _framesRead < framesFromNow ? ulong.MaxValue : _framesRead + framesFromNow;
            return TryCommitPreparedAtPosition(
                candidateId,
                commitPosition,
                out error);
        }
    }

    internal bool TryCommitPreparedAtPosition(
        ulong candidateId,
        ulong candidateFramePosition,
        out AudioPcmTrackSwitchError? error)
    {
        error = null;
        if (candidateId == 0)
        {
            error = new(
                AudioPcmTrackSwitchFailureCode.InvalidCandidateId,
                "待提交音轨候选 ID 必须大于 0。");
            return false;
        }

        lock (_gate)
        {
            if (_preparedSource is null || _preparedCandidateId != candidateId)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CandidateNotPrepared,
                    "指定的下一音轨候选尚未完成登记。");
                return false;
            }

            if (_commitRequested)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CommitAlreadyRequested,
                    "下一音轨候选已经请求提交。");
                return false;
            }

            _commitRequested = true;
            _commitAtFramesRead = candidateFramePosition;
            return true;
        }
    }

    /// <summary>
    /// 在没有该分支消费者时完成已提交候选的切换。调用方必须已确认当前源已关闭；
    /// 有消费者的实时分支仍应等待 <see cref="TryReadRealtime" /> 自然排空。
    /// </summary>
    public bool TryPromoteCommittedWithoutRead(
        ulong candidateId,
        out AudioPcmTrackSwitchError? error)
    {
        error = null;
        lock (_gate)
        {
            if (_preparedSource is null || _preparedCandidateId != candidateId)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CandidateNotPrepared,
                    "指定的下一音轨候选尚未完成登记。");
                return false;
            }

            if (!_commitRequested)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CommitAlreadyRequested,
                    "下一音轨候选尚未请求提交。");
                return false;
            }

            if (!_activeSource.IsClosed)
            {
                return false;
            }

            _activeSource = _preparedSource;
            _framesRead = 0;
            _candidateStartTotalFrames = _totalFramesRead;
            _activeCandidateId = _preparedCandidateId;
            _preparedSource = null;
            _preparedCandidateId = 0;
            _commitRequested = false;
            _commitAtFramesRead = null;
            return true;
        }
    }

    /// <summary>无消费者分支直接丢弃旧源并推进已提交候选。</summary>
    public bool TryForcePromoteCommittedWithoutRead(
        ulong candidateId,
        out AudioPcmTrackSwitchError? error)
    {
        error = null;
        lock (_gate)
        {
            if (_preparedSource is null || _preparedCandidateId != candidateId)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CandidateNotPrepared,
                    "指定的下一音轨候选尚未完成登记。");
                return false;
            }

            if (!_commitRequested)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CommitAlreadyRequested,
                    "下一音轨候选尚未请求提交。");
                return false;
            }

            _activeSource = _preparedSource;
            _framesRead = 0;
            _candidateStartTotalFrames = _totalFramesRead;
            _activeCandidateId = _preparedCandidateId;
            _preparedSource = null;
            _preparedCandidateId = 0;
            _commitRequested = false;
            _commitAtFramesRead = null;
            return true;
        }
    }

    /// <summary>撤销尚未提交的 N+1 候选；提交后只能通过停止整个所有者回收。</summary>
    public bool TryCancelPrepared(
        ulong candidateId,
        out AudioPcmTrackSwitchError? error)
    {
        error = null;
        lock (_gate)
        {
            if (_preparedSource is null || _preparedCandidateId != candidateId)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CandidateNotPrepared,
                    "指定的下一音轨候选尚未完成登记。");
                return false;
            }

            if (_commitRequested)
            {
                error = new(
                    AudioPcmTrackSwitchFailureCode.CommitAlreadyRequested,
                    "下一音轨候选已经请求提交，不能撤销。");
                return false;
            }

            _preparedSource = null;
            _preparedCandidateId = 0;
            _commitAtFramesRead = null;
            return true;
        }
    }

    /// <inheritdoc />
    public bool TryRead(
        Span<float> destination,
        out int framesRead,
        out PcmRingBufferError? error) =>
        TryReadCore(destination, realtime: false, out framesRead, out error);

    /// <inheritdoc />
    public bool TryReadRealtime(
        Span<float> destination,
        out int framesRead,
        out PcmRingBufferError? error) =>
        TryReadCore(destination, realtime: true, out framesRead, out error);

    internal bool TryReadOneCandidate(Span<float> destination, bool realtime,
        out int framesRead, out PcmRingBufferError? error) =>
        TryReadCore(destination, realtime, out framesRead, out error, oneCandidate: true);

    private bool TryReadCore(
        Span<float> destination,
        bool realtime,
        out int framesRead,
        out PcmRingBufferError? error,
        bool oneCandidate = false)
    {
        framesRead = 0;
        error = null;
        if (destination.Length % _channels != 0)
        {
            error = new(
                PcmRingBufferFailureCode.InvalidFrameShape,
                "PCM 音轨切换输出目标必须完整对齐到当前声道帧。");
            return false;
        }

        if (realtime)
        {
            if (!Monitor.TryEnter(_gate))
            {
                error = new(PcmRingBufferFailureCode.ConsumerBusy, "PCM 音轨切换临界区繁忙。");
                return false;
            }
        }
        else
        {
            Monitor.Enter(_gate);
        }
        try
        {
            SynchronizeCandidate();
            while (framesRead < destination.Length / _channels)
            {
                IAudioPcmOutputSource source;
                lock (_gate)
                {
                    if (_commitRequested
                        && _commitAtFramesRead is ulong commitAtFramesRead
                        && _framesRead >= commitAtFramesRead
                        && _preparedSource is not null)
                    {
                        if (oneCandidate && framesRead > 0)
                        {
                            return true;
                        }
                        _activeSource = _preparedSource;
                        _framesRead = 0;
                        _candidateStartTotalFrames = _totalFramesRead;
                        _activeCandidateId = _preparedCandidateId;
                        _preparedSource = null;
                        _preparedCandidateId = 0;
                        _commitRequested = false;
                        _commitAtFramesRead = null;
                    }

                    source = _activeSource;
                }

                var target = destination.Slice(framesRead * _channels);
                if (_commitRequested && _commitAtFramesRead is ulong boundary)
                {
                    target = target[..((int)Math.Min((ulong)(target.Length / _channels), boundary - _framesRead) * _channels)];
                }
                if (_readLeader is not null)
                {
                    lock (_readLeader._gate)
                    {
                        var available = _readLeader._totalFramesRead > _totalFramesRead
                            ? _readLeader._totalFramesRead - _totalFramesRead
                            : 0;
                        target = target[..((int)Math.Min((ulong)(target.Length / _channels), available) * _channels)];
                    }
                    if (target.IsEmpty)
                    {
                        return true;
                    }
                }
                var readSucceeded = realtime
                    ? source.TryReadRealtime(target, out var sourceFrames, out error)
                    : source.TryRead(target, out sourceFrames, out error);
                if (!readSucceeded)
                {
                    return false;
                }

                if (sourceFrames < 0 || sourceFrames > target.Length / _channels)
                {
                    error = new(
                        PcmRingBufferFailureCode.InvalidFrameShape,
                        "PCM 音轨输出源返回了超过目标容量的帧数。");
                    return false;
                }

                framesRead += sourceFrames;
                lock (_gate)
                {
                    _framesRead = ulong.MaxValue - (ulong)sourceFrames < _framesRead
                        ? ulong.MaxValue
                        : _framesRead + (ulong)sourceFrames;
                    _totalFramesRead = ulong.MaxValue - (ulong)sourceFrames < _totalFramesRead
                        ? ulong.MaxValue
                        : _totalFramesRead + (ulong)sourceFrames;
                }
                if (framesRead == destination.Length / _channels)
                {
                    return true;
                }

                if (_commitRequested && _commitAtFramesRead is ulong reached && _framesRead >= reached)
                {
                    continue;
                }

                if (!source.IsClosed)
                {
                    return true;
                }

                if (oneCandidate && framesRead > 0)
                {
                    return true;
                }

                if (!TryPromotePrepared(source))
                {
                    return true;
                }
            }

            return true;
        }
        finally
        {
            Monitor.Exit(_gate);
        }
    }

    private bool TryPromotePrepared(IAudioPcmOutputSource drainedSource)
    {
        lock (_gate)
        {
            if (!ReferenceEquals(_activeSource, drainedSource)
                || _candidateLeader is not null
                || !_commitRequested
                || _preparedSource is null)
            {
                return false;
            }

            _activeSource = _preparedSource;
            _framesRead = 0;
            _candidateStartTotalFrames = _totalFramesRead;
            _activeCandidateId = _preparedCandidateId;
            _preparedSource = null;
            _preparedCandidateId = 0;
            _commitRequested = false;
            _commitAtFramesRead = null;
            return true;
        }
    }
}
