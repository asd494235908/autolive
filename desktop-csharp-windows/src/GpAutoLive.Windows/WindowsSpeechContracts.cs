using System.Collections.ObjectModel;

namespace GpAutoLive.Windows;

/// <summary>Windows 本地固定话术适配器的公开状态。</summary>
public enum WindowsSpeechAdapterState
{
    Idle,
    Starting,
    Playing,
    Completed,
    Cancelled,
    Failed,
    Closed
}

/// <summary>Windows 本地语音边界的稳定错误分类。</summary>
public enum WindowsSpeechFailureCode
{
    InvalidCommand,
    MicrophonePriority,
    AlreadyActive,
    StartupTimeout,
    Cancelled,
    VoiceUnavailable,
    SapiUnavailable,
    BridgeUnavailable,
    FinalPcmBusUnavailable,
    SpeechFailed,
    Closed
}

/// <summary>不包含 COM 异常正文、文件路径或语音令牌的错误。</summary>
public sealed record WindowsSpeechError(
    WindowsSpeechFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>
/// 本地 voice 的脱敏描述。Key 是内存中的稳定哈希，不是 SAPI token ID；token 不跨边界返回或持久化。
/// </summary>
public sealed record WindowsSpeechVoiceDescriptor(
    string Key,
    string Culture)
{
    /// <summary>不泄露系统注册表中的 voice 名称。</summary>
    public string DisplayName => "本地系统语音";
}

/// <summary>voice 目录读取结果。</summary>
public sealed record WindowsSpeechVoiceCatalogResult(
    bool IsAvailable,
    IReadOnlyList<WindowsSpeechVoiceDescriptor> Voices,
    WindowsSpeechError? Error = null)
{
    /// <summary>无 voice 时返回的不可变空集合。</summary>
    public static WindowsSpeechVoiceCatalogResult EmptyUnavailable(WindowsSpeechError error) =>
        new(false, Array.Empty<WindowsSpeechVoiceDescriptor>(), error);

    /// <summary>构造只读 voice 集合，避免调用方修改桥接器快照。</summary>
    public static WindowsSpeechVoiceCatalogResult Available(
        IEnumerable<WindowsSpeechVoiceDescriptor> voices) =>
        new(
            true,
            new ReadOnlyCollection<WindowsSpeechVoiceDescriptor>(
                voices.Distinct().ToArray()));
}

/// <summary>SAPI 操作启动结果。</summary>
public sealed record WindowsSpeechBridgeStartResult(
    bool IsSuccess,
    WindowsSpeechError? Error = null);

/// <summary>SAPI 操作终态。</summary>
public enum WindowsSpeechBridgeCompletionKind
{
    Completed,
    Cancelled,
    Failed
}

/// <summary>SAPI 操作完成通知；错误摘要必须已脱敏。</summary>
public sealed record WindowsSpeechBridgeCompletion(
    WindowsSpeechBridgeCompletionKind Kind,
    WindowsSpeechError? Error = null);

/// <summary>
/// 单个 Windows 语音操作。实现必须保持自己的 COM 资源所有权，并在 Cancel/Dispose 后有界收敛。
/// </summary>
public interface IWindowsSpeechOperation : IAsyncDisposable
{
    /// <summary>本地语音真正接受 Speak 后完成；启动失败也通过该任务返回。</summary>
    Task<WindowsSpeechBridgeStartResult> Started { get; }

    /// <summary>朗读完成、取消或失败的终态。</summary>
    Task<WindowsSpeechBridgeCompletion> Completion { get; }

    /// <summary>请求停止本次朗读；不得启动第二个操作。</summary>
    Task CancelAsync(CancellationToken cancellationToken = default);
}

/// <summary>Windows 本地语音桥接器，可由测试替身注入。</summary>
public interface IWindowsSpeechBridge : IAsyncDisposable
{
    /// <summary>读取已脱敏的本地 voice 目录。</summary>
    WindowsSpeechVoiceCatalogResult GetVoices();

    /// <summary>
    /// 创建单个本地朗读操作；实际启动由操作的 Started 任务确认。
    /// pcmSink 接收 48kHz、16-bit little-endian PCM；返回 false 表示最终 PCM 总线已关闭，
    /// 桥接器必须停止 SAPI，不得回退到系统默认音频设备。
    /// </summary>
    Task<IWindowsSpeechOperation> StartAsync(
        string text,
        string? voiceKey,
        int pcmChannels,
        Func<ReadOnlyMemory<byte>, bool>? pcmSink,
        CancellationToken cancellationToken = default);
}

/// <summary>适配器状态快照；不保存固定话术正文或 COM token。</summary>
public sealed record WindowsSpeechAdapterSnapshot(
    WindowsSpeechAdapterState State,
    string? OperationId,
    string? VoiceKey,
    string? Error)
{
    /// <summary>初始快照。</summary>
    public static WindowsSpeechAdapterSnapshot Initial { get; } =
        new(WindowsSpeechAdapterState.Idle, null, null, null);
}

/// <summary>Speak 命令立即返回的结果；成功时 Completion 提供该操作终态。</summary>
public sealed record WindowsSpeechOperationResult(
    bool IsAccepted,
    string? OperationId,
    WindowsSpeechAdapterSnapshot Snapshot,
    WindowsSpeechError? Error = null,
    Task<WindowsSpeechTerminalResult>? Completion = null);

/// <summary>固定话术操作的最终结果。</summary>
public sealed record WindowsSpeechTerminalResult(
    bool IsSuccess,
    string OperationId,
    WindowsSpeechAdapterSnapshot Snapshot,
    WindowsSpeechError? Error = null);
