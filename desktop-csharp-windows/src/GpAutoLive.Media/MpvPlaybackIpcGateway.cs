using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Media;

/// <summary>
/// 将单一 mpv 会话的身份门禁与命名管道传输组合起来。
/// 该类型不启动 mpv，只拥有 request_id 分配、固定命令发送和响应归属校验。
/// </summary>
public sealed class MpvPlaybackIpcGateway : IAsyncDisposable
{
    private readonly MpvPlaybackSession _session;
    private readonly MpvNamedPipeClient _client;
    private long _nextRequestId;

    public MpvPlaybackIpcGateway(
        MpvPlaybackSession session,
        MpvNamedPipeClient client)
    {
        _session = session ?? throw new ArgumentNullException(nameof(session));
        _client = client ?? throw new ArgumentNullException(nameof(client));
    }

    public MpvPlaybackSession Session => _session;

    public MpvNamedPipeClient Client => _client;

    /// <summary>连接到已由受管 mpv 创建的命名管道。</summary>
    public Task<MpvIpcPipeConnectionResult> ConnectAsync(CancellationToken cancellationToken = default) =>
        _client.ConnectAsync(cancellationToken);

    /// <summary>
    /// 在当前活动源身份下发送一条固定命令，并拒绝切源期间到达的迟到响应。
    /// 失败结果只携带稳定的 IPC 或会话错误，不携带原始帧。
    /// </summary>
    public async Task<MpvIpcDispatchResult> DispatchAsync(
        MpvIpcCommand? command,
        MediaPlaybackIdentity expectedIdentity,
        CancellationToken cancellationToken = default)
    {
        if (command is null)
        {
            return MpvIpcDispatchResult.Failed(
                sessionError: new(
                    MpvSessionFailureCode.InvalidStateTransition,
                    "mpv IPC 命令不能为空。"));
        }

        var requestId = NextRequestId();
        if (requestId == 0)
        {
            return MpvIpcDispatchResult.Failed(
                sessionError: new(
                    MpvSessionFailureCode.InvalidPlaybackIdentity,
                    "mpv IPC request_id 已耗尽。"));
        }

        if (!_session.TryCreateRequest(
                requestId,
                command,
                expectedIdentity,
                out var request,
                out var sessionError)
            || request is null)
        {
            return MpvIpcDispatchResult.Failed(sessionError: sessionError ?? new(
                MpvSessionFailureCode.InvalidPlaybackIdentity,
                "mpv IPC 请求身份无效。"));
        }

        var ipcResult = await _client.ExecuteAsync(
            requestId,
            command,
            cancellationToken).ConfigureAwait(false);
        if (!ipcResult.IsSuccess || ipcResult.Frame is null)
        {
            return MpvIpcDispatchResult.Failed(request, ipcError: ipcResult.Error);
        }

        if (ipcResult.Frame.IsSuccess is not true)
        {
            return MpvIpcDispatchResult.Failed(
                request,
                ipcError: ipcResult.Frame.Error ?? new MpvIpcError(
                    MpvIpcFailureCode.CommandRejected,
                    "mpv 命令被拒绝。",
                    Retryable: false,
                    request.RequestId,
                    command.Kind));
        }

        if (!_session.AcceptResponse(request, ipcResult.Frame, out sessionError))
        {
            return MpvIpcDispatchResult.Failed(request, sessionError: sessionError ?? new(
                MpvSessionFailureCode.ResponseDoesNotBelongToSession,
                "mpv IPC 响应身份无效。"));
        }

        return MpvIpcDispatchResult.Succeeded(request, ipcResult.Frame);
    }

    public ValueTask DisposeAsync() => _client.DisposeAsync();

    private ulong NextRequestId()
    {
        var next = Interlocked.Increment(ref _nextRequestId);
        return next > 0 ? (ulong)next : 0;
    }
}

/// <summary>一次固定 mpv 命令的传输、会话身份和响应结果。</summary>
public sealed record MpvIpcDispatchResult(
    bool IsSuccess,
    MpvPlaybackRequest? Request,
    MpvIpcFrame? Frame,
    MpvIpcError? IpcError,
    MpvSessionError? SessionError)
{
    public static MpvIpcDispatchResult Succeeded(
        MpvPlaybackRequest request,
        MpvIpcFrame frame) =>
        new(true, request, frame, null, null);

    public static MpvIpcDispatchResult Failed(
        MpvPlaybackRequest? request = null,
        MpvIpcError? ipcError = null,
        MpvSessionError? sessionError = null) =>
        new(false, request, null, ipcError, sessionError);
}
