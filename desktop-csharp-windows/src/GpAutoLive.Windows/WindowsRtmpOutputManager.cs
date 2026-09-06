using System.Buffers;
using System.Diagnostics;
using System.Globalization;
using System.Runtime.InteropServices;
using GpAutoLive.Contracts;
using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>Windows RTMP 进程宿主的稳定错误分类。</summary>
public enum WindowsRtmpFailureCode
{
    /// <summary>当前平台不是 Windows。</summary>
    NotWindows,
    /// <summary>FFmpeg 计划或运行资源无效。</summary>
    InvalidPlan,
    /// <summary>已有 RTMP 会话。</summary>
    AlreadyRunning,
    /// <summary>FFmpeg 进程无法启动。</summary>
    StartFailed,
    /// <summary>FFmpeg 启动后立即退出。</summary>
    ProcessExited,
    /// <summary>停止预算内未能回收进程。</summary>
    StopTimedOut,
    /// <summary>调用方取消了操作。</summary>
    Cancelled,
    /// <summary>当前没有可写入的 RTMP 会话。</summary>
    NotRunning,
    /// <summary>当前计划未启用最终 PCM 输入。</summary>
    AudioInputUnavailable,
    /// <summary>最终 PCM 写入失败。</summary>
    AudioInputFailed,
    /// <summary>宿主已关闭。</summary>
    Closed,
    /// <summary>有限重连次数已用尽。</summary>
    ReconnectExhausted,
}

