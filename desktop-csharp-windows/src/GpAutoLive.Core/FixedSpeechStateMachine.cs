using GpAutoLive.Contracts;

namespace GpAutoLive.Core;

/// <summary>固定话术纯逻辑运行状态；不创建语音设备或后台任务。</summary>
public enum FixedSpeechRuntimeState
{
    /// <summary>当前没有已开始的朗读操作。</summary>
    Idle,

    /// <summary>已接受文本，等待 Windows 本地语音能力开始。</summary>
    Starting,

    /// <summary>本地语音正在朗读。</summary>
    Playing,

    /// <summary>最近一次朗读已完成。</summary>
    Completed,

    /// <summary>最近一次朗读已取消。</summary>
    Cancelled,

    /// <summary>最近一次朗读失败。</summary>
    Failed
}

/// <summary>固定话术状态机变更类型。</summary>
public enum FixedSpeechTransitionKind
{
    /// <summary>新朗读已接受。</summary>
    Accepted,

    /// <summary>当前操作已取消。</summary>
    Cancelled,

    /// <summary>当前操作已正常结束。</summary>
    Completed,

    /// <summary>当前操作已失败。</summary>
    Failed,

    /// <summary>命令有效，但不匹配当前活动操作，状态未改变。</summary>
    Ignored,

    /// <summary>命令输入无效。</summary>
    Rejected
}

/// <summary>固定话术状态机快照；不保存文本正文，避免正文进入长期状态。</summary>
public sealed record FixedSpeechSnapshot
{
    /// <summary>初始空快照。</summary>
    public static FixedSpeechSnapshot Initial { get; } = new();

    /// <summary>当前状态。</summary>
    public FixedSpeechRuntimeState State { get; init; } = FixedSpeechRuntimeState.Idle;

    /// <summary>当前或最近一次操作 ID。</summary>
    public string? OperationId { get; init; }

    /// <summary>最近一次失败的脱敏错误摘要。</summary>
    public string? Error { get; init; }

    /// <summary>是否存在需要语音适配器处理的活动操作。</summary>
    public bool IsActive => State is FixedSpeechRuntimeState.Starting or FixedSpeechRuntimeState.Playing;
}

/// <summary>固定话术命令处理结果。</summary>
public sealed record FixedSpeechTransition(
    FixedSpeechTransitionKind Kind,
    string? OperationId,
    FixedSpeechSnapshot Snapshot,
    string? SupersededOperationId = null,
    FixedSpeechValidationError? Error = null)
{
    /// <summary>该结果是否接受了新的有效操作。</summary>
    public bool IsAccepted => Kind == FixedSpeechTransitionKind.Accepted;

    /// <summary>该结果是否改变了当前活动操作的终态。</summary>
    public bool IsTerminal => Kind is FixedSpeechTransitionKind.Cancelled
        or FixedSpeechTransitionKind.Completed
        or FixedSpeechTransitionKind.Failed;
}

/// <summary>
/// 固定话术单操作所有者。它只管理命令、身份、优先级和终态，
/// Windows 语音适配器负责实际调用本机 SAPI；不访问网络、不启动进程、不读写音频设备。
/// </summary>
public sealed class FixedSpeechStateMachine
{
    private FixedSpeechSnapshot _snapshot = FixedSpeechSnapshot.Initial;

    /// <summary>当前不可变快照。</summary>
    public FixedSpeechSnapshot Snapshot => _snapshot;

    /// <summary>
    /// 接受一条开始朗读命令。新命令会抢占旧的 starting/playing 操作，
    /// 由调用方先对 <see cref="FixedSpeechTransition.SupersededOperationId" /> 发布 cancelled。
    /// 麦克风处于说话状态时只取消新命令，不覆盖当前状态。
    /// </summary>
    public FixedSpeechTransition BeginSpeak(
        FixedSpeechCommandDto? command,
        bool microphonePriorityActive = false)
    {
        if (command is null)
        {
            return Rejected(null, new(FixedSpeechErrorCodes.InvalidCommand, "固定话术命令无效"));
        }

        if (command.Action != FixedSpeechAction.Speak)
        {
            return Rejected(command.OperationId, new(FixedSpeechErrorCodes.InvalidCommand, "固定话术命令动作无效"));
        }

        if (!FixedSpeechContractValidation.TryValidateCommand(command, out var error))
        {
            return Rejected(command.OperationId, error!);
        }

        if (!FixedSpeechContractValidation.TryNormalizeText(command.Text, out _, out error))
        {
            return Rejected(command.OperationId, error!);
        }

        if (microphonePriorityActive)
        {
            return new(
                FixedSpeechTransitionKind.Cancelled,
                command.OperationId,
                _snapshot);
        }

        var superseded = _snapshot.IsActive ? _snapshot.OperationId : null;
        if (string.Equals(superseded, command.OperationId, StringComparison.Ordinal))
        {
            return Rejected(
                command.OperationId,
                new(FixedSpeechErrorCodes.InvalidCommand, "固定话术操作 ID 不能重复使用"));
        }

        _snapshot = new FixedSpeechSnapshot
        {
            State = FixedSpeechRuntimeState.Starting,
            OperationId = command.OperationId,
            Error = null
        };
        return new(
            FixedSpeechTransitionKind.Accepted,
            command.OperationId,
            _snapshot,
            superseded);
    }

