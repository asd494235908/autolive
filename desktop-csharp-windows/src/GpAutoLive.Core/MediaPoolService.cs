using System.Collections.Immutable;
using System.IO;
using System.Text;
using GpAutoLive.Contracts;

namespace GpAutoLive.Core;

/// <summary>播放池操作失败时返回的脱敏错误。</summary>
public sealed record MediaPoolError(
    string Code,
    string Message,
    int? ItemIndex = null);

/// <summary>播放池操作的不可变结果；失败时快照一定是提交前快照。</summary>
public sealed record MediaPoolOperationResult(
    bool IsSuccess,
    bool Changed,
    AppState Snapshot,
    MediaPoolError? Error = null)
{
    /// <summary>构造成功结果。</summary>
    public static MediaPoolOperationResult Success(AppState snapshot, bool changed) =>
        new(true, changed, snapshot);

    /// <summary>构造失败结果。</summary>
    public static MediaPoolOperationResult Failure(AppState snapshot, MediaPoolError error) =>
        new(false, false, snapshot, error);
}

/// <summary>用于拒绝迟到 EOF/播放回调的四维媒体身份。</summary>
public sealed record MediaPlaybackIdentity(
    ulong PlaybackGeneration,
    ulong SourceRevision,
    int SourceMediaIndex,
    ulong LoopIndex);

/// <summary>
/// 播放池与播放身份的唯一所有者。类内只保留有界不可变快照，不启动 FFprobe、播放器或其它媒体进程。
/// </summary>
public sealed class MediaPoolOwner
{
    private readonly object _gate = new();
    private AppState _snapshot = AppState.Initial;

    /// <summary>获取当前不可变快照。</summary>
    public AppState Snapshot
    {
        get
        {
            lock (_gate)
            {
                return _snapshot;
            }
        }
    }

    /// <summary>获取当前播放身份，用于异步回调的 fail-closed 校验。</summary>
    public MediaPlaybackIdentity CurrentIdentity
    {
        get
        {
            lock (_gate)
            {
                return IdentityOf(_snapshot);
            }
        }
    }

    /// <summary>原子替换整个播放池；空列表应使用 Clear。</summary>
    public MediaPoolOperationResult ReplaceAll(IReadOnlyList<SourceMediaDto>? candidates) =>
        Edit(candidates, static (_, normalized) => normalized.ToImmutableArray(), append: false);

    /// <summary>原子追加候选媒体。</summary>
    public MediaPoolOperationResult Append(IReadOnlyList<SourceMediaDto>? candidates) =>
        Edit(candidates, static (current, normalized) =>
        {
            var combined = new List<SourceMediaDto>(current.SourceMediaPool.Length + normalized.Count);
            combined.AddRange(current.SourceMediaPool);
            combined.AddRange(normalized);
            return combined.ToImmutableArray();
        }, append: true);

    /// <summary>原子替换指定索引的媒体项。</summary>
    public MediaPoolOperationResult ReplaceAt(int index, SourceMediaDto? candidate)
    {
        lock (_gate)
        {
            if (!TryValidateIndex(_snapshot, index, out var error))
            {
                return MediaPoolOperationResult.Failure(_snapshot, error!);
            }

            if (!TryNormalizeCandidate(candidate, out var normalized, out error, index))
            {
                return MediaPoolOperationResult.Failure(_snapshot, error!);
            }

            var edited = _snapshot.SourceMediaPool.ToArray();
            edited[index] = normalized!;
            if (!TryValidateDistinctPaths(edited, out error))
            {
                return MediaPoolOperationResult.Failure(_snapshot, error!);
            }

            return CommitEditedPool(edited.ToImmutableArray());
        }
    }