/// <summary>不包含地址、路径、命令行或 FFmpeg 原文的 RTMP 宿主错误。</summary>
public sealed record WindowsRtmpError(
    WindowsRtmpFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>RTMP 宿主的脱敏进程快照。</summary>
public sealed record WindowsRtmpSnapshot(
    RtmpOutputState State,
    string? TargetUrl,
    string? Encoder,
    int? ProcessId,
    int? ExitCode,
    bool FinalPcmInputOpen,
    int RetryCount,
    string? ErrorCode,
    string? Error);

/// <summary>RTMP 宿主操作结果。</summary>
public sealed record WindowsRtmpResult(
    bool IsSuccess,
    WindowsRtmpSnapshot Snapshot,
    WindowsRtmpError? Error = null);

/// <summary>
/// Windows 专用 RTMP 长生命周期宿主。它只消费已校验的 FFmpeg 参数计划，
/// 通过 Job Object 优先、Process.Kill 兜底回收进程树；声音输入由最终 PCM 总线写入。
/// </summary>
public sealed class WindowsRtmpOutputManager : IAsyncDisposable
{
    private const int MaxProgressLineCharacters = 8 * 1024;
    private const int MaxPcmFloatsPerWrite = 48_000 * 2;
    private static readonly TimeSpan CleanupTimeout = TimeSpan.FromSeconds(2);
    private readonly object _gate = new();
    private readonly SemaphoreSlim _serial = new(1, 1);
    private readonly SemaphoreSlim _audioSerial = new(1, 1);
    private Process? _process;
    private WindowsJobObject? _job;
    private Stream? _standardInput;
    private Task? _stderrTask;
    private CancellationTokenSource? _stderrCancellation;
    private RtmpOutputState _state = RtmpOutputState.Idle;
    private string? _targetUrl;
    private string? _encoder;
    private int? _exitCode;
    private string? _errorCode;
    private string? _error;
    private bool _disposed;

    /// <summary>进程终态或停止完成后通知上层刷新脱敏状态。</summary>
    public event Action<WindowsRtmpSnapshot>? SnapshotChanged;

    /// <summary>读取当前脱敏状态。</summary>
    public WindowsRtmpSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return new(
                    _state,
                    _targetUrl,
                    _encoder,
                    SafeProcessId(_process),
                    _exitCode,
                    _standardInput is not null,
                    0,
                    _errorCode,
                    _error);
            }
        }
    }

    /// <summary>
    /// 构造并启动一条 RTMP FFmpeg 会话。当前方法不自动重试；调用方可在 Exited 后重新提交同一配置。
    /// </summary>
    public async Task<WindowsRtmpResult> StartAsync(
        RtmpOutputConfig? config,
        SourceMediaDto? source,
        string? ffmpegPath,
        string? preferredEncoder = null,
        RtmpSourceIdentity? sourceIdentity = null,
        MpvVideoEffectSnapshot? videoEffects = null,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(WindowsRtmpFailureCode.Cancelled, "RTMP 启动已取消。", retryable: true);
        }

        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsRtmpFailureCode.Cancelled, "RTMP 启动已取消。", retryable: true);
        }

        try
        {
            if (_disposed)
            {
                return Failure(WindowsRtmpFailureCode.Closed, "RTMP 输出宿主已关闭。", retryable: false);
            }

            if (!OperatingSystem.IsWindows())
            {
                return Failure(WindowsRtmpFailureCode.NotWindows, "RTMP 输出宿主只支持 Windows。", retryable: false);
            }

            if (!RtmpFfmpegCommandBuilder.TryCreate(
                    config,
                    source,
                    ffmpegPath,
                    preferredEncoder,
                    sourceIdentity,
                    out var plan,
                    out var commandError,
                    videoEffects)
                || plan is null)
            {
                return Failure(
                    WindowsRtmpFailureCode.InvalidPlan,
                    commandError?.Message ?? "RTMP 启动计划无效。",
                    retryable: false);
            }

            if (_process is not null && !HasExited(_process))
            {
                return Failure(WindowsRtmpFailureCode.AlreadyRunning, "RTMP 推流已经在运行。", retryable: false);
            }

            if (!ReleaseExitedProcess())
            {
                return Failure(
                    WindowsRtmpFailureCode.StopTimedOut,
                    "上一次 RTMP 会话的 PCM 写入尚未结束。",
                    retryable: true);
            }
            if (!File.Exists(plan.ProcessPlan.ExecutablePath))
            {
                return Failure(WindowsRtmpFailureCode.InvalidPlan, "FFmpeg 运行资源不可用。", retryable: false);
            }

            var process = new Process
            {
                StartInfo = CreateStartInfo(plan),
                // 进程启动和宿主字段初始化必须先完成，再接收 Exited；否则快速退出会
                // 在 _standardInput/_stderrTask 写入前清理宿主，随后启动路径又写回悬空资源。
                EnableRaisingEvents = false,
            };
            process.Exited += Process_Exited;
            lock (_gate)
            {
                _process = process;
                _job = null;
                _standardInput = null;
                _stderrTask = null;
                _stderrCancellation = null;
                _state = RtmpOutputState.Starting;
                _targetUrl = plan.RedactedTargetUrl;
                _encoder = string.IsNullOrEmpty(plan.Encoder) ? null : plan.Encoder;
                _exitCode = null;
                _errorCode = null;
                _error = null;
            }

            try
            {
                if (!process.Start())
                {
                    return await FailStartAsync(process).ConfigureAwait(false);
                }

                WindowsJobObject? candidateJob = null;
                if (WindowsJobObject.TryCreate(out var createdJob)
                    && createdJob is not null
                    && createdJob.TryAssign(process))
                {
                    candidateJob = createdJob;
                }
                else
                {
                    createdJob?.Dispose();
                }

                var stderrCancellation = new CancellationTokenSource();
                lock (_gate)
                {
                    _job = candidateJob;
                    _standardInput = plan.RequiresFinalPcmInput ? process.StandardInput.BaseStream : null;
                    _stderrCancellation = stderrCancellation;
                    _stderrTask = DrainStderrAsync(process.StandardError, stderrCancellation.Token);
                }

                process.EnableRaisingEvents = true;
                if (HasExited(process))
                {
                    Process_Exited(process, EventArgs.Empty);
                    return Failure(
                        WindowsRtmpFailureCode.ProcessExited,
                        "RTMP 进程启动后立即退出。",
                        retryable: true);
                }

                if (cancellationToken.IsCancellationRequested)
                {
                    await StopCoreAsync().ConfigureAwait(false);
                    return Failure(WindowsRtmpFailureCode.Cancelled, "RTMP 启动已取消。", retryable: true);
                }

                return Success();
            }
            catch (Exception exception) when (IsStartFailure(exception))
            {
                return await FailStartAsync(process).ConfigureAwait(false);
            }
        }
        finally
        {
            _serial.Release();
        }
    }

    /// <summary>向已启用声音轨道的会话写入一段交错 float PCM；写入量有界且不会扩容历史缓冲。</summary>
    public async Task<WindowsRtmpResult> WriteFinalPcmAsync(
        ReadOnlyMemory<float> interleavedPcm,
        CancellationToken cancellationToken = default)
    {
        if (interleavedPcm.IsEmpty || interleavedPcm.Length > MaxPcmFloatsPerWrite)
        {
            return Failure(WindowsRtmpFailureCode.AudioInputFailed, "最终 PCM 分片大小无效。", retryable: false);
        }

        Stream? input;
        lock (_gate)
        {
            if (_disposed)
            {
                return Failure(WindowsRtmpFailureCode.Closed, "RTMP 输出宿主已关闭。", retryable: false);
            }

            if (_state is not (RtmpOutputState.Starting or RtmpOutputState.Publishing))
            {
                return Failure(WindowsRtmpFailureCode.NotRunning, "RTMP 推流当前未运行。", retryable: true);
            }

            input = _standardInput;
            if (input is null)
            {
                return Failure(WindowsRtmpFailureCode.AudioInputUnavailable, "当前 RTMP 计划未启用最终 PCM 输入。", retryable: false);
            }
        }

        if (!BitConverter.IsLittleEndian)
        {
            return Failure(WindowsRtmpFailureCode.AudioInputFailed, "当前平台不支持 little-endian PCM。", retryable: false);
        }

        var enteredAudioSerial = false;
        try
        {
            await _audioSerial.WaitAsync(cancellationToken).ConfigureAwait(false);
            enteredAudioSerial = true;
            var byteCount = checked(interleavedPcm.Length * sizeof(float));
            var buffer = ArrayPool<byte>.Shared.Rent(byteCount);
            try
            {
                MemoryMarshal.AsBytes(interleavedPcm.Span).CopyTo(buffer.AsSpan(0, byteCount));
                await input.WriteAsync(buffer.AsMemory(0, byteCount), cancellationToken).ConfigureAwait(false);
            }
            finally
            {
                ArrayPool<byte>.Shared.Return(buffer);
            }

            return Success();
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsRtmpFailureCode.Cancelled, "最终 PCM 写入已取消。", retryable: true);
        }
        catch (IOException)
        {
            return Failure(WindowsRtmpFailureCode.AudioInputFailed, "最终 PCM 写入失败。", retryable: true);
        }
        catch (ObjectDisposedException)
        {
            return Failure(WindowsRtmpFailureCode.AudioInputFailed, "最终 PCM 输入已关闭。", retryable: true);
        }
        finally
        {
            if (enteredAudioSerial)
            {
                _audioSerial.Release();
            }
        }
    }

    /// <summary>停止并有界回收 FFmpeg、Job Object、PCM 输入和 stderr 读取任务。</summary>
    public async Task<WindowsRtmpResult> StopAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsRtmpFailureCode.Cancelled, "RTMP 停止已取消。", retryable: true);
        }

        try
        {
            if (_disposed && _process is null)
            {
                return Success();
            }

            return await StopCoreAsync().ConfigureAwait(false);
        }
        finally
        {
            _serial.Release();
        }
    }

    /// <summary>关闭宿主并 Join 当前进程资源；重复调用幂等。</summary>
    public async ValueTask DisposeAsync()
    {
        try
        {
            await _serial.WaitAsync().ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return;
        }

        try
        {
            if (_disposed && _process is null)
            {
                return;
            }

            _disposed = true;
            var stopped = await StopCoreAsync().ConfigureAwait(false);
            if (stopped.IsSuccess)
            {
                lock (_gate)
                {
                    _state = RtmpOutputState.Idle;
                }
            }
        }
        finally
        {
            _serial.Release();
        }

        GC.SuppressFinalize(this);
    }

    private static ProcessStartInfo CreateStartInfo(RtmpFfmpegLaunchPlan plan)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = plan.ProcessPlan.ExecutablePath,
            WorkingDirectory = Path.GetDirectoryName(plan.ProcessPlan.ExecutablePath) ?? AppContext.BaseDirectory,
            UseShellExecute = false,
            CreateNoWindow = true,
            WindowStyle = ProcessWindowStyle.Hidden,
            RedirectStandardInput = plan.RequiresFinalPcmInput,
            RedirectStandardOutput = false,
            RedirectStandardError = true,
        };

        foreach (var argument in plan.ProcessPlan.Arguments)
        {
            startInfo.ArgumentList.Add(argument);
        }

        return startInfo;
    }

    private async Task<WindowsRtmpResult> StopCoreAsync()
    {
        Process? process;
        WindowsJobObject? job;
        Stream? input;
        Task? stderrTask;
        CancellationTokenSource? stderrCancellation;
        var hasProcess = true;
        lock (_gate)
        {
            process = _process;
            job = _job;
            input = _standardInput;
            stderrTask = _stderrTask;
            stderrCancellation = _stderrCancellation;
            if (process is null)
            {
                _state = RtmpOutputState.Idle;
                hasProcess = false;
            }
            else
            {
                _state = RtmpOutputState.Stopping;
            }
        }

        if (!hasProcess)
        {
            PublishSnapshot();
            return Success();
        }

        var activeProcess = process!;

        stderrCancellation?.Cancel();
        var exited = HasExited(activeProcess);
        if (!exited)
        {
            KillProcessTree(activeProcess, job);
            exited = await WaitForExitBoundedAsync(activeProcess).ConfigureAwait(false);
        }

        if (!exited)
        {
            // 未确认进程退出前不能丢弃 PID、Job 或 stdin；保留 Stopping 状态，
            // 允许后续 StopAsync 重试，避免无 Job 兜底时留下孤儿 FFmpeg。
            return Failure(
                WindowsRtmpFailureCode.StopTimedOut,
                "RTMP 进程未能在停止预算内退出。",
                retryable: true);
        }

        var audioSerialAcquired = false;
        if (input is not null)
        {
            audioSerialAcquired = await _audioSerial.WaitAsync(CleanupTimeout).ConfigureAwait(false);
        }

        if (input is not null && !audioSerialAcquired)
        {
            // 不能关闭仍可能被写入的 stdin；保留资源给下一次 StopAsync 重试，避免
            // 画面进程、最终 PCM 泵和音频写入者出现半回收状态。
            return Failure(
                WindowsRtmpFailureCode.StopTimedOut,
                "RTMP PCM 写入未能在停止预算内结束。",
                retryable: true);
        }

        try
        {
            input?.Dispose();
            lock (_gate)
            {
                if (ReferenceEquals(_process, process))
                {
                    _standardInput = null;
                    _stderrTask = null;
                    _stderrCancellation = null;
                }
            }
        }
        catch (IOException)
        {
        }
        finally
        {
            if (audioSerialAcquired)
            {
                _audioSerial.Release();
            }
        }

        if (stderrTask is not null)
        {
            try
            {
                await stderrTask.WaitAsync(CleanupTimeout).ConfigureAwait(false);
            }
            catch (Exception)
            {
                // 读取任务只保存有界状态，不把 stderr 原文传播到 UI。
            }
        }

        stderrCancellation?.Dispose();
        var exitCode = SafeExitCode(activeProcess);
        activeProcess.Exited -= Process_Exited;
        activeProcess.Dispose();
        job?.Dispose();
        lock (_gate)
        {
            _process = null;
            _job = null;
            _exitCode = exitCode;
            _state = RtmpOutputState.Idle;
        }
        PublishSnapshot();

        return exited
            ? Success()
            : Failure(WindowsRtmpFailureCode.StopTimedOut, "RTMP 进程未能在停止预算内退出。", retryable: false);
    }

    private async Task<WindowsRtmpResult> FailStartAsync(Process process)
    {
        WindowsJobObject? job;
        lock (_gate)
        {
            job = _job;
            _state = RtmpOutputState.Failed;
        }

        KillProcessTree(process, job);
        var exited = await WaitForExitBoundedAsync(process).ConfigureAwait(false);
        if (!exited)
        {
            lock (_gate)
            {
                _errorCode = "rtmp_process_start_failed";
                _error = "RTMP 推流进程无法启动。";
            }

            return Failure(WindowsRtmpFailureCode.StartFailed, "RTMP 推流进程无法启动。", retryable: true);
        }
        var exitCode = SafeExitCode(process);
        process.Exited -= Process_Exited;
        process.Dispose();
        job?.Dispose();
        lock (_gate)
        {
            _process = null;
            _job = null;
            _exitCode = exitCode;
            _errorCode = "rtmp_process_start_failed";
            _error = "RTMP 推流进程无法启动。";
        }

        return Failure(WindowsRtmpFailureCode.StartFailed, "RTMP 推流进程无法启动。", retryable: true);
    }

    private async Task DrainStderrAsync(StreamReader reader, CancellationToken cancellationToken)
    {
        var buffer = new char[4_096];
        var line = new System.Text.StringBuilder(MaxProgressLineCharacters);
        var lineTruncated = false;
        try
        {
            while (true)
            {
                var read = await reader.ReadAsync(buffer.AsMemory(), cancellationToken).ConfigureAwait(false);
                if (read == 0)
                {
                    if (line.Length > 0 && !lineTruncated)
                    {
                        ObserveProgressLine(line.ToString());
                    }

                    return;
                }

                for (var index = 0; index < read; index++)
                {
                    var character = buffer[index];
                    if (character == '\n')
                    {
                        if (!lineTruncated)
                        {
                            ObserveProgressLine(line.ToString());
                        }

                        line.Clear();
                        lineTruncated = false;
                        continue;
                    }

                    if (character == '\r')
                    {
                        continue;
                    }

                    if (line.Length < MaxProgressLineCharacters)
                    {
                        line.Append(character);
                    }
                    else
                    {
                        lineTruncated = true;
                    }
                }
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch (IOException)
        {
            lock (_gate)
            {
                if (_state is not (RtmpOutputState.Stopping or RtmpOutputState.Idle))
                {
                    _errorCode = "rtmp_stderr_read_failed";
                    _error = "RTMP 状态读取失败。";
                }
            }
        }
        catch (ObjectDisposedException)
        {
        }
    }

    internal static bool IsProgressOutputAdvanced(string line)
    {
        var separator = line.IndexOf('=');
        if (separator <= 0 || separator == line.Length - 1)
        {
            return false;
        }

        var key = line[..separator].Trim();
        if (key is not ("out_time_ms" or "out_time_us" or "total_size"))
        {
            return false;
        }

        return ulong.TryParse(
                   line[(separator + 1)..].Trim(),
                   NumberStyles.None,
                   CultureInfo.InvariantCulture,
                   out var value)
            && value > 0;
    }

    private void ObserveProgressLine(string line)
    {
        if (!IsProgressOutputAdvanced(line))
        {
            return;
        }

        var stateChanged = false;
        lock (_gate)
        {
            if (_process is not null && _state == RtmpOutputState.Starting)
            {
                _state = RtmpOutputState.Publishing;
                stateChanged = true;
            }
        }

        if (stateChanged)
        {
            PublishSnapshot();
        }
    }

    private void Process_Exited(object? sender, EventArgs e)
    {
        if (sender is not Process process)
        {
            return;
        }

        var release = false;
        lock (_gate)
        {
            if (!ReferenceEquals(_process, process))
            {
                return;
            }

            _exitCode = SafeExitCode(process);
            if (_state is not (RtmpOutputState.Stopping or RtmpOutputState.Idle))
            {
                _state = RtmpOutputState.Failed;
                _errorCode = "rtmp_process_exited";
                _error = "RTMP 推流进程已退出。";
                release = true;
            }
        }

        if (release)
        {
            // 自然退出是失败终态：关闭 PCM stdin、取消 stderr 读取并清除 PID，
            // 让上层不会继续向已退出的画面/声音组合写入。
            if (ReleaseExitedProcess(preserveFailure: true))
            {
                PublishSnapshot();
            }
        }
    }

    private void PublishSnapshot()
    {
        var handler = SnapshotChanged;
        if (handler is null)
        {
            return;
        }

        try
        {
            handler(Snapshot);
        }
        catch (Exception)
        {
            // 状态通知不能反向破坏进程退出和资源回收路径。
        }
    }

    private bool ReleaseExitedProcess(bool preserveFailure = false)
    {
        Process? process;
        WindowsJobObject? job;
        Stream? input;
        Task? stderrTask;
        CancellationTokenSource? stderrCancellation;
        lock (_gate)
        {
            process = _process;
            job = _job;
            input = _standardInput;
            stderrTask = _stderrTask;
            stderrCancellation = _stderrCancellation;
            if (process is null || !HasExited(process))
            {
                return true;
            }
        }

        // Exited 事件可能与最后一段 PCM 写入并发；只有取得写入串行锁后才可
        // 关闭 stdin 和释放进程资源。
        var audioSerialEntered = input is null || _audioSerial.Wait(CleanupTimeout);
        if (!audioSerialEntered)
        {
            return false;
        }

        try
        {
            lock (_gate)
            {
                if (!ReferenceEquals(_process, process))
                {
                    return true;
                }

                _process = null;
                _job = null;
                _standardInput = null;
                _stderrTask = null;
                _stderrCancellation = null;
                _state = preserveFailure ? RtmpOutputState.Failed : RtmpOutputState.Idle;
            }

            stderrCancellation?.Cancel();
            WaitForTaskBounded(stderrTask);
            stderrCancellation?.Dispose();
            try
            {
                input?.Dispose();
            }
            catch (IOException)
            {
            }
            process.Exited -= Process_Exited;
            process.Dispose();
            job?.Dispose();
            return true;
        }
        finally
        {
            if (audioSerialEntered && input is not null)
            {
                _audioSerial.Release();
            }
        }
    }

    private static void WaitForTaskBounded(Task? task)
    {
        if (task is null)
        {
            return;
        }

        try
        {
            task.Wait(CleanupTimeout);
        }
        catch (AggregateException)
        {
            // 读取任务内部只记录脱敏状态；清理阶段无需向上传播其异常。
        }
    }

    private static async Task<bool> WaitForExitBoundedAsync(Process process)
    {
        try
        {
            await process.WaitForExitAsync().WaitAsync(CleanupTimeout).ConfigureAwait(false);
            return true;
        }
        catch (InvalidOperationException)
        {
            return true;
        }
        catch (TimeoutException)
        {
            return HasExited(process);
        }
    }

    private static void KillProcessTree(Process process, WindowsJobObject? job)
    {
        if (job is not null && job.TryTerminate())
        {
            return;
        }

        try
        {
            if (!process.HasExited)
            {
                process.Kill(entireProcessTree: true);
            }
        }
        catch (InvalidOperationException)
        {
        }
        catch (System.ComponentModel.Win32Exception)
        {
        }
        catch (PlatformNotSupportedException)
        {
        }
    }

    private static bool HasExited(Process process)
    {
        try
        {
            return process.HasExited;
        }
        catch (InvalidOperationException)
        {
            return true;
        }
        catch (System.ComponentModel.Win32Exception)
        {
            return true;
        }
    }

    private static int? SafeExitCode(Process process)
    {
        try
        {
            return process.HasExited ? process.ExitCode : null;
        }
        catch (InvalidOperationException)
        {
            return null;
        }
        catch (System.ComponentModel.Win32Exception)
        {
            return null;
        }
    }

    private static int? SafeProcessId(Process? process)
    {
        try
        {
            return process?.Id;
        }
        catch (InvalidOperationException)
        {
            return null;
        }
    }

    private WindowsRtmpResult Success() => new(true, Snapshot);

    private WindowsRtmpResult Failure(
        WindowsRtmpFailureCode code,
        string message,
        bool retryable) =>
        new(false, Snapshot, new(code, message, retryable));

    private static bool IsStartFailure(Exception exception) =>
        exception is InvalidOperationException
            or System.ComponentModel.Win32Exception
            or ArgumentException
            or NotSupportedException
            or UnauthorizedAccessException
            or IOException
            or System.Security.SecurityException;
}
