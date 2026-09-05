using System.Collections.Immutable;
using GpAutoLive.Contracts;

namespace GpAutoLive.Core;

/// <summary>
/// GUI 使用的最小不可变状态投影。媒体管理器仍是媒体真实状态的所有者，
/// 后续阶段只通过快照更新本类型，不在 ViewModel 中推算成功。
/// </summary>
public sealed record AppState
{
    /// <summary>进程启动时可复用的空状态快照。</summary>
    public static AppState Initial { get; } = new();

    /// <summary>是否已通过客户端登录门禁。</summary>
    public bool IsAuthenticated { get; init; }

    /// <summary>当前播放状态。</summary>
    public PlaybackState PlaybackState { get; init; } = PlaybackState.Stopped;

    /// <summary>播放会话代次，用于隔离迟到响应。</summary>
    public ulong PlaybackGeneration { get; init; }

    /// <summary>媒体源修订号，用于隔离旧快照。</summary>
    public ulong SourceRevision { get; init; }

    /// <summary>当前媒体源循环次数。</summary>
    public ulong LoopIndex { get; init; }

    /// <summary>多项播放池完整回绕次数；单项循环与 LoopIndex 同步推进。</summary>
    public ulong PlaybackPoolCycle { get; init; }

    /// <summary>当前播放池的不可变副本。</summary>
    public ImmutableArray<SourceMediaDto> SourceMediaPool { get; init; } = [];

    /// <summary>当前播放池索引。</summary>
    public int SourceMediaIndex { get; init; }
}