    /// <summary>按稳定规范化路径原子替换媒体项。</summary>
    public MediaPoolOperationResult ReplacePath(string? sourcePath, SourceMediaDto? candidate)
    {
        lock (_gate)
        {
            if (!TryCanonicalizePath(sourcePath, out var canonical, out var error))
            {
                return MediaPoolOperationResult.Failure(_snapshot, error!);
            }

            var index = FindIndex(_snapshot.SourceMediaPool, canonical!);
            if (index < 0)
            {
                return MediaPoolOperationResult.Failure(
                    _snapshot,
                    new MediaPoolError("source_media_not_found", "播放池中不存在要替换的媒体项"));
            }

            return ReplaceAtCore(index, candidate);
        }
    }

    /// <summary>提交完整且不重复的当前路径顺序；相同顺序是幂等空操作。</summary>
    public MediaPoolOperationResult Reorder(IReadOnlyList<string>? orderedSourcePaths)
    {
        lock (_gate)
        {
            if (orderedSourcePaths is null)
            {
                return MediaPoolOperationResult.Failure(
                    _snapshot,
                    new MediaPoolError("null_source_media_paths", "媒体路径顺序不能为空"));
            }

            if (orderedSourcePaths.Count != _snapshot.SourceMediaPool.Length)
            {
                return MediaPoolOperationResult.Failure(
                    _snapshot,
                    new MediaPoolError("source_media_order_mismatch", "媒体路径顺序必须完整覆盖当前播放池"));
            }

            var canonicalPaths = new string[orderedSourcePaths.Count];
            var seen = new HashSet<string>(PathComparer);
            for (var index = 0; index < orderedSourcePaths.Count; index++)
            {
                if (!TryCanonicalizePath(orderedSourcePaths[index], out var canonical, out var error, index))
                {
                    return MediaPoolOperationResult.Failure(_snapshot, error!);
                }

                if (!seen.Add(canonical!))
                {
                    return MediaPoolOperationResult.Failure(
                        _snapshot,
                        new MediaPoolError("duplicate_source_media_path", "媒体路径顺序包含重复项", index));
                }

                canonicalPaths[index] = canonical!;
            }

            var reordered = new SourceMediaDto[canonicalPaths.Length];
            for (var index = 0; index < canonicalPaths.Length; index++)
            {
                var existingIndex = FindIndex(_snapshot.SourceMediaPool, canonicalPaths[index]);
                if (existingIndex < 0)
                {
                    return MediaPoolOperationResult.Failure(
                        _snapshot,
                        new MediaPoolError("source_media_not_found", "媒体路径顺序包含当前播放池之外的项", index));
                }

                reordered[index] = _snapshot.SourceMediaPool[existingIndex];
            }

            return CommitEditedPool(reordered.ToImmutableArray());
        }
    }

    /// <summary>将媒体项上移；首项上移是幂等空操作。</summary>
    public MediaPoolOperationResult MoveUp(int index) => Move(index, -1);

    /// <summary>将媒体项下移；末项下移是幂等空操作。</summary>
    public MediaPoolOperationResult MoveDown(int index) => Move(index, 1);

    /// <summary>按索引删除一个媒体项。</summary>
    public MediaPoolOperationResult RemoveAt(int index)
    {
        lock (_gate)
        {
            if (!TryValidateIndex(_snapshot, index, out var error))
            {
                return MediaPoolOperationResult.Failure(_snapshot, error!);
            }

            var edited = _snapshot.SourceMediaPool
                .Where((_, itemIndex) => itemIndex != index)
                .ToImmutableArray();
            return CommitEditedPool(edited);
        }
    }

    /// <summary>按稳定规范化路径删除一个媒体项。</summary>
    public MediaPoolOperationResult RemovePath(string? sourcePath)
    {
        lock (_gate)
        {
            if (!TryCanonicalizePath(sourcePath, out var canonical, out var error))
            {
                return MediaPoolOperationResult.Failure(_snapshot, error!);
            }

            var index = FindIndex(_snapshot.SourceMediaPool, canonical!);
            if (index < 0)
            {
                return MediaPoolOperationResult.Failure(
                    _snapshot,
                    new MediaPoolError("source_media_not_found", "播放池中不存在要删除的媒体项"));
            }

            var edited = _snapshot.SourceMediaPool
                .Where((_, itemIndex) => itemIndex != index)
                .ToImmutableArray();
            return CommitEditedPool(edited);
        }
    }

