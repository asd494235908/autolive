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
            _commitAtFramesRead = ulong.MaxValue - _framesRead < framesFromNow
                ? ulong.MaxValue
                : _framesRead + framesFromNow;
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

    private bool TryReadCore(
        Span<float> destination,
        bool realtime,
        out int framesRead,
        out PcmRingBufferError? error)
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
                    _activeSource = _preparedSource;
                    _activeCandidateId = _preparedCandidateId;
                    _preparedSource = null;
                    _preparedCandidateId = 0;
                    _commitRequested = false;
                    _commitAtFramesRead = null;
                }

                source = _activeSource;
            }

            var target = destination.Slice(framesRead * _channels);
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
            }
            if (framesRead == destination.Length / _channels || !source.IsClosed)
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

    private bool TryPromotePrepared(IAudioPcmOutputSource drainedSource)
    {
        lock (_gate)
        {
            if (!ReferenceEquals(_activeSource, drainedSource)
                || !_commitRequested
                || _preparedSource is null)
            {
                return false;
            }

            _activeSource = _preparedSource;
            _activeCandidateId = _preparedCandidateId;
            _preparedSource = null;
            _preparedCandidateId = 0;
            _commitRequested = false;
            _commitAtFramesRead = null;
            return true;
        }
    }
}
