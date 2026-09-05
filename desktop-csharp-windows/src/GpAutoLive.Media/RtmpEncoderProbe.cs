using System.Collections.Immutable;
using GpAutoLive.Contracts;
using GpAutoLive.Core.Processes;

namespace GpAutoLive.Media;

/// <summary>Windows FFmpeg H.264 编码器探测的稳定错误分类。</summary>
public enum RtmpEncoderProbeFailureCode
{
    InvalidArguments,
    NotWindows,
    Cancelled,
    ProcessFailed,
    NoEncoder,
}

/// <summary>不包含路径、命令行或 FFmpeg 原文的编码器探测错误。</summary>
public sealed record RtmpEncoderProbeError(
    RtmpEncoderProbeFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>按候选顺序返回有限的编码器可用性结果。</summary>
public sealed record RtmpEncoderProbeCandidate(
    string Encoder,
    bool IsAvailable);

/// <summary>编码器探测脱敏快照。</summary>
public sealed record RtmpEncoderProbeSnapshot(
    ImmutableArray<RtmpEncoderProbeCandidate> Candidates,
    string? SelectedEncoder);

/// <summary>编码器探测操作结果。</summary>
public sealed record RtmpEncoderProbeResult(
    bool IsSuccess,
    RtmpEncoderProbeSnapshot Snapshot,
    RtmpEncoderProbeError? Error = null);

/// <summary>
/// 使用 FFmpeg 的一帧本地 lavfi 黑帧逐项探测 H.264 编码器。
/// 每项都有独立超时和固定输出上限，不访问 RTMP 网络，也不把 stderr 原文传播到 UI。
/// </summary>
public static class RtmpEncoderProbe
{
    private static readonly TimeSpan ProbeTimeout = TimeSpan.FromSeconds(5);
    private const int MaxOutputBytes = 16 * 1024;
    private const int MaxErrorBytes = 16 * 1024;

    public static async Task<RtmpEncoderProbeResult> ProbeAsync(
        string? ffmpegPath,
        string? preferredEncoder,
        IExternalProcessRunner runner,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(runner);

        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(
                RtmpEncoderProbeFailureCode.Cancelled,
                "RTMP 编码器探测已取消。",
                retryable: true);
        }

        if (!OperatingSystem.IsWindows())
        {
            return Failure(
                RtmpEncoderProbeFailureCode.NotWindows,
                "RTMP 编码器探测只支持 Windows。",
                retryable: false);
        }

        if (string.IsNullOrWhiteSpace(ffmpegPath)
            || !Path.IsPathFullyQualified(ffmpegPath)
            || ffmpegPath.Any(char.IsControl))
        {
            return Failure(
                RtmpEncoderProbeFailureCode.InvalidArguments,
                "FFmpeg 运行资源路径无效。",
                retryable: false);
        }

        if (!string.IsNullOrWhiteSpace(preferredEncoder)
            && !RtmpOutputRules.H264EncoderOrder.Contains(preferredEncoder, StringComparer.Ordinal))
        {
            return Failure(
                RtmpEncoderProbeFailureCode.InvalidArguments,
                "FFmpeg 视频编码器不受支持。",
                retryable: false);
        }

        var candidates = ImmutableArray.CreateBuilder<RtmpEncoderProbeCandidate>();
        string? selected = null;
        foreach (var encoder in RtmpOutputRules.EncoderAttemptOrder(preferredEncoder))
        {
            if (cancellationToken.IsCancellationRequested)
            {
                return Failure(
                    RtmpEncoderProbeFailureCode.Cancelled,
                    "RTMP 编码器探测已取消。",
                    retryable: true,
                    candidates: candidates.ToImmutable());
            }

            ExternalProcessResult processResult;
            try
            {
                processResult = await runner
                    .RunAsync(CreatePlan(ffmpegPath, encoder), cancellationToken)
                    .ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                return Failure(
                    RtmpEncoderProbeFailureCode.Cancelled,
                    "RTMP 编码器探测已取消。",
                    retryable: true,
                    candidates: candidates.ToImmutable());
            }
            catch (Exception)
            {
                processResult = new(
                    ExternalProcessRunStatus.StartFailed,
                    null,
                    string.Empty,
                    string.Empty);
            }

            var available = processResult.Status is ExternalProcessRunStatus.Completed
                && processResult.ExitCode is 0;
            candidates.Add(new(encoder, available));
            if (available)
            {
                selected = encoder;
                break;
            }
        }

        var snapshot = new RtmpEncoderProbeSnapshot(candidates.ToImmutable(), selected);
        return selected is null
            ? new(
                false,
                snapshot,
                new(
                    RtmpEncoderProbeFailureCode.NoEncoder,
                    "本机 FFmpeg 没有可用的 H.264 编码器。",
                    Retryable: false))
            : new(true, snapshot);
    }

    private static ExternalProcessPlan CreatePlan(string ffmpegPath, string encoder) =>
        new(
            ffmpegPath,
            [
                "-hide_banner",
                "-loglevel", "error",
                "-nostdin",
                "-f", "lavfi",
                "-i", "color=c=black:s=128x72:r=1",
                "-frames:v", "1",
                "-an",
                "-c:v", encoder,
                "-pix_fmt", "yuv420p",
                "-f", "null",
                "NUL",
            ],
            ProcessLaunchPolicy.HiddenNoShellProcessTree,
            ProbeTimeout,
            MaxOutputBytes,
            MaxErrorBytes);

    private static RtmpEncoderProbeResult Failure(
        RtmpEncoderProbeFailureCode code,
        string message,
        bool retryable,
        ImmutableArray<RtmpEncoderProbeCandidate> candidates = default) =>
        new(
            false,
            new(candidates.IsDefault ? ImmutableArray<RtmpEncoderProbeCandidate>.Empty : candidates, null),
            new(code, message, retryable));
}