    /// <summary>清空播放池并将播放状态设为 Stopped。</summary>
    public MediaPoolOperationResult Clear()
    {
        lock (_gate)
        {
            if (_snapshot.SourceMediaPool.IsEmpty)
            {
                return MediaPoolOperationResult.Success(_snapshot, changed: false);
            }

            return CommitEditedPool([]);
        }
    }

    /// <summary>开始播放当前项；重复开始是幂等操作。</summary>
    public MediaPoolOperationResult StartPlayback()
    {
        lock (_gate)
        {
            if (_snapshot.SourceMediaPool.IsEmpty)
            {
                return Failure("source_media_pool_empty", "播放池为空，无法开始播放");
            }

            if (_snapshot.PlaybackState == PlaybackState.Playing)
            {
                return MediaPoolOperationResult.Success(_snapshot, changed: false);
            }

            return SetPlaybackState(PlaybackState.Playing);
        }
    }

    /// <summary>暂停当前播放；非 Playing 状态不进行猜测性修改。</summary>
    public MediaPoolOperationResult PausePlayback()
    {
        lock (_gate)
        {
            if (_snapshot.PlaybackState != PlaybackState.Playing)
            {
                return Failure("invalid_playback_transition", $"当前状态 {_snapshot.PlaybackState} 不能暂停");
            }

            return SetPlaybackState(PlaybackState.Paused);
        }
    }

    /// <summary>继续播放；Ready 和 Paused 都允许进入 Playing。</summary>
    public MediaPoolOperationResult ResumePlayback()
    {
        lock (_gate)
        {
            if (_snapshot.SourceMediaPool.IsEmpty)
            {
                return Failure("source_media_pool_empty", "播放池为空，无法继续播放");
            }

            if (_snapshot.PlaybackState is PlaybackState.Playing)
            {
                return MediaPoolOperationResult.Success(_snapshot, changed: false);
            }

            if (_snapshot.PlaybackState is not (PlaybackState.Ready or PlaybackState.Paused))
            {
                return Failure("invalid_playback_transition", $"当前状态 {_snapshot.PlaybackState} 不能继续播放");
            }

            return SetPlaybackState(PlaybackState.Playing);
        }
    }

    /// <summary>停止播放并递增播放代次，使在途媒体回调失效。</summary>
    public MediaPoolOperationResult StopPlayback()
    {
        lock (_gate)
        {
            if (!TryIncrement(_snapshot.PlaybackGeneration, out var generation))
            {
                return Failure("playback_identity_exhausted", "播放代次已达到上限，拒绝继续生成新身份");
            }

            _snapshot = _snapshot with
            {
                PlaybackState = PlaybackState.Stopped,
                PlaybackGeneration = generation,
            };
            return MediaPoolOperationResult.Success(_snapshot, changed: true);
        }
    }

    /// <summary>
    /// 手动选择播放池中的一项。调用方必须携带发起操作时的四维身份，
    /// 以便切源请求和迟到的旧 UI/播放器事件不会覆盖新快照。
    /// 选择只改变当前索引和播放代次，不改变播放池修订号或播放状态；
    /// 目标项变化后循环索引归零。
    /// </summary>
    public MediaPoolOperationResult SelectAt(int index, MediaPlaybackIdentity expectedIdentity)
    {
        lock (_gate)
        {
            return SelectAtCore(index, expectedIdentity);
        }
    }

    /// <summary>按当前身份手动选择上一项；首项向末项回绕。</summary>
    public MediaPoolOperationResult Previous(MediaPlaybackIdentity expectedIdentity) =>
        SelectRelative(expectedIdentity, delta: -1);

    /// <summary>按当前身份手动选择下一项；末项向首项回绕。</summary>
    public MediaPoolOperationResult Next(MediaPlaybackIdentity expectedIdentity) =>
        SelectRelative(expectedIdentity, delta: 1);

