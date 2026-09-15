namespace GpAutoLive.Contracts;

/// <summary>仅在桌面内存展示的弹幕；不得写入日志或持久化存储。</summary>
public sealed record DouyinChatDisplayMessage(
    string MessageId,
    DateTimeOffset ReceivedAtUtc,
    string Nickname,
    string Text,
    bool IsSelf);