    /// <summary>处理取消命令；过期或重复取消保持幂等且不污染新操作。</summary>
    public FixedSpeechTransition Cancel(FixedSpeechCommandDto? command)
    {
        if (command is null)
        {
            return Rejected(null, new(FixedSpeechErrorCodes.InvalidCommand, "固定话术取消命令无效"));
        }

        if (command.Action != FixedSpeechAction.Cancel)
        {
            return Rejected(command.OperationId, new(FixedSpeechErrorCodes.InvalidCommand, "固定话术取消命令动作无效"));
        }

        if (!FixedSpeechContractValidation.TryValidateCommand(command, out var error))
        {
            return Rejected(command.OperationId, error!);
        }

        if (!_snapshot.IsActive
            || !string.Equals(_snapshot.OperationId, command.OperationId, StringComparison.Ordinal))
        {
            return new(FixedSpeechTransitionKind.Ignored, command.OperationId, _snapshot);
        }

        _snapshot = _snapshot with
        {
            State = FixedSpeechRuntimeState.Cancelled,
            Error = null
        };
        return new(FixedSpeechTransitionKind.Cancelled, command.OperationId, _snapshot);
    }

    /// <summary>标记本地语音已真正开始；只接受当前 starting 操作。</summary>
    public FixedSpeechTransition MarkPlaying(string? operationId)
    {
        if (!TryValidateOperationId(operationId, out var error))
        {
            return Rejected(operationId, error!);
        }

        if (_snapshot.State != FixedSpeechRuntimeState.Starting
            || !string.Equals(_snapshot.OperationId, operationId, StringComparison.Ordinal))
        {
            return new(FixedSpeechTransitionKind.Ignored, operationId, _snapshot);
        }

        _snapshot = _snapshot with { State = FixedSpeechRuntimeState.Playing, Error = null };
        return new(FixedSpeechTransitionKind.Accepted, operationId, _snapshot);
    }

    /// <summary>标记当前朗读已完成；过期回调不会覆盖新操作。</summary>
    public FixedSpeechTransition Complete(string? operationId)
    {
        if (!TryValidateOperationId(operationId, out var error))
        {
            return Rejected(operationId, error!);
        }

        if (!_snapshot.IsActive
            || !string.Equals(_snapshot.OperationId, operationId, StringComparison.Ordinal))
        {
            return new(FixedSpeechTransitionKind.Ignored, operationId, _snapshot);
        }

        _snapshot = _snapshot with { State = FixedSpeechRuntimeState.Completed, Error = null };
        return new(FixedSpeechTransitionKind.Completed, operationId, _snapshot);
    }

    /// <summary>标记当前朗读失败；错误摘要无效时收敛到固定脱敏文案。</summary>
    public FixedSpeechTransition Fail(string? operationId, string? errorMessage)
    {
        if (!TryValidateOperationId(operationId, out var validationError))
        {
            return Rejected(operationId, validationError!);
        }

        if (!_snapshot.IsActive
            || !string.Equals(_snapshot.OperationId, operationId, StringComparison.Ordinal))
        {
            return new(FixedSpeechTransitionKind.Ignored, operationId, _snapshot);
        }

        var error = errorMessage is not null
            && errorMessage.Length <= FixedSpeechInputLimits.MaxErrorLength
            && !errorMessage.Contains('\0')
            ? errorMessage
            : "系统语音播放失败";
        _snapshot = _snapshot with
        {
            State = FixedSpeechRuntimeState.Failed,
            Error = error
        };
        return new(FixedSpeechTransitionKind.Failed, operationId, _snapshot);
    }

    /// <summary>取消当前活动操作，用于麦克风抢占、媒体切源和统一退出。</summary>
    public FixedSpeechTransition CancelActive()
    {
        if (!_snapshot.IsActive || _snapshot.OperationId is null)
        {
            return new(FixedSpeechTransitionKind.Ignored, null, _snapshot);
        }

        _snapshot = _snapshot with
        {
            State = FixedSpeechRuntimeState.Cancelled,
            Error = null
        };
        return new(FixedSpeechTransitionKind.Cancelled, _snapshot.OperationId, _snapshot);
    }

    private static bool TryValidateOperationId(
        string? operationId,
        out FixedSpeechValidationError? error) =>
        FixedSpeechContractValidation.TryValidateOperationId(operationId, out error);

    private FixedSpeechTransition Rejected(
        string? operationId,
        FixedSpeechValidationError error) =>
        new(FixedSpeechTransitionKind.Rejected, operationId, _snapshot, Error: error);
}