    /// <summary>
    /// 在当前项自然 EOF 后推进一次。必须携带最新四维身份，迟到或重复回调 fail-closed。
    /// </summary>
    public MediaPoolOperationResult CompleteCurrent(
        ulong expectedPlaybackGeneration,
        ulong expectedSourceRevision,
        int expectedSourceMediaIndex,
        ulong expectedLoopIndex)
    {
        lock (_gate)
        {
            if (_snapshot.PlaybackState != PlaybackState.Playing)
            {
                return Failure("invalid_playback_transition", "只有 Playing 状态可以完成当前媒体项");
            }

            var expected = new MediaPlaybackIdentity(
                expectedPlaybackGeneration,
                expectedSourceRevision,
                expectedSourceMediaIndex,
                expectedLoopIndex);
            if (expected != IdentityOf(_snapshot))
            {
                return Failure("stale_playback_identity", "媒体完成回调身份已过期，已拒绝推进播放池");
            }

            if (_snapshot.SourceMediaPool.Length == 1)
            {
                if (!TryIncrement(_snapshot.LoopIndex, out var loopIndex)
                    || !TryIncrement(_snapshot.PlaybackPoolCycle, out var singlePoolCycle))
                {
                    return Failure("playback_identity_exhausted", "循环身份已达到上限，拒绝继续推进");
                }

                _snapshot = _snapshot with
                {
                    LoopIndex = loopIndex,
                    PlaybackPoolCycle = singlePoolCycle,
                };
                return MediaPoolOperationResult.Success(_snapshot, changed: true);
            }

            var nextIndex = _snapshot.SourceMediaIndex + 1;
            var wrapped = nextIndex == _snapshot.SourceMediaPool.Length;
            if (wrapped)
            {
                nextIndex = 0;
            }

            var nextPoolCycle = _snapshot.PlaybackPoolCycle;
            if (!TryIncrement(_snapshot.PlaybackGeneration, out var generation)
                || (wrapped && !TryIncrement(_snapshot.PlaybackPoolCycle, out nextPoolCycle)))
            {
                return Failure("playback_identity_exhausted", "播放身份已达到上限，拒绝继续推进");
            }

            _snapshot = _snapshot with
            {
                SourceMediaIndex = nextIndex,
                PlaybackGeneration = generation,
                LoopIndex = 0,
                PlaybackPoolCycle = nextPoolCycle,
            };
            return MediaPoolOperationResult.Success(_snapshot, changed: true);
        }
    }

    private MediaPoolOperationResult SelectRelative(MediaPlaybackIdentity expectedIdentity, int delta)
    {
        lock (_gate)
        {
            if (_snapshot.SourceMediaPool.IsEmpty)
            {
                return Failure("source_media_pool_empty", "播放池为空，无法切换媒体项");
            }

            if (expectedIdentity is null)
            {
                return Failure("null_playback_identity", "媒体切换请求缺少播放身份");
            }

            if (expectedIdentity != IdentityOf(_snapshot))
            {
                return Failure("stale_playback_identity", "媒体切换请求身份已过期，已拒绝切换播放项");
            }

            var length = _snapshot.SourceMediaPool.Length;
            var target = _snapshot.SourceMediaIndex + delta;
            if (target < 0)
            {
                target = length - 1;
            }
            else if (target >= length)
            {
                target = 0;
            }

            return SelectAtCore(target, expectedIdentity, identityAlreadyValidated: true);
        }
    }

