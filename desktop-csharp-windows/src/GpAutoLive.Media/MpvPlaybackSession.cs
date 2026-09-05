using System.Collections.Immutable;
using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Media;

/// <summary>mpv 会话的生命周期；一个会话最多绑定一个活动视频源。</summary>
public enum MpvSessionState
{
    Closed,
    Ready,
    Playing,
    Paused,
    Faulted,
}

/// <summary>mpv 会话边界的稳定错误分类。</summary>
public enum MpvSessionFailureCode
{
    NullSource,
    SourceMustBeVideo,
    InvalidSourcePath,
    SourcePathMismatch,
    InvalidPlaybackIdentity,
    SessionClosed,
    NoActiveSource,
    InvalidStateTransition,
    StalePlaybackIdentity,
    InvalidEffectSnapshot,
    ResponseDoesNotBelongToSession,
}

/// <summary>脱敏会话错误，不保存路径、媒体名或 IPC 原始文本。</summary>
public sealed record MpvSessionError(
    MpvSessionFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>与 Core 播放池当前身份绑定的单一 mpv 活动源。</summary>
public sealed record MpvActiveSource(
    ValidatedMediaPath MediaPath,
    MediaPlaybackIdentity Identity,
    ulong? DurationMs)
{
    /// <summary>
    /// 从播放池源创建活动视频源。该方法只验证和复制快照，不访问文件或启动进程。
    /// </summary>
    public static bool TryCreate(
        SourceMediaDto? source,
        MediaPlaybackIdentity identity,
        out MpvActiveSource? activeSource,
        out MpvSessionError? error)
    {
        activeSource = null;
        error = null;
        if (source is null)
        {
            error = new(MpvSessionFailureCode.NullSource, "mpv 活动源不能为空。");
            return false;
        }

        if (source.MediaKind is not MediaKind.Video)
        {
            error = new(MpvSessionFailureCode.SourceMustBeVideo, "纯音频源不能绑定到 mpv 视频会话。");
            return false;
        }

        var pathError = MediaPathPolicy.Validate(
            source.SourcePath,
            requireExistingFile: false,
            out var validatedPath);
        if (pathError is not null || validatedPath.Kind is not MediaKind.Video)
        {
            error = new(MpvSessionFailureCode.InvalidSourcePath, "mpv 活动源路径无效。");
            return false;
        }

        var playbackPathError = MediaPathPolicy.Validate(
            source.PlaybackReference,
            requireExistingFile: false,
            out var playbackPath);
        if (playbackPathError is not null
            || !StringComparer.OrdinalIgnoreCase.Equals(
                validatedPath.CanonicalPath,
                playbackPath.CanonicalPath))
        {
            error = new(MpvSessionFailureCode.SourcePathMismatch, "mpv 活动源播放引用与源路径不一致。");
            return false;
        }

        if (identity.SourceMediaIndex < 0)
        {
            error = new(MpvSessionFailureCode.InvalidPlaybackIdentity, "mpv 活动源媒体索引无效。");
            return false;
        }

        if (source.DurationMs is 0
            || source.DurationMs is ulong duration && duration > 0 && source.AudioStartMs is ulong start && start >= duration)
        {
            error = new(MpvSessionFailureCode.InvalidPlaybackIdentity, "mpv 活动源时长无效。");
            return false;
        }

        activeSource = new MpvActiveSource(validatedPath, identity, source.DurationMs);
        return true;
    }
}

/// <summary>用于绑定 IPC 请求与当前活动源的最小身份。</summary>
public sealed record MpvPlaybackRequest(
    ulong RequestId,
    MpvIpcCommandKind CommandKind,
    MediaPlaybackIdentity Identity);

/// <summary>mpv 会话的不可变状态快照。</summary>
public sealed record MpvSessionSnapshot(
    MpvSessionState State,
    MpvActiveSource? ActiveSource,
    MpvVideoEffectSnapshot EffectSnapshot,
    ulong EffectRevision)
{
    public static MpvSessionSnapshot Initial { get; } = new(
        MpvSessionState.Closed,
        ActiveSource: null,
        MpvVideoEffectSnapshot.Default,
        EffectRevision: 0);
}

/// <summary>mpv 会话操作结果；成功操作可携带待发送的固定 IPC 命令。</summary>
public sealed record MpvSessionOperationResult(
    bool IsSuccess,
    bool Changed,
    MpvSessionSnapshot Snapshot,
    ImmutableArray<MpvIpcCommand> Commands,
    MpvSessionError? Error = null)
{
    public static MpvSessionOperationResult Success(
        MpvSessionSnapshot snapshot,
        bool changed,
        ImmutableArray<MpvIpcCommand> commands = default) =>
        new(true, changed, snapshot, commands.IsDefault ? [] : commands, null);

    public static MpvSessionOperationResult Failure(
        MpvSessionSnapshot snapshot,
        MpvSessionError error) =>
        new(false, false, snapshot, [], error);
}

/// <summary>
/// 单一 mpv 视频会话的纯逻辑所有者。此类不打开管道、不启动进程，
/// 只生成固定 IPC 命令并拒绝旧源的迟到响应。
/// </summary>
public sealed class MpvPlaybackSession
{
    private readonly object _gate = new();
    private MpvSessionSnapshot _snapshot = MpvSessionSnapshot.Initial;

    /// <summary>当前不可变会话快照。</summary>
    public MpvSessionSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return _snapshot;
            }
        }
    }

    /// <summary>
    /// 绑定或替换唯一活动视频源。替换源会重置到 Ready，并让旧身份失效。
    /// </summary>
    public MpvSessionOperationResult BindSource(MpvActiveSource? source, ulong sourceStartMs = 0)
    {
        lock (_gate)
        {
            if (source is null)
            {
                return MpvSessionOperationResult.Failure(
                    _snapshot,
                    new(MpvSessionFailureCode.NullSource, "mpv 活动源不能为空。"));
            }

            if (source.MediaPath.Kind is not MediaKind.Video
                || MediaPathPolicy.Validate(
                    source.MediaPath.CanonicalPath,
                    requireExistingFile: false,
                    out var validatedPath) is not null
                || validatedPath.Kind is not MediaKind.Video)
            {
                return MpvSessionOperationResult.Failure(
                    _snapshot,
                    new(MpvSessionFailureCode.InvalidSourcePath, "mpv 活动源路径无效。"));
            }

            if (sourceStartMs > 0
                && source.DurationMs is ulong duration
                && sourceStartMs >= duration)
            {
                return MpvSessionOperationResult.Failure(
                    _snapshot,
                    new(MpvSessionFailureCode.InvalidPlaybackIdentity, "mpv 源起始位置超出媒体时长。"));
            }

            if (_snapshot.EffectRevision == ulong.MaxValue)
            {
                return MpvSessionOperationResult.Failure(
                    _snapshot,
                    new(MpvSessionFailureCode.InvalidEffectSnapshot, "mpv 参数修订号已耗尽。"));
            }

            var unchanged = _snapshot.ActiveSource == source
                && _snapshot.State is not MpvSessionState.Closed;
            if (unchanged)
            {
                return MpvSessionOperationResult.Success(_snapshot, changed: false);
            }

            var command = MpvIpcCommand.LoadFileReplace(source, sourceStartMs);
            _snapshot = _snapshot with
            {
                State = MpvSessionState.Ready,
                ActiveSource = source,
                EffectSnapshot = MpvVideoEffectSnapshot.Default,
                EffectRevision = _snapshot.EffectRevision + 1,
            };
            return MpvSessionOperationResult.Success(_snapshot, changed: true, [command]);
        }
    }

    /// <summary>创建与当前身份绑定的播放请求；请求 ID 必须由唯一 IPC 所有者分配。</summary>
    public bool TryCreateRequest(
        ulong requestId,
        MpvIpcCommand command,
        MediaPlaybackIdentity expectedIdentity,
        out MpvPlaybackRequest? request,
        out MpvSessionError? error)
    {
        lock (_gate)
        {
            request = null;
            error = null;
            if (requestId == 0)
            {
                error = new(MpvSessionFailureCode.InvalidPlaybackIdentity, "mpv 请求身份无效。");
                return false;
            }

            if (_snapshot.State is MpvSessionState.Closed or MpvSessionState.Faulted
                || _snapshot.ActiveSource is null)
            {
                error = new(MpvSessionFailureCode.SessionClosed, "mpv 会话当前不可用。", Retryable: true);
                return false;
            }

            if (_snapshot.ActiveSource.Identity != expectedIdentity)
            {
                error = new(MpvSessionFailureCode.StalePlaybackIdentity, "mpv 播放身份已过期。");
                return false;
            }

            if (command is null)
            {
                error = new(MpvSessionFailureCode.InvalidStateTransition, "mpv IPC 命令不能为空。");
                return false;
            }

            if (!command.IsCompatibleWith(_snapshot.ActiveSource))
            {
                error = new(MpvSessionFailureCode.StalePlaybackIdentity, "mpv IPC 命令不属于当前活动源。");
                return false;
            }

            request = new MpvPlaybackRequest(requestId, command.Kind, expectedIdentity);
            return true;
        }
    }

    /// <summary>验证响应是否仍属于请求创建时的活动源和会话。</summary>
    public bool AcceptResponse(
        MpvPlaybackRequest? request,
        MpvIpcFrame? frame,
        out MpvSessionError? error)
    {
        lock (_gate)
        {
            error = null;
            if (request is null || frame is null)
            {
                error = new(MpvSessionFailureCode.ResponseDoesNotBelongToSession, "mpv IPC 响应身份无效。");
                return false;
            }

            if (_snapshot.ActiveSource is null
                || _snapshot.ActiveSource.Identity != request.Identity
                || frame.Kind is not MpvIpcFrameKind.Response
                || frame.RequestId != request.RequestId)
            {
                error = new(MpvSessionFailureCode.StalePlaybackIdentity, "mpv IPC 响应已过期，已拒绝应用。");
                return false;
            }

            return true;
        }
    }

    /// <summary>开始播放当前活动源。</summary>
    public MpvSessionOperationResult Start()
    {
        lock (_gate)
        {
            if (_snapshot.ActiveSource is null)
            {
                return Failure(MpvSessionFailureCode.NoActiveSource, "mpv 没有活动源。");
            }

            if (_snapshot.State is MpvSessionState.Playing)
            {
                return MpvSessionOperationResult.Success(_snapshot, changed: false);
            }

            if (_snapshot.State is not (MpvSessionState.Ready or MpvSessionState.Paused))
            {
                return Failure(MpvSessionFailureCode.InvalidStateTransition, "mpv 当前状态不能开始播放。");
            }

            _snapshot = _snapshot with { State = MpvSessionState.Playing };
            return MpvSessionOperationResult.Success(_snapshot, changed: true, [MpvIpcCommand.SetPause(false)]);
        }
    }

    /// <summary>暂停当前活动源。</summary>
    public MpvSessionOperationResult Pause()
    {
        lock (_gate)
        {
            if (_snapshot.State is not MpvSessionState.Playing)
            {
                return Failure(MpvSessionFailureCode.InvalidStateTransition, "mpv 当前状态不能暂停。");
            }

            _snapshot = _snapshot with { State = MpvSessionState.Paused };
            return MpvSessionOperationResult.Success(_snapshot, changed: true, [MpvIpcCommand.SetPause(true)]);
        }
    }

    /// <summary>恢复暂停的活动源。</summary>
    public MpvSessionOperationResult Resume()
    {
        lock (_gate)
        {
            if (_snapshot.State is not MpvSessionState.Paused)
            {
                return Failure(MpvSessionFailureCode.InvalidStateTransition, "mpv 当前状态不能恢复播放。");
            }

            _snapshot = _snapshot with { State = MpvSessionState.Playing };
            return MpvSessionOperationResult.Success(_snapshot, changed: true, [MpvIpcCommand.SetPause(false)]);
        }
    }

    /// <summary>停止当前会话；活动源保留以便用户再次开始。</summary>
    public MpvSessionOperationResult Stop()
    {
        lock (_gate)
        {
            if (_snapshot.ActiveSource is null)
            {
                return MpvSessionOperationResult.Success(_snapshot, changed: false);
            }

            _snapshot = _snapshot with { State = MpvSessionState.Ready };
            return MpvSessionOperationResult.Success(_snapshot, changed: true, [MpvIpcCommand.SetPause(true)]);
        }
    }

    /// <summary>关闭会话并清除活动源；之后旧请求全部失效。</summary>
    public MpvSessionOperationResult Close()
    {
        lock (_gate)
        {
            if (_snapshot.State is MpvSessionState.Closed && _snapshot.ActiveSource is null)
            {
                return MpvSessionOperationResult.Success(_snapshot, changed: false);
            }

            if (_snapshot.EffectRevision == ulong.MaxValue)
            {
                return Failure(MpvSessionFailureCode.InvalidEffectSnapshot, "mpv 参数修订号已耗尽。");
            }

            _snapshot = MpvSessionSnapshot.Initial with
            {
                EffectRevision = _snapshot.EffectRevision + 1,
            };
            return MpvSessionOperationResult.Success(_snapshot, changed: true, [MpvIpcCommand.Quit()]);
        }
    }

    /// <summary>为当前活动源原子替换视频效果快照，并返回固定命令序列。</summary>
    public MpvSessionOperationResult UpdateEffects(
        MediaPlaybackIdentity expectedIdentity,
        MpvVideoEffectSnapshot? next)
    {
        lock (_gate)
        {
            if (_snapshot.ActiveSource is null)
            {
                return Failure(MpvSessionFailureCode.NoActiveSource, "mpv 没有活动源。");
            }

            if (_snapshot.ActiveSource.Identity != expectedIdentity)
            {
                return Failure(MpvSessionFailureCode.StalePlaybackIdentity, "mpv 播放身份已过期。");
            }

            if (next is null || !next.TryValidate(out _))
            {
                return Failure(MpvSessionFailureCode.InvalidEffectSnapshot, "mpv 视频参数快照无效。");
            }

            if (!TryBuildEffectCommands(next, out var commands))
            {
                return Failure(MpvSessionFailureCode.InvalidEffectSnapshot, "mpv 视频参数快照无效。");
            }

            if (_snapshot.EffectSnapshot == next)
            {
                return MpvSessionOperationResult.Success(_snapshot, changed: false);
            }

            if (_snapshot.EffectRevision == ulong.MaxValue)
            {
                return Failure(MpvSessionFailureCode.InvalidEffectSnapshot, "mpv 参数修订号已耗尽。");
            }

            _snapshot = _snapshot with
            {
                EffectSnapshot = next,
                EffectRevision = _snapshot.EffectRevision + 1,
            };
            return MpvSessionOperationResult.Success(_snapshot, changed: true, commands);
        }
    }

    /// <summary>在当前会话身份下构造绝对 seek 命令，不改变播放状态。</summary>
    public MpvSessionOperationResult Seek(
        MediaPlaybackIdentity expectedIdentity,
        ulong positionMs)
    {
        lock (_gate)
        {
            if (_snapshot.ActiveSource is null)
            {
                return Failure(MpvSessionFailureCode.NoActiveSource, "mpv 没有活动源。");
            }

            if (_snapshot.ActiveSource.Identity != expectedIdentity)
            {
                return Failure(MpvSessionFailureCode.StalePlaybackIdentity, "mpv 播放身份已过期。");
            }

            var command = MpvIpcCommand.SeekAbsoluteMs(positionMs);
            return MpvSessionOperationResult.Success(_snapshot, changed: false, [command]);
        }
    }

    private static bool TryBuildEffectCommands(
        MpvVideoEffectSnapshot snapshot,
        out ImmutableArray<MpvIpcCommand> commands)
    {
        commands = [];
        if (!snapshot.TryValidate(out _))
        {
            return false;
        }

        var builder = ImmutableArray.CreateBuilder<MpvIpcCommand>();
        switch (snapshot.Mode)
        {
            case MpvVideoProcessingMode.Original:
                builder.Add(MpvIpcCommand.RemoveCpu4FilterChain());
                builder.Add(MpvIpcCommand.SetShaderOptions(MpvShaderOptionsSnapshot.Empty));
                break;
            case MpvVideoProcessingMode.Gpu83:
                builder.Add(MpvIpcCommand.RemoveCpu4FilterChain());
                builder.Add(MpvIpcCommand.SetShaderOptions(snapshot.ShaderOptions));
                break;
            case MpvVideoProcessingMode.Cpu4:
                builder.Add(MpvIpcCommand.SetShaderOptions(MpvShaderOptionsSnapshot.Empty));
                builder.Add(MpvIpcCommand.InstallCpu4FilterChain(snapshot));
                break;
            default:
                return false;
        }

        commands = builder.ToImmutable();
        return true;
    }

    private MpvSessionOperationResult Failure(MpvSessionFailureCode code, string message) =>
        MpvSessionOperationResult.Failure(_snapshot, new(code, message));
}
