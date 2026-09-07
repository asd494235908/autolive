using System.Collections.Immutable;
using System.Text;
using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>抖音 M1 本地会话状态；凭据不在此合同中保存。</summary>
public enum DouyinLiveState
{
    /// <summary>未运行。</summary>
    Idle,
    /// <summary>等待扫码。</summary>
    WaitingQr,
    /// <summary>扫码登录已确认。</summary>
    LoggedIn,
    /// <summary>直播间已解析。</summary>
    RoomResolved,
    /// <summary>正在监听公屏。</summary>
    Listening,
    /// <summary>已暂停，允许修改本地配置。</summary>
    Paused,
    /// <summary>正在停止。</summary>
    Stopping,
    /// <summary>本轮发生确定性失败。</summary>
    Failed,
    /// <summary>本轮在时间预算内没有收集到完整正向证据。</summary>
    Inconclusive,
    /// <summary>兼容探针已完成正向验证。</summary>
    Passed
}

/// <summary>M1 发送结果；outcome_unknown 不自动重试。</summary>
public enum DouyinSendOutcome
{
    /// <summary>平台明确接受。</summary>
    Accepted,
    /// <summary>未发送，例如任务过期或主动取消。</summary>
    NotSent,
    /// <summary>平台或输入明确拒绝。</summary>
    Rejected,
    /// <summary>结果未知，不重试避免重复发送。</summary>
    OutcomeUnknown
}

/// <summary>抖音本地回复池和有界发送队列配置。</summary>
public sealed record DouyinLiveConfig
{
    /// <summary>是否允许自动回应；默认关闭。</summary>
    [property: JsonPropertyName("enabled")]
    public bool Enabled { get; init; }

    /// <summary>直播间号或标准 live.douyin.com URL。</summary>
    [property: JsonPropertyName("room_id")]
    public string RoomId { get; init; } = string.Empty;

    /// <summary>本地回复候选，正文不进入 Go 或模型。</summary>
    [property: JsonPropertyName("replies")]
    public ImmutableArray<string> Replies { get; init; } = ["GpAutoLive探针✅"];

    /// <summary>串行发送队列容量。</summary>
    [property: JsonPropertyName("queue_capacity")]
    public int QueueCapacity { get; init; } = DouyinLiveRules.DefaultQueueCapacity;

    /// <summary>安全的默认配置。</summary>
    public static DouyinLiveConfig Default { get; } = new();
}

/// <summary>抖音本地配置校验失败类别。</summary>
public enum DouyinLiveConfigFailureCode
{
    /// <summary>直播间号无效。</summary>
    RoomIdInvalid,
    /// <summary>回复池数量无效。</summary>
    ReplyCountOutOfRange,
    /// <summary>回复正文无效。</summary>
    ReplyInvalid,
    /// <summary>队列容量无效。</summary>
    QueueCapacityOutOfRange
}

/// <summary>抖音配置校验的脱敏错误。</summary>
public sealed record DouyinLiveConfigError(DouyinLiveConfigFailureCode Code, string Message);

/// <summary>M1 固定数量、字符、队列和过期边界。</summary>
public static class DouyinLiveRules
{
    /// <summary>回复池最小数量。</summary>
    public const int MinReplyCount = 1;
    /// <summary>回复池最大数量。</summary>
    public const int MaxReplyCount = 100;
    /// <summary>单条回复最大 Unicode 字符数。</summary>
    public const int MaxReplyCharacters = 80;
    /// <summary>单条回复最大 UTF-8 字节数。</summary>
    public const int MaxReplyBytes = 320;
    /// <summary>队列最小容量。</summary>
    public const int MinQueueCapacity = 10;
    /// <summary>队列默认容量。</summary>
    public const int DefaultQueueCapacity = 500;
    /// <summary>队列最大容量。</summary>
    public const int MaxQueueCapacity = 5_000;
    /// <summary>任务最大等待时间。</summary>
    public static readonly TimeSpan TaskMaxAge = TimeSpan.FromSeconds(60);
    /// <summary>消息 ID 最大字节数。</summary>
    public const int MaxMessageIdBytes = 256;
    /// <summary>sidecar 允许报告的弹幕正文长度上限；只传长度，不传正文。</summary>
    public const int MaxChatTextLength = 16 * 1024;
    /// <summary>消息去重集合容量。</summary>
    public const int SeenMessageCapacity = 4_096;