    private MediaPoolOperationResult SelectAtCore(
        int index,
        MediaPlaybackIdentity? expectedIdentity,
        bool identityAlreadyValidated = false)
    {
        if (_snapshot.SourceMediaPool.IsEmpty)
        {
            return Failure("source_media_pool_empty", "播放池为空，无法切换媒体项");
        }

        if (!identityAlreadyValidated)
        {
            if (expectedIdentity is null)
            {
                return Failure("null_playback_identity", "媒体切换请求缺少播放身份");
            }

            if (expectedIdentity != IdentityOf(_snapshot))
            {
                return Failure("stale_playback_identity", "媒体切换请求身份已过期，已拒绝切换播放项");
            }
        }

        if (!TryValidateIndex(_snapshot, index, out var error))
        {
            return MediaPoolOperationResult.Failure(_snapshot, error!);
        }

        if (index == _snapshot.SourceMediaIndex)
        {
            return MediaPoolOperationResult.Success(_snapshot, changed: false);
        }

        if (!TryIncrement(_snapshot.PlaybackGeneration, out var generation))
        {
            return Failure("playback_identity_exhausted", "播放代次已达到上限，拒绝切换媒体项");
        }

        _snapshot = _snapshot with
        {
            SourceMediaIndex = index,
            PlaybackGeneration = generation,
            LoopIndex = 0,
        };
        return MediaPoolOperationResult.Success(_snapshot, changed: true);
    }

    private MediaPoolOperationResult ReplaceAtCore(int index, SourceMediaDto? candidate)
    {
        if (!TryNormalizeCandidate(candidate, out var normalized, out var error, index))
        {
            return MediaPoolOperationResult.Failure(_snapshot, error!);
        }

        var edited = _snapshot.SourceMediaPool.ToArray();
        edited[index] = normalized!;
        if (!TryValidateDistinctPaths(edited, out error))
        {
            return MediaPoolOperationResult.Failure(_snapshot, error!);
        }

        return CommitEditedPool(edited.ToImmutableArray());
    }

    private MediaPoolOperationResult Move(int index, int delta)
    {
        lock (_gate)
        {
            if (!TryValidateIndex(_snapshot, index, out var error))
            {
                return MediaPoolOperationResult.Failure(_snapshot, error!);
            }

            var target = index + delta;
            if (target < 0 || target >= _snapshot.SourceMediaPool.Length)
            {
                return MediaPoolOperationResult.Success(_snapshot, changed: false);
            }

            var edited = _snapshot.SourceMediaPool.ToArray();
            (edited[index], edited[target]) = (edited[target], edited[index]);
            return CommitEditedPool(edited.ToImmutableArray());
        }
    }

    private MediaPoolOperationResult Edit(
        IReadOnlyList<SourceMediaDto>? candidates,
        Func<AppState, List<SourceMediaDto>, ImmutableArray<SourceMediaDto>> edit,
        bool append)
    {
        lock (_gate)
        {
            if (candidates is null)
            {
                return MediaPoolOperationResult.Failure(
                    _snapshot,
                    new MediaPoolError("null_source_media_candidates", "媒体候选不能为空"));
            }

            if (candidates.Count == 0)
            {
                return MediaPoolOperationResult.Failure(
                    _snapshot,
                    new MediaPoolError("source_media_candidates_empty", "媒体候选不能为空；清空播放池请使用 Clear"));
            }

            if (candidates.Count > MediaPoolRules.MaxItems
                || (append && _snapshot.SourceMediaPool.Length + candidates.Count > MediaPoolRules.MaxItems))
            {
                return MediaPoolOperationResult.Failure(
                    _snapshot,
                    new MediaPoolError("source_media_pool_too_large", $"播放池最多允许 {MediaPoolRules.MaxItems} 项媒体"));
            }

            var normalized = new List<SourceMediaDto>(candidates.Count);
            for (var index = 0; index < candidates.Count; index++)
            {
                if (!TryNormalizeCandidate(candidates[index], out var candidate, out var error, index))
                {
                    return MediaPoolOperationResult.Failure(_snapshot, error!);
                }

                normalized.Add(candidate!);
            }

            ImmutableArray<SourceMediaDto> edited;
            try
            {
                edited = edit(_snapshot, normalized);
            }
            catch (OverflowException)
            {
                return Failure("source_media_pool_too_large", $"播放池最多允许 {MediaPoolRules.MaxItems} 项媒体");
            }

            if (!TryValidateDistinctPaths(edited, out var duplicateError))
            {
                return MediaPoolOperationResult.Failure(_snapshot, duplicateError!);
            }

            return CommitEditedPool(edited);
        }
    }

