using System.Collections.Immutable;
using GpAutoLive.Contracts;

namespace GpAutoLive.Core;

/// <summary>抖音 M1 状态操作的稳定错误。</summary>
public sealed record DouyinLiveOperationError(string Code, string Message);

/// <summary>收到弹幕后本地队列的处理结果。</summary>
public enum DouyinEnqueueDecision
{
    /// <summary>已随机选句并加入队列。</summary>
    Enqueued,
    /// <summary>过滤本账号回显。</summary>
    IgnoredSelf,
    /// <summary>过滤重连重放消息。</summary>
    IgnoredReplay,
    /// <summary>过滤已观察过的消息 ID。</summary>
    IgnoredDuplicate
}

/// <summary>抖音弹幕入队结果；正文只保留在本地进程内存。</summary>
public sealed record DouyinEnqueueResult(
    bool IsAccepted,
    DouyinEnqueueDecision Decision,
    DouyinLiveStatus Snapshot,
    DouyinReplyTask? Task = null,
    DouyinLiveOperationError? Error = null);

/// <summary>抖音会话生命周期操作结果。</summary>
public sealed record DouyinLiveOperationResult(
    bool IsSuccess,
    DouyinLiveStatus Snapshot,
    DouyinLiveOperationError? Error = null)
{
    /// <summary>构造成功结果。</summary>
    public static DouyinLiveOperationResult Success(DouyinLiveStatus snapshot) => new(true, snapshot);

    /// <summary>构造失败结果。</summary>
    public static DouyinLiveOperationResult Failure(DouyinLiveStatus snapshot, DouyinLiveOperationError error) =>
        new(false, snapshot, error);
}

/// <summary>
/// 抖音直播 M1 的本地唯一所有者。只管理扫码会话状态、弹幕去重、随机回复和有界串行任务队列；
/// 不保存凭据、不访问网络、不调用模型、不经过 Go，也不创建后台发送 Worker。
/// </summary>
public sealed class DouyinLiveManager
{
    private const int EventHistoryCapacity = 32;
    private readonly object _gate = new();
    private readonly Random _random;
    private readonly Queue<DouyinReplyTask> _queue = new();
    private readonly Queue<string> _seenMessageOrder = new(DouyinLiveRules.SeenMessageCapacity);
    private readonly HashSet<string> _seenMessageIds = new(StringComparer.Ordinal);
    private readonly Queue<string> _eventHistory = new(EventHistoryCapacity);
    private DouyinLiveConfig _config = DouyinLiveConfig.Default;
    private DouyinLiveState _state = DouyinLiveState.Idle;
    private ulong _generation = 1;
    private DouyinQueueMetrics _metrics = new();
    private string? _lastEvent;
    private string? _error;
    private bool _roomResolved;
    private bool _chatReceived;
    private bool _replyAttempted;
    private bool _selfEchoFiltered;
    private bool _replySendingBlocked;
    private int? _lastReplyIndex;
    private string? _lastGapReason;

    /// <summary>使用可注入随机源创建管理器；随机源只在控制线程使用。</summary>
    public DouyinLiveManager(Random? random = null)
    {
        _random = random ?? Random.Shared;
    }

