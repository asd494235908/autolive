using System.Collections.Immutable;

namespace GpAutoLive.Core.Processes;

/// <summary>
/// 外部进程的最小启动策略。该类型放在 Core，使 Media 只依赖进程合同，
/// Windows 程序集可以实现真正的本机适配器而不形成 Media ↔ Windows 循环引用。
/// </summary>
public sealed record ProcessLaunchPolicy(
    bool UseShellExecute,
    bool CreateNoWindow,
    bool RedirectStandardOutput,
    bool RedirectStandardError,
    bool KillProcessTreeOnTimeout,
    bool KillProcessTreeOnCancellation)
{
    /// <summary>
    /// FFprobe/FFmpeg 等受管媒体进程的默认安全边界：禁用 Shell、隐藏控制台、
    /// 重定向标准流，并在超时或取消时清理进程树。
    /// </summary>
    public static ProcessLaunchPolicy HiddenNoShellProcessTree { get; } = new(
        UseShellExecute: false,
        CreateNoWindow: true,
        RedirectStandardOutput: true,
        RedirectStandardError: true,
        KillProcessTreeOnTimeout: true,
        KillProcessTreeOnCancellation: true);
}

/// <summary>Windows 进程适配器消费的不可变执行计划。</summary>
public sealed record ExternalProcessPlan(
    string ExecutablePath,
    ImmutableArray<string> Arguments,
    ProcessLaunchPolicy LaunchPolicy,
    TimeSpan Timeout,
    int MaxStandardOutputBytes,
    int MaxStandardErrorBytes);

/// <summary>受管外部进程的有限执行结果；不携带命令行或路径错误正文。</summary>
public sealed record ExternalProcessResult(
    ExternalProcessRunStatus Status,
    int? ExitCode,
    string StandardOutput,
    string StandardError);

/// <summary>受管进程执行的稳定结果分类。</summary>
public enum ExternalProcessRunStatus
{
    /// <summary>进程正常启动并退出；退出码仍需由调用方判断。</summary>
    Completed,
    /// <summary>进程未能启动或计划不满足安全策略。</summary>
    StartFailed,
    /// <summary>超过计划时间预算，进程树已请求终止。</summary>
    TimedOut,
    /// <summary>调用方取消了执行，进程树已请求终止。</summary>
    Cancelled,
    /// <summary>标准输出超过计划上限，进程树已请求终止。</summary>
    StandardOutputLimitExceeded,
    /// <summary>标准错误超过计划上限，进程树已请求终止。</summary>
    StandardErrorLimitExceeded,
    /// <summary>读取进程输出失败，结果不能视为成功。</summary>
    OutputReadFailed
}

/// <summary>受管外部进程的唯一执行入口。</summary>
public interface IExternalProcessRunner
{
    /// <summary>异步执行一个有界外部进程计划。</summary>
    Task<ExternalProcessResult> RunAsync(
        ExternalProcessPlan plan,
        CancellationToken cancellationToken);
}