    private MediaPoolOperationResult CommitEditedPool(ImmutableArray<SourceMediaDto> edited)
    {
        if (SequenceEqual(_snapshot.SourceMediaPool, edited))
        {
            return MediaPoolOperationResult.Success(_snapshot, changed: false);
        }

        if (edited.Length > MediaPoolRules.MaxItems)
        {
            return Failure("source_media_pool_too_large", $"播放池最多允许 {MediaPoolRules.MaxItems} 项媒体");
        }

        if (!TryIncrement(_snapshot.SourceRevision, out var revision)
            || !TryIncrement(_snapshot.PlaybackGeneration, out var generation))
        {
            return Failure("playback_identity_exhausted", "播放身份已达到上限，拒绝提交新的播放池");
        }

        _snapshot = _snapshot with
        {
            SourceMediaPool = edited,
            SourceMediaIndex = 0,
            SourceRevision = revision,
            PlaybackGeneration = generation,
            LoopIndex = 0,
            PlaybackPoolCycle = 0,
            PlaybackState = edited.IsEmpty ? PlaybackState.Stopped : PlaybackState.Ready,
        };
        return MediaPoolOperationResult.Success(_snapshot, changed: true);
    }

    private MediaPoolOperationResult SetPlaybackState(PlaybackState playbackState)
    {
        _snapshot = _snapshot with { PlaybackState = playbackState };
        return MediaPoolOperationResult.Success(_snapshot, changed: true);
    }

    private MediaPoolOperationResult Failure(string code, string message, int? index = null) =>
        MediaPoolOperationResult.Failure(_snapshot, new MediaPoolError(code, message, index));

    private static bool TryValidateIndex(AppState snapshot, int index, out MediaPoolError? error)
    {
        if (index >= 0 && index < snapshot.SourceMediaPool.Length)
        {
            error = null;
            return true;
        }

        error = new MediaPoolError(
            "source_media_index_out_of_range",
            $"媒体索引 {index} 超出播放池范围",
            index);
        return false;
    }

    private static bool TryValidateDistinctPaths(
        IReadOnlyList<SourceMediaDto> candidates,
        out MediaPoolError? error)
    {
        var paths = new HashSet<string>(PathComparer);
        for (var index = 0; index < candidates.Count; index++)
        {
            if (!paths.Add(candidates[index].SourcePath))
            {
                error = new MediaPoolError(
                    "duplicate_source_media_path",
                    "播放池不允许重复媒体路径",
                    index);
                return false;
            }
        }

        error = null;
        return true;
    }

