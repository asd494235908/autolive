using System.Collections.Immutable;
using GpAutoLive.Contracts;

namespace GpAutoLive.Windows;

/// <summary>由宿主锁保护的本轮弹幕记录；清屏不清去重状态。</summary>
internal sealed class WindowsDouyinChatBuffer
{
    private readonly Queue<DouyinChatDisplayMessage> _messages = new();
    private readonly HashSet<string> _seen = new(StringComparer.Ordinal);
    private readonly Queue<string> _seenOrder = new();
    private ulong _generation;
    private string? _sessionId;

    internal void Reset(ulong generation)
    {
        _generation = generation;
        _sessionId = null;
        _messages.Clear();
        _seen.Clear();
        _seenOrder.Clear();
    }

    internal void Add(DouyinChatDisplayMessage message, string sessionId, ulong generation)
    {
        if (generation != _generation || (_sessionId is not null && _sessionId != sessionId))
        {
            return;
        }
        _sessionId = sessionId;
        if (!_seen.Add(message.MessageId))
        {
            return;
        }
        _seenOrder.Enqueue(message.MessageId);
        if (_seenOrder.Count > DouyinLiveRules.SeenMessageCapacity)
        {
            _seen.Remove(_seenOrder.Dequeue());
        }
        _messages.Enqueue(message);
        if (_messages.Count > 500)
        {
            _messages.Dequeue();
        }
    }

    internal ImmutableArray<DouyinChatDisplayMessage> Snapshot() => [.. _messages];

    internal void Clear() => _messages.Clear();
}
