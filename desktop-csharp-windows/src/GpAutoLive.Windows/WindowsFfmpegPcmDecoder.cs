using System.Buffers;
using System.Buffers.Binary;
using System.Diagnostics;
using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>Windows FFmpeg PCM 解码器的稳定错误分类。</summary>
public enum WindowsFfmpegPcmDecoderFailureCode
{
    NotWindows,
    InvalidPlan,
    AlreadyRunning,
    StartFailed,
    ProcessFailed,
    OutputReadFailed,
    OutputOverflow,
    Cancelled,
    TimedOut,
    DestinationClosed,
    Closed,
}

/// <summary>不包含媒体路径、命令或 FFmpeg 原文的解码状态。</summary>
public sealed record WindowsFfmpegPcmDecoderSnapshot(
    bool IsRunning,
    int? ProcessId,
    ulong DecodedFrames,
    ulong DroppedFrames,
    WindowsFfmpegPcmDecoderFailureCode? ErrorCode,
    string? Error);

/// <summary>PCM 解码运行结果。</summary>
public sealed record WindowsFfmpegPcmDecoderError(
    WindowsFfmpegPcmDecoderFailureCode Code,
    string Message,
    bool Retryable = false);

public sealed record WindowsFfmpegPcmDecoderResult(
    bool IsSuccess,
    WindowsFfmpegPcmDecoderSnapshot Snapshot,
    WindowsFfmpegPcmDecoderError? Error = null);

/// <summary>
/// 受管 FFmpeg f32le 解码器。输出进入固定容量 PCM 环缓或最终 PCM 总线，不把完整音轨载入内存。
/// </summary>
public sealed class WindowsFfmpegPcmDecoder : IAsyncDisposable
{
    private const int ReadBufferBytes = 64 * 1024;
    private const int MaxStderrBytes = 16 * 1024;
    private static readonly TimeSpan CleanupTimeout = TimeSpan.FromSeconds(2);
    private readonly object _gate = new();
    private readonly SemaphoreSlim _lifecycle = new(1, 1);
    private Process? _process;
    private WindowsJobObject? _job;
    private CancellationTokenSource? _activeCancellation;
    private bool _disposed;
    private ulong _decodedFrames;
    private ulong _droppedFrames;
    private WindowsFfmpegPcmDecoderError? _lastError;

    public WindowsFfmpegPcmDecoderSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshotUnsafe();
            }
        }
    }

    /// <summary>启动一次有限 PCM 解码；调用方负责保存并等待返回任务。</summary>
    public async Task<WindowsFfmpegPcmDecoderResult> DecodeAsync(
        FfmpegPcmDecodePlan? plan,
        AudioPcmRingBuffer? destination,
        CancellationToken cancellationToken = default,
        Func<CancellationToken, ValueTask>? pauseWaiter = null,
        FinalPcmBus? finalPcmBus = null,
        Func<AudioPcmMixPolicy>? baseMixPolicyProvider = null,
        bool finalPcmOverlay = false,
        Func<int, CancellationToken, ValueTask>? beforePublishWaiter = null)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(WindowsFfmpegPcmDecoderFailureCode.Cancelled, "PCM 解码已取消。", retryable: true);
        }

        lock (_gate)
        {
            if (_disposed)
            {
                return Failure(WindowsFfmpegPcmDecoderFailureCode.Closed, "PCM 解码器已关闭。", retryable: false);
            }
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Failure(WindowsFfmpegPcmDecoderFailureCode.Closed, "PCM 解码器已关闭。", retryable: false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsFfmpegPcmDecoderFailureCode.Cancelled, "PCM 解码已取消。", retryable: true);
        }

        try
        {
            if (_disposed)
            {
                return Failure(WindowsFfmpegPcmDecoderFailureCode.Closed, "PCM 解码器已关闭。", retryable: false);
            }

            if (!OperatingSystem.IsWindows())
            {
                return Failure(WindowsFfmpegPcmDecoderFailureCode.NotWindows, "PCM 解码器只支持 Windows。", retryable: false);
            }

            if (!TryValidatePlan(plan, destination, finalPcmBus, finalPcmOverlay))
            {
                return Failure(WindowsFfmpegPcmDecoderFailureCode.InvalidPlan, "PCM 解码启动计划无效。", retryable: false);
            }

            var validatedPlan = plan!;
            var validatedDestination = destination;

            lock (_gate)
            {
                if (_process is not null)
                {
                    return Failure(WindowsFfmpegPcmDecoderFailureCode.AlreadyRunning, "PCM 解码已经在运行。", retryable: false);
                }

                _decodedFrames = 0;
                _droppedFrames = 0;
                _lastError = null;
            }

            using var process = new Process
            {
                StartInfo = CreateStartInfo(validatedPlan),
                EnableRaisingEvents = false,
            };
            WindowsJobObject? job = null;
            try
            {
                if (!process.Start())
                {
                    return Failure(WindowsFfmpegPcmDecoderFailureCode.StartFailed, "FFmpeg PCM 解码进程无法启动。", retryable: true);
                }

                if (WindowsJobObject.TryCreate(out var candidateJob)
                    && candidateJob is not null
                    && candidateJob.TryAssign(process))
                {
                    job = candidateJob;
                }
                else
                {
                    candidateJob?.Dispose();
                }
            }
            catch (Exception exception) when (IsStartFailure(exception))
            {
                return Failure(WindowsFfmpegPcmDecoderFailureCode.StartFailed, "FFmpeg PCM 解码进程无法启动。", retryable: true);
            }

            using var runCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            runCancellation.CancelAfter(validatedPlan.Timeout);
            lock (_gate)
            {
                _process = process;
                _job = job;
                _activeCancellation = runCancellation;
            }

            var stdoutTask = ReadPcmAsync(
                process.StandardOutput.BaseStream,
                validatedDestination,
                finalPcmBus,
                validatedPlan,
                runCancellation.Token,
                pauseWaiter,
                baseMixPolicyProvider,
                finalPcmOverlay,
                beforePublishWaiter);
            var stderrTask = DrainStderrAsync(process.StandardError.BaseStream, runCancellation.Token);
            var processExited = false;
            var mustTerminate = false;
            try
            {
                await process.WaitForExitAsync(runCancellation.Token).ConfigureAwait(false);
                processExited = true;
                runCancellation.CancelAfter(Timeout.InfiniteTimeSpan);
            }
            catch (OperationCanceledException) when (runCancellation.IsCancellationRequested)
            {
                mustTerminate = true;
            }

            if (mustTerminate || !processExited)
            {
                KillProcessTree(process, job);
                await WaitForExitBoundedAsync(process).ConfigureAwait(false);
            }

            var stdoutResult = await JoinTaskAsync(stdoutTask).ConfigureAwait(false);
            var stderrOk = await JoinTaskAsync(stderrTask).ConfigureAwait(false);
            var exitCode = SafeExitCode(process);
            if (cancellationToken.IsCancellationRequested)
            {
                return Failure(WindowsFfmpegPcmDecoderFailureCode.Cancelled, "PCM 解码已取消。", retryable: true);
            }

            if (runCancellation.IsCancellationRequested && !processExited)
            {
                return Failure(WindowsFfmpegPcmDecoderFailureCode.TimedOut, "PCM 解码超过时间预算。", retryable: true);
            }

            if (!stdoutResult || !stderrOk)
            {
                return Failure(WindowsFfmpegPcmDecoderFailureCode.OutputReadFailed, "PCM 解码输出读取失败。", retryable: true);
            }

            if ((validatedDestination?.Snapshot.IsClosed ?? false)
                || (finalPcmBus?.Snapshot.IsClosed ?? false))
            {
                return Failure(WindowsFfmpegPcmDecoderFailureCode.DestinationClosed, "PCM 解码目标已关闭。", retryable: false);
            }

            return exitCode is 0
                ? Success()
                : Failure(WindowsFfmpegPcmDecoderFailureCode.ProcessFailed, "FFmpeg PCM 解码失败。", retryable: true);
        }
        finally
        {
            Process? process;
            WindowsJobObject? job;
            lock (_gate)
            {
                process = _process;
                job = _job;
                _process = null;
                _job = null;
                _activeCancellation = null;
            }

            if (process is not null)
            {
                process.Dispose();
            }

            job?.Dispose();
            _lifecycle.Release();
        }
    }

    /// <summary>请求当前 FFmpeg 进程停止；不会等待调用线程上的解码任务。</summary>
    public void Stop()
    {
        Process? process;
        WindowsJobObject? job;
        lock (_gate)
        {
            _activeCancellation?.Cancel();
            process = _process;
            job = _job;
        }

        if (process is not null)
        {
            KillProcessTree(process, job);
        }
    }

    public async ValueTask DisposeAsync()
    {
        lock (_gate)
        {
            _disposed = true;
        }

        Stop();
        try
        {
            await _lifecycle.WaitAsync(CleanupTimeout).ConfigureAwait(false);
            _lifecycle.Release();
        }
        catch (TimeoutException)
        {
            // DecodeAsync 的所有进程/句柄仍由其 finally 有界回收；不阻塞应用退出。
        }

        _lifecycle.Dispose();
        GC.SuppressFinalize(this);
    }

    private async Task<bool> ReadPcmAsync(
        Stream stdout,
        AudioPcmRingBuffer? destination,
        FinalPcmBus? finalPcmBus,
        FfmpegPcmDecodePlan plan,
        CancellationToken cancellationToken,
        Func<CancellationToken, ValueTask>? pauseWaiter,
        Func<AudioPcmMixPolicy>? baseMixPolicyProvider,
        bool finalPcmOverlay,
        Func<int, CancellationToken, ValueTask>? beforePublishWaiter)
    {
        var byteBuffer = ArrayPool<byte>.Shared.Rent(ReadBufferBytes + 3);
        var sampleBuffer = ArrayPool<float>.Shared.Rent(4_096 * plan.Channels);
        var carryBytes = 0;
        var bufferedSamples = 0;
        try
        {
            while (true)
            {
                if (pauseWaiter is not null)
                {
                    await pauseWaiter(cancellationToken).ConfigureAwait(false);
                }

                var read = await stdout.ReadAsync(
                    byteBuffer.AsMemory(carryBytes, ReadBufferBytes),
                    cancellationToken).ConfigureAwait(false);
                if (read == 0)
                {
                    if (carryBytes % sizeof(float) != 0 || bufferedSamples % plan.Channels != 0)
                    {
                        return false;
                    }

                    if (bufferedSamples > 0)
                    {
                        if (!await WriteSamplesAsync(
                                destination,
                                finalPcmBus,
                                sampleBuffer,
                                bufferedSamples,
                                plan.Channels,
                                baseMixPolicyProvider,
                                finalPcmOverlay,
                                beforePublishWaiter,
                                cancellationToken)
                            .ConfigureAwait(false))
                        {
                            return false;
                        }
                    }

                    return true;
                }

                var totalBytes = carryBytes + read;
                var completeBytes = totalBytes - totalBytes % sizeof(float);
                var sampleCount = completeBytes / sizeof(float);
                for (var sampleIndex = 0; sampleIndex < sampleCount; sampleIndex++)
                {
                    var offset = sampleIndex * sizeof(float);
                    var bits = BinaryPrimitives.ReadInt32LittleEndian(byteBuffer.AsSpan(offset, sizeof(float)));
                    sampleBuffer[bufferedSamples++] = float.IsFinite(BitConverter.Int32BitsToSingle(bits))
                        ? BitConverter.Int32BitsToSingle(bits)
                        : 0F;
                    if (bufferedSamples == sampleBuffer.Length)
                    {
                        if (!await WriteSamplesAsync(
                                destination,
                                finalPcmBus,
                                sampleBuffer,
                                bufferedSamples,
                                plan.Channels,
                                baseMixPolicyProvider,
                                finalPcmOverlay,
                                beforePublishWaiter,
                                cancellationToken)
                            .ConfigureAwait(false))
                        {
                            return false;
                        }

                        bufferedSamples = 0;
                    }
                }

                carryBytes = totalBytes - completeBytes;
                if (carryBytes > 0)
                {
                    byteBuffer.AsSpan(completeBytes, carryBytes).CopyTo(byteBuffer);
                }
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return false;
        }
        catch (IOException)
        {
            return false;
        }
        finally
        {
            ArrayPool<byte>.Shared.Return(byteBuffer);
            ArrayPool<float>.Shared.Return(sampleBuffer);
        }
    }

    private async ValueTask<bool> WriteSamplesAsync(
        AudioPcmRingBuffer? destination,
        FinalPcmBus? finalPcmBus,
        float[] samples,
        int sampleCount,
        int channels,
        Func<AudioPcmMixPolicy>? baseMixPolicyProvider,
        bool finalPcmOverlay,
        Func<int, CancellationToken, ValueTask>? beforePublishWaiter,
        CancellationToken cancellationToken)
    {
        var completeSamples = sampleCount - sampleCount % channels;
        if (completeSamples == 0)
        {
            return true;
        }

        if (beforePublishWaiter is not null)
        {
            await beforePublishWaiter(completeSamples / channels, cancellationToken).ConfigureAwait(false);
        }

        if (!finalPcmOverlay && baseMixPolicyProvider is not null
            && !AudioPcmMixer.TryApplyBasePolicy(
                samples.AsSpan(0, completeSamples),
                channels,
                baseMixPolicyProvider(),
                out _))
        {
            lock (_gate)
            {
                _lastError = new(
                    WindowsFfmpegPcmDecoderFailureCode.OutputOverflow,
                    "PCM 优先级策略应用失败。",
                    Retryable: false);
            }

            return false;
        }

        if (finalPcmBus is not null)
        {
            var busDroppedBefore = finalPcmBus.Snapshot.OutputDroppedFrames;
            int framesPublished;
            FinalPcmBusError? busError;
            var publishSucceeded = finalPcmOverlay
                ? finalPcmBus.TryPublishOverlay(
                    samples.AsSpan(0, completeSamples),
                    out framesPublished,
                    out busError)
                : finalPcmBus.TryPublish(
                    samples.AsSpan(0, completeSamples),
                    out framesPublished,
                    out busError);
            if (!publishSucceeded)
            {
                lock (_gate)
                {
                    _lastError = new(
                        busError?.Code is FinalPcmBusFailureCode.Closed
                            ? WindowsFfmpegPcmDecoderFailureCode.DestinationClosed
                            : WindowsFfmpegPcmDecoderFailureCode.OutputOverflow,
                        busError?.Message ?? "PCM 解码输出无法写入最终总线。",
                        Retryable: busError?.Code is not FinalPcmBusFailureCode.Closed);
                }

                return false;
            }

            var busDroppedAfter = finalPcmBus.Snapshot.OutputDroppedFrames;
            AddCounter(ref _decodedFrames, (ulong)framesPublished);
            if (busDroppedAfter > busDroppedBefore)
            {
                AddCounter(ref _droppedFrames, busDroppedAfter - busDroppedBefore);
            }

            AddCounter(ref _droppedFrames, (ulong)Math.Max(0, completeSamples / channels - framesPublished));
            return true;
        }

        if (destination is null)
        {
            lock (_gate)
            {
                _lastError = new(
                    WindowsFfmpegPcmDecoderFailureCode.DestinationClosed,
                    "PCM 解码目标不可用。");
            }

            return false;
        }

        var droppedBefore = destination.Snapshot.DroppedFrames;
        if (!destination.TryWrite(samples.AsSpan(0, completeSamples), out var framesWritten, out var error))
        {
            lock (_gate)
            {
                _lastError = error?.Code is PcmRingBufferFailureCode.Closed
                    ? new(WindowsFfmpegPcmDecoderFailureCode.DestinationClosed, "PCM 解码目标已关闭。")
                    : new(WindowsFfmpegPcmDecoderFailureCode.OutputOverflow, "PCM 解码输出无法写入固定缓冲。", Retryable: true);
            }
            return false;
        }

        var frames = completeSamples / channels;
        AddCounter(ref _decodedFrames, (ulong)frames);
        var droppedAfter = destination.Snapshot.DroppedFrames;
        if (droppedAfter > droppedBefore)
        {
            AddCounter(ref _droppedFrames, droppedAfter - droppedBefore);
        }

        AddCounter(ref _droppedFrames, (ulong)Math.Max(0, frames - framesWritten));
        return true;
    }

    private static async Task<bool> DrainStderrAsync(Stream stderr, CancellationToken cancellationToken)
    {
        var buffer = ArrayPool<byte>.Shared.Rent(4_096);
        var observedBytes = 0;
        try
        {
            while (true)
            {
                var read = await stderr.ReadAsync(buffer.AsMemory(0, buffer.Length), cancellationToken).ConfigureAwait(false);
                if (read == 0)
                {
                    return true;
                }

                if (observedBytes < MaxStderrBytes)
                {
                    observedBytes = Math.Min(MaxStderrBytes, observedBytes + read);
                }

                // stderr 只保留固定上限的计数，不保留原文；达到上限后仍持续读取并丢弃，
                // 避免 FFmpeg 因管道反压停住。读取缓冲始终来自 ArrayPool，内存保持有界。
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return false;
        }
        catch (IOException)
        {
            return false;
        }
        finally
        {
            ArrayPool<byte>.Shared.Return(buffer);
        }
    }

    private static ProcessStartInfo CreateStartInfo(FfmpegPcmDecodePlan plan)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = plan.ExecutablePath,
            WorkingDirectory = Path.GetDirectoryName(plan.ExecutablePath)!,
            UseShellExecute = false,
            CreateNoWindow = true,
            WindowStyle = ProcessWindowStyle.Hidden,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };
        foreach (var argument in plan.Arguments)
        {
            startInfo.ArgumentList.Add(argument);
        }

        return startInfo;
    }

    private static bool TryValidatePlan(
        FfmpegPcmDecodePlan? plan,
        AudioPcmRingBuffer? destination,
        FinalPcmBus? finalPcmBus,
        bool finalPcmOverlay)
    {
        if (plan is null
            || (destination is null && finalPcmBus is null)
            || (destination is not null && finalPcmBus is not null)
            || (finalPcmOverlay && finalPcmBus is null)
            || string.IsNullOrWhiteSpace(plan.ExecutablePath)
            || !Path.IsPathFullyQualified(plan.ExecutablePath)
            || !File.Exists(plan.ExecutablePath)
            || plan.Arguments.IsDefaultOrEmpty
            || plan.Arguments.Length > FfmpegPcmDecodePlanBuilder.MaxArgumentCount
            || !File.Exists(plan.SourcePath)
            || (destination is not null && destination.Snapshot.Channels != plan.Channels)
            || (finalPcmBus is not null && finalPcmBus.Channels != plan.Channels)
            || plan.SampleRateHz is not (44_100 or 48_000)
            || plan.Channels is < 1 or > 2
            || plan.Timeout <= TimeSpan.Zero
            || plan.Timeout > TimeSpan.FromHours(1))
        {
            return false;
        }

        try
        {
            var commandLength = plan.ExecutablePath.Length;
            foreach (var argument in plan.Arguments)
            {
                if (argument.Any(char.IsControl)
                    || argument.Length > FfmpegPcmDecodePlanBuilder.MaxArgumentCharacters)
                {
                    return false;
                }

                commandLength = checked(commandLength + argument.Length + 1);
            }

            return commandLength <= FfmpegPcmDecodePlanBuilder.MaxCommandCharacters;
        }
        catch (ArgumentException)
        {
            return false;
        }
        catch (NotSupportedException)
        {
            return false;
        }
    }

    private static async Task<bool> JoinTaskAsync(Task<bool> task)
    {
        try
        {
            return await task.WaitAsync(CleanupTimeout).ConfigureAwait(false);
        }
        catch (TimeoutException)
        {
            return false;
        }
        catch (OperationCanceledException)
        {
            return false;
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

    private static async Task WaitForExitBoundedAsync(Process process)
    {
        try
        {
            await process.WaitForExitAsync().WaitAsync(CleanupTimeout).ConfigureAwait(false);
        }
        catch (InvalidOperationException)
        {
        }
        catch (TimeoutException)
        {
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

    private static bool IsStartFailure(Exception exception) =>
        exception is InvalidOperationException
            or System.ComponentModel.Win32Exception
            or ArgumentException
            or NotSupportedException
            or UnauthorizedAccessException
            or System.Security.SecurityException;

    private WindowsFfmpegPcmDecoderResult Success() =>
        new(true, Snapshot with { IsRunning = false, ProcessId = null });

    private WindowsFfmpegPcmDecoderResult Failure(
        WindowsFfmpegPcmDecoderFailureCode code,
        string message,
        bool retryable)
    {
        var error = new WindowsFfmpegPcmDecoderError(code, message, retryable);
        lock (_gate)
        {
            _lastError = error;
            return new(false, CreateSnapshotUnsafe() with { IsRunning = false, ProcessId = null }, error);
        }
    }

    private WindowsFfmpegPcmDecoderSnapshot CreateSnapshotUnsafe() =>
        new(
            _process is not null,
            SafeProcessId(_process),
            _decodedFrames,
            _droppedFrames,
            _lastError?.Code,
            _lastError?.Message);

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

    private static void AddCounter(ref ulong target, ulong value)
    {
        while (true)
        {
            var current = Volatile.Read(ref target);
            var next = ulong.MaxValue - current < value ? ulong.MaxValue : current + value;
            if (Interlocked.CompareExchange(ref target, next, current) == current)
            {
                return;
            }
        }
    }
}