    private static bool TryNormalizeCandidate(
        SourceMediaDto? candidate,
        out SourceMediaDto? normalized,
        out MediaPoolError? error,
        int? index = null)
    {
        normalized = null;
        if (candidate is null)
        {
            error = new MediaPoolError("null_source_media", "媒体候选不能为空", index);
            return false;
        }

        if (!TryCanonicalizePath(candidate.SourcePath, out var sourcePath, out error, index))
        {
            return false;
        }

        if (!MediaPoolRules.TryGetMediaKind(sourcePath!, out var detectedKind))
        {
            error = new MediaPoolError("unsupported_source_media_extension", "媒体扩展名不在允许列表中", index);
            return false;
        }

        if (candidate.MediaKind != detectedKind)
        {
            error = new MediaPoolError("media_kind_mismatch", "媒体类别与文件扩展名不一致", index);
            return false;
        }

        if (!TryCanonicalizePath(candidate.PlaybackReference, out var playbackReference, out error, index))
        {
            return false;
        }

        if (!PathComparer.Equals(sourcePath, playbackReference))
        {
            error = new MediaPoolError(
                "playback_reference_mismatch",
                "当前阶段播放引用必须与规范化源路径一致",
                index);
            return false;
        }

        if (candidate.FileSizeBytes == 0)
        {
            error = new MediaPoolError("source_media_empty", "媒体文件大小必须大于 0", index);
            return false;
        }

        if (candidate.DurationMs == 0
            || candidate.AudioStartMs == 0
            || candidate.AudioEndMs == 0
            || candidate.DurationMs is ulong duration
                && ((candidate.AudioStartMs is ulong start && start >= duration)
                    || (candidate.AudioEndMs is ulong end && end > duration))
            || candidate.AudioStartMs is ulong audioStart
                && candidate.AudioEndMs is ulong audioEnd
                && audioStart >= audioEnd)
        {
            error = new MediaPoolError("invalid_source_media_metadata", "媒体时长或音频时间窗无效", index);
            return false;
        }

        if (candidate.Width is 0
            || candidate.Height is 0
            || candidate.AudioSampleRateHz is 0
            || candidate.AudioChannelCount is 0
            || candidate.FrameRateFps is double fps && (!double.IsFinite(fps) || fps <= 0))
        {
            error = new MediaPoolError("invalid_source_media_metadata", "媒体尺寸、帧率或音频参数无效", index);
            return false;
        }

        if (!Enum.IsDefined(candidate.MediaKind)
            || !Enum.IsDefined(candidate.CompatibilityMode)
            || string.IsNullOrWhiteSpace(candidate.Mp4HashStatus))
        {
            error = new MediaPoolError("invalid_source_media_metadata", "媒体类别、兼容模式或哈希状态无效", index);
            return false;
        }

        normalized = candidate with
        {
            SourcePath = sourcePath!,
            PlaybackReference = playbackReference!,
            FileName = Path.GetFileName(sourcePath!),
        };
        error = null;
        return true;
    }

    private static bool TryCanonicalizePath(
        string? path,
        out string? canonical,
        out MediaPoolError? error,
        int? index = null)
    {
        canonical = null;
        if (string.IsNullOrWhiteSpace(path))
        {
            error = new MediaPoolError("empty_source_media_path", "媒体路径不能为空", index);
            return false;
        }

        var trimmed = path.Trim();
        if (Encoding.UTF8.GetByteCount(trimmed) > MediaPoolRules.MaxSourcePathBytes)
        {
            error = new MediaPoolError(
                "source_media_path_too_long",
                $"媒体路径超过 {MediaPoolRules.MaxSourcePathBytes} 字节上限",
                index);
            return false;
        }

        try
        {
            if (!Path.IsPathFullyQualified(trimmed))
            {
                error = new MediaPoolError("source_media_path_not_absolute", "媒体路径必须是绝对路径", index);
                return false;
            }

            canonical = Path.TrimEndingDirectorySeparator(Path.GetFullPath(trimmed));
            if (Encoding.UTF8.GetByteCount(canonical) > MediaPoolRules.MaxSourcePathBytes)
            {
                error = new MediaPoolError(
                    "source_media_path_too_long",
                    $"规范化媒体路径超过 {MediaPoolRules.MaxSourcePathBytes} 字节上限",
                    index);
                return false;
            }

            error = null;
            return true;
        }
        catch (Exception exception) when (exception is ArgumentException or IOException or NotSupportedException)
        {
            error = new MediaPoolError("invalid_source_media_path", "媒体路径无法规范化", index);
            return false;
        }
    }

    private static int FindIndex(ImmutableArray<SourceMediaDto> pool, string canonicalPath)
    {
        for (var index = 0; index < pool.Length; index++)
        {
            if (PathComparer.Equals(pool[index].SourcePath, canonicalPath))
            {
                return index;
            }
        }

        return -1;
    }

    private static MediaPlaybackIdentity IdentityOf(AppState snapshot) =>
        new(snapshot.PlaybackGeneration, snapshot.SourceRevision, snapshot.SourceMediaIndex, snapshot.LoopIndex);

    private static bool SequenceEqual(
        ImmutableArray<SourceMediaDto> left,
        ImmutableArray<SourceMediaDto> right)
    {
        if (left.Length != right.Length)
        {
            return false;
        }

        for (var index = 0; index < left.Length; index++)
        {
            if (left[index] != right[index])
            {
                return false;
            }
        }

        return true;
    }