    /// <summary>校验并规范化本地抖音配置。</summary>
    public static bool TryNormalize(
        DouyinLiveConfig? config,
        out DouyinLiveConfig? normalized,
        out DouyinLiveConfigError? error)
    {
        normalized = null;
        error = null;
        if (config is null)
        {
            error = Invalid(DouyinLiveConfigFailureCode.RoomIdInvalid, "抖音配置不能为空。");
            return false;
        }

        if (!TryNormalizeRoomId(config.RoomId, out var roomId))
        {
            error = Invalid(DouyinLiveConfigFailureCode.RoomIdInvalid, "直播间号只接受 1～20 位数字或标准 live.douyin.com URL。");
            return false;
        }

        if (config.Replies.IsDefaultOrEmpty
            || config.Replies.Length is < MinReplyCount or > MaxReplyCount)
        {
            error = Invalid(DouyinLiveConfigFailureCode.ReplyCountOutOfRange, "回复候选必须为 1～100 条。");
            return false;
        }

        var uniqueReplies = new List<string>(config.Replies.Length);
        foreach (var reply in config.Replies)
        {
            var text = reply?.Trim();
            if (string.IsNullOrEmpty(text)
                || text.EnumerateRunes().Count() > MaxReplyCharacters
                || Encoding.UTF8.GetByteCount(text) > MaxReplyBytes
                || text.Any(char.IsControl))
            {
                error = Invalid(DouyinLiveConfigFailureCode.ReplyInvalid, "每条回复必须是 1～80 个可打印 Unicode 字符且不超过 320 UTF-8 字节。");
                return false;
            }

            if (!uniqueReplies.Contains(text, StringComparer.Ordinal))
            {
                uniqueReplies.Add(text);
            }
        }

        if (config.QueueCapacity is < MinQueueCapacity or > MaxQueueCapacity)
        {
            error = Invalid(DouyinLiveConfigFailureCode.QueueCapacityOutOfRange, "队列容量必须在 10～5000 之间。");
            return false;
        }

        normalized = config with
        {
            RoomId = roomId,
            Replies = uniqueReplies.ToImmutableArray()
        };
        return true;
    }

    /// <summary>规范化数字直播间号或标准 HTTPS URL。</summary>
    public static bool TryNormalizeRoomId(string? value, out string roomId)
    {
        roomId = string.Empty;
        var input = value?.Trim() ?? string.Empty;
        const string prefix = "https://live.douyin.com/";
        if (input.StartsWith(prefix, StringComparison.Ordinal))
        {
            input = input[prefix.Length..];
        }

        if (input.Length is < 1 or > 20 || !input.All(static character => character is >= '0' and <= '9'))
        {
            return false;
        }

        roomId = input;
        return true;
    }

    /// <summary>校验消息 ID 是否适合进入有限去重集合。</summary>
    public static bool IsValidMessageId(string? messageId) =>
        !string.IsNullOrWhiteSpace(messageId)
        && Encoding.UTF8.GetByteCount(messageId.Trim()) <= MaxMessageIdBytes;

    private static DouyinLiveConfigError Invalid(DouyinLiveConfigFailureCode code, string message) => new(code, message);
}

/// <summary>从 WebcastChatMessage 提取的最小脱敏弹幕 DTO。</summary>
public sealed record DouyinChatMessage(
    string MessageId,
    string SenderId,
    string Text,
    bool IsSelf,
    bool IsReplay,
    DateTimeOffset ReceivedAtUtc);

/// <summary>sidecar 转发给桌面端的最小弹幕元数据；不携带弹幕正文。</summary>
public sealed record DouyinChatMessageMetadata(
    string RoomId,
    string MessageId,
    string SenderId,
    int TextLength,
    bool IsSelf,
    bool IsReplay);

/// <summary>已选定回复的有界发送任务。</summary>
public sealed record DouyinReplyTask(
    ulong Generation,
    string MessageId,
    string ReplyText,
    DateTimeOffset EnqueuedAtUtc,
    string ClientActionId);

/// <summary>本地队列统计，不包含弹幕正文或凭据。</summary>
public sealed record DouyinQueueMetrics(
    ulong Enqueued = 0,
    ulong Dequeued = 0,
    ulong DroppedOldest = 0,
    ulong DroppedExpired = 0,
    ulong IgnoredSelf = 0,
    ulong IgnoredReplay = 0,
    ulong IgnoredDuplicate = 0,
    ulong Accepted = 0,
    ulong NotSent = 0,
    ulong Rejected = 0,
    ulong OutcomeUnknown = 0,
    ulong GapEvents = 0,
    ulong GapDroppedCount = 0);

/// <summary>抖音 M1 脱敏状态快照。</summary>
public sealed record DouyinLiveStatus(
    bool Running,
    DouyinLiveState State,
    ulong Generation,
    string? LastEvent,
    ImmutableArray<string> EventHistory,
    bool RoomResolved,
    bool ChatReceived,
    bool ReplyAttempted,
    bool SelfEchoFiltered,
    int QueueCount,
    int QueueCapacity,
    DouyinQueueMetrics Metrics,
    string? Error,
    bool ReplySendingBlocked = false,
    string? LastGapReason = null);