    /// <summary>获取不包含凭据或完整弹幕历史的状态快照。</summary>
    public DouyinLiveStatus Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateStatus();
            }
        }
    }

    /// <summary>启动一轮等待扫码会话；会话重启时不恢复上次凭据。</summary>
    public DouyinLiveOperationResult TryStart(DouyinLiveConfig? config)
    {
        lock (_gate)
        {
            if (!DouyinLiveRules.TryNormalize(config, out var normalized, out var validationError))
            {
                return Failure(
                    new DouyinLiveOperationError(
                        "douyin_config_invalid",
                        validationError?.Message ?? "抖音配置无效"));
            }

            if (IsRunning())
            {
                return Failure(new DouyinLiveOperationError(
                    "douyin_invalid_transition",
                    "抖音 M1 会话已在运行"));
            }

            _config = normalized!;
            _queue.Clear();
            ClearSeenMessages();
            _metrics = new();
            _lastReplyIndex = null;
            _roomResolved = false;
            _chatReceived = false;
            _replyAttempted = false;
            _selfEchoFiltered = false;
            _replySendingBlocked = false;
            _lastGapReason = null;
            _error = null;
            InvalidateGeneration();
            _state = DouyinLiveState.WaitingQr;
            RecordEvent("probe_started");
            return Success();
        }
    }

    /// <summary>记录二维码已生成；二维码文件本身不由核心状态所有者持有。</summary>
    public DouyinLiveOperationResult MarkQrIssued() => SetEvent(
        DouyinLiveState.WaitingQr,
        "qr_issued",
        static state => state == DouyinLiveState.WaitingQr);

    /// <summary>记录扫码登录已确认。</summary>
    public DouyinLiveOperationResult MarkLoggedIn() => SetEvent(
        DouyinLiveState.LoggedIn,
        "login_confirmed",
        static state => state == DouyinLiveState.WaitingQr);

    /// <summary>记录直播间已解析。</summary>
    public DouyinLiveOperationResult MarkRoomResolved()
    {
        lock (_gate)
        {
            if (_state != DouyinLiveState.LoggedIn)
            {
                return Failure(InvalidTransition("mark_room_resolved"));
            }

            _state = DouyinLiveState.RoomResolved;
            _roomResolved = true;
            RecordEvent("room_resolved");
            return Success();
        }
    }

    /// <summary>进入单直播间公屏监听状态。</summary>
    public DouyinLiveOperationResult BeginListening()
    {
        lock (_gate)
        {
            if (_state is not (DouyinLiveState.RoomResolved or DouyinLiveState.Paused))
            {
                return Failure(InvalidTransition("begin_listening"));
            }

            _state = DouyinLiveState.Listening;
            RecordEvent("websocket_connected");
            return Success();
        }
    }

    /// <summary>暂停发送；暂停期间才允许修改队列容量。</summary>
    public DouyinLiveOperationResult Pause()
    {
        lock (_gate)
        {
            if (_state != DouyinLiveState.Listening)
            {
                return Failure(InvalidTransition("pause"));
            }

            _state = DouyinLiveState.Paused;
            RecordEvent("paused");
            return Success();
        }
    }

    /// <summary>从暂停恢复监听。</summary>
    public DouyinLiveOperationResult Resume()
    {
        lock (_gate)
        {
            if (_state != DouyinLiveState.Paused)
            {
                return Failure(InvalidTransition("resume"));
            }

            _state = DouyinLiveState.Listening;
            RecordEvent("resumed");
            return Success();
        }
    }

    /// <summary>停止并清理内存中的队列、去重集合和会话状态。</summary>
    public DouyinLiveOperationResult Stop()
    {
        lock (_gate)
        {
            if (_state == DouyinLiveState.Idle)
            {
                return Success();
            }

            _state = DouyinLiveState.Stopping;
            RecordEvent("probe_stopping");
            InvalidateGeneration();
            _queue.Clear();
            ClearSeenMessages();
            _state = DouyinLiveState.Idle;
            _roomResolved = false;
            _chatReceived = false;
            _replyAttempted = false;
            _selfEchoFiltered = false;
            _replySendingBlocked = false;
            _lastGapReason = null;
            _error = null;
            RecordEvent("probe_stopped");
            return Success();
        }
    }

    /// <summary>记录确定性失败；本地播放和其他输出不受影响。</summary>
    public DouyinLiveOperationResult Fail(string reason)
    {
        lock (_gate)
        {
            InvalidateGeneration();
            _state = DouyinLiveState.Failed;
            _queue.Clear();
            _error = NormalizeReason(reason, "抖音 M1 会话失败");
            RecordEvent("probe_failed");
            return Success();
        }
    }

    /// <summary>记录 sidecar 已观察到一条外部弹幕；正文仍只由 sidecar 保持在内存中。</summary>
    public DouyinLiveOperationResult MarkChatObserved() => SetObservedFlag(
        "chat_received",
        static state => state == DouyinLiveState.Listening,
        static manager => manager._chatReceived = true);

    /// <summary>记录 sidecar 已尝试发送一条回复。</summary>
    public DouyinLiveOperationResult MarkReplyAttempted() => SetObservedFlag(
        "reply_attempted",
        static state => state == DouyinLiveState.Listening,
        static manager => manager._replyAttempted = true);

    /// <summary>记录 sidecar 已过滤本账号回显。</summary>
    public DouyinLiveOperationResult MarkSelfEchoFiltered() => SetObservedFlag(
        "self_echo_filtered",
        static state => state is DouyinLiveState.Listening or DouyinLiveState.Paused,
        static manager => manager._selfEchoFiltered = true);

    /// <summary>停止当前会话的新回复发送；读取侧仍可继续观察和按容量丢弃任务。</summary>
    public DouyinLiveOperationResult BlockReplySending(string reason)
    {
        lock (_gate)
        {
            if (_state is not (DouyinLiveState.Listening or DouyinLiveState.Paused))
            {
                return Failure(InvalidTransition("block_reply_sending"));
            }

            if (_replySendingBlocked)
            {
                return Success();
            }

            _replySendingBlocked = true;
            _error = NormalizeReason(reason, "回复发送已暂停");
            RecordEvent("reply_sending_blocked");
            return Success();
        }
    }

    /// <summary>记录 sidecar 报告的有限数据缺口；不恢复缺失弹幕，也不触发自动重试。</summary>
    public DouyinLiveOperationResult RecordLiveGap(string? reason, ulong droppedCount)
    {
        lock (_gate)
        {
            if (_state is not (DouyinLiveState.RoomResolved or DouyinLiveState.Listening or DouyinLiveState.Paused)
                || !IsValidGapReason(reason)
                || droppedCount == 0)
            {
                return Failure(new DouyinLiveOperationError(
                    "douyin_gap_invalid",
                    "sidecar 数据缺口的原因或丢弃数量无效"));
            }

            _lastGapReason = reason;
            _metrics = _metrics with
            {
                GapEvents = SaturatingIncrement(_metrics.GapEvents),
                GapDroppedCount = SaturatingAdd(_metrics.GapDroppedCount, droppedCount)
            };
            RecordEvent("live_gap");
            return Success();
        }
    }

    /// <summary>记录 sidecar 已完成最小正向兼容验证。</summary>
    public DouyinLiveOperationResult MarkPassed()
    {
        lock (_gate)
        {
            if (_state is not (DouyinLiveState.Listening or DouyinLiveState.Paused))
            {
                return Failure(InvalidTransition("mark_passed"));
            }

            _state = DouyinLiveState.Passed;
            _replyAttempted = true;
            _selfEchoFiltered = true;
            RecordEvent("probe_passed");
            return Success();
        }
    }

    /// <summary>记录 sidecar 在超时前未获得完整证据；该状态不是成功。</summary>
    public DouyinLiveOperationResult MarkInconclusive(string reason)
    {
        lock (_gate)
        {
            if (!IsRunning())
            {
                return Failure(InvalidTransition("mark_inconclusive"));
            }

            InvalidateGeneration();
            _state = DouyinLiveState.Inconclusive;
            _queue.Clear();
            _error = NormalizeReason(reason, "抖音 M1 证据不足");
            RecordEvent("probe_inconclusive");
            return Success();
        }
    }

    /// <summary>
    /// 接收最小化弹幕 DTO。只接受监听状态和唯一消息 ID；本账号回显、重放和重复 ID 不入队。
    /// </summary>
    public DouyinEnqueueResult ObserveChatMessage(DouyinChatMessage? message, DateTimeOffset now)
    {
        lock (_gate)
        {
            if (_state != DouyinLiveState.Listening)
            {
                return EnqueueFailure(new DouyinLiveOperationError(
                    "douyin_invalid_transition",
                    "当前未处于抖音公屏监听状态"));
            }

            if (!_config.Enabled)
            {
                return EnqueueFailure(new DouyinLiveOperationError(
                    "douyin_auto_reply_disabled",
                    "抖音自动回应未启用"));
            }

            if (message is null || !DouyinLiveRules.IsValidMessageId(message.MessageId))
            {
                return EnqueueFailure(new DouyinLiveOperationError(
                    "douyin_message_invalid",
                    "弹幕消息 ID 无效"));
            }

            return ObserveChatLocked(message.MessageId, message.IsSelf, message.IsReplay, now);
        }
    }

    /// <summary>
    /// 接收 sidecar 的脱敏弹幕元数据。正文不进入 C#，但仍校验单房间、发送者和正文长度边界。
    /// </summary>
    public DouyinEnqueueResult ObserveChatMetadata(DouyinChatMessageMetadata? metadata, DateTimeOffset now)
    {
        lock (_gate)
        {
            if (_state != DouyinLiveState.Listening)
            {
                return EnqueueFailure(new DouyinLiveOperationError(
                    "douyin_invalid_transition",
                    "当前未处于抖音公屏监听状态"));
            }

            if (!_config.Enabled)
            {
                return EnqueueFailure(new DouyinLiveOperationError(
                    "douyin_auto_reply_disabled",
                    "抖音自动回应未启用"));
            }

            if (!IsValidChatMetadata(metadata))
            {
                return EnqueueFailure(new DouyinLiveOperationError(
                    "douyin_message_invalid",
                    "弹幕元数据无效"));
            }

            return ObserveChatLocked(metadata!.MessageId, metadata.IsSelf, metadata.IsReplay, now);
        }
    }

    private DouyinEnqueueResult ObserveChatLocked(
        string messageIdValue,
        bool isSelf,
        bool isReplay,
        DateTimeOffset now)
    {
        var messageId = messageIdValue.Trim();
        if (isSelf)
        {
            _selfEchoFiltered = true;
            _metrics = _metrics with { IgnoredSelf = SaturatingIncrement(_metrics.IgnoredSelf) };
            RecordEvent("self_echo_filtered");
            return Ignored(DouyinEnqueueDecision.IgnoredSelf);
        }

        if (isReplay)
        {
            _metrics = _metrics with { IgnoredReplay = SaturatingIncrement(_metrics.IgnoredReplay) };
            RecordEvent("replay_filtered");
            return Ignored(DouyinEnqueueDecision.IgnoredReplay);
        }

        if (!_seenMessageIds.Add(messageId))
        {
            _metrics = _metrics with { IgnoredDuplicate = SaturatingIncrement(_metrics.IgnoredDuplicate) };
            RecordEvent("duplicate_filtered");
            return Ignored(DouyinEnqueueDecision.IgnoredDuplicate);
        }

        _seenMessageOrder.Enqueue(messageId);
        if (_seenMessageOrder.Count > DouyinLiveRules.SeenMessageCapacity)
        {
            _seenMessageIds.Remove(_seenMessageOrder.Dequeue());
        }

        var replyIndex = SelectReplyIndex();
        var task = new DouyinReplyTask(
            _generation,
            messageId,
            _config.Replies[replyIndex],
            now,
            $"chat-{Guid.NewGuid():N}");
        if (_queue.Count >= _config.QueueCapacity)
        {
            _queue.Dequeue();
            _metrics = _metrics with { DroppedOldest = SaturatingIncrement(_metrics.DroppedOldest) };
        }

        _queue.Enqueue(task);
        _chatReceived = true;
        _metrics = _metrics with { Enqueued = SaturatingIncrement(_metrics.Enqueued) };
        RecordEvent("reply_selected");
        return new(true, DouyinEnqueueDecision.Enqueued, CreateStatus(), task);
    }

    private bool IsValidChatMetadata(DouyinChatMessageMetadata? metadata) =>
        metadata is not null
        && DouyinLiveRules.TryNormalizeRoomId(metadata.RoomId, out var roomId)
        && string.Equals(roomId, _config.RoomId, StringComparison.Ordinal)
        && DouyinLiveRules.IsValidMessageId(metadata.MessageId)
        && !string.IsNullOrWhiteSpace(metadata.SenderId)
        && System.Text.Encoding.UTF8.GetByteCount(metadata.SenderId.Trim()) <= DouyinLiveRules.MaxMessageIdBytes
        && metadata.TextLength is >= 0 and <= DouyinLiveRules.MaxChatTextLength;

    /// <summary>
    /// 取出最早未过期任务。等待超过 60 秒的任务丢弃；不负责向平台发送或自动重试。
    /// </summary>
    public bool TryDequeue(
        DateTimeOffset now,
        out DouyinReplyTask? task,
        ulong? expectedGeneration = null)
    {
        lock (_gate)
        {
            while (_queue.Count > 0)
            {
                var candidate = _queue.Dequeue();
                if (expectedGeneration is { } generation
                    && candidate.Generation != generation)
                {
                    continue;
                }

                if (now >= candidate.EnqueuedAtUtc
                    && now - candidate.EnqueuedAtUtc > DouyinLiveRules.TaskMaxAge)
                {
                    _metrics = _metrics with { DroppedExpired = SaturatingIncrement(_metrics.DroppedExpired) };
                    continue;
                }

                _metrics = _metrics with { Dequeued = SaturatingIncrement(_metrics.Dequeued) };
                task = candidate;
                return true;
            }

            task = null;
            return false;
        }
    }

    /// <summary>记录一次发送终态；outcome_unknown 不会重新入队。</summary>
    public DouyinLiveOperationResult RecordSendOutcome(DouyinSendOutcome outcome)
    {
        lock (_gate)
        {
            if (_state is not (DouyinLiveState.Listening or DouyinLiveState.Paused))
            {
                return Failure(InvalidTransition("record_send_outcome"));
            }

            _metrics = outcome switch
            {
                DouyinSendOutcome.Accepted => _metrics with { Accepted = SaturatingIncrement(_metrics.Accepted) },
                DouyinSendOutcome.NotSent => _metrics with { NotSent = SaturatingIncrement(_metrics.NotSent) },
                DouyinSendOutcome.Rejected => _metrics with { Rejected = SaturatingIncrement(_metrics.Rejected) },
                DouyinSendOutcome.OutcomeUnknown => _metrics with { OutcomeUnknown = SaturatingIncrement(_metrics.OutcomeUnknown) },
                _ => _metrics
            };
            if (outcome is not DouyinSendOutcome.NotSent)
            {
                _replyAttempted = true;
                RecordEvent("reply_attempted");
            }

            return Success();
        }
    }

    /// <summary>暂停或空闲时调整队列容量；缩容按规则丢弃最旧未发送任务。</summary>
    public DouyinLiveOperationResult SetQueueCapacity(int capacity)
    {
        lock (_gate)
        {
            if (capacity is < DouyinLiveRules.MinQueueCapacity or > DouyinLiveRules.MaxQueueCapacity)
            {
                return Failure(new DouyinLiveOperationError(
                    "douyin_queue_capacity_invalid",
                    "队列容量必须在 10～5000 之间"));
            }

            if (IsRunning() && _state != DouyinLiveState.Paused)
            {
                return Failure(new DouyinLiveOperationError(
                    "douyin_queue_capacity_locked",
                    "运行中的抖音会话必须暂停后才能修改队列容量"));
            }

            _config = _config with { QueueCapacity = capacity };
            while (_queue.Count > capacity)
            {
                _queue.Dequeue();
                _metrics = _metrics with { DroppedOldest = SaturatingIncrement(_metrics.DroppedOldest) };
            }

            return Success();
        }
    }

    private int SelectReplyIndex()
    {
        var count = _config.Replies.Length;
        var index = _random.Next(count);
        if (count > 1 && _lastReplyIndex == index)
        {
            index = (index + 1) % count;
        }

        _lastReplyIndex = index;
        return index;
    }

    private DouyinLiveOperationResult SetEvent(
        DouyinLiveState target,
        string eventName,
        Func<DouyinLiveState, bool> isValid)
    {
        lock (_gate)
        {
            if (!isValid(_state))
            {
                return Failure(InvalidTransition(eventName));
            }

            _state = target;
            RecordEvent(eventName);
            return Success();
        }
    }

    private DouyinLiveOperationResult SetObservedFlag(
        string eventName,
        Func<DouyinLiveState, bool> isValid,
        Action<DouyinLiveManager> apply)
    {
        lock (_gate)
        {
            if (!isValid(_state))
            {
                return Failure(InvalidTransition(eventName));
            }

            apply(this);
            RecordEvent(eventName);
            return Success();
        }
    }

    private DouyinLiveStatus CreateStatus() => new(
        IsRunning(),
        _state,
        _generation,
        _lastEvent,
        _eventHistory.ToImmutableArray(),
        _roomResolved,
        _chatReceived,
        _replyAttempted,
        _selfEchoFiltered,
        _queue.Count,
        _config.QueueCapacity,
        _metrics,
        _error,
        _replySendingBlocked,
        _lastGapReason);

    private bool IsRunning() => _state is not (
        DouyinLiveState.Idle
        or DouyinLiveState.Failed
        or DouyinLiveState.Inconclusive
        or DouyinLiveState.Passed);

    private DouyinLiveOperationResult Success() => DouyinLiveOperationResult.Success(CreateStatus());

    private DouyinLiveOperationResult Failure(DouyinLiveOperationError error) =>
        DouyinLiveOperationResult.Failure(CreateStatus(), error);

    private DouyinEnqueueResult EnqueueFailure(DouyinLiveOperationError error) =>
        new(false, DouyinEnqueueDecision.Enqueued, CreateStatus(), Error: error);

    private DouyinEnqueueResult Ignored(DouyinEnqueueDecision decision) =>
        new(false, decision, CreateStatus());

    private DouyinLiveOperationError InvalidTransition(string operation) => new(
        "douyin_invalid_transition",
        $"抖音 M1 状态 {_state} 不允许执行 {operation}");

    private void RecordEvent(string eventName)
    {
        _lastEvent = eventName;
        _eventHistory.Enqueue(eventName);
        while (_eventHistory.Count > EventHistoryCapacity)
        {
            _eventHistory.Dequeue();
        }
    }

    private void ClearSeenMessages()
    {
        _seenMessageIds.Clear();
        _seenMessageOrder.Clear();
    }

    private void InvalidateGeneration()
    {
        _generation = _generation == ulong.MaxValue ? 1 : _generation + 1;
    }

    private static ulong SaturatingIncrement(ulong value) => value == ulong.MaxValue ? value : value + 1;

    private static ulong SaturatingAdd(ulong value, ulong increment) =>
        ulong.MaxValue - value < increment ? ulong.MaxValue : value + increment;

    private static bool IsValidGapReason(string? reason) => reason is
        "sidecar_backpressure"
        or "reconnect"
        or "no_replay";

    private static string NormalizeReason(string? reason, string fallback)
    {
        var normalized = reason?.Trim();
        return string.IsNullOrWhiteSpace(normalized)
            ? fallback
            : normalized.Length <= 512 ? normalized : normalized[..512];
    }
}