    private static bool TryIncrement(ulong value, out ulong result)
    {
        if (value == ulong.MaxValue)
        {
            result = value;
            return false;
        }

        result = value + 1;
        return true;
    }

    private static StringComparer PathComparer =>
        OperatingSystem.IsWindows() ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal;
}

/// <summary>播放池的应用层入口；保留单一所有者，避免 UI 直接改写 AppState。</summary>
public sealed class MediaPoolService
{
    private readonly MediaPoolOwner _owner;

    /// <summary>使用新的内存播放池创建服务。</summary>
    public MediaPoolService(MediaPoolOwner? owner = null) => _owner = owner ?? new MediaPoolOwner();

    /// <summary>当前快照。</summary>
    public AppState Snapshot => _owner.Snapshot;

    /// <summary>当前播放身份。</summary>
    public MediaPlaybackIdentity CurrentIdentity => _owner.CurrentIdentity;

    /// <summary>原子替换播放池。</summary>
    public MediaPoolOperationResult ReplaceAll(IReadOnlyList<SourceMediaDto>? candidates) => _owner.ReplaceAll(candidates);

    /// <summary>原子追加媒体。</summary>
    public MediaPoolOperationResult Append(IReadOnlyList<SourceMediaDto>? candidates) => _owner.Append(candidates);

    /// <summary>原子替换索引媒体。</summary>
    public MediaPoolOperationResult ReplaceAt(int index, SourceMediaDto? candidate) => _owner.ReplaceAt(index, candidate);

    /// <summary>按路径原子替换媒体。</summary>
    public MediaPoolOperationResult ReplacePath(string? sourcePath, SourceMediaDto? candidate) => _owner.ReplacePath(sourcePath, candidate);

    /// <summary>提交完整路径顺序。</summary>
    public MediaPoolOperationResult Reorder(IReadOnlyList<string>? paths) => _owner.Reorder(paths);

    /// <summary>上移媒体。</summary>
    public MediaPoolOperationResult MoveUp(int index) => _owner.MoveUp(index);

    /// <summary>下移媒体。</summary>
    public MediaPoolOperationResult MoveDown(int index) => _owner.MoveDown(index);

    /// <summary>删除索引媒体。</summary>
    public MediaPoolOperationResult RemoveAt(int index) => _owner.RemoveAt(index);

    /// <summary>按路径删除媒体。</summary>
    public MediaPoolOperationResult RemovePath(string? path) => _owner.RemovePath(path);

    /// <summary>清空媒体。</summary>
    public MediaPoolOperationResult Clear() => _owner.Clear();

    /// <summary>开始播放。</summary>
    public MediaPoolOperationResult StartPlayback() => _owner.StartPlayback();

    /// <summary>暂停播放。</summary>
    public MediaPoolOperationResult PausePlayback() => _owner.PausePlayback();

    /// <summary>继续播放。</summary>
    public MediaPoolOperationResult ResumePlayback() => _owner.ResumePlayback();

    /// <summary>停止播放。</summary>
    public MediaPoolOperationResult StopPlayback() => _owner.StopPlayback();

    /// <summary>按四维身份选择播放池项。</summary>
    public MediaPoolOperationResult SelectAt(int index, MediaPlaybackIdentity expectedIdentity) =>
        _owner.SelectAt(index, expectedIdentity);

    /// <summary>按四维身份选择上一项。</summary>
    public MediaPoolOperationResult Previous(MediaPlaybackIdentity expectedIdentity) =>
        _owner.Previous(expectedIdentity);

    /// <summary>按四维身份选择下一项。</summary>
    public MediaPoolOperationResult Next(MediaPlaybackIdentity expectedIdentity) =>
        _owner.Next(expectedIdentity);

    /// <summary>按身份完成当前项。</summary>
    public MediaPoolOperationResult CompleteCurrent(ulong generation, ulong revision, int index, ulong loopIndex) =>
        _owner.CompleteCurrent(generation, revision, index, loopIndex);
}
