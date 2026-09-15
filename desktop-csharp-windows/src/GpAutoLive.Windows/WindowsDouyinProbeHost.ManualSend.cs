using GpAutoLive.Contracts;

namespace GpAutoLive.Windows;

/// <summary>单条发送的提交结果；Accepted 不代表已观察到平台公屏回显。</summary>
public sealed record WindowsDouyinChatSendResult(DouyinSendOutcome Outcome, string Message);

public sealed partial class WindowsDouyinProbeHost
{
    /// <summary>向当前已认证且监听中的直播间手动发送一次；不入自动回复队列，不自动重试。</summary>
    public async Task<WindowsDouyinChatSendResult> SendChatAsync(
        string? text, CancellationToken cancellationToken = default)
    {
        if (!DouyinLiveRules.TryNormalizeReply(text, out var content))
        {
            return new(DouyinSendOutcome.NotSent, "正文须为 1～80 个可打印 Unicode 字符且不超过 320 UTF-8 字节。");
        }

        System.Diagnostics.Process? process;
        string sessionId;
        ulong generation;
        CancellationToken runToken;
        lock (_gate)
        {
            process = _process;
            if (!_authenticated || _disposed || process is null || _runCancellation is null
                || !TryGetCanonicalIdentity(out sessionId, out generation))
            {
                return new(DouyinSendOutcome.NotSent, "请先登录并连接直播间；暂停时请先恢复监听。");
            }
            runToken = _runCancellation.Token;
        }

        using var sendCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, runToken);
        var actionId = CreateRequestId("manual");
        var task = new DouyinReplyTask(generation, actionId, content, DateTimeOffset.UtcNow, actionId);
        var outcome = await SendReplyTaskAsync(process, sessionId, generation, task,
            sendCancellation.Token, manual: true).ConfigureAwait(false);
        if (_manager.Snapshot.Generation == generation)
        {
            _manager.RecordSendOutcome(outcome);
        }
        PublishSnapshot();
        return new(outcome, outcome switch
        {
            DouyinSendOutcome.Accepted => "平台已接受提交，请在弹幕列表核对本人回显。",
            DouyinSendOutcome.Rejected => "平台拒绝了本次发送。",
            DouyinSendOutcome.OutcomeUnknown => "发送结果未知，程序不会重试；请先核对直播间。",
            _ => "本条未发送：连接已变化、发送忙或触发限频，请检查状态后再试。"
        });
    }
}
